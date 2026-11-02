use std::collections::HashMap;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Row, params};

use crate::error::{AppError, ErrorCode, Result};
use crate::model::{
    Account, AuthKind, ChatState, ChatSummary, Message, Person, Provider, Security, Server,
};
use crate::parse::Parsed;

/// Each entry upgrades the schema by one version (tracked in `PRAGMA user_version`).
const MIGRATIONS: &[&str] = &[
    r#"
    CREATE TABLE accounts (
        id            TEXT PRIMARY KEY,
        provider      TEXT NOT NULL,
        label         TEXT NOT NULL,
        email         TEXT NOT NULL UNIQUE COLLATE NOCASE,
        display_name  TEXT NOT NULL,
        color         TEXT NOT NULL,
        imap_host     TEXT NOT NULL,
        imap_port     INTEGER NOT NULL,
        imap_security TEXT NOT NULL,
        smtp_host     TEXT NOT NULL,
        smtp_port     INTEGER NOT NULL,
        smtp_security TEXT NOT NULL,
        auth          TEXT NOT NULL,
        created_at    INTEGER NOT NULL
    );
"#,
    r#"
    ALTER TABLE accounts ADD COLUMN username TEXT NOT NULL DEFAULT '';
"#,
    r#"
    CREATE TABLE folders (
        account_id   TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        name         TEXT NOT NULL,
        uid_validity INTEGER NOT NULL,
        last_uid     INTEGER NOT NULL,
        PRIMARY KEY (account_id, name)
    );
    CREATE TABLE messages (
        id          INTEGER PRIMARY KEY,
        account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
        folder      TEXT NOT NULL,
        uid         INTEGER NOT NULL,
        message_id  TEXT,
        chat_key    TEXT NOT NULL,
        outgoing    INTEGER NOT NULL,
        from_json   TEXT NOT NULL,
        others_json TEXT NOT NULL,
        subject     TEXT NOT NULL,
        body        TEXT NOT NULL,
        quoted      TEXT,
        attachments TEXT NOT NULL,
        sent_at     INTEGER NOT NULL,
        seen        INTEGER NOT NULL,
        UNIQUE (account_id, folder, uid)
    );
    CREATE INDEX messages_chat ON messages (chat_key, sent_at);
    CREATE INDEX messages_msgid ON messages (account_id, message_id);
    CREATE TABLE chat_state (
        chat_key      TEXT PRIMARY KEY,
        pinned        INTEGER NOT NULL DEFAULT 0,
        muted         INTEGER NOT NULL DEFAULT 0,
        archived      INTEGER NOT NULL DEFAULT 0,
        manual_status TEXT
    );
"#,
    r#"
    ALTER TABLE messages ADD COLUMN deleted_at INTEGER;
"#,
    // HTML bodies are new; drop the cache so the next sync refetches mail with them.
    // Chat state (pins, archive, status) lives in its own table and survives.
    r#"
    ALTER TABLE messages ADD COLUMN html TEXT;
    DELETE FROM messages;
    DELETE FROM folders;
    CREATE TABLE remote_rules (
        kind       TEXT NOT NULL,
        value      TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        PRIMARY KEY (kind, value)
    );
"#,
    // Calendar invitations are kept too; refetch so existing mail gets them.
    r#"
    ALTER TABLE messages ADD COLUMN ics TEXT;
    DELETE FROM messages;
    DELETE FROM folders;
"#,
    // Previews are now derived from HTML without scripts/JSON-LD; refetch to rebuild them.
    r#"
    DELETE FROM messages;
    DELETE FROM folders;
"#,
    // HTML entities (&scaron;, &bull;, …) are now decoded in previews; refetch.
    r#"
    DELETE FROM messages;
    DELETE FROM folders;
"#,
    // Attachments get stable ids (MIME part index) for opening; refetch.
    r#"
    DELETE FROM messages;
    DELETE FROM folders;
"#,
    // Archive state now comes from the server (INBOX vs Archive, Gmail's \Inbox label);
    // refetch so every message knows whether it is in the inbox.
    r#"
    ALTER TABLE messages ADD COLUMN in_inbox INTEGER NOT NULL DEFAULT 0;
    DELETE FROM messages;
    DELETE FROM folders;
"#,
    // Replies need the thread's References; read/unread changes wait in `seen_dirty` until
    // they reach the server. Older rows keep NULL refs (replies then cite the parent only).
    r#"
    ALTER TABLE messages ADD COLUMN refs TEXT;
    ALTER TABLE messages ADD COLUMN seen_dirty INTEGER NOT NULL DEFAULT 0;
"#,
    // Automated mail (newsletters, receipts, no-reply) is flagged so its chats default to
    // Done, and entities in plain-text parts are decoded; refetch.
    r#"
    ALTER TABLE messages ADD COLUMN automated INTEGER;
    DELETE FROM messages;
    DELETE FROM folders;
"#,
    // Full-text search over all cached mail. Kept in sync by triggers, so every way a message
    // is added or removed (sync, delete, account removal via cascade) updates the index.
    // Diacritics are folded so "zlava" finds "zľava"; the cache is indexed as it is.
    r#"
    CREATE VIRTUAL TABLE messages_fts USING fts5(
        subject, body, sender, participants,
        tokenize = 'unicode61 remove_diacritics 2'
    );
    CREATE TRIGGER messages_fts_insert AFTER INSERT ON messages BEGIN
        INSERT INTO messages_fts (rowid, subject, body, sender, participants)
        VALUES (new.id, new.subject, new.body,
            (new.from_json ->> '$.name') || ' ' || (new.from_json ->> '$.address'),
            (SELECT group_concat((value ->> 'name') || ' ' || (value ->> 'address'), ' ')
               FROM json_each(new.others_json)));
    END;
    CREATE TRIGGER messages_fts_delete AFTER DELETE ON messages BEGIN
        DELETE FROM messages_fts WHERE rowid = old.id;
    END;
    CREATE TRIGGER messages_fts_update AFTER UPDATE OF subject, body, from_json, others_json
    ON messages BEGIN
        DELETE FROM messages_fts WHERE rowid = old.id;
        INSERT INTO messages_fts (rowid, subject, body, sender, participants)
        VALUES (new.id, new.subject, new.body,
            (new.from_json ->> '$.name') || ' ' || (new.from_json ->> '$.address'),
            (SELECT group_concat((value ->> 'name') || ' ' || (value ->> 'address'), ' ')
               FROM json_each(new.others_json)));
    END;
    INSERT INTO messages_fts (rowid, subject, body, sender, participants)
    SELECT id, subject, body,
        (from_json ->> '$.name') || ' ' || (from_json ->> '$.address'),
        (SELECT group_concat((value ->> 'name') || ' ' || (value ->> 'address'), ' ')
           FROM json_each(others_json))
    FROM messages;
"#,
    // Older mail is fetched on demand: `min_uid` is the lowest UID the cache covers in each
    // folder (1 = nothing older left). Existing folders start from what they already hold.
    r#"
    ALTER TABLE folders ADD COLUMN min_uid INTEGER NOT NULL DEFAULT 1;
    UPDATE folders SET min_uid = COALESCE(
        (SELECT MIN(uid) FROM messages m
          WHERE m.account_id = folders.account_id AND m.folder = folders.name),
        1);
"#,
    // A chat with only your own mail has nothing in the inbox to archive on the server, so
    // archiving it is kept here. NULL = automatic (see `list_chats`).
    r#"
    ALTER TABLE chat_state ADD COLUMN local_archived INTEGER;
"#,
    // List-Unsubscribe headers (JSON `Unsubscribe`). Older rows stay NULL; the UI then looks
    // for an unsubscribe link in the body.
    r#"
    ALTER TABLE messages ADD COLUMN unsubscribe TEXT;
"#,
    // Other addresses of a mailbox and the one new mail goes out from (empty = the main
    // address). `own_address` says which of them an incoming message was sent to.
    r#"
    ALTER TABLE accounts ADD COLUMN aliases TEXT NOT NULL DEFAULT '[]';
    ALTER TABLE accounts ADD COLUMN default_from TEXT NOT NULL DEFAULT '';
    ALTER TABLE messages ADD COLUMN own_address TEXT;
"#,
];

/// A chat with only your own mail stays in the list this long after the last message
/// (waiting for a reply), then counts as archived like it is in other mail apps.
const SENT_ONLY_DAYS: i64 = 14;

/// Folder of messages sent from Mailbub that sync has not seen on the server yet. No IMAP
/// folder has an empty name, so server actions and reconciling skip these rows.
pub const LOCAL_FOLDER: &str = "";

/// The database file in the app data folder (plus its `-wal` and `-shm` companions).
pub const FILE: &str = "mailbub.db";

pub fn open(path: &Path) -> Result<Connection> {
    let mut conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    migrate(&mut conn)?;
    Ok(conn)
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", (i + 1) as i64)?;
        tx.commit()?;
    }
    Ok(())
}

fn to_text<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn from_text<T: serde::de::DeserializeOwned>(s: String) -> rusqlite::Result<T> {
    serde_json::from_value(serde_json::Value::String(s)).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, e.into())
    })
}

fn row_to_account(r: &Row) -> rusqlite::Result<Account> {
    Ok(Account {
        id: r.get("id")?,
        provider: from_text::<Provider>(r.get("provider")?)?,
        label: r.get("label")?,
        email: r.get("email")?,
        username: {
            let u: String = r.get("username")?;
            if u.is_empty() { r.get("email")? } else { u }
        },
        display_name: r.get("display_name")?,
        aliases: from_json(r.get("aliases")?)?,
        default_from: {
            let d: String = r.get("default_from")?;
            if d.is_empty() { r.get("email")? } else { d }
        },
        color: r.get("color")?,
        imap: Server {
            host: r.get("imap_host")?,
            port: r.get("imap_port")?,
            security: from_text::<Security>(r.get("imap_security")?)?,
        },
        smtp: Server {
            host: r.get("smtp_host")?,
            port: r.get("smtp_port")?,
            security: from_text::<Security>(r.get("smtp_security")?)?,
        },
        auth: from_text::<AuthKind>(r.get("auth")?)?,
        created_at: r.get("created_at")?,
    })
}

pub fn list_accounts(conn: &Connection) -> Result<Vec<Account>> {
    let mut stmt = conn.prepare("SELECT * FROM accounts ORDER BY created_at")?;
    let rows = stmt.query_map([], row_to_account)?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn email_exists(conn: &Connection, email: &str) -> Result<bool> {
    Ok(conn
        .query_row("SELECT 1 FROM accounts WHERE email = ?1", [email], |_| {
            Ok(())
        })
        .optional()?
        .is_some())
}

pub fn insert_account(conn: &Connection, a: &Account) -> Result<()> {
    conn.execute(
        "INSERT INTO accounts (id, provider, label, email, display_name, color,
            imap_host, imap_port, imap_security, smtp_host, smtp_port, smtp_security, auth, created_at,
            username, aliases, default_from)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            a.id,
            to_text(&a.provider),
            a.label,
            a.email,
            a.display_name,
            a.color,
            a.imap.host,
            a.imap.port,
            to_text(&a.imap.security),
            a.smtp.host,
            a.smtp.port,
            to_text(&a.smtp.security),
            to_text(&a.auth),
            a.created_at,
            a.username,
            json(&a.aliases),
            a.default_from,
        ],
    )
    .map_err(|e| match e {
        rusqlite::Error::SqliteFailure(f, _)
            if f.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            AppError::new(ErrorCode::Duplicate, "This address is already connected")
        }
        e => e.into(),
    })?;
    Ok(())
}

pub fn get_account(conn: &Connection, id: &str) -> Result<Account> {
    conn.query_row("SELECT * FROM accounts WHERE id = ?1", [id], row_to_account)
        .optional()?
        .ok_or_else(|| AppError::new(ErrorCode::NotFound, "Account not found"))
}

/// Saves the editable fields; the address, provider, auth kind and creation time stay.
pub fn update_account(conn: &Connection, a: &Account) -> Result<()> {
    let n = conn.execute(
        "UPDATE accounts SET label = ?2, display_name = ?3, color = ?4, username = ?5,
            imap_host = ?6, imap_port = ?7, imap_security = ?8,
            smtp_host = ?9, smtp_port = ?10, smtp_security = ?11,
            aliases = ?12, default_from = ?13
         WHERE id = ?1",
        params![
            a.id,
            a.label,
            a.display_name,
            a.color,
            a.username,
            a.imap.host,
            a.imap.port,
            to_text(&a.imap.security),
            a.smtp.host,
            a.smtp.port,
            to_text(&a.smtp.security),
            json(&a.aliases),
            a.default_from,
        ],
    )?;
    if n == 0 {
        return Err(AppError::new(ErrorCode::NotFound, "Account not found"));
    }
    Ok(())
}

/// Files cached mail again after addresses became the account's own, as sync would now: mail
/// from them is outgoing, and they no longer count as people in a chat. Pins, archive and
/// status follow a chat to its new key. Returns how many messages changed.
pub fn claim_addresses(conn: &Connection, account_id: &str, own: &[String]) -> Result<usize> {
    type Row = (i64, String, bool, Person, Vec<Person>, Option<String>);
    let tx = conn.unchecked_transaction()?;
    let rows: Vec<Row> = tx
        .prepare(
            "SELECT id, chat_key, outgoing, from_json, others_json, own_address
             FROM messages WHERE account_id = ?1",
        )?
        .query_map([account_id], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                from_json(r.get(3)?)?,
                from_json(r.get(4)?)?,
                r.get(5)?,
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut changed = 0;
    for (id, old_key, was_outgoing, from, others, own_address) in rows {
        let (mine, mut rest): (Vec<Person>, Vec<Person>) =
            others.into_iter().partition(|p| own.contains(&p.address));
        let outgoing = was_outgoing || own.contains(&from.address);
        if mine.is_empty() && outgoing == was_outgoing {
            continue;
        }
        if rest.is_empty() {
            // A note to self: the chat is with yourself.
            rest.push(from);
        }
        let own_address = match outgoing {
            true => None,
            false => own_address.or(mine.into_iter().next().map(|p| p.address)),
        };
        let key = crate::parse::chat_key(account_id, &rest);
        tx.execute(
            "UPDATE messages SET chat_key = ?2, outgoing = ?3, others_json = ?4, own_address = ?5,
                seen = seen OR ?3, automated = automated AND NOT ?3
             WHERE id = ?1",
            params![id, key, outgoing, json(&rest), own_address],
        )?;
        if key != old_key {
            tx.execute(
                "UPDATE OR IGNORE chat_state SET chat_key = ?2 WHERE chat_key = ?1",
                params![old_key, key],
            )?;
        }
        changed += 1;
    }
    tx.commit()?;
    Ok(changed)
}

pub fn delete_account(conn: &Connection, id: &str) -> Result<()> {
    let n = conn.execute("DELETE FROM accounts WHERE id = ?1", [id])?;
    if n == 0 {
        return Err(AppError::new(ErrorCode::NotFound, "Account not found"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FolderState {
    pub uid_validity: u32,
    pub last_uid: u32,
    /// Lowest UID the cache covers; everything from here up to `last_uid` was fetched.
    /// 1 means there is nothing older on the server.
    pub min_uid: u32,
}

pub fn folder_state(
    conn: &Connection,
    account_id: &str,
    name: &str,
) -> Result<Option<FolderState>> {
    Ok(conn
        .query_row(
            "SELECT uid_validity, last_uid, min_uid FROM folders
             WHERE account_id = ?1 AND name = ?2",
            params![account_id, name],
            |r| {
                Ok(FolderState {
                    uid_validity: r.get(0)?,
                    last_uid: r.get(1)?,
                    min_uid: r.get(2)?,
                })
            },
        )
        .optional()?)
}

/// Saves sync progress. `min_uid` is only taken for a new folder or a new UIDVALIDITY;
/// otherwise it belongs to `set_min_uid`, so a sync running next to a backfill can't undo it.
pub fn set_folder_state(
    conn: &Connection,
    account_id: &str,
    name: &str,
    st: FolderState,
) -> Result<()> {
    conn.execute(
        "INSERT INTO folders (account_id, name, uid_validity, last_uid, min_uid)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (account_id, name) DO UPDATE SET
            min_uid = CASE WHEN uid_validity = ?3 THEN min_uid ELSE ?5 END,
            uid_validity = ?3, last_uid = ?4",
        params![account_id, name, st.uid_validity, st.last_uid, st.min_uid],
    )?;
    Ok(())
}

/// Records how far back a folder has been fetched; ignored if the folder was renumbered.
pub fn set_min_uid(
    conn: &Connection,
    account_id: &str,
    name: &str,
    uid_validity: u32,
    min_uid: u32,
) -> Result<()> {
    conn.execute(
        "UPDATE folders SET min_uid = ?4
         WHERE account_id = ?1 AND name = ?2 AND uid_validity = ?3",
        params![account_id, name, uid_validity, min_uid],
    )?;
    Ok(())
}

/// False once every synced folder of the account has been fetched back to its first message.
/// An account that hasn't synced yet still has everything ahead of it.
pub fn has_older_mail(conn: &Connection, account_id: &str) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT NOT EXISTS (SELECT 1 FROM folders WHERE account_id = ?1)
             OR EXISTS (SELECT 1 FROM folders WHERE account_id = ?1 AND min_uid > 1)",
        [account_id],
        |r| r.get(0),
    )?)
}

/// UIDVALIDITY changed: the server renumbered the folder, so its cached messages are stale.
pub fn clear_folder(conn: &Connection, account_id: &str, name: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM messages WHERE account_id = ?1 AND folder = ?2",
        params![account_id, name],
    )?;
    Ok(())
}

pub fn message_count(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))?)
}

/// Drops the cached mail and every folder's sync state, so the next sync downloads again.
/// Kept: accounts, chat state, image rules, and messages with local work still waiting for
/// the server (sent from Mailbub, deleted, read/unread changed); sync matches them back up
/// by Message-ID. Then compacts the file.
pub fn clear_mail_cache(conn: &Connection) -> Result<()> {
    conn.execute(
        "DELETE FROM messages
         WHERE folder != ?1 AND deleted_at IS NULL AND seen_dirty = 0",
        [LOCAL_FOLDER],
    )?;
    conn.execute("DELETE FROM folders", [])?;
    conn.execute_batch("VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(())
}

fn json<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "null".into())
}

fn from_json<T: serde::de::DeserializeOwned>(s: String) -> rusqlite::Result<T> {
    serde_json::from_str(&s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, e.into())
    })
}

/// Stores one message. Returns false if it was already known (same UID, or the same
/// Message-ID seen in another folder, e.g. Gmail labels or a mail to yourself). A message
/// sent from Mailbub and kept in `LOCAL_FOLDER` moves to its place on the server.
pub fn insert_message(
    conn: &Connection,
    account_id: &str,
    folder: &str,
    uid: u32,
    seen: bool,
    in_inbox: bool,
    m: &Parsed,
) -> Result<bool> {
    if let Some(id) = &m.message_id {
        let dup: Option<(i64, String)> = conn
            .query_row(
                "SELECT id, folder FROM messages WHERE account_id = ?1 AND message_id = ?2
                 ORDER BY folder != ?3 LIMIT 1",
                params![account_id, id, LOCAL_FOLDER],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((row, dup_folder)) = dup {
            if dup_folder == LOCAL_FOLDER && folder != LOCAL_FOLDER {
                relocate(conn, row, folder, uid, in_inbox)?;
                return Ok(true);
            }
            return Ok(false);
        }
    }
    insert_row(conn, account_id, folder, uid, seen, in_inbox, m)
}

fn insert_row(
    conn: &Connection,
    account_id: &str,
    folder: &str,
    uid: u32,
    seen: bool,
    in_inbox: bool,
    m: &Parsed,
) -> Result<bool> {
    let refs = (!m.references.is_empty()).then(|| m.references.join(" "));
    let n = conn.execute(
        "INSERT OR IGNORE INTO messages (account_id, folder, uid, message_id, chat_key, outgoing,
            from_json, others_json, subject, body, quoted, attachments, sent_at, seen, html, ics,
            in_inbox, refs, automated, unsubscribe, own_address)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18,
            ?19, ?20, ?21)",
        params![
            account_id,
            folder,
            uid,
            m.message_id,
            crate::parse::chat_key(account_id, &m.others),
            m.outgoing,
            json(&m.from),
            json(&m.others),
            m.subject,
            m.body,
            m.quoted,
            json(&m.attachments),
            m.sent_at,
            // Your own sent mail is always read.
            seen || m.outgoing,
            m.html,
            m.ics,
            in_inbox,
            refs,
            m.automated,
            m.unsubscribe.as_ref().map(json),
            m.own_address,
        ],
    )?;
    Ok(n > 0)
}

/// Stores a message just sent from Mailbub under `LOCAL_FOLDER`, so it shows at once; the
/// next sync swaps in the server's copy (matched by Message-ID). Returns its chat key.
/// Returns the chat key and the new row's id.
pub fn insert_sent(conn: &Connection, account_id: &str, m: &Parsed) -> Result<(String, i64)> {
    let uid: u32 = conn.query_row(
        "SELECT COALESCE(MAX(uid), 0) + 1 FROM messages WHERE account_id = ?1 AND folder = ?2",
        params![account_id, LOCAL_FOLDER],
        |r| r.get(0),
    )?;
    insert_row(conn, account_id, LOCAL_FOLDER, uid, true, false, m)?;
    // A fresh uid in LOCAL_FOLDER, so the insert is never ignored.
    Ok((
        crate::parse::chat_key(account_id, &m.others),
        conn.last_insert_rowid(),
    ))
}

/// What a reply in a chat needs from the message it answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyContext {
    pub message_id: Option<String>,
    pub references: Vec<String>,
    pub subject: String,
    pub others: Vec<Person>,
}

/// The context of a reply to `message` (a message id in the chat), or else to the chat's
/// newest message.
pub fn reply_context(
    conn: &Connection,
    chat_key: &str,
    message: Option<i64>,
) -> Result<ReplyContext> {
    conn.query_row(
        "SELECT message_id, refs, subject, others_json FROM messages
         WHERE chat_key = ?1 AND deleted_at IS NULL AND (?2 IS NULL OR id = ?2)
         ORDER BY sent_at DESC, id DESC LIMIT 1",
        params![chat_key, message],
        |r| {
            let refs: Option<String> = r.get(1)?;
            Ok(ReplyContext {
                message_id: r.get(0)?,
                references: refs
                    .map(|s| s.split_whitespace().map(str::to_owned).collect())
                    .unwrap_or_default(),
                subject: r.get(2)?,
                others: from_json(r.get(3)?)?,
            })
        },
    )
    .optional()?
    .ok_or_else(|| AppError::new(ErrorCode::NotFound, "Conversation not found"))
}

fn row_to_message(r: &Row, with_html: bool) -> rusqlite::Result<Message> {
    Ok(Message {
        id: r.get::<_, i64>("id")?.to_string(),
        from: from_json(r.get("from_json")?)?,
        outgoing: r.get("outgoing")?,
        own_address: r.get("own_address")?,
        sent_at: r.get("sent_at")?,
        subject: r.get("subject")?,
        body: r.get("body")?,
        quoted: r.get("quoted")?,
        html: if with_html { r.get("html")? } else { None },
        ics: if with_html { r.get("ics")? } else { None },
        attachments: from_json(r.get("attachments")?)?,
        read: r.get("seen")?,
        unsubscribe: if with_html {
            r.get::<_, Option<String>>("unsubscribe")?
                .and_then(|s| serde_json::from_str(&s).ok())
        } else {
            None
        },
        reply_to: None,
    })
}

pub fn list_chats(conn: &Connection) -> Result<Vec<ChatSummary>> {
    let mut stmt = conn.prepare(
        "SELECT m.*,
            (SELECT COUNT(*) FROM messages u
              WHERE u.chat_key = m.chat_key AND u.seen = 0 AND u.outgoing = 0
                AND u.deleted_at IS NULL) AS unread,
            COALESCE(cs.pinned, 0) AS pinned, COALESCE(cs.muted, 0) AS muted,
            -- Archived = nothing left in the inbox, as on the server. A chat with only your
            -- own mail has no inbox to go by: it is archived by hand, or once it has been
            -- quiet for SENT_ONLY_DAYS.
            CASE
              WHEN EXISTS (SELECT 1 FROM messages i WHERE i.chat_key = m.chat_key
                             AND i.in_inbox = 1 AND i.deleted_at IS NULL) THEN 0
              WHEN EXISTS (SELECT 1 FROM messages o WHERE o.chat_key = m.chat_key
                             AND o.outgoing = 0 AND o.deleted_at IS NULL) THEN 1
              ELSE COALESCE(cs.local_archived, m.sent_at < unixepoch() - ?1)
            END AS archived,
            -- Automated = the newest mail came from a machine and you never wrote in the chat.
            (COALESCE(m.automated, 0) = 1
             AND NOT EXISTS (SELECT 1 FROM messages w WHERE w.chat_key = m.chat_key
                               AND w.outgoing = 1 AND w.deleted_at IS NULL)) AS chat_automated,
            cs.manual_status
         FROM messages m
         LEFT JOIN chat_state cs ON cs.chat_key = m.chat_key
         WHERE m.id = (SELECT x.id FROM messages x
                       WHERE x.chat_key = m.chat_key AND x.deleted_at IS NULL
                       ORDER BY x.sent_at DESC, x.id DESC LIMIT 1)
         ORDER BY m.sent_at DESC",
    )?;
    let rows = stmt.query_map([SENT_ONLY_DAYS * 86_400], |r| {
        let participants = from_json::<Vec<Person>>(r.get("others_json")?)?;
        // Chats with no-reply addresses only are Done even if you wrote there.
        let no_reply = participants
            .iter()
            .all(|p| crate::parse::is_no_reply_address(&p.address));
        Ok(ChatSummary {
            id: r.get("chat_key")?,
            account_id: r.get("account_id")?,
            participants,
            last: row_to_message(r, false)?,
            unread: r.get("unread")?,
            automated: no_reply || r.get::<_, bool>("chat_automated")?,
            state: ChatState {
                pinned: r.get("pinned")?,
                muted: r.get("muted")?,
                archived: r.get("archived")?,
                manual_status: r.get("manual_status")?,
            },
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn chat_messages(conn: &Connection, chat_key: &str) -> Result<Vec<Message>> {
    let mut stmt = conn.prepare(
        "SELECT * FROM messages WHERE chat_key = ?1 AND deleted_at IS NULL ORDER BY sent_at, id",
    )?;
    let rows = stmt.query_map([chat_key], |r| {
        let message_id: Option<String> = r.get("message_id")?;
        let refs: Option<String> = r.get("refs")?;
        // The parent is the last of References (or In-Reply-To, which parsing falls back to).
        let parent = refs.and_then(|s| s.split_whitespace().last().map(bare_message_id));
        Ok((
            row_to_message(r, true)?,
            message_id.map(|id| bare_message_id(&id)),
            parent,
        ))
    })?;
    let rows: Vec<_> = rows.collect::<rusqlite::Result<_>>()?;
    let ids: HashMap<&str, &str> = rows
        .iter()
        .filter_map(|(m, id, _)| Some((id.as_deref()?, m.id.as_str())))
        .collect();
    let reply_to: Vec<Option<String>> = rows
        .iter()
        .map(|(m, _, parent)| {
            let target = ids.get(parent.as_deref()?)?;
            (*target != m.id).then(|| target.to_string())
        })
        .collect();
    Ok(rows
        .into_iter()
        .zip(reply_to)
        .map(|((mut m, _, _), reply_to)| {
            m.reply_to = reply_to;
            m
        })
        .collect())
}

fn bare_message_id(id: &str) -> String {
    id.trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .to_owned()
}

/// Marks a chat read (every message) or unread (its newest incoming message). Changed
/// messages are flagged `seen_dirty` until `flags::push_seen` copies them to the server.
pub fn set_chat_read(conn: &Connection, chat_key: &str, read: bool) -> Result<()> {
    if read {
        conn.execute(
            "UPDATE messages SET seen = 1, seen_dirty = 1 WHERE chat_key = ?1 AND seen = 0",
            [chat_key],
        )?;
    } else {
        conn.execute(
            "UPDATE messages SET seen = 0, seen_dirty = 1 WHERE seen = 1 AND id = (
               SELECT id FROM messages WHERE chat_key = ?1 AND outgoing = 0
               ORDER BY sent_at DESC LIMIT 1)",
            [chat_key],
        )?;
    }
    Ok(())
}

/// A read/unread change made in Mailbub that the server does not know yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeenChange {
    pub id: i64,
    pub folder: String,
    pub uid: u32,
    pub seen: bool,
}

pub fn pending_seen(conn: &Connection, account_id: &str) -> Result<Vec<SeenChange>> {
    let mut stmt = conn.prepare(
        "SELECT id, folder, uid, seen FROM messages
         WHERE account_id = ?1 AND seen_dirty = 1 AND folder != ?2
         ORDER BY folder, uid",
    )?;
    let rows = stmt.query_map(params![account_id, LOCAL_FOLDER], |r| {
        Ok(SeenChange {
            id: r.get(0)?,
            folder: r.get(1)?,
            uid: r.get(2)?,
            seen: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// The server has `seen` for this message. If it was toggled again meanwhile, it stays dirty.
pub fn seen_pushed(conn: &Connection, id: i64, seen: bool) -> Result<()> {
    conn.execute(
        "UPDATE messages SET seen_dirty = 0 WHERE id = ?1 AND seen = ?2",
        params![id, seen],
    )?;
    Ok(())
}

pub fn set_chat_state(conn: &Connection, chat_key: &str, st: &ChatState) -> Result<()> {
    conn.execute(
        "INSERT INTO chat_state (chat_key, pinned, muted, archived, manual_status)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (chat_key) DO UPDATE SET pinned = ?2, muted = ?3, archived = ?4, manual_status = ?5",
        params![chat_key, st.pinned, st.muted, st.archived, st.manual_status],
    )?;
    Ok(())
}

/// Archives (or brings back) a chat with only your own mail, which has nothing to move on
/// the server.
pub fn set_local_archived(conn: &Connection, chat_key: &str, archived: bool) -> Result<()> {
    conn.execute(
        "INSERT INTO chat_state (chat_key, local_archived) VALUES (?1, ?2)
         ON CONFLICT (chat_key) DO UPDATE SET local_archived = ?2",
        params![chat_key, archived],
    )?;
    Ok(())
}

/// Hides a chat's messages now; they move to the server's Trash once the undo window passes.
pub fn mark_chat_deleted(conn: &Connection, chat_key: &str, at: i64) -> Result<()> {
    conn.execute(
        "UPDATE messages SET deleted_at = ?2 WHERE chat_key = ?1 AND deleted_at IS NULL",
        params![chat_key, at],
    )?;
    Ok(())
}

/// Undo of `mark_chat_deleted`, possible until the deletion is committed to the server.
pub fn restore_chat(conn: &Connection, chat_key: &str) -> Result<()> {
    conn.execute(
        "UPDATE messages SET deleted_at = NULL WHERE chat_key = ?1",
        [chat_key],
    )?;
    Ok(())
}

/// A message waiting to be moved to Trash on the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingDelete {
    pub id: i64,
    pub folder: String,
    pub uid: u32,
}

pub fn pending_deletions(
    conn: &Connection,
    account_id: &str,
    before: i64,
) -> Result<Vec<PendingDelete>> {
    let mut stmt = conn.prepare(
        "SELECT id, folder, uid FROM messages
         WHERE account_id = ?1 AND deleted_at IS NOT NULL AND deleted_at <= ?2 AND folder != ?3
         ORDER BY folder, uid",
    )?;
    // Messages still in LOCAL_FOLDER wait: they get deleted once sync finds them on the server.
    let rows = stmt.query_map(params![account_id, before, LOCAL_FOLDER], |r| {
        Ok(PendingDelete {
            id: r.get(0)?,
            folder: r.get(1)?,
            uid: r.get(2)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Drops messages from the cache once the server has them in Trash.
pub fn forget_messages(conn: &Connection, ids: &[i64]) -> Result<()> {
    let mut stmt = conn.prepare("DELETE FROM messages WHERE id = ?1")?;
    for id in ids {
        stmt.execute([id])?;
    }
    Ok(())
}

/// Where a stored message lives on the server: (account id, folder, UID).
pub fn message_location(conn: &Connection, id: i64) -> Result<(String, String, u32)> {
    conn.query_row(
        "SELECT account_id, folder, uid FROM messages WHERE id = ?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()?
    .ok_or_else(|| AppError::new(ErrorCode::NotFound, "Message not found"))
}

/// Local ids and UIDs of the messages stored for one folder.
pub fn folder_uids(conn: &Connection, account_id: &str, folder: &str) -> Result<Vec<(i64, u32)>> {
    let mut stmt =
        conn.prepare("SELECT id, uid FROM messages WHERE account_id = ?1 AND folder = ?2")?;
    let rows = stmt.query_map(params![account_id, folder], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Returns true if the value changed.
pub fn set_in_inbox(conn: &Connection, id: i64, in_inbox: bool) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE messages SET in_inbox = ?2 WHERE id = ?1 AND in_inbox != ?2",
        params![id, in_inbox],
    )? > 0)
}

/// A chat message's place on the server, for archiving.
#[derive(Debug, Clone)]
pub struct MessageLoc {
    pub id: i64,
    pub folder: String,
    pub uid: u32,
    pub message_id: Option<String>,
    pub outgoing: bool,
    pub in_inbox: bool,
}

pub fn chat_locations(conn: &Connection, chat_key: &str) -> Result<Vec<MessageLoc>> {
    let mut stmt = conn.prepare(
        "SELECT id, folder, uid, message_id, outgoing, in_inbox FROM messages
         WHERE chat_key = ?1 AND deleted_at IS NULL",
    )?;
    let rows = stmt.query_map([chat_key], |r| {
        Ok(MessageLoc {
            id: r.get(0)?,
            folder: r.get(1)?,
            uid: r.get(2)?,
            message_id: r.get(3)?,
            outgoing: r.get(4)?,
            in_inbox: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// After a move on the server: the message's new folder and UID.
/// Server places of stored messages, each with its account id. Unknown ids are skipped.
pub fn message_locs(conn: &Connection, ids: &[i64]) -> Result<Vec<(String, MessageLoc)>> {
    let mut stmt = conn.prepare(
        "SELECT account_id, id, folder, uid, message_id, outgoing, in_inbox FROM messages
         WHERE id = ?1 AND deleted_at IS NULL",
    )?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let row = stmt
            .query_row([id], |r| {
                Ok((
                    r.get(0)?,
                    MessageLoc {
                        id: r.get(1)?,
                        folder: r.get(2)?,
                        uid: r.get(3)?,
                        message_id: r.get(4)?,
                        outgoing: r.get(5)?,
                        in_inbox: r.get(6)?,
                    },
                ))
            })
            .optional()?;
        out.extend(row);
    }
    Ok(out)
}

pub fn relocate(conn: &Connection, id: i64, folder: &str, uid: u32, in_inbox: bool) -> Result<()> {
    conn.execute(
        "UPDATE OR IGNORE messages SET folder = ?2, uid = ?3, in_inbox = ?4 WHERE id = ?1",
        params![id, folder, uid, in_inbox],
    )?;
    Ok(())
}

/// Snippets mark matches with \u{2} … \u{3}: control characters never present in mail text.
const MARK_START: char = '\u{2}';

/// Turns what the user typed into an FTS5 query: every word must appear, as a word prefix
/// ("nov" finds "Novák"), each quoted so FTS syntax (`-`, `:`, `*`, `OR`, …) is plain text.
/// Words without letters or digits are dropped; None when nothing searchable is left.
pub fn fts_query(input: &str) -> Option<String> {
    let terms: Vec<String> = input
        .split_whitespace()
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .map(|w| format!("\"{}\"*", w.replace('"', "\"\"")))
        .collect();
    (!terms.is_empty()).then(|| terms.join(" "))
}

/// A chat that matches a search, with a highlighted excerpt of its best-matching message
/// when the match is in the text (not only in names or addresses).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub chat_id: String,
    pub message_id: String,
    pub snippet: Option<String>,
}

/// Chats whose mail matches `input`, best first (bm25; subject and names weigh more than
/// body text). Deleted messages are left out.
pub fn search_chats(conn: &Connection, input: &str, limit: usize) -> Result<Vec<SearchHit>> {
    let Some(query) = fts_query(input) else {
        return Ok(vec![]);
    };
    // FTS5 functions can't run inside an aggregate, so score every match first, then keep
    // each chat's best message (a bare column next to MIN() comes from the minimum's row).
    let mut stmt = conn.prepare(
        "WITH hits AS MATERIALIZED (
            SELECT m.chat_key, m.id, bm25(messages_fts, 4.0, 1.0, 3.0, 2.0) AS score
            FROM messages_fts JOIN messages m ON m.id = messages_fts.rowid
            WHERE messages_fts MATCH ?1 AND m.deleted_at IS NULL
         )
         SELECT chat_key, id, MIN(score) AS score FROM hits
         GROUP BY chat_key ORDER BY score LIMIT ?2",
    )?;
    let best = stmt
        .query_map(params![query, limit as i64], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    if best.is_empty() {
        return Ok(vec![]);
    }

    let ids = json(&best.iter().map(|(_, id)| *id).collect::<Vec<_>>());
    let mut stmt = conn.prepare(
        "SELECT rowid,
            snippet(messages_fts, 1, char(2), char(3), '…', 12),
            snippet(messages_fts, 0, char(2), char(3), '…', 12)
         FROM messages_fts
         WHERE messages_fts MATCH ?1 AND rowid IN (SELECT value FROM json_each(?2))",
    )?;
    let mut snippets: HashMap<i64, String> = stmt
        .query_map(params![query, ids], |r| {
            let body: Option<String> = r.get(1)?;
            let subject: Option<String> = r.get(2)?;
            Ok((r.get::<_, i64>(0)?, [body, subject]))
        })?
        .filter_map(|row| {
            let (id, texts) = row.ok()?;
            // Only an excerpt that shows the match; a hit in names keeps the normal preview.
            let snippet = texts
                .into_iter()
                .flatten()
                .find(|s| s.contains(MARK_START))?;
            Some((id, snippet))
        })
        .collect();
    Ok(best
        .into_iter()
        .map(|(chat_id, id)| SearchHit {
            chat_id,
            message_id: id.to_string(),
            snippet: snippets.remove(&id),
        })
        .collect())
}

/// "Always load remote content" rule: by image `host`, by `sender` address, or by sender
/// `domain` (which also covers its subdomains).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RemoteRule {
    pub kind: String,
    pub value: String,
}

pub const RULE_KINDS: &[&str] = &["host", "sender", "domain"];

pub fn list_remote_rules(conn: &Connection) -> Result<Vec<RemoteRule>> {
    let mut stmt = conn.prepare("SELECT kind, value FROM remote_rules ORDER BY kind, value")?;
    let rows = stmt.query_map([], |r| {
        Ok(RemoteRule {
            kind: r.get(0)?,
            value: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn add_remote_rules(conn: &Connection, rules: &[RemoteRule], at: i64) -> Result<()> {
    for rule in rules {
        if !RULE_KINDS.contains(&rule.kind.as_str()) || rule.value.trim().is_empty() {
            return Err(AppError::new(
                ErrorCode::Invalid,
                "Unknown remote content rule",
            ));
        }
        conn.execute(
            "INSERT OR IGNORE INTO remote_rules (kind, value, created_at) VALUES (?1, ?2, ?3)",
            params![rule.kind, rule.value.trim().to_lowercase(), at],
        )?;
    }
    Ok(())
}

pub fn remove_remote_rule(conn: &Connection, rule: &RemoteRule) -> Result<()> {
    conn.execute(
        "DELETE FROM remote_rules WHERE kind = ?1 AND value = ?2",
        params![rule.kind, rule.value],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: &str, email: &str) -> Account {
        Account {
            id: id.into(),
            provider: Provider::Imap,
            label: "Work".into(),
            email: email.into(),
            username: String::new(),
            display_name: "Alex".into(),
            aliases: vec![],
            default_from: email.into(),
            color: "teal".into(),
            imap: Server {
                host: "imap.example.com".into(),
                port: 993,
                security: Security::Tls,
            },
            smtp: Server {
                host: "smtp.example.com".into(),
                port: 587,
                security: Security::Starttls,
            },
            auth: AuthKind::Password,
            created_at: 1,
        }
    }

    fn memory() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        migrate(&mut conn).unwrap();
        conn
    }

    #[test]
    fn round_trips_accounts() {
        let conn = memory();
        insert_account(&conn, &sample("a", "me@example.com")).unwrap();
        let all = list_accounts(&conn).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].smtp.security, Security::Starttls);
        assert_eq!(all[0].auth, AuthKind::Password);
        assert_eq!(
            all[0].username, "me@example.com",
            "empty username falls back to email"
        );
    }

    #[test]
    fn rejects_duplicate_email_case_insensitively() {
        let conn = memory();
        insert_account(&conn, &sample("a", "me@example.com")).unwrap();
        let err = insert_account(&conn, &sample("b", "ME@example.com")).unwrap_err();
        assert_eq!(err.code, ErrorCode::Duplicate);
        assert!(email_exists(&conn, "Me@Example.com").unwrap());
    }

    #[test]
    fn updates_editable_fields_only() {
        let conn = memory();
        insert_account(&conn, &sample("a", "me@example.com")).unwrap();
        let mut a = get_account(&conn, "a").unwrap();
        a.label = "Personal".into();
        a.color = "plum".into();
        a.smtp.port = 465;
        a.smtp.security = Security::Tls;
        a.email = "other@example.com".into();
        update_account(&conn, &a).unwrap();

        let saved = get_account(&conn, "a").unwrap();
        assert_eq!(saved.label, "Personal");
        assert_eq!(saved.color, "plum");
        assert_eq!(saved.smtp.security, Security::Tls);
        assert_eq!(saved.email, "me@example.com", "address is not editable");
        assert_eq!(
            update_account(&conn, &sample("nope", "x@example.com"))
                .unwrap_err()
                .code,
            ErrorCode::NotFound
        );
    }

    #[test]
    fn keeps_aliases_and_default_address() {
        let conn = memory();
        let mut a = sample("a", "me@example.com");
        insert_account(&conn, &a).unwrap();
        assert!(get_account(&conn, "a").unwrap().aliases.is_empty());
        assert_eq!(get_account(&conn, "a").unwrap().default_from, a.email);

        a.aliases = vec!["hi@alias.example".into()];
        a.default_from = "hi@alias.example".into();
        update_account(&conn, &a).unwrap();
        let saved = get_account(&conn, "a").unwrap();
        assert_eq!(saved.aliases, ["hi@alias.example"]);
        assert_eq!(saved.default_from, "hi@alias.example");
    }

    #[test]
    fn claiming_an_alias_refiles_cached_mail() {
        let conn = memory();
        insert_account(&conn, &sample("a", "me@example.com")).unwrap();
        let person = |address: &str| Person {
            name: address.into(),
            address: address.into(),
        };
        let alias = "hi@alias.example";
        // Bob wrote to the alias: seen as a group chat with it.
        let mut to_alias = parsed(Some("1@x"), "bob@x.sk", false, 1);
        to_alias.others = vec![person("bob@x.sk"), person(alias)];
        insert_message(&conn, "a", "INBOX", 1, false, true, &to_alias).unwrap();
        // Sent from the alias outside a Sent folder: seen as mail from someone else.
        let mut from_alias = parsed(Some("2@x"), alias, false, 2);
        from_alias.others = vec![person(alias), person("bob@x.sk")];
        from_alias.automated = true;
        insert_message(&conn, "a", "INBOX", 2, false, true, &from_alias).unwrap();
        // Already right, and on another chat.
        let mut other = parsed(Some("3@x"), "ann@x.sk", false, 3);
        other.others = vec![person("ann@x.sk")];
        insert_message(&conn, "a", "INBOX", 3, false, true, &other).unwrap();

        let old_key = crate::parse::chat_key("a", &to_alias.others);
        set_chat_state(
            &conn,
            &old_key,
            &ChatState {
                pinned: true,
                ..Default::default()
            },
        )
        .unwrap();

        let own = ["me@example.com".to_owned(), alias.to_owned()];
        assert_eq!(claim_addresses(&conn, "a", &own).unwrap(), 2);
        assert_eq!(
            claim_addresses(&conn, "a", &own).unwrap(),
            0,
            "nothing left"
        );

        let chats = list_chats(&conn).unwrap();
        assert_eq!(chats.len(), 2);
        let bob = chats.iter().find(|c| c.id == "a|bob@x.sk").unwrap();
        assert!(bob.state.pinned, "state follows the chat");
        assert_eq!(bob.participants, [person("bob@x.sk")]);
        assert_eq!(bob.unread, 1, "the mail from the alias is read now");
        let messages = chat_messages(&conn, "a|bob@x.sk").unwrap();
        assert_eq!(messages.len(), 2);
        assert!(!messages[0].outgoing);
        assert_eq!(messages[0].own_address.as_deref(), Some(alias));
        assert!(messages[1].outgoing);
        assert_eq!(messages[1].own_address, None);
    }

    fn parsed(id: Option<&str>, from: &str, outgoing: bool, at: i64) -> Parsed {
        Parsed {
            message_id: id.map(str::to_owned),
            references: vec![],
            from: Person {
                name: from.into(),
                address: from.into(),
            },
            outgoing,
            others: vec![Person {
                name: "Bob".into(),
                address: "bob@x.sk".into(),
            }],
            own_address: None,
            subject: "Hi".into(),
            body: format!("body {at}"),
            quoted: None,
            html: Some(format!("<p>body {at}</p>")),
            ics: None,
            attachments: vec![],
            sent_at: at,
            automated: false,
            unsubscribe: None,
        }
    }

    #[test]
    fn chat_is_automated_until_you_write() {
        let conn = memory();
        insert_account(&conn, &sample("a", "me@example.com")).unwrap();
        let news = Parsed {
            automated: true,
            ..parsed(Some("<n1>"), "bob@x.sk", false, 10)
        };
        insert_message(&conn, "a", "INBOX", 1, false, true, &news).unwrap();
        assert!(list_chats(&conn).unwrap()[0].automated);

        // A person answers after the automated mail: the newest message decides.
        let human = parsed(Some("<n2>"), "bob@x.sk", false, 20);
        insert_message(&conn, "a", "INBOX", 2, false, true, &human).unwrap();
        assert!(!list_chats(&conn).unwrap()[0].automated);

        // Once you have written in the chat, even automated mail may need a reply.
        let reply = parsed(Some("<n3>"), "me@example.com", true, 30);
        insert_message(&conn, "a", "Sent", 1, true, false, &reply).unwrap();
        let later = Parsed {
            automated: true,
            ..parsed(Some("<n4>"), "bob@x.sk", false, 40)
        };
        insert_message(&conn, "a", "INBOX", 3, false, true, &later).unwrap();
        assert!(!list_chats(&conn).unwrap()[0].automated);
    }

    #[test]
    fn no_reply_chat_stays_automated_after_you_write() {
        let conn = memory();
        insert_account(&conn, &sample("a", "me@example.com")).unwrap();
        let shop = vec![Person {
            name: "Shop".into(),
            address: "noreply@shop.sk".into(),
        }];
        let reply = Parsed {
            others: shop,
            ..parsed(Some("<r1>"), "me@example.com", true, 10)
        };
        insert_message(&conn, "a", "Sent", 1, true, false, &reply).unwrap();
        let chats = list_chats(&conn).unwrap();
        assert!(chats[0].last.outgoing);
        assert!(chats[0].automated, "nobody reads a reply to noreply@");
    }

    #[test]
    fn chat_with_only_your_mail_archives_when_quiet_or_by_hand() {
        let conn = memory();
        insert_account(&conn, &sample("a", "me@example.com")).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let sent = parsed(Some("<s1>"), "me@example.com", true, now - 3600);
        insert_message(&conn, "a", "Sent", 1, true, false, &sent).unwrap();
        let chat = || list_chats(&conn).unwrap().remove(0);
        assert!(!chat().state.archived, "waiting for a reply");

        set_local_archived(&conn, &chat().id, true).unwrap();
        assert!(chat().state.archived);
        set_local_archived(&conn, &chat().id, false).unwrap();
        assert!(!chat().state.archived);

        conn.execute("DELETE FROM chat_state", []).unwrap();
        conn.execute("UPDATE messages SET sent_at = 10", [])
            .unwrap();
        assert!(
            chat().state.archived,
            "quiet for weeks, as in other mail apps"
        );
        set_local_archived(&conn, &chat().id, false).unwrap();
        assert!(!chat().state.archived, "brought back by hand");

        // A reply arrives: the server's inbox decides again.
        let reply = parsed(Some("<r1>"), "bob@x.sk", false, now);
        insert_message(&conn, "a", "INBOX", 2, false, true, &reply).unwrap();
        assert!(!chat().state.archived);
        conn.execute("UPDATE messages SET in_inbox = 0", [])
            .unwrap();
        assert!(chat().state.archived);
    }

    #[test]
    fn messages_group_into_chats_and_dedupe() {
        let conn = memory();
        insert_account(&conn, &sample("a", "me@example.com")).unwrap();
        assert!(
            insert_message(
                &conn,
                "a",
                "INBOX",
                1,
                false,
                true,
                &parsed(Some("<1>"), "bob@x.sk", false, 10)
            )
            .unwrap()
        );
        assert!(
            insert_message(
                &conn,
                "a",
                "Sent",
                7,
                false,
                false,
                &parsed(Some("<2>"), "me@example.com", true, 20)
            )
            .unwrap()
        );
        // Same Message-ID in another folder, and same UID again: both ignored.
        assert!(
            !insert_message(
                &conn,
                "a",
                "Archive",
                3,
                false,
                false,
                &parsed(Some("<1>"), "bob@x.sk", false, 10)
            )
            .unwrap()
        );
        assert!(
            !insert_message(
                &conn,
                "a",
                "INBOX",
                1,
                false,
                true,
                &parsed(None, "bob@x.sk", false, 10)
            )
            .unwrap()
        );

        let chats = list_chats(&conn).unwrap();
        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0].unread, 1);
        assert!(chats[0].last.outgoing, "latest message is the reply");
        assert_eq!(chat_messages(&conn, &chats[0].id).unwrap().len(), 2);
        assert!(chats[0].last.html.is_none(), "chat list leaves HTML out");
        assert!(
            chat_messages(&conn, &chats[0].id).unwrap()[0]
                .html
                .is_some()
        );

        assert!(
            !chats[0].state.archived,
            "an incoming message is still in the inbox"
        );
        let locs = chat_locations(&conn, &chats[0].id).unwrap();
        let inbox = locs.iter().find(|l| l.folder == "INBOX").unwrap();
        // Moved to Archive on the server: no longer in the inbox, so the chat is archived.
        relocate(&conn, inbox.id, "Archive", 42, false).unwrap();
        assert!(list_chats(&conn).unwrap()[0].state.archived);
        assert_eq!(
            folder_uids(&conn, "a", "Archive").unwrap(),
            [(inbox.id, 42)]
        );
        assert!(set_in_inbox(&conn, inbox.id, true).unwrap());
        assert!(
            !set_in_inbox(&conn, inbox.id, true).unwrap(),
            "no change reported twice"
        );
        assert!(!list_chats(&conn).unwrap()[0].state.archived);

        set_chat_read(&conn, &chats[0].id, true).unwrap();
        set_chat_state(
            &conn,
            &chats[0].id,
            &ChatState {
                pinned: true,
                ..Default::default()
            },
        )
        .unwrap();
        let chats = list_chats(&conn).unwrap();
        assert_eq!(chats[0].unread, 0);
        assert!(chats[0].state.pinned);

        // Deleting hides the chat until undone or committed.
        mark_chat_deleted(&conn, &chats[0].id, 100).unwrap();
        assert!(list_chats(&conn).unwrap().is_empty());
        assert!(
            pending_deletions(&conn, "a", 99).unwrap().is_empty(),
            "still inside undo window"
        );
        assert_eq!(pending_deletions(&conn, "a", 100).unwrap().len(), 2);
        restore_chat(&conn, &chats[0].id).unwrap();
        assert_eq!(list_chats(&conn).unwrap().len(), 1);
        mark_chat_deleted(&conn, &chats[0].id, 100).unwrap();
        let ids: Vec<i64> = pending_deletions(&conn, "a", 100)
            .unwrap()
            .iter()
            .map(|p| p.id)
            .collect();
        forget_messages(&conn, &ids).unwrap();
        restore_chat(&conn, &chats[0].id).unwrap();
        assert!(
            list_chats(&conn).unwrap().is_empty(),
            "committed deletes cannot be restored"
        );

        // Removing the account removes its mail.
        delete_account(&conn, "a").unwrap();
        assert!(list_chats(&conn).unwrap().is_empty());
    }

    #[test]
    fn builds_fts_queries() {
        assert_eq!(fts_query("  "), None);
        assert_eq!(fts_query("- * ()"), None, "nothing searchable");
        assert_eq!(fts_query("nov"), Some(r#""nov"*"#.into()));
        assert_eq!(
            fts_query("zľava  OR bob@x.sk"),
            Some(r#""zľava"* "OR"* "bob@x.sk"*"#.into()),
            "words are ANDed; operators and punctuation stay literal"
        );
        assert_eq!(fts_query(r#"say "hi""#), Some(r#""say"* """hi"""*"#.into()));
    }

    fn person(name: &str, address: &str) -> Person {
        Person {
            name: name.into(),
            address: address.into(),
        }
    }

    #[test]
    fn searches_all_mail_with_diacritics_and_prefixes() {
        let conn = memory();
        insert_account(&conn, &sample("a", "me@example.com")).unwrap();
        let mut m1 = parsed(Some("<1>"), "bob@x.sk", false, 10);
        m1.subject = "Zľava na letenky".into();
        m1.body = "Ahoj, posielam faktúru za október.".into();
        let mut m2 = parsed(Some("<2>"), "bob@x.sk", false, 20);
        m2.body = "Newest message, nothing special".into();
        let mut m3 = parsed(Some("<3>"), "olena@ukr.net", false, 30);
        m3.from = person("Олена Коваль", "olena@ukr.net");
        m3.others = vec![person("Олена Коваль", "olena@ukr.net")];
        m3.subject = "Розклад".into();
        m3.body = "Зустріч у четвер".into();
        for (uid, m) in [(1, &m1), (2, &m2), (3, &m3)] {
            insert_message(&conn, "a", "INBOX", uid, true, true, m).unwrap();
        }
        let bob = crate::parse::chat_key("a", &m1.others);
        let olena = crate::parse::chat_key("a", &m3.others);

        // An older message of the chat, without diacritics, as a prefix.
        let hits = search_chats(&conn, "faktur", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chat_id, bob);
        assert_eq!(
            hits[0].snippet.as_deref(),
            Some("Ahoj, posielam \u{2}faktúru\u{3} za október.")
        );
        assert_eq!(search_chats(&conn, "ZLAVA", 10).unwrap()[0].chat_id, bob);
        assert_eq!(search_chats(&conn, "октябрь", 10).unwrap(), []);
        // Cyrillic, and names/addresses (no text snippet then).
        assert_eq!(search_chats(&conn, "зустр", 10).unwrap()[0].chat_id, olena);
        let by_name = search_chats(&conn, "коваль", 10).unwrap();
        assert_eq!(by_name[0].chat_id, olena);
        assert_eq!(by_name[0].snippet, None);
        assert_eq!(search_chats(&conn, "bob@x", 10).unwrap()[0].chat_id, bob);
        // All words must match; one result per chat.
        assert_eq!(search_chats(&conn, "faktúru newest", 10).unwrap(), []);
        assert_eq!(search_chats(&conn, "bob", 10).unwrap().len(), 1);
        assert_eq!(search_chats(&conn, "-", 10).unwrap(), []);

        // Deleted messages drop out at once and leave the index when forgotten.
        mark_chat_deleted(&conn, &bob, 100).unwrap();
        assert_eq!(search_chats(&conn, "faktur", 10).unwrap(), []);
        restore_chat(&conn, &bob).unwrap();
        let ids: Vec<i64> = folder_uids(&conn, "a", "INBOX")
            .unwrap()
            .iter()
            .filter(|(_, uid)| *uid == 1)
            .map(|(id, _)| *id)
            .collect();
        forget_messages(&conn, &ids).unwrap();
        assert_eq!(search_chats(&conn, "faktur", 10).unwrap(), []);
        let fts_rows = |conn: &Connection| -> i64 {
            conn.query_row("SELECT COUNT(*) FROM messages_fts", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(fts_rows(&conn), 2);
        // Removing the account (cascade) empties the index too.
        delete_account(&conn, "a").unwrap();
        assert_eq!(fts_rows(&conn), 0);
    }

    #[test]
    fn migration_indexes_and_keeps_existing_mail() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", true).unwrap();
        // Schema as it was before search and backfill.
        let before = MIGRATIONS
            .iter()
            .position(|m| m.contains("CREATE VIRTUAL TABLE messages_fts"))
            .unwrap();
        for (i, sql) in MIGRATIONS[..before].iter().enumerate() {
            conn.execute_batch(sql).unwrap();
            conn.pragma_update(None, "user_version", (i + 1) as i64)
                .unwrap();
        }
        // Raw rows: `insert_account` and `insert_message` write columns later migrations add.
        conn.execute(
            "INSERT INTO accounts (id, provider, label, email, display_name, color, imap_host,
                imap_port, imap_security, smtp_host, smtp_port, smtp_security, auth, created_at)
             VALUES ('a', 'imap', 'Work', 'me@example.com', 'Alex', 'teal', 'imap.example.com',
                993, 'tls', 'smtp.example.com', 587, 'starttls', 'password', 1)",
            [],
        )
        .unwrap();
        let bob = json(&parsed(None, "bob@x.sk", false, 0).from);
        let others = json(&parsed(None, "bob@x.sk", false, 0).others);
        for (uid, message_id, body, at) in [
            (40, Some("<1>"), "Pozvánka na stretnutie", 10),
            (41, None, "body 11", 11),
        ] {
            conn.execute(
                "INSERT INTO messages (account_id, folder, uid, message_id, chat_key, outgoing,
                    from_json, others_json, subject, body, attachments, sent_at, seen, in_inbox)
                 VALUES ('a', 'INBOX', ?1, ?2, 'a|bob@x.sk', 0, ?3, ?4, 'Hi', ?5, '[]', ?6, 0, 1)",
                params![uid, message_id, bob, others, body, at],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO folders (account_id, name, uid_validity, last_uid)
             VALUES ('a', 'INBOX', 7, 41), ('a', 'Sent', 7, 0)",
            [],
        )
        .unwrap();

        migrate(&mut conn).unwrap();
        assert_eq!(list_chats(&conn).unwrap().len(), 1, "cache kept");
        assert_eq!(search_chats(&conn, "pozvanka", 10).unwrap().len(), 1);
        assert_eq!(
            folder_state(&conn, "a", "INBOX").unwrap().unwrap().min_uid,
            40
        );
        assert_eq!(
            folder_state(&conn, "a", "Sent").unwrap().unwrap().min_uid,
            1,
            "empty folder has nothing older"
        );
        assert!(has_older_mail(&conn, "a").unwrap());
    }

    #[test]
    fn folder_min_uid_survives_sync_progress() {
        let conn = memory();
        insert_account(&conn, &sample("a", "me@example.com")).unwrap();
        assert!(has_older_mail(&conn, "a").unwrap(), "not synced yet");
        let st = |uid_validity, last_uid, min_uid| FolderState {
            uid_validity,
            last_uid,
            min_uid,
        };
        set_folder_state(&conn, "a", "INBOX", st(7, 500, 200)).unwrap();
        set_min_uid(&conn, "a", "INBOX", 7, 120).unwrap();
        // A sync that read the state before the backfill finished must not undo it.
        set_folder_state(&conn, "a", "INBOX", st(7, 510, 200)).unwrap();
        assert_eq!(
            folder_state(&conn, "a", "INBOX").unwrap(),
            Some(st(7, 510, 120))
        );
        // Renumbered folder: a backfill for the old numbering is ignored, sync starts over.
        set_min_uid(&conn, "a", "INBOX", 6, 1).unwrap();
        set_folder_state(&conn, "a", "INBOX", st(8, 30, 5)).unwrap();
        assert_eq!(
            folder_state(&conn, "a", "INBOX").unwrap(),
            Some(st(8, 30, 5))
        );
        assert!(has_older_mail(&conn, "a").unwrap());
        set_min_uid(&conn, "a", "INBOX", 8, 1).unwrap();
        assert!(!has_older_mail(&conn, "a").unwrap());
    }

    #[test]
    fn remote_rules_round_trip() {
        let conn = memory();
        let rule = |k: &str, v: &str| RemoteRule {
            kind: k.into(),
            value: v.into(),
        };
        add_remote_rules(
            &conn,
            &[rule("host", "Images.Shop.sk"), rule("domain", "shop.sk")],
            1,
        )
        .unwrap();
        add_remote_rules(&conn, &[rule("host", "images.shop.sk")], 2).unwrap();
        assert_eq!(
            list_remote_rules(&conn).unwrap().len(),
            2,
            "stored lowercase, no duplicates"
        );
        assert!(add_remote_rules(&conn, &[rule("everything", "x")], 3).is_err());
        remove_remote_rule(&conn, &rule("domain", "shop.sk")).unwrap();
        assert_eq!(
            list_remote_rules(&conn).unwrap(),
            [rule("host", "images.shop.sk")]
        );
    }

    #[test]
    fn replies_point_at_their_parent_in_the_chat() {
        let conn = memory();
        insert_account(&conn, &sample("a", "me@example.com")).unwrap();
        let first = parsed(Some("one@x.sk"), "bob@x.sk", false, 10);
        let mut second = parsed(Some("two@x.sk"), "bob@x.sk", false, 20);
        second.references = vec!["<one@x.sk>".into()];
        let mut stray = parsed(Some("three@x.sk"), "bob@x.sk", false, 25);
        stray.references = vec!["elsewhere@x.sk".into()];
        insert_message(&conn, "a", "INBOX", 1, false, true, &first).unwrap();
        insert_message(&conn, "a", "INBOX", 2, false, true, &second).unwrap();
        insert_message(&conn, "a", "INBOX", 3, false, true, &stray).unwrap();
        let chat = list_chats(&conn).unwrap()[0].id.clone();

        // Replying to the first message, not the newest one.
        let ids: Vec<String> = chat_messages(&conn, &chat)
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        let ctx = reply_context(&conn, &chat, Some(ids[0].parse().unwrap())).unwrap();
        assert_eq!(ctx.message_id.as_deref(), Some("one@x.sk"));
        let mut reply = parsed(Some("four@example.com"), "me@example.com", true, 30);
        reply.references =
            crate::send::thread_references(ctx.message_id.as_deref(), &ctx.references);
        insert_sent(&conn, "a", &reply).unwrap();

        let got: Vec<Option<String>> = chat_messages(&conn, &chat)
            .unwrap()
            .into_iter()
            .map(|m| m.reply_to)
            .collect();
        assert_eq!(
            got,
            [None, Some(ids[0].clone()), None, Some(ids[0].clone())],
            "a parent outside the chat is left out"
        );
        assert!(
            reply_context(&conn, &chat, Some(9999)).is_err(),
            "the message must be in the chat"
        );
    }

    #[test]
    fn sent_message_shows_at_once_and_sync_takes_it_over() {
        let conn = memory();
        insert_account(&conn, &sample("a", "me@example.com")).unwrap();
        let mut incoming = parsed(Some("<in>"), "bob@x.sk", false, 10);
        incoming.references = vec!["<root>".into()];
        insert_message(&conn, "a", "INBOX", 1, false, true, &incoming).unwrap();
        let chat = list_chats(&conn).unwrap()[0].id.clone();

        let ctx = reply_context(&conn, &chat, None).unwrap();
        assert_eq!(ctx.message_id.as_deref(), Some("<in>"));
        assert_eq!(ctx.references, ["<root>"]);
        assert_eq!(ctx.others[0].address, "bob@x.sk");
        assert_eq!(
            reply_context(&conn, "a|nobody", None).unwrap_err().code,
            ErrorCode::NotFound
        );

        let sent = parsed(Some("<out>"), "me@example.com", true, 20);
        assert_eq!(insert_sent(&conn, "a", &sent).unwrap().0, chat);
        let second = parsed(Some("<out2>"), "me@example.com", true, 30);
        insert_sent(&conn, "a", &second).unwrap();
        let chats = list_chats(&conn).unwrap();
        assert!(chats[0].last.outgoing, "the reply is the newest message");
        assert_eq!(chat_messages(&conn, &chat).unwrap().len(), 3);
        assert!(
            folder_uids(&conn, "a", "Sent").unwrap().is_empty(),
            "local copies are not in a server folder yet"
        );

        // Sync finds the server's copy: the local one moves there instead of duplicating.
        assert!(insert_message(&conn, "a", "Sent", 9, true, false, &sent).unwrap());
        assert_eq!(chat_messages(&conn, &chat).unwrap().len(), 3);
        assert_eq!(folder_uids(&conn, "a", "Sent").unwrap().len(), 1);
        assert!(!insert_message(&conn, "a", "Sent", 9, true, false, &sent).unwrap());

        // A deleted chat's local copy waits for sync before going to Trash.
        mark_chat_deleted(&conn, &chat, 100).unwrap();
        let pending = pending_deletions(&conn, "a", 100).unwrap();
        assert_eq!(pending.len(), 2);
        assert!(pending.iter().all(|p| p.folder != LOCAL_FOLDER));
    }

    #[test]
    fn read_changes_wait_for_the_server() {
        let conn = memory();
        insert_account(&conn, &sample("a", "me@example.com")).unwrap();
        for (uid, at) in [(1, 10), (2, 20)] {
            let m = parsed(Some(&format!("<{uid}>")), "bob@x.sk", false, at);
            insert_message(&conn, "a", "INBOX", uid, false, true, &m).unwrap();
        }
        let out = parsed(Some("<3>"), "me@example.com", true, 30);
        insert_message(&conn, "a", "Sent", 3, true, false, &out).unwrap();
        let chat = list_chats(&conn).unwrap()[0].id.clone();
        assert!(
            pending_seen(&conn, "a").unwrap().is_empty(),
            "sync is not a change"
        );

        set_chat_read(&conn, &chat, true).unwrap();
        let pending = pending_seen(&conn, "a").unwrap();
        assert_eq!(
            pending.iter().map(|p| (p.uid, p.seen)).collect::<Vec<_>>(),
            [(1, true), (2, true)],
            "only messages that changed"
        );
        for p in &pending {
            seen_pushed(&conn, p.id, p.seen).unwrap();
        }
        assert!(pending_seen(&conn, "a").unwrap().is_empty());

        set_chat_read(&conn, &chat, false).unwrap();
        let pending = pending_seen(&conn, "a").unwrap();
        assert_eq!(
            pending.iter().map(|p| (p.uid, p.seen)).collect::<Vec<_>>(),
            [(2, false)],
            "newest incoming message becomes unread"
        );
        // Read again before the push finished: the stale push must not clear the flag.
        set_chat_read(&conn, &chat, true).unwrap();
        seen_pushed(&conn, pending[0].id, false).unwrap();
        assert!(pending_seen(&conn, "a").unwrap()[0].seen);
    }

    #[test]
    fn delete_reports_missing() {
        let conn = memory();
        assert_eq!(
            delete_account(&conn, "nope").unwrap_err().code,
            ErrorCode::NotFound
        );
    }
}
