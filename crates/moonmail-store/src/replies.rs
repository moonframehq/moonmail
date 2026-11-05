use {
    crate::{Archive, Error, IngestError},
    moonmail_core::{BlobStore, MessageId, MimeParser, ReplyAddresses, ReplyHeaders},
    rusqlite::{OptionalExtension, params},
};

impl Archive {
    /// Resolve immutable MIME only when explicitly inspecting/exporting a message.
    pub fn message_blob(&self, id: MessageId) -> crate::Result<String> {
        self.connection
            .query_row(
                "SELECT blob FROM messages WHERE id=?1",
                [id.0 as i64],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(Error::NotFound)
    }

    /// Read retained threading/recipient headers. Legacy messages are recovered lazily
    /// from immutable MIME on the worker, one selected message at a time, then
    /// cached. Failure leaves the legacy marker intact so a later call can retry.
    pub fn reply_headers<P: MimeParser, B: BlobStore>(
        &self,
        id: MessageId,
        parser: &P,
        blobs: &B,
    ) -> Result<ReplyHeaders, IngestError<P::Error, B::Error>> {
        let row = self
            .connection
            .query_row(
                "SELECT rfc_id,references_json,in_reply_to,blob,reply_addresses_json FROM messages WHERE id=?1",
                [id.0 as i64],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()
            .map_err(Error::from)?
            .ok_or(Error::NotFound)?;
        let (message_id, references, in_reply_to, blob, addresses) = row;
        if let (Some(references), Some(addresses)) = (references, addresses) {
            return Ok(ReplyHeaders {
                message_id,
                references: serde_json::from_str(&references).map_err(Error::from)?,
                in_reply_to,
                addresses: serde_json::from_str(&addresses).map_err(Error::from)?,
            });
        }
        let raw = blobs.get(&blob).map_err(IngestError::Blob)?;
        let parsed = parser.parse(&raw).map_err(IngestError::Parser)?;
        let addresses = ReplyAddresses::from(&parsed);
        let transaction = self
            .connection
            .unchecked_transaction()
            .map_err(Error::from)?;
        transaction.execute(
            "UPDATE messages SET references_json=?2,in_reply_to=?3,reply_addresses_json=?4 WHERE id=?1 AND (references_json IS NULL OR reply_addresses_json IS NULL)",
            params![id.0 as i64, serde_json::to_string(&parsed.references).map_err(Error::from)?, parsed.in_reply_to,serde_json::to_string(&addresses).map_err(Error::from)?],
        ).map_err(Error::from)?;
        crate::thread_graph::update(&transaction, id.0 as i64)?;
        transaction.commit().map_err(Error::from)?;
        Ok(ReplyHeaders {
            message_id,
            references: parsed.references,
            in_reply_to: parsed.in_reply_to,
            addresses,
        })
    }

    pub fn message_account(&self, id: MessageId) -> crate::Result<String> {
        self.connection
            .query_row(
                "SELECT account FROM messages WHERE id=?1",
                [id.0 as i64],
                |row| row.get(0),
            )
            .optional()?
            .ok_or(Error::NotFound)
    }
}
