use {
    crate::{Archive, Error, Result},
    moonmail_core::MailPredicate,
    rusqlite::{OptionalExtension, params, types::Value},
    serde::{Deserialize, Serialize},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartitionField {
    Sender,
    Recipient,
    Keyword,
}

impl PartitionField {
    pub fn name(self) -> &'static str {
        match self {
            Self::Sender => "sender",
            Self::Recipient => "recipient",
            Self::Keyword => "keyword",
        }
    }
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "sender" => Ok(Self::Sender),
            "recipient" => Ok(Self::Recipient),
            "keyword" => Ok(Self::Keyword),
            _ => Err(Error::Invalid("Unknown partition field")),
        }
    }
    pub fn predicate(self, value: &str) -> MailPredicate {
        match self {
            Self::Sender => MailPredicate::SenderAddress(value.into()),
            Self::Recipient => MailPredicate::RecipientAddress(value.into()),
            Self::Keyword => MailPredicate::Keyword(value.into()),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct PartitionBucket {
    pub value: String,
    pub total: u64,
    pub unread: u64,
}
#[derive(Debug, Serialize)]
pub struct PartitionPage {
    pub rows: Vec<PartitionBucket>,
    pub next: Option<String>,
}

impl Archive {
    /// Configuration follows the saved-search identity and cascades on deletion.
    /// Editing its predicate/sources keeps the chosen field and changes groups live.
    pub fn set_search_partition(&self, id: i64, field: Option<PartitionField>) -> Result<()> {
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM saved_searches WHERE id=?1)",
            [id],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(Error::NotFound);
        }
        if let Some(field) = field {
            self.connection.execute("INSERT INTO search_partitions VALUES (?1,?2) ON CONFLICT(search_id) DO UPDATE SET field=excluded.field",params![id,field.name()])?;
        } else {
            self.connection
                .execute("DELETE FROM search_partitions WHERE search_id=?1", [id])?;
        }
        Ok(())
    }
    pub fn search_partition(&self, id: i64) -> Result<Option<PartitionField>> {
        let field: Option<String> = self
            .connection
            .query_row(
                "SELECT field FROM search_partitions WHERE search_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        field.as_deref().map(PartitionField::parse).transpose()
    }

    /// Group the entire prepared predicate, not a navigation page. One SQL read
    /// snapshot counts distinct live messages; multivalued fields can put a message
    /// in multiple groups. Missing/empty values are omitted. Binary value ordering
    /// and exclusive keyset cursors cover every group without a first-100 cutoff.
    pub fn partition_page(
        &self,
        field: PartitionField,
        after: Option<&str>,
        limit: usize,
    ) -> Result<PartitionPage> {
        let compiled = self
            .search_predicate
            .as_ref()
            .filter(|_| self.search_prepared)
            .ok_or(Error::Invalid(
                "Prepare a saved predicate before partitioning",
            ))?;
        let limit = limit.clamp(1, 100);
        let projection = match field {
            PartitionField::Sender => {
                "SELECT a.message_id AS id,a.address AS value FROM addresses a WHERE a.role='from'"
            },
            PartitionField::Recipient => {
                "SELECT a.message_id AS id,a.address AS value FROM addresses a WHERE a.role IN ('to','cc')"
            },
            PartitionField::Keyword => {
                "SELECT message_id AS id,keyword AS value FROM occurrence_keywords"
            },
        };
        let sql=format!("WITH matched AS (SELECT m.id FROM messages m WHERE EXISTS(SELECT 1 FROM occurrences o WHERE o.message_id=m.id) AND ({})),
            values_by_message AS ({projection})
            SELECT p.value,COUNT(DISTINCT p.id),COUNT(DISTINCT CASE WHEN EXISTS(SELECT 1 FROM occurrences o WHERE o.message_id=p.id AND o.seen=0) THEN p.id END)
            FROM values_by_message p JOIN matched m ON m.id=p.id WHERE p.value>COALESCE(?, '') GROUP BY p.value ORDER BY p.value LIMIT ?",compiled.sql);
        let mut args = compiled.arguments.clone();
        args.push(
            after
                .map(|s| Value::from(s.to_owned()))
                .unwrap_or(Value::Null),
        );
        args.push(Value::from((limit + 1) as i64));
        let mut query = self.connection.prepare(&sql)?;
        let mut rows: Vec<PartitionBucket> = query
            .query_map(rusqlite::params_from_iter(args), |r| {
                Ok(PartitionBucket {
                    value: r.get(0)?,
                    total: r.get::<_, i64>(1)? as u64,
                    unread: r.get::<_, i64>(2)? as u64,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let more = rows.len() > limit;
        rows.truncate(limit);
        let next = if more {
            rows.last().map(|r| r.value.clone())
        } else {
            None
        };
        Ok(PartitionPage { rows, next })
    }
}
