use {
    crate::{
        Submission, SubmissionFailure,
        jmap::{Client, JmapError, Result, field},
        jmap_writes::pointer,
    },
    serde_json::{Value, json},
};
#[derive(Debug, thiserror::Error)]
pub enum SubmissionError {
    #[error("JMAP submission preparation failed")]
    Prepare(#[from] JmapError),
    #[error("JMAP submission outcome is uncertain; inspect Sent before retrying")]
    Unknown(#[source] JmapError),
}
fn lookup<'a>(v: &'a Value, key: &str, expected: &str) -> Result<&'a str> {
    let mut matches = v["list"]
        .as_array()
        .ok_or(JmapError::Protocol)?
        .iter()
        .filter(|v| {
            v[key]
                .as_str()
                .is_some_and(|s| s.eq_ignore_ascii_case(expected))
        });
    let id = field(matches.next().ok_or(JmapError::Rejected)?, "id")?;
    if matches.next().is_some() {
        return Err(JmapError::Rejected);
    }
    Ok(id)
}
impl Submission for Client {
    type Error = SubmissionError;
    fn failure(error: &Self::Error) -> SubmissionFailure {
        match error {
            SubmissionError::Unknown(_) => SubmissionFailure::Unknown,
            SubmissionError::Prepare(_) => SubmissionFailure::NotSubmitted,
        }
    }
    fn submit(
        &mut self,
        sender: &str,
        recipients: &[String],
        raw: &[u8],
    ) -> std::result::Result<(), Self::Error> {
        let identities = self.method("Identity/get", json!({}))?;
        let identity = lookup(&identities, "email", sender)?;
        let folders = self.method("Mailbox/get", json!({"properties":["id","role"]}))?;
        let drafts = lookup(&folders, "role", "drafts")?;
        let sent = lookup(&folders, "role", "sent")?;
        let blob = self.upload(raw)?;
        let imported = self.method(
            "Email/import",
            json!({
                "emails": {
                    "message": {
                        "blobId": blob,
                        "mailboxIds": { drafts: true },
                        "keywords": { "$draft": true, "$seen": true }
                    }
                }
            }),
        )?;
        let email = field(&imported["created"]["message"], "id")?;
        let rcpt = recipients
            .iter()
            .map(|r| json!({"email":r}))
            .collect::<Vec<_>>();
        let submission = json!({
            "create": {
                "send": {
                    "emailId": email,
                    "identityId": identity,
                    "envelope": { "mailFrom": { "email": sender }, "rcptTo": rcpt }
                }
            },
            "onSuccessUpdateEmail": {
                "#send": {
                    format!("mailboxIds/{}", pointer(drafts)): null,
                    format!("mailboxIds/{}", pointer(sent)): true,
                    "keywords/$draft": null
                }
            }
        });
        let response =
            self.method("EmailSubmission/set", submission)
                .map_err(|error| match error {
                    JmapError::Rejected | JmapError::Unauthorized => {
                        SubmissionError::Prepare(error)
                    },
                    _ => SubmissionError::Unknown(error),
                })?;
        if response["notCreated"].get("send").is_some() {
            return Err(JmapError::Rejected.into());
        }
        field(&response["created"]["send"], "id").map_err(SubmissionError::Unknown)?;
        Ok(())
    }
}

impl Client {
    pub fn check_identity(&self, address: &str) -> Result<()> {
        let identities = self.method("Identity/get", json!({}))?;
        lookup(&identities, "email", address)?;
        Ok(())
    }
}
