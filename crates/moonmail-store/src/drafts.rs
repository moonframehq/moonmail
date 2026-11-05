use {
    crate::{Archive, Error, Result},
    rusqlite::{OptionalExtension, params},
    serde::{Deserialize, Serialize},
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Draft {
    pub id: String,
    /// Original compose input, retaining display names and groups until send.
    pub recipients: String,
    pub cc: String,
    pub bcc: String,
    /// Empty only for legacy drafts or new drafts before an account is connected.
    pub account: String,
    pub subject: String,
    pub body: String,
    pub reply_id: String,
    pub references: Vec<String>,
    pub attachments: Vec<DraftAttachment>,
    pub identity: Option<crate::Identity>,
}

pub const MAX_DRAFT_ATTACHMENTS: usize = 16;
pub const MAX_ATTACHMENT_BYTES: u64 = 25 * 1024 * 1024;

/// Frozen content addressed independently of the original local path. Duplicate
/// filenames and identical files are allowed; the stable slot ID binds removals.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DraftAttachment {
    pub id: String,
    pub name: String,
    pub blob: String,
    pub bytes: u64,
}

impl Draft {
    pub fn sender(&self) -> &str {
        self.identity
            .as_ref()
            .map_or(self.account.as_str(), |identity| identity.address.as_str())
    }

    pub fn validate_identity(&self) -> Result<()> {
        if let Some(identity) = &self.identity {
            identity.validate()?;
            if identity.account != self.account {
                return Err(Error::Invalid("Draft identity belongs to another account"));
            }
        }
        Ok(())
    }

    pub fn validate_attachments(&self) -> Result<()> {
        if self.attachments.len() > MAX_DRAFT_ATTACHMENTS {
            return Err(Error::Invalid("A draft supports at most 16 attachments"));
        }
        let mut total = 0u64;
        let mut ids = std::collections::HashSet::new();
        for item in &self.attachments {
            if item.id.is_empty()
                || !ids.insert(&item.id)
                || item.name.is_empty()
                || item.name.chars().any(char::is_control)
                || item.name.contains(['/', '\\'])
                || item.blob.len() != 64
                || !item.blob.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(Error::Invalid("Invalid draft attachment metadata"));
            }
            total = total
                .checked_add(item.bytes)
                .ok_or(Error::Invalid("Attachment size overflow"))?;
            if total > MAX_ATTACHMENT_BYTES {
                return Err(Error::Invalid("Attachments exceed 25 MiB total"));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct DraftSummary {
    pub id: String,
    pub recipients: String,
    pub subject: String,
    pub account: String,
    pub updated: i64,
}

/// Reuse only within the same unchanged ordering. Editing a draft promotes it;
/// restart pagination to include edits made above an existing cursor.
#[derive(Clone, Debug, Serialize)]
pub struct DraftCursor {
    pub updated: i64,
    pub id: String,
}

#[derive(Serialize)]
pub struct DraftPage {
    pub rows: Vec<DraftSummary>,
    pub next: Option<DraftCursor>,
}

impl Archive {
    pub fn save_draft(&self, draft: &Draft) -> Result<()> {
        Self::persist_draft(&self.connection, draft)
    }

    pub(crate) fn persist_draft(connection: &rusqlite::Connection, draft: &Draft) -> Result<()> {
        draft.validate_attachments()?;
        draft.validate_identity()?;
        if draft.id.is_empty() {
            return Err(Error::Invalid("Draft ID is empty"));
        }
        // Monotonic milliseconds give deterministic edit order even for multiple
        // saves in one clock tick. The outbox tombstone prevents stale UI events
        // from resurrecting an already queued draft.
        let count = connection.execute(
            "INSERT INTO drafts(id,recipients,subject,body,reply_id,updated,references_json,cc,bcc,account,attachments_json,identity_json)
             SELECT ?1,?2,?3,?4,?5,MAX(CAST(unixepoch('subsec')*1000 AS INTEGER),COALESCE((SELECT MAX(updated)+1 FROM drafts),0)),?6,?7,?8,?9,?10,?11
             WHERE NOT EXISTS(SELECT 1 FROM outbox WHERE draft_id=?1)
             ON CONFLICT(id) DO UPDATE SET recipients=excluded.recipients,subject=excluded.subject,body=excluded.body,reply_id=excluded.reply_id,references_json=excluded.references_json,cc=excluded.cc,bcc=excluded.bcc,account=excluded.account,attachments_json=excluded.attachments_json,identity_json=excluded.identity_json,updated=excluded.updated",
            params![draft.id,draft.recipients,draft.subject,draft.body,draft.reply_id,serde_json::to_string(&draft.references)?,draft.cc,draft.bcc,draft.account,serde_json::to_string(&draft.attachments)?,draft.identity.as_ref().map(serde_json::to_string).transpose()?],
        )?;
        if count == 0 {
            return Err(Error::Invalid("This draft has already been queued"));
        }
        Ok(())
    }

    pub fn draft(&self, id: &str) -> Result<Option<Draft>> {
        let row = self.connection.query_row("SELECT id,recipients,subject,body,reply_id,references_json,cc,bcc,account,attachments_json,identity_json FROM drafts WHERE id=?1", [id], |row|Ok((Draft {
            id:row.get(0)?,recipients:row.get(1)?,subject:row.get(2)?,body:row.get(3)?,reply_id:row.get(4)?,references:Vec::new(),cc:row.get(6)?,bcc:row.get(7)?,account:row.get(8)?,attachments:Vec::new(),identity:None,
        },row.get::<_,String>(5)?,row.get::<_,String>(9)?,row.get::<_,Option<String>>(10)?))).optional()?;
        row.map(|(mut draft, references, attachments, identity)| {
            draft.references = serde_json::from_str(&references)?;
            draft.attachments = serde_json::from_str(&attachments)?;
            draft.validate_attachments()?;
            draft.identity = identity.as_deref().map(serde_json::from_str).transpose()?;
            draft.validate_identity()?;
            Ok(draft)
        })
        .transpose()
    }

    pub fn latest_draft(&self) -> Result<Option<Draft>> {
        let id: Option<String> = self
            .connection
            .query_row(
                "SELECT id FROM drafts ORDER BY updated DESC,id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        match id {
            Some(id) => self.draft(&id),
            None => Ok(None),
        }
    }

    pub fn discard_draft(&self, id: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM drafts WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn draft_count(&self) -> Result<u64> {
        Ok(self
            .connection
            .query_row("SELECT COUNT(*) FROM drafts", [], |row| {
                row.get::<_, i64>(0)
            })? as u64)
    }

    /// Page metadata only; body and Bcc are loaded only when opening a draft.
    pub fn drafts(&self, before: Option<&DraftCursor>, limit: usize) -> Result<DraftPage> {
        let limit = limit.clamp(1, 100);
        let mut statement=self.connection.prepare("SELECT id,recipients,subject,account,updated FROM drafts WHERE ?1 IS NULL OR (updated,id)<(?1,?2) ORDER BY updated DESC,id DESC LIMIT ?3")?;
        let mut rows = statement
            .query_map(
                params![
                    before.map(|c| c.updated),
                    before.map(|c| c.id.as_str()),
                    (limit + 1) as i64
                ],
                |row| {
                    Ok(DraftSummary {
                        id: row.get(0)?,
                        recipients: row.get(1)?,
                        subject: row.get(2)?,
                        account: row.get(3)?,
                        updated: row.get(4)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let more = rows.len() > limit;
        rows.truncate(limit);
        let next = if more {
            rows.last().map(|row| DraftCursor {
                updated: row.updated,
                id: row.id.clone(),
            })
        } else {
            None
        };
        Ok(DraftPage { rows, next })
    }
}
