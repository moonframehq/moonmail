use {
    crate::{Archive, Error, Result, SearchError},
    moonmail_core::{MailPredicate, SearchIndex, TextTerm},
    rusqlite::{params, types::Value},
};

pub(crate) struct CompiledPredicate {
    pub sql: String,
    pub arguments: Vec<Value>,
    pub terms: Vec<TextTerm>,
}

pub(crate) fn compile(predicate: &MailPredicate) -> Result<CompiledPredicate> {
    let mut compiled = CompiledPredicate {
        sql: String::new(),
        arguments: Vec::new(),
        terms: Vec::new(),
    };
    let mut nodes = 0;
    compiled.sql = expression(predicate, &mut compiled, 0, &mut nodes)?;
    Ok(compiled)
}

fn expression(
    predicate: &MailPredicate,
    out: &mut CompiledPredicate,
    depth: usize,
    nodes: &mut usize,
) -> Result<String> {
    *nodes += 1;
    if depth > 16 || *nodes > 64 {
        return Err(Error::Invalid(
            "Search supports at most 64 conditions and 16 nesting levels",
        ));
    }
    let text = |value: &str| -> Result<()> {
        if value.trim().is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
            Err(Error::Invalid("Invalid search value"))
        } else {
            Ok(())
        }
    };
    Ok(match predicate {
        MailPredicate::Text(term) => {
            text(&term.value)?;
            out.arguments.push((out.terms.len() as i64).into());
            out.terms.push(term.clone());
            "m.id IN (SELECT id FROM search_terms WHERE term=?)".into()
        },
        MailPredicate::Account(value) => {
            text(value)?;
            out.arguments.push(value.clone().into());
            "m.account=?".into()
        },
        MailPredicate::Mailbox(value) => {
            text(value)?;
            out.arguments.push(value.clone().into());
            "EXISTS(SELECT 1 FROM occurrences o JOIN mailboxes b ON b.id=o.mailbox_id WHERE o.message_id=m.id AND b.name=?)".into()
        },
        MailPredicate::Unread => {
            "EXISTS(SELECT 1 FROM occurrences o WHERE o.message_id=m.id AND o.seen=0)".into()
        },
        MailPredicate::SenderAddress(value) | MailPredicate::RecipientAddress(value) => {
            text(value)?;
            out.arguments
                .push(crate::archive::canonical_address(value).into());
            format!(
                "EXISTS(SELECT 1 FROM addresses a WHERE a.message_id=m.id AND a.address=? AND {})",
                if matches!(predicate, MailPredicate::SenderAddress(_)) {
                    "a.role='from'"
                } else {
                    "a.role IN ('to','cc')"
                }
            )
        },
        MailPredicate::Keyword(value) => {
            let keyword = moonmail_core::canonical_keyword(value)
                .ok_or(Error::Invalid("Invalid IMAP keyword"))?;
            out.arguments.push(keyword.into());
            "EXISTS(SELECT 1 FROM occurrence_keywords k WHERE k.message_id=m.id AND k.keyword=?)"
                .into()
        },
        MailPredicate::Tag(value) => {
            text(value)?;
            out.arguments.push(value.clone().into());
            "EXISTS(SELECT 1 FROM occurrence_keywords k JOIN keyword_labels l ON l.account=m.account AND l.keyword=k.keyword WHERE k.message_id=m.id AND l.label=?)".into()
        },
        MailPredicate::Starred => {
            "EXISTS(SELECT 1 FROM occurrences o WHERE o.message_id=m.id AND o.flagged=1)".into()
        },
        MailPredicate::After(value) => {
            out.arguments.push((*value).into());
            "m.date>=?".into()
        },
        MailPredicate::Before(value) => {
            out.arguments.push((*value).into());
            "m.date<?".into()
        },
        MailPredicate::Not(child) => format!("NOT ({})", expression(child, out, depth + 1, nodes)?),
        MailPredicate::All(children) | MailPredicate::Any(children) => {
            if children.is_empty() || children.len() > 64 {
                return Err(Error::Invalid("Empty or oversized Boolean search group"));
            }
            let parts = children
                .iter()
                .map(|child| expression(child, out, depth + 1, nodes))
                .collect::<Result<Vec<_>>>()?;
            format!(
                "({})",
                parts.join(if matches!(predicate, MailPredicate::All(_)) {
                    " AND "
                } else {
                    " OR "
                })
            )
        },
    })
}

impl Archive {
    /// Compile live metadata predicates and atomically stage all indexed term
    /// matches from a single index snapshot. On failure the previous query and
    /// match set remain usable. Sorting/grouping/pagination happen afterward.
    pub fn prepare_predicate<I: SearchIndex>(
        &mut self,
        index: &I,
        predicate: &MailPredicate,
    ) -> std::result::Result<(), SearchError<I::Error>> {
        let compiled = compile(predicate)?;
        let transaction = self.connection.transaction().map_err(Error::from)?;
        transaction.execute_batch("CREATE TEMP TABLE IF NOT EXISTS search_terms(term INTEGER,id INTEGER,PRIMARY KEY(term,id)); DELETE FROM search_terms;").map_err(Error::from)?;
        let mut failure = None;
        {
            let mut insert = transaction
                .prepare("INSERT INTO search_terms VALUES (?1,?2)")
                .map_err(Error::from)?;
            index
                .visit_terms(&compiled.terms, |term, id| {
                    let result = i64::try_from(id.0)
                        .map_err(|_| Error::Invalid("Invalid search ID"))
                        .and_then(|id| {
                            insert
                                .execute(params![term as i64, id])
                                .map_err(Error::from)
                        });
                    match result {
                        Ok(_) => true,
                        Err(error) => {
                            failure = Some(error);
                            false
                        },
                    }
                })
                .map_err(SearchError::Index)?;
        }
        if let Some(error) = failure {
            return Err(error.into());
        }
        transaction.commit().map_err(Error::from)?;
        self.search_predicate = Some(compiled);
        self.search_prepared = true;
        Ok(())
    }
}
