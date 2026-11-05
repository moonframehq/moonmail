use {
    crate::{Archive, Error, Result},
    rusqlite::{OptionalExtension, TransactionBehavior, params},
    serde::{Deserialize, Serialize},
};

/// An explicitly configured sending identity. Authentication remains attached
/// to `account`; address/name/Reply-To and signature are frozen into each draft.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Identity {
    pub id: String,
    pub account: String,
    pub name: String,
    pub address: String,
    pub reply_to: String,
    pub signature: String,
    pub is_default: bool,
}

impl Identity {
    /// Structural bounds only; the composition adapter validates address syntax.
    pub fn validate(&self) -> Result<()> {
        if self.id.is_empty()
            || self.account.is_empty()
            || self.address.is_empty()
            || [
                &self.id,
                &self.account,
                &self.name,
                &self.address,
                &self.reply_to,
            ]
            .iter()
            .any(|field| field.len() > 998 || field.chars().any(char::is_control))
            || self.signature.len() > 16 * 1024
            || self.signature.contains('\0')
        {
            return Err(Error::Invalid(
                "Invalid identity fields or signature exceeds 16 KiB",
            ));
        }
        Ok(())
    }
}

fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Identity> {
    Ok(Identity {
        id: row.get(0)?,
        account: row.get(1)?,
        name: row.get(2)?,
        address: row.get(3)?,
        reply_to: row.get(4)?,
        signature: row.get(5)?,
        is_default: row.get(6)?,
    })
}

impl Archive {
    pub fn identities(&self, account: &str) -> Result<Vec<Identity>> {
        let mut query = self.connection.prepare("SELECT id,account,name,address,reply_to,signature,is_default FROM identities WHERE account=?1 ORDER BY is_default DESC,name,id LIMIT 32")?;
        Ok(query
            .query_map([account], row)?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn identity(&self, account: &str, id: &str) -> Result<Identity> {
        let identity = self.connection.query_row("SELECT id,account,name,address,reply_to,signature,is_default FROM identities WHERE account=?1 AND id=?2",params![account,id],row)?;
        identity.validate()?;
        Ok(identity)
    }

    pub fn save_identity(&mut self, identity: &Identity) -> Result<()> {
        identity.validate()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let owner: Option<String> = transaction
            .query_row(
                "SELECT account FROM identities WHERE id=?1",
                [&identity.id],
                |row| row.get(0),
            )
            .optional()?;
        if owner
            .as_ref()
            .is_some_and(|owner| owner != &identity.account)
        {
            return Err(Error::Invalid("Identity belongs to another account"));
        }
        let count: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM identities WHERE account=?1",
            [&identity.account],
            |row| row.get(0),
        )?;
        if owner.is_none() && count >= 32 {
            return Err(Error::Invalid("At most 32 identities per account"));
        }
        if identity.is_default {
            transaction.execute(
                "UPDATE identities SET is_default=0 WHERE account=?1",
                [&identity.account],
            )?;
        }
        transaction.execute("INSERT INTO identities(id,account,name,address,reply_to,signature,is_default) VALUES (?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(id) DO UPDATE SET name=excluded.name,address=excluded.address,reply_to=excluded.reply_to,signature=excluded.signature,is_default=excluded.is_default",params![identity.id,identity.account,identity.name,identity.address,identity.reply_to,identity.signature,identity.is_default])?;
        transaction.commit()?;
        Ok(())
    }

    /// Draft snapshots intentionally survive removal or subsequent edits.
    pub fn delete_identity(&self, account: &str, id: &str) -> Result<()> {
        if self.connection.execute(
            "DELETE FROM identities WHERE account=?1 AND id=?2",
            params![account, id],
        )? != 1
        {
            return Err(Error::NotFound);
        }
        Ok(())
    }
}
