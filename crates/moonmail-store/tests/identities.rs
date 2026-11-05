use moonmail_store::{Archive, Identity};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn identity_defaults_are_atomic_and_account_ownership_cannot_be_changed() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let first = Identity {
        id: "first".into(),
        account: "me@example.org".into(),
        address: "me@example.org".into(),
        is_default: true,
        ..Default::default()
    };
    archive.save_identity(&first)?;
    let second = Identity {
        id: "second".into(),
        address: "alias@example.org".into(),
        ..first.clone()
    };
    let db = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
    db.execute_batch("CREATE TRIGGER fail_identity BEFORE INSERT ON identities WHEN NEW.id='second' BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;")?;
    assert!(archive.save_identity(&second).is_err());
    assert!(archive.identity(&first.account, &first.id)?.is_default);
    db.execute_batch("DROP TRIGGER fail_identity;")?;
    archive.save_identity(&second)?;
    assert!(!archive.identity(&first.account, &first.id)?.is_default);
    assert!(archive.identity(&second.account, &second.id)?.is_default);
    assert!(
        archive
            .save_identity(&Identity {
                account: "other@example.org".into(),
                ..second.clone()
            })
            .is_err()
    );
    assert!(
        archive
            .delete_identity("other@example.org", &second.id)
            .is_err()
    );
    assert!(archive.identity("other@example.org", &second.id).is_err());
    for n in 2..32 {
        archive.save_identity(&Identity {
            id: n.to_string(),
            is_default: false,
            ..first.clone()
        })?;
    }
    assert!(
        archive
            .save_identity(&Identity {
                id: "overflow".into(),
                is_default: false,
                ..first.clone()
            })
            .is_err()
    );
    assert_eq!(archive.identities(&first.account)?.len(), 32);
    Ok(())
}
