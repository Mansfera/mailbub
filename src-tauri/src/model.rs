use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Gmail,
    Microsoft,
    Imap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    /// Implicit TLS (IMAP 993, SMTP 465).
    Tls,
    /// Plain connection upgraded with STARTTLS (SMTP 587).
    Starttls,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthKind {
    Password,
    Oauth,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Server {
    pub host: String,
    pub port: u16,
    pub security: Security,
}

/// Mirrors `ACCOUNT_COLORS` in the UI.
pub const ACCOUNT_COLORS: &[&str] = &["red", "teal", "gold", "plum", "olive", "slate"];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub provider: Provider,
    pub label: String,
    pub email: String,
    /// Login name for IMAP/SMTP; usually the address, but not always.
    pub username: String,
    pub display_name: String,
    /// Other addresses of this mailbox, lowercase: it receives at them and can send from them.
    pub aliases: Vec<String>,
    /// The address new messages go out from: `email` or one of `aliases`.
    pub default_from: String,
    pub color: String,
    pub imap: Server,
    pub smtp: Server,
    pub auth: AuthKind,
    pub created_at: i64,
}

/// What the setup form sends; the id, auth kind and timestamps are filled in by the core.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDraft {
    pub provider: Provider,
    pub label: String,
    pub email: String,
    /// Empty means "same as the address".
    #[serde(default)]
    pub username: String,
    pub display_name: String,
    /// Other addresses of the mailbox, as Apple Mail lists them.
    #[serde(default)]
    pub aliases: Vec<String>,
    pub color: String,
    pub imap: Server,
    pub smtp: Server,
}

/// What the account settings form sends. The address and provider are fixed once added.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountUpdate {
    pub label: String,
    pub display_name: String,
    pub aliases: Vec<String>,
    /// Anything but an address of the account means the main address.
    pub default_from: String,
    pub color: String,
    /// Empty means "same as the address".
    #[serde(default)]
    pub username: String,
    pub imap: Server,
    pub smtp: Server,
    /// A new password; `None` keeps the stored one. Password accounts only.
    #[serde(default)]
    pub password: Option<String>,
}

/// Credentials from the setup form. OAuth tokens never travel through the UI:
/// it only passes the id of a finished sign-in held by the core.
#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum SecretInput {
    Password { password: String },
    Oauth { session: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AddAccountResult {
    pub account: Account,
    /// Set when IMAP worked but the SMTP check failed: receiving works, sending may not.
    pub smtp_warning: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Discovered {
    /// Which connector fits this address better, if not plain IMAP.
    pub provider: Provider,
    pub imap: Option<Server>,
    pub smtp: Option<Server>,
}

/// An account found in Apple Mail, resolved to a connector and servers where possible.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportCandidate {
    /// Account name in Mail, used as the label.
    pub name: String,
    pub email: String,
    /// Other addresses on the same Mail account (aliases).
    pub aliases: Vec<String>,
    pub username: String,
    pub display_name: String,
    pub provider: Provider,
    pub imap: Option<Server>,
    pub smtp: Option<Server>,
    pub already_connected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    pub name: String,
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub from: Person,
    pub outgoing: bool,
    /// Which of the account's addresses an incoming message was sent to, when that is known.
    pub own_address: Option<String>,
    /// Unix seconds.
    pub sent_at: i64,
    pub subject: String,
    pub body: String,
    pub quoted: Option<String>,
    /// Only filled when a chat is opened; chat lists leave it out to stay small.
    pub html: Option<String>,
    /// Calendar invitation text; like `html`, only filled when a chat is opened.
    pub ics: Option<String>,
    pub attachments: Vec<Attachment>,
    pub read: bool,
    /// Mailing-list unsubscribe options; like `html`, only filled when a chat is opened.
    pub unsubscribe: Option<crate::unsubscribe::Unsubscribe>,
    /// Id of the message in the same chat this one replies to; only filled when a chat is
    /// opened.
    pub reply_to: Option<String>,
}

/// User-set state of a chat, kept apart from synced mail so a resync never loses it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatState {
    pub pinned: bool,
    pub muted: bool,
    pub archived: bool,
    pub manual_status: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSummary {
    pub id: String,
    pub account_id: String,
    pub participants: Vec<Person>,
    pub last: Message,
    pub unread: u32,
    /// Only machines wrote here (newsletters, receipts) and you never replied, or the chat
    /// is with no-reply addresses only: shown as Done.
    pub automated: bool,
    #[serde(flatten)]
    pub state: ChatState,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncError {
    pub account_id: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub running: bool,
    /// Unix seconds of the last finished run.
    pub last_run: Option<i64>,
    pub errors: Vec<SyncError>,
}
