use {
    moonmail_core::{MailQuery, MailRepository, SortField},
    moonmail_store::{Archive, FileBlobs},
    std::collections::HashSet,
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn sorting_and_thread_pagination_cover_each_item_once() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    moonmail_store::seed_demo(&mut archive, &FileBlobs::new(dir.path()))?;
    for threaded in [false, true] {
        for sort in [SortField::Date, SortField::Sender, SortField::Subject] {
            for ascending in [false, true] {
                let mut query = MailQuery {
                    threaded,
                    sort,
                    ascending,
                    ..Default::default()
                };
                let mut seen = HashSet::new();
                let mut previous = None;
                loop {
                    let page = archive.page(&query, 2)?;
                    assert!(page.rows.len() <= 2);
                    for row in &page.rows {
                        assert!(seen.insert(row.id), "duplicate across page boundary");
                        if threaded {
                            assert_eq!(row.thread_count, 3);
                            assert!(
                                !row.subject.starts_with("Re:"),
                                "the RFC root represents the group"
                            );
                            let children = archive.page(
                                &MailQuery {
                                    thread: Some(row.id),
                                    ..Default::default()
                                },
                                100,
                            )?;
                            assert_eq!(children.rows.len(), 3);
                        }
                        if sort != SortField::Date {
                            let key = (if sort == SortField::Sender {
                                &row.sender
                            } else {
                                &row.subject
                            })
                            .to_lowercase();
                            if let Some(previous) = &previous {
                                assert!(if ascending {
                                    previous <= &key
                                } else {
                                    previous >= &key
                                });
                            }
                            previous = Some(key);
                        }
                    }
                    match page.next {
                        Some(cursor) => query.before = Some(cursor),
                        None => break,
                    }
                }
                assert_eq!(seen.len(), if threaded { 24 } else { 72 });
            }
        }
    }
    let first = archive
        .page(
            &MailQuery {
                threaded: true,
                ..Default::default()
            },
            1,
        )?
        .rows
        .remove(0);
    let children = archive.page(
        &MailQuery {
            thread: Some(first.id),
            ..Default::default()
        },
        100,
    )?;
    let reply = children
        .rows
        .iter()
        .find(|row| row.id != first.id)
        .ok_or("No reply")?;
    let filtered = archive.page(
        &MailQuery {
            threaded: true,
            ids: Some(vec![reply.id]),
            ..Default::default()
        },
        10,
    )?;
    assert_eq!(filtered.rows.len(), 1);
    assert_eq!(filtered.rows[0].id, reply.id);
    assert_eq!(filtered.rows[0].thread_count, 1);
    Ok(())
}

#[test]
fn version_one_archives_gain_hierarchy_without_losing_mailboxes() -> Result {
    let dir = tempfile::tempdir()?;
    let connection = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
    connection.execute_batch(include_str!("../src/schema.sql"))?;
    connection.execute(
        "INSERT INTO mailboxes(account,name) VALUES ('synthetic@example.org','Parent.Child')",
        [],
    )?;
    drop(connection);
    let archive = Archive::open(dir.path())?;
    let folder = archive.mailboxes()?.remove(0);
    assert_eq!(folder.name, "Parent.Child");
    assert_eq!(folder.delimiter.as_deref(), Some("/"));
    archive.mailbox_hierarchy(folder.id, Some("."), false)?;
    drop(archive);
    let folder = Archive::open(dir.path())?.mailboxes()?.remove(0);
    assert_eq!(folder.delimiter.as_deref(), Some("."));
    assert!(!folder.selectable);
    Ok(())
}
