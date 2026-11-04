use {
    crate::{Archive, Error, Result},
    moonmail_core::{
        MailDetail, MailPage, MailQuery, MailRepository, MailSummary, Mailbox, MessageId,
        PageCursor, SortField, ViewFilter,
    },
    rusqlite::{OptionalExtension, params, params_from_iter, types::Value},
    time::{OffsetDateTime, format_description::well_known::Rfc2822},
};

const SUMMARY: &str = "SELECT m.id,m.subject,m.sender,m.address,m.recipients,m.date,m.snippet,
    EXISTS(SELECT 1 FROM occurrences o WHERE o.message_id=m.id AND seen=0),
    EXISTS(SELECT 1 FROM occurrences o WHERE o.message_id=m.id AND flagged=1),1 FROM messages m";
const LIVE: &str = "EXISTS(SELECT 1 FROM occurrences o WHERE o.message_id=m.id)";

impl MailRepository for Archive {
    type Error = Error;

    fn mailboxes(&self) -> Result<Vec<Mailbox>> {
        let mut statement = self.connection.prepare_cached("SELECT b.id,b.account,b.name,COUNT(o.message_id),COALESCE(SUM(o.seen=0),0),b.delimiter,b.selectable FROM mailboxes b LEFT JOIN occurrences o ON o.mailbox_id=b.id GROUP BY b.id ORDER BY b.name COLLATE NOCASE")?;
        let rows = statement.query_map([], |row| {
            Ok(Mailbox {
                id: row.get(0)?,
                account: row.get(1)?,
                name: row.get(2)?,
                total: row.get::<_, i64>(3)? as u64,
                unread: row.get::<_, i64>(4)? as u64,
                delimiter: row.get(5)?,
                selectable: row.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn page(&self, query: &MailQuery, limit: usize) -> Result<MailPage> {
        let limit = limit.clamp(1, 200);
        let mut clauses = vec![LIVE.to_owned()];
        let mut arguments = Vec::<Value>::new();
        if let Some(mailbox) = query.mailbox {
            clauses.push(
                "EXISTS(SELECT 1 FROM occurrences o WHERE o.message_id=m.id AND o.mailbox_id=?)"
                    .into(),
            );
            arguments.push(mailbox.into());
        }
        match query.filter {
            ViewFilter::Unread => clauses.push(
                "EXISTS(SELECT 1 FROM occurrences o WHERE o.message_id=m.id AND seen=0)".into(),
            ),
            ViewFilter::Starred => clauses.push(
                "EXISTS(SELECT 1 FROM occurrences o WHERE o.message_id=m.id AND flagged=1)".into(),
            ),
            ViewFilter::All => {},
        }
        if let Some(id) = query.thread {
            clauses.push(
                "(m.account,m.thread_key)=(SELECT account,thread_key FROM messages WHERE id=?)"
                    .into(),
            );
            arguments.push((id.0 as i64).into());
        }
        if let Some(ids) = &query.ids {
            if ids.is_empty() {
                return Ok(MailPage {
                    rows: Vec::new(),
                    next: None,
                });
            }
            if ids.len() > 1000 {
                return Err(Error::Invalid("search candidate limit is 1000"));
            }
            clauses.push(format!("m.id IN ({})", vec!["?"; ids.len()].join(",")));
            arguments.extend(ids.iter().map(|id| Value::Integer(id.0 as i64)));
        }
        // Group and page metadata first; enrich only the bounded IDs. Thread
        // members respect the same folder/filter/search scope as their header.
        let matching = format!(
            "SELECT m.id,m.account,m.thread_key,m.rfc_id,m.date,m.sender,m.subject FROM messages m WHERE {}",
            clauses.join(" AND ")
        );
        let groups = if query.threaded {
            "SELECT id,sender,subject,MAX(date) OVER w AS activity,COUNT(*) OVER w AS members,
             ROW_NUMBER() OVER (PARTITION BY account,thread_key ORDER BY (rfc_id=thread_key) DESC,date,id) AS rank
             FROM matching WINDOW w AS (PARTITION BY account,thread_key)"
        } else {
            "SELECT id,sender,subject,date AS activity,1 AS members,1 AS rank FROM matching"
        };
        let text = match query.sort {
            SortField::Date => "''",
            SortField::Sender => "sender",
            SortField::Subject => "subject",
        };
        let key = if query.sort == SortField::Date {
            "activity"
        } else {
            "sort_text COLLATE NOCASE"
        };
        let direction = if query.ascending { "ASC" } else { "DESC" };
        let comparison = if query.ascending { ">" } else { "<" };
        let after = if let Some(cursor) = &query.before {
            arguments.push(if query.sort == SortField::Date {
                cursor.date.into()
            } else {
                cursor.text.clone().into()
            });
            arguments.push((cursor.id.0 as i64).into());
            format!("WHERE ({key},id){comparison}(?,?)")
        } else {
            String::new()
        };
        arguments.push(((limit + 1) as i64).into());
        let enrichment = SUMMARY.replace(
            ",1 FROM messages m",
            ",p.members,p.activity,p.sort_text FROM messages m JOIN paged p ON p.id=m.id",
        );
        let final_key = if query.sort == SortField::Date {
            "p.activity"
        } else {
            "p.sort_text COLLATE NOCASE"
        };
        let sql = format!("WITH matching AS ({matching}), ranked AS ({groups}), representatives AS (
            SELECT id,activity,members,{text} AS sort_text FROM ranked WHERE rank=1
        ), paged AS (SELECT * FROM representatives {after} ORDER BY {key} {direction},id {direction} LIMIT ?)
        {enrichment} ORDER BY {final_key} {direction},m.id {direction}");
        let mut statement = self.connection.prepare_cached(&sql)?;
        let mut rows = statement
            .query_map(params_from_iter(arguments), |row| {
                Ok((
                    summary(row)?,
                    PageCursor {
                        date: row.get(10)?,
                        id: MessageId(row.get::<_, i64>(0)? as u64),
                        text: row.get(11)?,
                    },
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let more = rows.len() > limit;
        rows.truncate(limit);
        let next = more
            .then(|| rows.last().map(|(_, cursor)| cursor.clone()))
            .flatten();
        Ok(MailPage {
            rows: rows.into_iter().map(|(row, _)| row).collect(),
            next,
        })
    }

    fn detail(&self, id: MessageId) -> Result<MailDetail> {
        let summary = self
            .connection
            .query_row(&format!("{SUMMARY} WHERE m.id=?1"), [id.0 as i64], summary)
            .optional()?
            .ok_or(Error::NotFound)?;
        let (text,html,blob,rfc_message_id) = self.connection.query_row("SELECT b.text,b.html,m.blob,m.rfc_id FROM bodies b JOIN messages m ON m.id=b.message_id WHERE m.id=?1",[id.0 as i64],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?)))?;
        Ok(MailDetail {
            summary,
            text,
            html,
            blob,
            rfc_message_id,
        })
    }

    fn correspondence(&self, id: MessageId, limit: usize) -> Result<Vec<MailSummary>> {
        // Match directional sender/recipient pairs in both directions; subjects
        // and thread keys deliberately play no role. Always retain the selection.
        let sql = format!("{SUMMARY} WHERE m.id=?1 OR ({LIVE} AND EXISTS(
            SELECT 1 FROM addresses sf JOIN addresses sr ON sr.message_id=sf.message_id
            JOIN addresses mf ON mf.message_id=m.id JOIN addresses mr ON mr.message_id=m.id
            WHERE sf.message_id=?1 AND sf.role='from' AND sr.role IN ('to','cc')
            AND mf.role='from' AND mr.role IN ('to','cc')
            AND ((sf.address=mf.address AND sr.address=mr.address) OR (sf.address=mr.address AND sr.address=mf.address))
        ) AND NOT EXISTS(SELECT 1 FROM occurrences o JOIN mailboxes b ON b.id=o.mailbox_id WHERE o.message_id=m.id AND lower(b.name) IN ('junk','spam','trash','deleted items')))
        ORDER BY m.date DESC,m.id DESC LIMIT ?2");
        let mut statement = self.connection.prepare_cached(&sql)?;
        Ok(statement
            .query_map(params![id.0 as i64, limit.clamp(1, 200) as i64], summary)?
            .collect::<rusqlite::Result<_>>()?)
    }

    fn thread(&self, id: MessageId, limit: usize) -> Result<Vec<MailSummary>> {
        let sql = format!(
            "{SUMMARY} WHERE {LIVE} AND (m.account,m.thread_key)=(SELECT account,thread_key FROM messages WHERE id=?1) ORDER BY m.date DESC,m.id DESC LIMIT ?2"
        );
        let mut statement = self.connection.prepare_cached(&sql)?;
        Ok(statement
            .query_map(params![id.0 as i64, limit.clamp(1, 200) as i64], summary)?
            .collect::<rusqlite::Result<_>>()?)
    }
}

fn summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<MailSummary> {
    let date: i64 = row.get(5)?;
    let date_label = OffsetDateTime::from_unix_timestamp(date)
        .ok()
        .and_then(|date| date.format(&Rfc2822).ok())
        .unwrap_or_default();
    Ok(MailSummary {
        id: MessageId(row.get::<_, i64>(0)? as u64),
        subject: row.get(1)?,
        sender: row.get(2)?,
        address: row.get(3)?,
        recipients: row.get(4)?,
        date,
        date_label,
        snippet: row.get(6)?,
        unread: row.get(7)?,
        starred: row.get(8)?,
        thread_count: row.get::<_, i64>(9)? as u64,
    })
}
