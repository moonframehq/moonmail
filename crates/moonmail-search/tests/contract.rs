use {
    moonmail_core::{IndexChange, MessageId, SearchDocument, SearchIndex, SearchQuery},
    moonmail_search::{Error, SqliteIndex},
    std::{error::Error as StdError, num::NonZeroUsize},
};

type Result<T = ()> = std::result::Result<T, Box<dyn StdError>>;

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
