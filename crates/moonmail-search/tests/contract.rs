use {
    moonmail_core::{IndexChange, MessageId, SearchDocument, SearchIndex, SearchQuery},
    moonmail_search::{Error, SqliteIndex},
    std::{error::Error as StdError, num::NonZeroUsize},
};

type Result<T = ()> = std::result::Result<T, Box<dyn StdError>>;

#[test]
fn field_terms_share_one_snapshot_even_when_another_connection_commits_between_terms() -> Result {
    use moonmail_core::{TextField, TextTerm};
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("index.sqlite3");
    let mut reader = SqliteIndex::open(&path)?;
    let mut writer = SqliteIndex::open(&path)?;
    reader.apply(&[IndexChange::Upsert(
        MessageId(1),
        &SearchDocument {
            subject: "alpha".into(),
            body: "beta".into(),
            ..Default::default()
        },
    )])?;
    let terms = [
        TextTerm {
            field: TextField::Subject,
            value: "alpha".into(),
        },
        TextTerm {
            field: TextField::Body,
            value: "beta".into(),
        },
        TextTerm {
            field: TextField::Cc,
            value: "beta".into(),
        },
    ];
    let mut found = Vec::new();
    let mut failure = None;
    reader.visit_terms(&terms, |slot, id| {
        found.push((slot, id));
        if slot == 0 {
            failure = writer
                .apply(&[IndexChange::Upsert(
                    MessageId(1),
                    &SearchDocument {
                        subject: "other".into(),
                        body: "other".into(),
                        ..Default::default()
                    },
                )])
                .err();
        }
        true
    })?;
    if let Some(error) = failure {
        return Err(error.into());
    }
    assert_eq!(found, vec![(0, MessageId(1)), (1, MessageId(1))]);
    let mut calls = 0;
    reader.visit_terms(&terms, |_, _| {
        calls += 1;
        true
    })?;
    assert_eq!(calls, 0);
    let terms = [
        TextTerm {
            field: TextField::Any,
            value: "other".into(),
        },
        TextTerm {
            field: TextField::Body,
            value: "other".into(),
        },
    ];
    reader.visit_terms(&terms, |_, _| {
        calls += 1;
        false
    })?;
    assert_eq!(calls, 1);
    Ok(())
}

#[allow(clippy::expect_used)]
fn query<'a>(phrases: &'a [&'a str]) -> SearchQuery<'a> {
    SearchQuery {
        phrases,
        limit: NonZeroUsize::new(20).expect("nonzero test limit"),
    }
}

fn lifecycle<I: SearchIndex>(mut index: I) -> Result {
    let original = SearchDocument {
        subject: "alpha quarterly".into(),
        ..Default::default()
    };
    let replacement = SearchDocument {
        subject: "beta annual".into(),
        ..Default::default()
    };
    index.apply(&[
        IndexChange::Upsert(MessageId(2), &original),
        IndexChange::Upsert(MessageId(1), &original),
    ])?;
    assert_eq!(
        index.search(query(&["alpha"]))?,
        [MessageId(1), MessageId(2)]
    );
    assert_eq!(
        index.search(SearchQuery {
            phrases: &["alpha"],
            limit: NonZeroUsize::MIN
        })?,
        [MessageId(1)]
    );
    index.apply(&[IndexChange::Upsert(MessageId(1), &replacement)])?;
    assert_eq!(index.search(query(&["alpha"]))?, [MessageId(2)]);
    assert_eq!(index.search(query(&["beta", "annual"]))?, [MessageId(1)]);
    index.apply(&[
        IndexChange::Delete(MessageId(2)),
        IndexChange::Delete(MessageId(2)),
        IndexChange::Upsert(MessageId(3), &original),
        IndexChange::Delete(MessageId(3)),
    ])?;
    assert!(index.search(query(&["alpha"]))?.is_empty());
    assert!(index.search(query(&[]))?.is_empty());
    assert!(index.search(query(&["  "]))?.is_empty());
    Ok(())
}

#[test]
fn memory_contract() -> Result {
    lifecycle(SqliteIndex::in_memory()?)
}

#[test]
fn disk_contract() -> Result {
    let directory = tempfile::tempdir()?;
    lifecycle(SqliteIndex::open(directory.path().join("search.db"))?)
}

#[test]
fn failed_batch_rolls_back_documents_and_fts() -> Result {
    let mut index = SqliteIndex::in_memory()?;
    let original = SearchDocument {
        body: "original".into(),
        ..Default::default()
    };
    let changed = SearchDocument {
        body: "changed".into(),
        ..Default::default()
    };
    index.apply(&[IndexChange::Upsert(MessageId(1), &original)])?;
    let result = index.apply(&[
        IndexChange::Upsert(MessageId(1), &changed),
        IndexChange::Upsert(MessageId(2), &changed),
        IndexChange::Upsert(MessageId(u64::MAX), &changed),
    ]);
    assert!(matches!(
        result,
        Err(Error::MessageIdOutOfRange(MessageId(u64::MAX)))
    ));
    assert_eq!(index.search(query(&["original"]))?, [MessageId(1)]);
    assert!(index.search(query(&["changed"]))?.is_empty());
    Ok(())
}

#[test]
fn disk_commit_survives_reopen() -> Result {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("search.db");
    let document = SearchDocument {
        subject: "persisted".into(),
        ..Default::default()
    };
    {
        let mut index = SqliteIndex::open(&path)?;
        index.apply(&[IndexChange::Upsert(MessageId(42), &document)])?;
    }
    assert_eq!(
        SqliteIndex::open(path)?.search(query(&["persisted"]))?,
        [MessageId(42)]
    );
    Ok(())
}

#[test]
fn query_operators_are_literal_and_unicode_is_folded() -> Result {
    let mut index = SqliteIndex::in_memory()?;
    let literal = SearchDocument {
        body: "alpha OR beta café".into(),
        ..Default::default()
    };
    let other = SearchDocument {
        body: "alpha gamma".into(),
        ..Default::default()
    };
    index.apply(&[
        IndexChange::Upsert(MessageId(1), &literal),
        IndexChange::Upsert(MessageId(2), &other),
    ])?;
    assert_eq!(index.search(query(&["alpha OR beta"]))?, [MessageId(1)]);
    assert_eq!(index.search(query(&["CAFÉ"]))?, [MessageId(1)]);
    assert_eq!(index.search(query(&["cafe"]))?, [MessageId(1)]);
    // Punctuation is removed by unicode61, leaving the literal token "OR";
    // it must not turn this into an OR/wildcard query matching the other row.
    assert_eq!(index.search(query(&["\" OR *"]))?, [MessageId(1)]);
    Ok(())
}

#[test]
fn streaming_matches_are_complete_literal_and_use_one_snapshot() -> Result {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("search.db");
    let mut index = SqliteIndex::open(&path)?;
    let document = SearchDocument {
        body: "alpha OR beta café".into(),
        ..Default::default()
    };
    let changes: Vec<_> = (1..=1205)
        .rev()
        .map(|id| IndexChange::Upsert(MessageId(id), &document))
        .collect();
    index.apply(&changes)?;
    let mut writer = SqliteIndex::open(&path)?;
    let mut found = Vec::new();
    let mut write_result = Ok(());
    index.visit_matches(&["alpha OR beta", "CAFE"], |id| {
        found.push(id);
        if found.len() == 1 {
            write_result = writer.apply(&[
                IndexChange::Delete(MessageId(1205)),
                IndexChange::Upsert(MessageId(1206), &document),
            ]);
        }
        true
    })?;
    write_result?;
    assert_eq!(found, (1..=1205).map(MessageId).collect::<Vec<_>>());
    let mut stopped = Vec::new();
    index.visit_matches(&["cafe"], |id| {
        stopped.push(id);
        stopped.len() < 2
    })?;
    assert_eq!(stopped, [MessageId(1), MessageId(2)]);
    for phrases in [&[][..], &[" "][..], &["absent"][..]] {
        index.visit_matches(phrases, |_| panic!("empty query/result must not visit"))?;
    }
    Ok(())
}
