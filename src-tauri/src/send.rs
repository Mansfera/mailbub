//! Sending mail over SMTP: replies in a chat and new messages. The sent message goes into
//! the local DB at once (see `db::insert_sent`) and, except on Gmail, which files sent mail
//! itself, into the server's Sent folder with IMAP APPEND.

use std::path::Path;
use std::sync::{LazyLock, Mutex};
use std::time::SystemTime;

use base64::Engine;
use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, MessageBuilder, MultiPart, SinglePart};
use lettre::{AsyncTransport, Message};
use regex::Regex;
use rusqlite::Connection;
use serde::Deserialize;

use crate::auth;
use crate::connect::{self, open_imap};
use crate::db;
use crate::error::{AppError, ErrorCode, Result};
use crate::model::{Account, Person, Provider};
use crate::parse::Parsed;
use crate::sync::{all_mail_folder, list_folders, own_addresses, sent_folder};

/// References kept on a reply; RFC 5322 allows trimming long threads.
const MAX_REFERENCES: usize = 20;

/// Total size of the files on one message. Gmail and most servers stop at 25 MB, and the
/// base64 transfer encoding adds a third on top.
pub const MAX_ATTACHMENTS_BYTES: usize = 25 * 1024 * 1024;

/// A file attached in the UI; its bytes come over IPC as base64.
#[derive(Debug, Clone, Deserialize)]
pub struct FileUpload {
    pub name: String,
    pub mime: String,
    pub data: String,
}

#[derive(Debug, Clone)]
pub struct OutgoingFile {
    pub name: String,
    pub mime: String,
    pub data: Vec<u8>,
}

/// Decodes the UI's files and checks they fit on one message.
pub fn decode_files(uploads: Vec<FileUpload>) -> Result<Vec<OutgoingFile>> {
    let mut total = 0;
    let mut out = Vec::with_capacity(uploads.len());
    for u in uploads {
        let data = base64::engine::general_purpose::STANDARD
            .decode(u.data.as_bytes())
            .map_err(|_| AppError::new(ErrorCode::Invalid, format!("Couldn't read {}", u.name)))?;
        total += data.len();
        if total > MAX_ATTACHMENTS_BYTES {
            return Err(AppError::new(
                ErrorCode::Invalid,
                "Attachments are over 25 MB in total",
            ));
        }
        let name = match u.name.trim() {
            "" => "attachment".to_owned(),
            n => n.to_owned(),
        };
        out.push(OutgoingFile {
            name,
            mime: u.mime,
            data,
        });
    }
    Ok(out)
}

/// A message ready to build. Message-IDs are stored without angle brackets, as
/// `mail-parser` gives them.
#[derive(Debug, Clone)]
pub struct Draft {
    pub from: Person,
    pub to: Vec<Person>,
    pub subject: String,
    pub body: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub attachments: Vec<OutgoingFile>,
}

static REPLY_PREFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^\s*((re|fwd?|aw|wg|odp|sv|vs|ha|відп|пересл)\s*(\[\d+\])?\s*:\s*)+")
        .expect("valid regex")
});

/// Strips Re:/Fwd: style prefixes; mirrors `normalizeSubject` in the UI.
pub fn normalize_subject(subject: &str) -> String {
    REPLY_PREFIX.replace(subject, "").trim().to_owned()
}

pub fn reply_subject(subject: &str) -> String {
    match normalize_subject(subject) {
        s if s.is_empty() => String::new(),
        s => format!("Re: {s}"),
    }
}

fn bare_id(id: &str) -> &str {
    id.trim().trim_start_matches('<').trim_end_matches('>')
}

/// The References of a reply: the parent's References plus the parent itself.
pub fn thread_references(parent: Option<&str>, parent_refs: &[String]) -> Vec<String> {
    let mut refs: Vec<String> = Vec::new();
    for id in parent_refs.iter().map(|s| s.as_str()).chain(parent) {
        let id = bare_id(id);
        if !id.is_empty() && !refs.iter().any(|r| r == id) {
            refs.push(id.to_owned());
        }
    }
    if refs.len() > MAX_REFERENCES {
        // Keep the thread root and the most recent ones.
        refs.drain(1..refs.len() - (MAX_REFERENCES - 1));
    }
    refs
}

fn looks_like_email(s: &str) -> bool {
    matches!(s.split_once('@'), Some((user, domain))
        if !user.is_empty() && domain.contains('.') && !s.contains(char::is_whitespace))
}

/// Addresses from the To field: separated by commas, semicolons or spaces.
pub fn parse_recipients(input: &str) -> Result<Vec<Person>> {
    let mut out: Vec<Person> = Vec::new();
    for raw in input.split([',', ';', ' ', '\n', '\t']) {
        let address = raw.trim().trim_matches(['<', '>']).to_lowercase();
        if address.is_empty() {
            continue;
        }
        if !looks_like_email(&address) {
            return Err(AppError::new(
                ErrorCode::Invalid,
                format!("Not an email address: {address}"),
            ));
        }
        if !out.iter().any(|p| p.address == address) {
            out.push(Person {
                name: address.clone(),
                address,
            });
        }
    }
    if out.is_empty() {
        return Err(AppError::new(ErrorCode::Invalid, "Add a recipient"));
    }
    Ok(out)
}

fn mailbox(p: &Person) -> Result<Mailbox> {
    let address = p
        .address
        .parse()
        .map_err(|_| AppError::new(ErrorCode::Invalid, format!("Bad address: {}", p.address)))?;
    let name = (!p.name.is_empty() && p.name != p.address).then(|| p.name.clone());
    Ok(Mailbox::new(name, address))
}

/// A fresh Message-ID (without brackets) on the sender's domain.
pub fn new_message_id(from: &str) -> String {
    let domain = from.rsplit_once('@').map_or("mailbub.local", |(_, d)| d);
    format!("{}@{domain}", uuid::Uuid::new_v4())
}

pub fn build(draft: &Draft, message_id: &str, date: SystemTime) -> Result<Message> {
    let mut b: MessageBuilder = Message::builder()
        .from(mailbox(&draft.from)?)
        .subject(draft.subject.clone())
        .message_id(Some(format!("<{message_id}>")))
        .date(date)
        .user_agent("Mailbub".into());
    for p in &draft.to {
        b = b.to(mailbox(p)?);
    }
    if let Some(parent) = &draft.in_reply_to {
        b = b.in_reply_to(format!("<{}>", bare_id(parent)));
    }
    if !draft.references.is_empty() {
        let refs: Vec<String> = draft
            .references
            .iter()
            .map(|r| format!("<{}>", bare_id(r)))
            .collect();
        b = b.references(refs.join(" "));
    }
    let built = if draft.attachments.is_empty() {
        b.header(ContentType::TEXT_PLAIN).body(draft.body.clone())
    } else {
        let mut parts = MultiPart::mixed().singlepart(SinglePart::plain(draft.body.clone()));
        for f in &draft.attachments {
            let mime = ContentType::parse(&f.mime)
                .unwrap_or_else(|_| ContentType::parse("application/octet-stream").expect("valid"));
            parts = parts.singlepart(Attachment::new(f.name.clone()).body(f.data.clone(), mime));
        }
        b.multipart(parts)
    };
    built.map_err(|e| AppError::new(ErrorCode::Invalid, e.to_string()))
}

/// The sender as recipients see it: the account's display name and its default address.
pub fn sender(account: &Account) -> Person {
    Person {
        name: match account.display_name.trim() {
            "" => account.default_from.clone(),
            n => n.to_owned(),
        },
        address: account.default_from.clone(),
    }
}

/// The sender with a chosen address, which has to be one of the account's.
pub fn sender_as(account: &Account, from: Option<&str>) -> Result<Person> {
    let default = sender(account);
    let Some(address) = from.map(|f| f.trim().to_lowercase()) else {
        return Ok(default);
    };
    if address != account.email.to_lowercase() && !account.aliases.contains(&address) {
        return Err(AppError::new(
            ErrorCode::Invalid,
            format!("{address} is not an address of this account"),
        ));
    }
    let name = if default.name == default.address {
        address.clone()
    } else {
        default.name
    };
    Ok(Person { name, address })
}

/// A reply to the newest message of a chat, to everyone in it.
pub fn reply_draft(
    from: Person,
    ctx: &db::ReplyContext,
    body: &str,
    attachments: Vec<OutgoingFile>,
) -> Draft {
    Draft {
        from,
        to: ctx.others.clone(),
        subject: reply_subject(&ctx.subject),
        body: body.to_owned(),
        in_reply_to: ctx.message_id.as_deref().map(|id| bare_id(id).to_owned()),
        references: thread_references(ctx.message_id.as_deref(), &ctx.references),
        attachments,
    }
}

/// What the local DB stores for a sent message (same shape as a synced one). Attachment
/// ids come from parsing the built message, so they match the server's copy.
fn as_parsed(
    account: &Account,
    draft: &Draft,
    raw: &[u8],
    message_id: &str,
    sent_at: i64,
) -> Parsed {
    let own = own_addresses(account);
    let mut others: Vec<Person> = draft
        .to
        .iter()
        .filter(|p| !own.contains(&p.address))
        .cloned()
        .collect();
    if others.is_empty() {
        others.push(draft.from.clone());
    }
    let attachments = if draft.attachments.is_empty() {
        vec![]
    } else {
        crate::parse::parse(raw, &own, true, sent_at)
            .map(|p| p.attachments)
            .unwrap_or_default()
    };
    Parsed {
        message_id: Some(message_id.to_owned()),
        references: draft.references.clone(),
        from: draft.from.clone(),
        outgoing: true,
        others,
        own_address: None,
        subject: draft.subject.clone(),
        body: draft.body.trim_end().to_owned(),
        quoted: None,
        html: None,
        ics: None,
        attachments,
        sent_at,
        automated: false,
        unsubscribe: None,
    }
}

/// Sends over SMTP and stores the message locally, with its files in the attachment cache
/// so they open before sync brings the server's copy. Returns the chat key and the raw
/// message (for `save_to_sent`).
pub async fn send(
    db: &Mutex<Connection>,
    cache: &Path,
    account: &Account,
    draft: &Draft,
) -> Result<(String, Vec<u8>)> {
    if draft.body.trim().is_empty() && draft.attachments.is_empty() {
        return Err(AppError::new(ErrorCode::Invalid, "The message is empty"));
    }
    let message_id = new_message_id(&draft.from.address);
    let now = SystemTime::now();
    let message = build(draft, &message_id, now)?;
    let raw = message.formatted();

    let creds = auth::credentials(account).await?;
    connect::smtp_transport(&account.smtp, creds.login())?
        .send(message)
        .await
        .map_err(connect::smtp_error)?;

    let parsed = as_parsed(account, draft, &raw, &message_id, crate::oauth::now());
    let (chat_key, id) = {
        let conn = db
            .lock()
            .map_err(|_| AppError::new(ErrorCode::Storage, "Database lock poisoned"))?;
        db::insert_sent(&conn, &account.id, &parsed)?
    };
    // Parsed in the order they were added; a mismatch only costs a download later.
    if parsed.attachments.len() == draft.attachments.len() {
        for (a, f) in parsed.attachments.iter().zip(&draft.attachments) {
            if let Err(e) = crate::files::cache_attachment(cache, id, &a.id, &a.name, &f.data) {
                eprintln!("[send] caching {}: {e}", a.name);
            }
        }
    }
    Ok((chat_key, raw))
}

/// Puts a copy of a sent message in the Sent folder, marked read. Gmail does this itself.
pub async fn save_to_sent(account: &Account, raw: &[u8]) -> Result<()> {
    if account.provider == Provider::Gmail {
        return Ok(());
    }
    let creds = auth::credentials(account).await?;
    let mut session = open_imap(&account.imap, creds.login()).await?;
    let folders = list_folders(&mut session).await?;
    let result = if all_mail_folder(&folders).is_some() {
        Ok(()) // Gmail added as a plain IMAP account
    } else {
        match sent_folder(&folders) {
            Some(sent) => session
                .append(&sent, Some("(\\Seen)"), None, raw)
                .await
                .map_err(|e| AppError::new(ErrorCode::ImapConnect, format!("APPEND: {e}"))),
            None => Err(AppError::new(ErrorCode::NotFound, "No Sent folder")),
        }
    };
    let _ = session.logout().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use mail_parser::MessageParser;

    fn person(name: &str, address: &str) -> Person {
        Person {
            name: name.into(),
            address: address.into(),
        }
    }

    fn account() -> Account {
        use crate::model::{AuthKind, Security, Server};
        let server = Server {
            host: "mail.example.com".into(),
            port: 993,
            security: Security::Tls,
        };
        Account {
            id: "acc".into(),
            provider: Provider::Imap,
            label: "Work".into(),
            email: "me@example.com".into(),
            username: "me@example.com".into(),
            display_name: "Alex Novak".into(),
            aliases: vec![],
            default_from: "me@example.com".into(),
            color: "teal".into(),
            imap: server.clone(),
            smtp: server,
            auth: AuthKind::Password,
            created_at: 0,
        }
    }

    #[test]
    fn reply_subjects() {
        assert_eq!(reply_subject("Proposal"), "Re: Proposal");
        assert_eq!(reply_subject("RE: Re[2]: Fwd: Proposal"), "Re: Proposal");
        assert_eq!(reply_subject("Відп: Зустріч"), "Re: Зустріч");
        assert_eq!(reply_subject("  "), "");
        assert_eq!(normalize_subject("Reading list"), "Reading list");
    }

    #[test]
    fn references_chain_and_trim() {
        assert_eq!(
            thread_references(Some("<c@x>"), &["a@x".into(), "<b@x>".into()]),
            ["a@x", "b@x", "c@x"]
        );
        assert_eq!(thread_references(Some("a@x"), &["a@x".into()]), ["a@x"]);
        assert!(thread_references(None, &[]).is_empty());
        let long: Vec<String> = (0..30).map(|i| format!("m{i}@x")).collect();
        let refs = thread_references(Some("last@x"), &long);
        assert_eq!(refs.len(), MAX_REFERENCES);
        assert_eq!(refs[0], "m0@x", "keeps the thread root");
        assert_eq!(refs.last().unwrap(), "last@x");
    }

    #[test]
    fn recipients() {
        let to = parse_recipients("Bob@X.sk, <ann@y.sk>; bob@x.sk\ncid@z.ua").unwrap();
        assert_eq!(
            to.iter().map(|p| p.address.as_str()).collect::<Vec<_>>(),
            ["bob@x.sk", "ann@y.sk", "cid@z.ua"]
        );
        assert_eq!(
            parse_recipients(" , ").unwrap_err().code,
            ErrorCode::Invalid
        );
        assert!(parse_recipients("bob@x.sk, nope").is_err());
    }

    #[test]
    fn reply_is_threaded_to_everyone_in_the_chat() {
        let ctx = db::ReplyContext {
            message_id: Some("p@studio.sk".into()),
            references: vec!["root@studio.sk".into()],
            subject: "Re: Proposal".into(),
            others: vec![
                person("Martin Kováč", "martin@studio.sk"),
                person("ann@studio.sk", "ann@studio.sk"),
            ],
        };
        let draft = reply_draft(sender(&account()), &ctx, "Ďakujem, platí.\nAlex", vec![]);
        let msg = build(&draft, "new@example.com", SystemTime::UNIX_EPOCH).unwrap();
        let raw = msg.formatted();
        let parsed = MessageParser::default().parse(&raw).unwrap();

        assert_eq!(parsed.subject(), Some("Re: Proposal"));
        assert_eq!(parsed.message_id(), Some("new@example.com"));
        assert_eq!(parsed.in_reply_to().as_text(), Some("p@studio.sk"));
        assert_eq!(
            parsed
                .references()
                .as_text_list()
                .unwrap()
                .iter()
                .map(|s| s.as_ref())
                .collect::<Vec<_>>(),
            ["root@studio.sk", "p@studio.sk"]
        );
        let from = parsed.from().unwrap().first().unwrap();
        assert_eq!(from.name(), Some("Alex Novak"));
        assert_eq!(from.address(), Some("me@example.com"));
        let to: Vec<_> = parsed
            .to()
            .unwrap()
            .iter()
            .map(|a| a.address().unwrap().to_owned())
            .collect();
        assert_eq!(to, ["martin@studio.sk", "ann@studio.sk"]);
        assert_eq!(
            parsed
                .body_text(0)
                .unwrap()
                .replace("\r\n", "\n")
                .trim_end(),
            "Ďakujem, platí.\nAlex"
        );

        // Stored locally in the same chat the reply belongs to.
        let local = as_parsed(&account(), &draft, &raw, "new@example.com", 5);
        assert!(local.outgoing);
        assert_eq!(
            crate::parse::chat_key("acc", &local.others),
            "acc|ann@studio.sk,martin@studio.sk"
        );
        // And the server's copy parses into that same chat.
        let synced =
            crate::parse::parse(&raw, &["me@example.com".into()], true, 0).expect("parses");
        assert_eq!(
            crate::parse::chat_key("acc", &synced.others),
            "acc|ann@studio.sk,martin@studio.sk"
        );
        assert_eq!(synced.message_id.as_deref(), Some("new@example.com"));
    }

    #[test]
    fn new_message_has_no_thread_headers() {
        let draft = Draft {
            from: sender(&account()),
            to: vec![person("bob@x.sk", "bob@x.sk")],
            subject: "Hello".into(),
            body: "Hi".into(),
            in_reply_to: None,
            references: vec![],
            attachments: vec![],
        };
        let raw = build(&draft, "n@example.com", SystemTime::UNIX_EPOCH)
            .unwrap()
            .formatted();
        let text = String::from_utf8_lossy(&raw);
        assert!(!text.contains("In-Reply-To"));
        assert!(!text.contains("References"));
        assert!(new_message_id("me@example.com").ends_with("@example.com"));

        // Writing to yourself: the chat is with yourself, as for synced mail.
        let mut to_self = draft.clone();
        to_self.to = vec![person("me@example.com", "me@example.com")];
        let local = as_parsed(&account(), &to_self, &raw, "s@example.com", 0);
        assert_eq!(local.others[0].address, "me@example.com");
    }

    #[test]
    fn attachments_round_trip() {
        let files = decode_files(vec![
            FileUpload {
                name: "Zmluva Alex.pdf".into(),
                mime: "application/pdf".into(),
                data: "JVBERi0xLjQK".into(), // "%PDF-1.4\n"
            },
            FileUpload {
                name: "звіт 2026.txt".into(),
                mime: String::new(), // the browser didn't know the type
                data: "0L/RgNC40LLRltGC".into(), // "привіт"
            },
        ])
        .unwrap();
        let draft = Draft {
            from: sender(&account()),
            to: vec![person("bob@x.sk", "bob@x.sk")],
            subject: "Docs".into(),
            body: String::new(),
            in_reply_to: None,
            references: vec![],
            attachments: files,
        };
        let raw = build(&draft, "a@example.com", SystemTime::UNIX_EPOCH)
            .unwrap()
            .formatted();
        let local = as_parsed(&account(), &draft, &raw, "a@example.com", 0);
        let names: Vec<_> = local.attachments.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Zmluva Alex.pdf", "звіт 2026.txt"]);
        let (name, bytes) = crate::parse::attachment_file(&raw, &local.attachments[1].id).unwrap();
        assert_eq!(name, "звіт 2026.txt");
        assert_eq!(bytes, "привіт".as_bytes());
        // The synced copy gets the same ids, so cached files stay valid.
        let synced = crate::parse::parse(&raw, &["me@example.com".into()], true, 0).unwrap();
        assert_eq!(
            synced.attachments.iter().map(|a| &a.id).collect::<Vec<_>>(),
            local.attachments.iter().map(|a| &a.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn rejects_bad_or_oversized_files() {
        let upload = |data: String| FileUpload {
            name: "a.bin".into(),
            mime: "application/octet-stream".into(),
            data,
        };
        assert!(decode_files(vec![upload("not base64!".into())]).is_err());
        let big = base64::engine::general_purpose::STANDARD.encode(vec![0u8; 13 * 1024 * 1024]);
        assert!(decode_files(vec![upload(big.clone())]).is_ok());
        assert!(decode_files(vec![upload(big.clone()), upload(big)]).is_err());
    }

    #[test]
    fn sends_from_an_alias_into_the_same_chat() {
        let mut a = account();
        a.aliases = vec!["hi@alias.example".into()];
        let from = sender_as(&a, Some(" Hi@Alias.example ")).unwrap();
        assert_eq!(from.address, "hi@alias.example");
        assert_eq!(from.name, "Alex Novak");
        assert_eq!(sender_as(&a, None).unwrap().address, "me@example.com");
        assert_eq!(
            sender_as(&a, Some("me@example.com")).unwrap().address,
            "me@example.com"
        );
        assert_eq!(
            sender_as(&a, Some("other@example.com")).unwrap_err().code,
            ErrorCode::Invalid
        );

        a.default_from = "hi@alias.example".into();
        assert_eq!(sender(&a).address, "hi@alias.example");

        let draft = Draft {
            from,
            to: vec![
                person("bob@x.sk", "bob@x.sk"),
                person("hi@alias.example", "hi@alias.example"),
            ],
            subject: "Hello".into(),
            body: "Hi".into(),
            in_reply_to: None,
            references: vec![],
            attachments: vec![],
        };
        let raw = build(&draft, "n@alias.example", SystemTime::UNIX_EPOCH)
            .unwrap()
            .formatted();
        let parsed = MessageParser::default().parse(&raw).unwrap();
        assert_eq!(
            parsed.from().unwrap().first().unwrap().address(),
            Some("hi@alias.example")
        );
        let local = as_parsed(&a, &draft, &raw, "n@alias.example", 0);
        assert_eq!(crate::parse::chat_key("acc", &local.others), "acc|bob@x.sk");
        let synced = crate::parse::parse(&raw, &own_addresses(&a), false, 0).unwrap();
        assert!(synced.outgoing);
        assert_eq!(
            crate::parse::chat_key("acc", &synced.others),
            "acc|bob@x.sk"
        );
    }

    #[test]
    fn sender_falls_back_to_address() {
        let mut a = account();
        a.display_name = " ".into();
        assert_eq!(sender(&a).name, "me@example.com");
    }
}
