use {
    crate::{
        Archive, Error, Result,
        queries::{SUMMARY, summary},
    },
    moonmail_core::{PageCursor, RelatedPage, RelatedQuery, Relationship, SortField},
    rusqlite::{OptionalExtension, params_from_iter, types::Value},
};

const PAIRS: &str = "EXISTS(
    SELECT 1 FROM addresses sf JOIN addresses sr ON sr.message_id=sf.message_id
    JOIN addresses mf ON mf.message_id=m.id JOIN addresses mr ON mr.message_id=m.id
    WHERE sf.message_id=?1 AND sf.role='from' AND sr.role IN ('to','cc')
    AND mf.role='from' AND mr.role IN ('to','cc')
    AND ((sf.address=mf.address AND sr.address=mr.address) OR (sf.address=mr.address AND sr.address=mf.address)))
    AND NOT EXISTS(SELECT 1 FROM occurrences o JOIN mailboxes b ON b.id=o.mailbox_id
        WHERE o.message_id=m.id AND COALESCE(b.user_role,b.server_role,
        CASE WHEN lower(b.name) IN ('junk','spam') THEN 'junk' WHEN lower(b.name) IN ('trash','deleted items') THEN 'trash' END) IN ('junk','trash'))";

pub(crate) fn page(archive: &Archive, query: &RelatedQuery, limit: usize) -> Result<RelatedPage> {
    let limit = limit.clamp(1, 200);
    // Keep the pinned anchor and this page in one read snapshot while the sync
    // connection may be adding occurrences. No message bodies enter this query.
    let transaction = archive.connection.unchecked_transaction()?;
    let anchor = transaction
        .query_row(
            &format!("{SUMMARY} WHERE m.id=?1"),
            [query.anchor.0 as i64],
            summary,
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    let relation = match query.relationship {
        Relationship::Correspondence => PAIRS.to_owned(),
        Relationship::Thread => crate::thread_graph::relation("?1"),
    };
    let key = match query.sort {
        SortField::Date => "m.date",
        SortField::Sender => "m.sender COLLATE NOCASE",
        SortField::Subject => "m.subject COLLATE NOCASE",
    };
    let direction = if query.ascending { "ASC" } else { "DESC" };
    let comparison = if query.ascending { ">" } else { "<" };
    let mut arguments = vec![Value::Integer(query.anchor.0 as i64)];
    let after = if let Some(cursor) = &query.before {
        arguments.push(if query.sort == SortField::Date {
            cursor.date.into()
        } else {
            cursor.text.clone().into()
        });
        arguments.push((cursor.id.0 as i64).into());
        format!("AND ({key},m.id){comparison}(?2,?3)")
    } else {
        String::new()
    };
    arguments.push(((limit + 1) as i64).into());
    let sql = format!(
        "WITH paged AS (SELECT m.id FROM messages m WHERE m.id<>?1
        AND EXISTS(SELECT 1 FROM occurrences o WHERE o.message_id=m.id) AND {relation} {after}
        ORDER BY {key} {direction},m.id {direction} LIMIT ?)
        {SUMMARY} JOIN paged p ON p.id=m.id ORDER BY {key} {direction},m.id {direction}"
    );
    let mut rows = {
        let mut statement = transaction.prepare(&sql)?;
        statement
            .query_map(params_from_iter(arguments), summary)?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let more = rows.len() > limit;
    rows.truncate(limit);
    let next = more
        .then(|| {
            rows.last().map(|row| PageCursor {
                id: row.id,
                date: row.date,
                text: match query.sort {
                    SortField::Date => String::new(),
                    SortField::Sender => row.sender.clone(),
                    SortField::Subject => row.subject.clone(),
                },
            })
        })
        .flatten();
    transaction.commit()?;
    Ok(RelatedPage { anchor, rows, next })
}
