use {
    crate::{Archive, Error, Result},
    moonmail_core::FolderChange,
    rusqlite::{Connection, OptionalExtension, params},
    serde::{Deserialize, Serialize},
};

pub enum FolderEdit {
    Create(String),
    Rename { id: i64, name: String },
    Delete(i64),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FolderMember {
    pub id: i64,
    pub name: String,
    pub epoch: u32,
    pub delimiter: Option<String>,
    pub selectable: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct FolderOperation {
    pub id: i64,
    pub account: String,
    pub change: FolderChange,
    #[serde(skip_serializing)]
    pub members: Vec<FolderMember>,
    pub state: String,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct FolderOperationSummary {
    pub id: i64,
    pub account: String,
    pub change: FolderChange,
    pub state: String,
    pub detail: String,
}

fn valid_name(name: &str) -> Result<()> {
    if name.trim().is_empty() || name.len() > 1024 || name.chars().any(char::is_control) {
        return Err(Error::Invalid(
            "Folder names must be nonempty, at most 1024 bytes, and contain no control characters",
        ));
    }
    if name.eq_ignore_ascii_case("INBOX") {
        return Err(Error::Invalid(
            "INBOX cannot be created, renamed, or deleted here",
        ));
    }
    Ok(())
}

fn member(row: &rusqlite::Row<'_>) -> rusqlite::Result<FolderMember> {
    Ok(FolderMember {
        id: row.get(0)?,
        name: row.get(1)?,
        epoch: row.get(2)?,
        delimiter: row.get(3)?,
        selectable: row.get(4)?,
    })
}

fn plan(db: &Connection, account: &str, edit: FolderEdit, remote: bool) -> Result<FolderOperation> {
    if account.is_empty() {
        return Err(Error::Invalid("Choose a folder account"));
    }
    let busy: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM folder_operations WHERE account=?1 AND state IN ('queued','running','unknown')) OR EXISTS(SELECT 1 FROM move_operations o JOIN mailboxes b ON b.id=o.source_id WHERE b.account=?1 AND o.state IN ('queued','running','unknown'))", [account], |row|row.get(0))?;
    if busy {
        return Err(Error::Invalid(
            "Finish, cancel, or review pending folder and message moves before editing folders",
        ));
    }
    let source = match &edit {
        FolderEdit::Create(_) => None,
        FolderEdit::Rename { id, .. } | FolderEdit::Delete(id) => Some(*id),
    };
    let mut members = Vec::new();
    if let Some(id) = source {
        let root = db.query_row("SELECT id,name,epoch,delimiter,selectable FROM mailboxes WHERE id=?1 AND account=?2 AND active=1", params![id,account], member).optional()?.ok_or(Error::NotFound)?;
        valid_name(&root.name)?;
        let prefix = root
            .delimiter
            .as_ref()
            .map(|delimiter| format!("{}{delimiter}", root.name));
        members = db.prepare("SELECT id,name,epoch,delimiter,selectable FROM mailboxes WHERE account=?1 AND active=1 AND (id=?2 OR (?3 IS NOT NULL AND substr(name,1,length(?3))=?3)) ORDER BY length(name),name LIMIT 1001")?
            .query_map(params![account,id,prefix], member)?.collect::<rusqlite::Result<Vec<_>>>()?;
        if members.len() > 1000 {
            return Err(Error::Invalid(
                "Folder edits support at most 1000 descendant folders",
            ));
        }
        if remote
            && members
                .iter()
                .any(|member| member.selectable && member.epoch == 0)
        {
            return Err(Error::Invalid(
                "Sync the source folders before renaming or deleting them",
            ));
        }
    }
    let change = match edit {
        FolderEdit::Create(name) => {
            valid_name(&name)?;
            let trailing:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM mailboxes WHERE account=?1 AND delimiter IS NOT NULL AND delimiter<>'' AND substr(?2,-length(delimiter))=delimiter)",params![account,name],|row|row.get(0))?;
            if trailing {
                return Err(Error::Invalid(
                    "Choose a folder name without a trailing hierarchy delimiter",
                ));
            }
            let exists: bool = db.query_row(
                "SELECT EXISTS(SELECT 1 FROM mailboxes WHERE account=?1 AND name=?2 AND active=1)",
                params![account, name],
                |row| row.get(0),
            )?;
            if exists {
                return Err(Error::Invalid("Folder already exists"));
            }
            FolderChange::Create { name }
        },
        FolderEdit::Rename { name, .. } => {
            valid_name(&name)?;
            let root = members.first().ok_or(Error::NotFound)?;
            if root
                .delimiter
                .as_ref()
                .is_some_and(|delimiter| !delimiter.is_empty() && name.ends_with(delimiter))
            {
                return Err(Error::Invalid(
                    "Choose a folder name without a trailing hierarchy delimiter",
                ));
            }
            for member in &members {
                valid_name(&format!(
                    "{}{}",
                    name,
                    member
                        .name
                        .strip_prefix(&root.name)
                        .ok_or(Error::Invalid("Invalid folder hierarchy"))?
                ))?;
            }
            let prefix = root
                .delimiter
                .as_ref()
                .map(|delimiter| format!("{name}{delimiter}"));
            let collision:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM mailboxes WHERE account=?1 AND (name=?2 OR (?3 IS NOT NULL AND substr(name,1,length(?3))=?3)))",params![account,name,prefix],|row|row.get(0))?;
            if collision
                || root
                    .delimiter
                    .as_ref()
                    .is_some_and(|delimiter| name.starts_with(&format!("{}{delimiter}", root.name)))
            {
                return Err(Error::Invalid(
                    "Choose an unused, nonoverlapping destination folder name; archived names also remain reserved",
                ));
            }
            FolderChange::Rename {
                source: root.name.clone(),
                destination: name,
            }
        },
        FolderEdit::Delete(_) => {
            if members.len() != 1 {
                return Err(Error::Invalid("Delete or move child folders first"));
            }
            FolderChange::Delete {
                name: members[0].name.clone(),
            }
        },
    };
    Ok(FolderOperation {
        id: 0,
        account: account.into(),
        change,
        members,
        state: "queued".into(),
        detail: String::new(),
    })
}

fn apply(db: &Connection, operation: &FolderOperation) -> Result<()> {
    for source in &operation.members {
        let same: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM mailboxes WHERE id=?1 AND account=?2 AND name=?3)",
            params![source.id, operation.account, source.name],
            |row| row.get(0),
        )?;
        if !same {
            return Err(Error::Invalid(
                "Folder metadata changed during the operation; sync and inspect its outcome",
            ));
        }
    }
    match &operation.change {
        FolderChange::Create { name } => {
            // Prefer the closest existing ancestor's namespace; otherwise use
            // the account's first folder, preserving a flat (NULL) delimiter.
            // A subsequent remote LIST remains authoritative.
            let delimiter:Option<String>=db.query_row("SELECT delimiter FROM mailboxes WHERE account=?1 AND active=1 ORDER BY CASE WHEN delimiter IS NOT NULL AND delimiter<>'' AND substr(?2,1,length(name||delimiter))=name||delimiter THEN length(name) ELSE -1 END DESC,id LIMIT 1",params![operation.account,name],|row|row.get(0)).optional()?.unwrap_or_else(||Some("/".into()));
            db.execute("INSERT INTO mailboxes(account,name,delimiter) VALUES (?1,?2,?3) ON CONFLICT(account,name) DO UPDATE SET active=1",params![operation.account,name,delimiter])?;
        },
        FolderChange::Rename {
            source,
            destination,
        } => {
            for entry in &operation.members {
                let suffix = entry
                    .name
                    .strip_prefix(source)
                    .ok_or(Error::Invalid("Invalid rename member"))?;
                db.execute(
                    "UPDATE mailboxes SET name=?2,active=1 WHERE id=?1",
                    params![entry.id, format!("{destination}{suffix}")],
                )?;
            }
        },
        FolderChange::Delete { .. } => {
            for entry in &operation.members {
                db.execute(
                    "DELETE FROM flag_operations WHERE mailbox_id=?1",
                    [entry.id],
                )?;
                db.execute("DELETE FROM occurrences WHERE mailbox_id=?1", [entry.id])?;
                db.execute("UPDATE mailboxes SET active=0 WHERE id=?1", [entry.id])?;
            }
        },
    }
    Ok(())
}

impl Archive {
    /// Freeze namespace intent and member identities atomically. Only one
    /// unresolved namespace mutation per account; no optimistic remote changes.
    pub fn queue_folder_edit(&mut self, account: &str, edit: FolderEdit) -> Result<i64> {
        let tx = self.connection.transaction()?;
        let operation = plan(&tx, account, edit, true)?;
        tx.execute("INSERT INTO folder_operations(account,change_json,members_json,state) VALUES (?1,?2,?3,'queued')",params![account,serde_json::to_string(&operation.change)?,serde_json::to_string(&operation.members)?])?;
        let id = tx.last_insert_rowid();
        tx.commit()?;
        Ok(id)
    }

    /// Composition root must call only for a local/demo namespace. Any remote
    /// UID in the account rejects local edits, including retired message origins.
    pub fn edit_local_folder(&mut self, account: &str, edit: FolderEdit) -> Result<()> {
        let tx = self.connection.transaction()?;
        let remote:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM mailboxes WHERE account=?1 AND epoch<>0) OR EXISTS(SELECT 1 FROM messages WHERE account=?1 AND origin LIKE 'imap:%') OR EXISTS(SELECT 1 FROM occurrences o JOIN mailboxes b ON b.id=o.mailbox_id WHERE b.account=?1 AND o.uid IS NOT NULL)",[account],|row|row.get(0))?;
        if remote {
            return Err(Error::Invalid(
                "Remote folders must be edited through queued server operations",
            ));
        }
        let operation = plan(&tx, account, edit, false)?;
        apply(&tx, &operation)?;
        tx.commit()?;
        Ok(())
    }

    pub fn folder_operations(&self) -> Result<Vec<FolderOperationSummary>> {
        let raw=self.connection.prepare("SELECT id,account,change_json,state,detail FROM folder_operations ORDER BY (state IN ('queued','running','unknown','failed')) DESC,id DESC LIMIT 100")?
            .query_map([],|row|Ok((row.get(0)?,row.get(1)?,row.get::<_,String>(2)?,row.get(3)?,row.get(4)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        raw.into_iter()
            .map(|(id, account, change, state, detail)| {
                Ok(FolderOperationSummary {
                    id,
                    account,
                    change: serde_json::from_str(&change)?,
                    state,
                    detail,
                })
            })
            .collect()
    }

    pub fn unresolved_folder_count(&self) -> Result<u64> {
        Ok(self.connection.query_row("SELECT COUNT(*) FROM folder_operations WHERE state IN ('queued','running','unknown','failed')",[],|row|row.get::<_,i64>(0))? as u64)
    }

    pub fn pending_folder_edit(&self, account: &str) -> Result<Option<FolderOperation>> {
        // One unresolved entry per account; lookup must not be truncated by UI history.
        let raw=self.connection.query_row("SELECT id,change_json,members_json FROM folder_operations WHERE account=?1 AND state='queued'",[account],|row|Ok((row.get(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?))).optional()?;
        raw.map(|(id, change, members)| {
            Ok(FolderOperation {
                id,
                account: account.into(),
                change: serde_json::from_str(&change)?,
                members: serde_json::from_str(&members)?,
                state: "queued".into(),
                detail: String::new(),
            })
        })
        .transpose()
    }

    pub fn claim_folder_edit(&self, id: i64) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE folder_operations SET state='running' WHERE id=?1 AND state='queued'",
            [id],
        )? == 1)
    }

    pub fn fail_folder_edit(&self, id: i64, detail: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE folder_operations SET state='failed',detail=?2 WHERE id=?1 AND state='queued'",
            params![id, detail],
        )?;
        Ok(())
    }

    /// Server acknowledgment and local name/occurrence changes commit together.
    /// Failed local commit leaves running, which startup recovers as unknown.
    pub fn finish_folder_edit(
        &mut self,
        operation: &FolderOperation,
        state: &str,
        detail: &str,
    ) -> Result<()> {
        if !matches!(state, "done" | "unknown" | "failed") {
            return Err(Error::Invalid("Invalid folder operation outcome"));
        }
        let tx = self.connection.transaction()?;
        let changed=tx.execute("UPDATE folder_operations SET state=?2,detail=?3 WHERE id=?1 AND state='running' AND account=?4 AND change_json=?5 AND members_json=?6",params![operation.id,state,detail,operation.account,serde_json::to_string(&operation.change)?,serde_json::to_string(&operation.members)?])?;
        if changed != 1 {
            return Err(Error::Invalid("Folder operation is no longer running"));
        }
        if state == "done" {
            apply(&tx, operation)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn cancel_folder_edit(&self, id: i64) -> Result<()> {
        if self.connection.execute("UPDATE folder_operations SET state='cancelled',detail='Cancelled before dispatch' WHERE id=?1 AND state IN ('queued','failed')",[id])?!=1 {return Err(Error::Invalid("Only queued or failed folder operations can be cancelled"));}
        Ok(())
    }

    pub fn dismiss_folder_edit(&self, id: i64) -> Result<()> {
        if self.connection.execute("UPDATE folder_operations SET state='dismissed',detail='Outcome inspected; no retry sent' WHERE id=?1 AND state='unknown'",[id])?!=1 {return Err(Error::Invalid("Folder operation is not awaiting outcome review"));}
        Ok(())
    }

    /// Startup only, while holding the archive application lock.
    pub fn recover_interrupted_folders(&self) -> Result<()> {
        self.connection.execute("UPDATE folder_operations SET state='unknown',detail='Interrupted namespace change. Sync and inspect folders before dismissing; no automatic retry.' WHERE state='running'",[])?;
        Ok(())
    }
}
