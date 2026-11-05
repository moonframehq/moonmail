use super::*;

impl Mail {
    pub(super) fn identity_settings_account(&self) -> Result<String> {
        if let Some(account) = &self.identity_account
            && self
                .sending_accounts()?
                .iter()
                .any(|item| item.email == *account)
        {
            return Ok(account.clone());
        }
        self.configured_account()
    }

    pub(super) fn sending_accounts(&self) -> Result<Vec<Account>> {
        let mut accounts = Account::load_all(&self.root)?;
        if accounts.is_empty() && self.demo {
            accounts.push(Account::fastmail("you@example.com")?);
        }
        Ok(accounts)
    }

    pub(super) fn sender_snapshot(&self, snapshot: &mut serde_json::Value) -> Result<()> {
        let accounts = self.sending_accounts()?;
        snapshot["accounts"] = json!(accounts.iter().map(|a| &a.email).collect::<Vec<_>>());
        snapshot["account_details"] = json!(accounts);
        snapshot["account_presets"] = json!(
            [
                moonmail_network::Provider::Fastmail,
                moonmail_network::Provider::Google,
                moonmail_network::Provider::Microsoft,
                moonmail_network::Provider::Icloud,
                moonmail_network::Provider::Yahoo,
                moonmail_network::Provider::Aol,
                moonmail_network::Provider::Other,
            ]
            .into_iter()
            .map(|provider| Account::preset(provider, "you@example.com"))
            .collect::<moonmail_network::Result<Vec<_>>>()?
        );
        let mut choices = Vec::new();
        for account in accounts {
            let current = self
                .draft
                .as_ref()
                .is_some_and(|draft| draft.account == account.email && draft.identity.is_none());
            choices.push(json!({"account":account.email,"identity":"","label":account.email,"current":current}));
            for identity in self.archive()?.identities(&account.email)? {
                let current = self.draft.as_ref().is_some_and(|draft| {
                    draft.account == account.email
                        && draft.identity.as_ref().is_some_and(|saved| {
                            saved.id == identity.id
                                && saved.account == identity.account
                                && saved.name == identity.name
                                && saved.address == identity.address
                                && saved.reply_to == identity.reply_to
                                && saved.signature == identity.signature
                        })
                });
                let label = if identity.name.is_empty() {
                    identity.address.clone()
                } else {
                    format!("{} <{}>", identity.name, identity.address)
                };
                choices.push(json!({"account":account.email,"identity":identity.id,"label":label,"current":current}));
            }
        }
        snapshot["from_accounts"] = json!(choices);
        Ok(())
    }

    pub(super) fn configured_account(&self) -> Result<String> {
        Ok(Account::load(&self.root)?
            .map(|account| account.email)
            .unwrap_or_else(|| {
                if self.demo {
                    "you@example.com".into()
                } else {
                    String::new()
                }
            }))
    }

    pub(super) fn handle_identity(&mut self, action: &str, message: &Data) -> Result<()> {
        let text = |key| {
            message
                .get(key)
                .and_then(Data::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        if action == "identity_account" {
            let account = text("account");
            anyhow::ensure!(
                self.sending_accounts()?
                    .iter()
                    .any(|item| item.email == account),
                "Choose a connected account"
            );
            self.identity_account = Some(account);
            self.identity_new_id = uuid::Uuid::new_v4().to_string();
            return Ok(());
        }
        let account = self.identity_settings_account()?;
        anyhow::ensure!(
            !account.is_empty() && text("account") == account,
            "Identity settings belong to another account; reopen them"
        );
        let id = text("id");
        match action {
            "identity_save" => {
                let existing = if id == self.identity_new_id {
                    None
                } else {
                    Some(self.archive()?.identity(&account, &id)?)
                };
                let identity = moonmail_store::Identity {
                    id: id.clone(),
                    account,
                    name: text("name"),
                    address: text("address"),
                    reply_to: text("reply_to"),
                    signature: text("signature"),
                    is_default: existing
                        .as_ref()
                        .is_some_and(|identity| identity.is_default),
                };
                moonmail_network::validate_identity(&identity)?;
                self.archive_mut()?.save_identity(&identity)?;
                if id == self.identity_new_id {
                    self.identity_new_id = uuid::Uuid::new_v4().to_string();
                }
                self.status =
                    "Identity saved. Existing drafts retain their saved identity and body.".into();
            },
            "identity_default" => {
                let mut identity = self.archive()?.identity(&account, &id)?;
                identity.is_default = true;
                self.archive_mut()?.save_identity(&identity)?;
            },
            "identity_delete" => self.archive()?.delete_identity(&account, &id)?,
            _ => {},
        }
        Ok(())
    }
}
