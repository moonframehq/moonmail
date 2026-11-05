# Gmail testing — setup for later

The optional live test is `.github/workflows/gmail-live.yml`. It complements the
local test mail daemon; ordinary protocol CI needs no Google account or secrets.

## Accounts and secrets

1. Create **two dedicated consumer Gmail accounts**, A and B. Use primary addresses;
   dots, `+tags`, and `googlemail.com` aliases do not create a second mailbox.
2. Enable **2-Step Verification** on each account.
3. Generate an **app password** for Moonmail testing on each account. Some Google
   account policies do not allow app passwords; a normal login password will not work.
4. Add these repository **Settings → Secrets and variables → Actions** secrets:

   | Secret | Value |
   | --- | --- |
   | `GMAIL_TEST_A_EMAIL` | A's primary address |
   | `GMAIL_TEST_A_APP_PASSWORD` | A's app password, without display spaces |
   | `GMAIL_TEST_B_EMAIL` | B's primary address |
   | `GMAIL_TEST_B_APP_PASSWORD` | B's app password, without display spaces |

Keep both mailboxes small and synthetic. Enter passwords locally or into GitHub's
secret fields, not source files or chat.

## Run it

After publishing the workflow on the default branch:

- **Actions → Gmail live round trip → Run workflow** starts a manual run.
- Set Actions variable **`GMAIL_LIVE_ENABLED=true`** to enable daily runs at
  **07:23 UTC**. Remove it or set it to `false` to stop scheduled runs.
- Runs are serialized. Only default-branch code receives credentials; pull requests
  do not trigger the live workflow.

Locally, export the same four secrets in your shell, then run:

```sh
MOONMAIL_LIVE_GMAIL=1 cargo test --locked -p moonmail-network --test gmail_live gmail_round_trip -- --ignored --exact --nocapture
```

Normal `just ci` compiles but ignores this live test. Missing credentials or missing
explicit opt-in fail the requested live run rather than silently passing it.

## What it checks

- IMAP and SMTP authentication for both accounts.
- A → B delivery with a UTF-8 body and a synthetic attachment.
- Moonmail's reply construction from the downloaded message and B → A delivery.
- Exact Message-IDs, sender/recipient, subject/body, attachment bytes, and reply ancestry.
- One observed Sent copy for each message and Outbox reconciliation.
- New connections and reopened disposable local archives.

Only the two configured addresses can receive test submissions. SMTP is attempted
once per message; polling for delivery has an eight-minute deadline. Logs contain
phase labels and a synthetic run UUID, not provider replies or message bodies.

Messages remain for inspection with subjects beginning **`[Moonmail CI]`** followed
by the run UUID. No mail is deleted. Search that subject/run ID in Gmail to inspect
or manually remove test messages. Check Spam if a delivery times out. Rate limits,
temporary provider outages, and Google policy can still make a live run fail.

## Browser OAuth is a separate check

App-password tests do **not** validate OAuth consent, refresh tokens, Keychain, or
the GUI. For browser sign-in:

1. Create a Google Cloud OAuth client of type **Desktop app**.
2. Configure the consent screen and Gmail scope `https://mail.google.com/`; add the
   test accounts as test users if the application is in testing.
3. Download client JSON. In Moonmail choose **Settings → Internet Accounts → Add
   Account → Google → Import OAuth client…**, then **Sign in with Google**.
4. Sync, read, send a test reply, inspect Sent, restart, and sync again. Test token
   refresh separately after token expiry. Verify Archive preserves mail in All Mail.

Google testing-mode grants may expire after seven days. A distributed application
using the restricted Gmail scope needs Google's verification process. No shared
Google client is bundled.

References: [desktop OAuth](https://developers.google.com/identity/protocols/oauth2/native-app),
[Gmail XOAUTH2](https://developers.google.com/workspace/gmail/imap/xoauth2-protocol).

Status: the live suite is implemented; authenticated Gmail validation still awaits
account creation and credentials.
