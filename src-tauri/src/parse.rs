//! Turns a raw RFC 5322 message into what a chat needs: who it's with, new text, attachments.

use std::collections::HashSet;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use std::sync::LazyLock;

use mail_parser::{Address, HeaderValue, Message, MessageParser, MessagePart, MimeHeaders};
use regex::Regex;
use unicode_normalization::UnicodeNormalization;

use crate::model::{Attachment, Person};
use crate::quotes;
use crate::unsubscribe::{self, Unsubscribe};

#[derive(Debug, Clone)]
pub struct Parsed {
    pub message_id: Option<String>,
    /// Message-IDs this one answers (References, else In-Reply-To), oldest first.
    pub references: Vec<String>,
    pub from: Person,
    pub outgoing: bool,
    /// Everyone on the message except the account owner; defines the chat.
    pub others: Vec<Person>,
    /// The first of the account's addresses on an incoming message; a reply goes out from it.
    pub own_address: Option<String>,
    pub subject: String,
    pub body: String,
    pub quoted: Option<String>,
    /// Original HTML body (unsanitised; the UI sanitises and sandboxes it), inline images
    /// embedded as data: URLs.
    pub html: Option<String>,
    /// First calendar invitation (text/calendar part or .ics attachment), if any.
    pub ics: Option<String>,
    pub attachments: Vec<Attachment>,
    /// Unix seconds.
    pub sent_at: i64,
    /// Sent by a machine (newsletter, receipt, no-reply notification): nobody expects a reply.
    pub automated: bool,
    /// Mailing-list unsubscribe options from the headers (incoming mail only).
    pub unsubscribe: Option<Unsubscribe>,
}

fn message_ids(v: &HeaderValue) -> Vec<String> {
    v.as_text_list()
        .map(|l| l.iter().map(|s| s.to_string()).collect())
        .unwrap_or_default()
}

fn people(addr: Option<&Address>) -> Vec<Person> {
    addr.map(|a| {
        a.iter()
            .filter_map(|p| {
                let address = p.address()?.trim().to_lowercase();
                // Some senders wrap the display name in extra quotes ('Дія', "Shop").
                let name = p
                    .name()
                    .map(|n| n.trim().trim_matches(['\'', '"', '`']).trim())
                    .filter(|n| !n.is_empty());
                Some(Person {
                    name: name.unwrap_or(&address).to_owned(),
                    address,
                })
            })
            .collect()
    })
    .unwrap_or_default()
}

const MAX_INLINE_IMAGE: usize = 2 * 1024 * 1024;

/// The HTML body with `cid:` references to inline images replaced by data: URLs, plus the
/// indexes of the parts that were inlined (so they don't also show up as attachments).
fn html_body(msg: &Message) -> (Option<String>, HashSet<usize>) {
    let mut inlined = HashSet::new();
    let Some(part) = msg.html_part(0).filter(|p| p.is_text_html()) else {
        return (None, inlined);
    };
    let Some(mut html) = part.text_contents().map(str::to_owned) else {
        return (None, inlined);
    };
    for (i, p) in msg.parts.iter().enumerate() {
        let Some(cid) = p.content_id().map(|c| c.trim_matches(['<', '>'])) else {
            continue;
        };
        let reference = format!("cid:{cid}");
        let is_image = p
            .content_type()
            .is_some_and(|t| t.ctype().eq_ignore_ascii_case("image"));
        if !is_image || p.contents().len() > MAX_INLINE_IMAGE || !html.contains(&reference) {
            continue;
        }
        let ct = p.content_type().expect("checked above");
        let mime = format!("image/{}", ct.subtype().unwrap_or("png"));
        let data = format!("data:{mime};base64,{}", STANDARD.encode(p.contents()));
        html = html.replace(&reference, &data);
        inlined.insert(i);
    }
    (Some(html), inlined)
}

static HIDDEN_BLOCKS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?is)<script\b.*?</script\s*>|<style\b.*?</style\s*>|<head\b.*?</head\s*>|<!--.*?-->",
    )
    .expect("valid regex")
});
static LINE_BREAKS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)<br\s*/?>|</(p|div|tr|li|h[1-6]|table|blockquote|section)\s*>")
        .expect("valid regex")
});
static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]*>").expect("valid regex"));
static ENTITY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"&(#x[0-9a-fA-F]{1,6}|#\d{1,7}|[a-zA-Z][a-zA-Z0-9]{1,31});").expect("valid regex")
});

/// Named entities seen in real mail; letters with accents are built from a base letter and a
/// combining mark (e.g. &scaron; = s + caron) and composed afterwards (NFC).
fn named_entity(name: &str) -> Option<String> {
    let fixed = match name {
        "nbsp" | "NBSP" | "ensp" | "emsp" | "thinsp" => " ",
        "amp" | "AMP" => "&",
        "lt" | "LT" => "<",
        "gt" | "GT" => ">",
        "quot" | "QUOT" => "\"",
        "apos" => "'",
        "zwnj" | "zwj" | "shy" | "lrm" | "rlm" => "",
        "ndash" => "–",
        "mdash" => "—",
        "hellip" => "…",
        "lsquo" => "‘",
        "rsquo" => "’",
        "sbquo" => "‚",
        "ldquo" => "“",
        "rdquo" => "”",
        "bdquo" => "„",
        "laquo" => "«",
        "raquo" => "»",
        "bull" => "•",
        "middot" => "·",
        "euro" => "€",
        "pound" => "£",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "times" => "×",
        "divide" => "÷",
        "deg" => "°",
        "szlig" => "ß",
        "rarr" => "→",
        "larr" => "←",
        "nbhy" => "‑",
        _ => "",
    };
    if !fixed.is_empty() || matches!(name, "zwnj" | "zwj" | "shy" | "lrm" | "rlm") {
        return Some(fixed.to_owned());
    }
    let marks = [
        ("acute", '\u{301}'),
        ("grave", '\u{300}'),
        ("circ", '\u{302}'),
        ("uml", '\u{308}'),
        ("caron", '\u{30C}'),
        ("tilde", '\u{303}'),
        ("ring", '\u{30A}'),
        ("cedil", '\u{327}'),
    ];
    let mut chars = name.chars();
    let base = chars.next().filter(char::is_ascii_alphabetic)?;
    let rest: String = chars.collect();
    let mark = marks.iter().find(|(n, _)| *n == rest)?.1;
    Some(format!("{base}{mark}"))
}

fn decode_entities(s: &str) -> String {
    let decoded = ENTITY.replace_all(s, |c: &regex::Captures| {
        let e = &c[1];
        let numeric = e
            .strip_prefix("#x")
            .or_else(|| e.strip_prefix("#X"))
            .map(|h| u32::from_str_radix(h, 16).ok())
            .or_else(|| e.strip_prefix('#').map(|d| d.parse().ok()));
        match numeric {
            Some(n) => n
                .and_then(char::from_u32)
                .map(String::from)
                .unwrap_or_default(),
            None => named_entity(e).unwrap_or_else(|| c[0].to_owned()),
        }
    });
    decoded.nfc().collect()
}

/// Readable text from HTML for previews and quote detection: drops scripts, styles
/// (and JSON-LD), turns block ends into line breaks, decodes common entities.
pub fn html_to_text(html: &str) -> String {
    let s = HIDDEN_BLOCKS.replace_all(html, "");
    let s = LINE_BREAKS.replace_all(&s, "\n");
    let s = TAGS.replace_all(&s, "");
    let s = decode_entities(&s);
    s.lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

fn looks_like_html(text: &str) -> bool {
    let head = text
        .trim_start()
        .get(..200)
        .unwrap_or(text.trim_start())
        .to_lowercase();
    head.starts_with("<!doctype") || head.starts_with("<html") || head.starts_with("<head")
}

/// Drops a schema.org JSON-LD blob some senders put at the very start of the text part.
fn strip_leading_json_ld(text: &str) -> &str {
    let t = text.trim_start();
    if !(t.starts_with('{') || t.starts_with('['))
        || !t.get(..400).unwrap_or(t).contains("@context")
    {
        return text;
    }
    let (mut depth, mut in_str, mut escaped) = (0i32, false, false);
    for (i, c) in t.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if in_str => escaped = true,
            '"' => in_str = !in_str,
            '{' | '[' if !in_str => depth += 1,
            '}' | ']' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    return t[i + 1..].trim_start();
                }
            }
            _ => {}
        }
    }
    text
}

/// Decodes entities in a plain-text part, but only when it clearly contains some (a known
/// named or numeric entity), so a literal `&` or `R&D;` in real plain text stays as written.
fn decode_text_entities(text: &str) -> String {
    let has_entity = ENTITY.captures_iter(text).any(|c| {
        let e = &c[1];
        e.starts_with('#') || named_entity(e).is_some()
    });
    if !has_entity {
        return text.to_owned();
    }
    // Invisible preheader padding (&zwnj;&nbsp;…) decodes to trailing spaces.
    decode_entities(text)
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The text a bubble and the chat list show: the plain-text part when it is real text,
/// otherwise text derived from the HTML.
fn plain_text(msg: &Message, html: Option<&str>) -> String {
    match msg
        .text_part(0)
        .filter(|p| !p.is_text_html())
        .and_then(|p| p.text_contents())
    {
        Some(t) if looks_like_html(t) => html_to_text(t),
        Some(t) => match strip_leading_json_ld(t) {
            rest if rest.trim().is_empty() => html.map(html_to_text).unwrap_or_default(),
            rest => decode_text_entities(rest),
        },
        None => html.map(html_to_text).unwrap_or_default(),
    }
}

const MAX_ICS: usize = 256 * 1024;

fn calendar_part(msg: &Message) -> Option<String> {
    msg.parts.iter().find_map(|p| {
        if !is_calendar(p) || p.contents().len() > MAX_ICS {
            return None;
        }
        let text = String::from_utf8_lossy(p.contents()).into_owned();
        text.contains("BEGIN:VEVENT").then_some(text)
    })
}

static AUTOMATED_SENDER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^(no[-_.]?reply|do[-_.]?not[-_.]?reply|mailer[-_.]?daemon|postmaster|bounces?|notifications?|newsletters?)([-_.+].*)?$",
    )
    .expect("valid regex")
});

/// Mail sent by a machine: mailing-list and bulk headers (RFC 2369/2919, RFC 3834
/// `Auto-Submitted`, `Precedence`) or a no-reply style sender. Generic role addresses like
/// `info@` or `support@` are often answered by people, so they don't count.
fn is_automated(msg: &Message, from: &Person) -> bool {
    let header = |name: &str| msg.header_raw(name).map(|v| v.trim().to_ascii_lowercase());
    let listed = header("List-Unsubscribe").is_some() || header("List-Id").is_some();
    let bulk = header("Precedence").is_some_and(|v| matches!(v.as_str(), "bulk" | "list" | "junk"));
    let auto = header("Auto-Submitted")
        .is_some_and(|v| !matches!(v.split(';').next().map(str::trim), Some("" | "no")));
    listed || bulk || auto || is_no_reply_address(&from.address)
}

/// A no-reply style address (noreply@, mailer-daemon@, notifications@, …): writing to it
/// reaches nobody.
pub fn is_no_reply_address(address: &str) -> bool {
    address
        .split('@')
        .next()
        .is_some_and(|local| AUTOMATED_SENDER.is_match(local))
}

/// Stable chat identity: account plus the sorted set of other addresses.
pub fn chat_key(account_id: &str, others: &[Person]) -> String {
    let mut addrs: Vec<&str> = others.iter().map(|p| p.address.as_str()).collect();
    addrs.sort_unstable();
    addrs.dedup();
    format!("{account_id}|{}", addrs.join(","))
}

/// `own` holds the account's addresses (lowercase). `in_sent` marks the Sent folder, where
/// everything is outgoing even if sent from an alias Mailbub doesn't know.
fn is_calendar(p: &MessagePart) -> bool {
    p.content_type().is_some_and(|t| {
        t.ctype().eq_ignore_ascii_case("text")
            && t.subtype()
                .is_some_and(|s| s.eq_ignore_ascii_case("calendar"))
    }) || p
        .attachment_name()
        .is_some_and(|n| n.to_lowercase().ends_with(".ics"))
}

/// Parts shown as attachment chips, keyed by their index in `msg.parts` (a stable id for
/// fetching the file later). Inline images and calendar invitations are shown elsewhere.
fn attachment_parts<'a>(
    msg: &'a Message<'a>,
    inlined: &HashSet<usize>,
) -> Vec<(usize, &'a MessagePart<'a>)> {
    msg.attachments()
        .filter_map(|part| {
            let id = msg.parts.iter().position(|p| std::ptr::eq(p, part))?;
            let keep = !inlined.contains(&id)
                && !is_calendar(part)
                && (!part.is_message() || part.attachment_name().is_some());
            keep.then_some((id, part))
        })
        .collect()
}

fn attachment_name(part: &MessagePart, n: usize) -> String {
    part.attachment_name()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("attachment-{}", n + 1))
}

/// File name and bytes of one attachment, by the id `parse` gave it.
pub fn attachment_file(raw: &[u8], id: &str) -> Option<(String, Vec<u8>)> {
    let msg = MessageParser::default().parse(raw)?;
    let (_, inlined) = html_body(&msg);
    let wanted: usize = id.parse().ok()?;
    attachment_parts(&msg, &inlined)
        .into_iter()
        .enumerate()
        .find(|(_, (pid, _))| *pid == wanted)
        .map(|(n, (_, part))| (attachment_name(part, n), part.contents().to_vec()))
}

pub fn parse(raw: &[u8], own: &[String], in_sent: bool, fallback_time: i64) -> Option<Parsed> {
    let msg = MessageParser::default().parse(raw)?;

    let from = people(msg.from()).into_iter().next().unwrap_or(Person {
        name: "Unknown".into(),
        address: String::new(),
    });
    let is_own = |p: &Person| own.iter().any(|o| o == &p.address);
    let outgoing = in_sent || is_own(&from);
    let automated = !outgoing && is_automated(&msg, &from);
    let unsubscribe = if outgoing {
        None
    } else {
        unsubscribe::from_headers(
            msg.header_raw("List-Unsubscribe"),
            msg.header_raw("List-Unsubscribe-Post"),
        )
    };

    let own_address = if outgoing {
        None
    } else {
        people(msg.to())
            .into_iter()
            .chain(people(msg.cc()))
            .find(is_own)
            .map(|p| p.address)
    };

    let mut others: Vec<Person> = Vec::new();
    let candidates = std::iter::once(from.clone())
        .chain(people(msg.to()))
        .chain(people(msg.cc()));
    for p in candidates {
        let skip = p.address.is_empty()
            || is_own(&p)
            || others.iter().any(|o| o.address == p.address)
            // In Sent, the From is an alias of ours even if we don't know it.
            || (in_sent && p.address == from.address);
        if !skip {
            others.push(p);
        }
    }
    if others.is_empty() {
        // A note to self: the chat is with yourself.
        others.push(from.clone());
    }

    let (html, inlined) = html_body(&msg);
    let text = plain_text(&msg, html.as_deref());
    let (body, quoted) = quotes::split(&text);
    let attachments = attachment_parts(&msg, &inlined)
        .into_iter()
        .enumerate()
        .map(|(n, (id, part))| Attachment {
            id: id.to_string(),
            name: attachment_name(part, n),
            size: part.len() as u64,
        })
        .collect();

    let mut references = message_ids(msg.references());
    if references.is_empty() {
        references = message_ids(msg.in_reply_to());
    }

    Some(Parsed {
        message_id: msg.message_id().map(|s| s.to_owned()),
        references,
        from,
        outgoing,
        others,
        own_address,
        subject: msg.subject().unwrap_or_default().trim().to_owned(),
        body,
        quoted,
        html,
        ics: calendar_part(&msg),
        attachments,
        sent_at: msg
            .date()
            .map(|d| d.to_timestamp())
            .unwrap_or(fallback_time),
        automated,
        unsubscribe,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN: &[&str] = &["me@example.com"];

    fn own() -> Vec<String> {
        OWN.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn incoming_one_to_one() {
        let raw = b"From: Martin Kovac <Martin@Studio.sk>\r\nTo: me@example.com\r\nSubject: Re: Proposal\r\nMessage-ID: <a1@studio.sk>\r\nDate: Mon, 26 Oct 2026 09:00:00 +0100\r\n\r\nWe'll take the full rebuild.\r\n\r\nOn Sun, Alex wrote:\r\n> Two options.\r\n";
        let p = parse(raw, &own(), false, 0).unwrap();
        assert!(!p.outgoing);
        assert_eq!(p.from.address, "martin@studio.sk");
        assert_eq!(p.others.len(), 1);
        assert_eq!(p.body, "We'll take the full rebuild.");
        assert!(p.quoted.is_some());
        assert_eq!(p.message_id.as_deref(), Some("a1@studio.sk"));
        assert!(p.references.is_empty());
        assert_eq!(chat_key("acc", &p.others), "acc|martin@studio.sk");
    }

    #[test]
    fn keeps_references_or_in_reply_to() {
        let raw = b"From: a@x.sk\r\nTo: me@example.com\r\nReferences: <r1@x> <r2@x>\r\nIn-Reply-To: <r2@x>\r\n\r\nhi";
        assert_eq!(
            parse(raw, &own(), false, 0).unwrap().references,
            ["r1@x", "r2@x"]
        );
        let raw = b"From: a@x.sk\r\nTo: me@example.com\r\nIn-Reply-To: <r2@x>\r\n\r\nhi";
        assert_eq!(parse(raw, &own(), false, 0).unwrap().references, ["r2@x"]);
    }

    #[test]
    fn outgoing_reply_lands_in_same_chat() {
        let raw = b"From: Me <me@example.com>\r\nTo: Martin Kovac <martin@studio.sk>\r\nSubject: Re: Proposal\r\n\r\nGreat, starting Monday.\r\n";
        let p = parse(raw, &own(), false, 0).unwrap();
        assert!(p.outgoing);
        assert_eq!(chat_key("acc", &p.others), "acc|martin@studio.sk");
    }

    #[test]
    fn group_chat_key_is_order_independent() {
        let a = b"From: a@x.sk\r\nTo: me@example.com, b@x.sk\r\nCc: c@x.sk\r\n\r\nhi";
        let b = b"From: me@example.com\r\nTo: c@x.sk, a@x.sk\r\nCc: b@x.sk\r\n\r\nhello";
        let pa = parse(a, &own(), false, 0).unwrap();
        let pb = parse(b, &own(), false, 0).unwrap();
        assert_eq!(chat_key("acc", &pa.others), chat_key("acc", &pb.others));
        assert_eq!(pa.others.len(), 3);
    }

    #[test]
    fn mail_to_an_alias_stays_one_to_one() {
        let raw = b"From: Bob <bob@x.sk>\r\nTo: Sales <hi@alias.example>\r\nCc: ann@x.sk\r\nSubject: A\r\n\r\nHi";
        let own = ["me@example.com".to_owned(), "hi@alias.example".to_owned()];
        let p = parse(raw, &own, false, 0).unwrap();
        assert!(!p.outgoing);
        assert_eq!(chat_key("acc", &p.others), "acc|ann@x.sk,bob@x.sk");
        assert_eq!(p.own_address.as_deref(), Some("hi@alias.example"));

        // Without the alias it looks like someone else in the chat.
        let p = parse(raw, &own[..1], false, 0).unwrap();
        assert_eq!(p.others.len(), 3);
        assert_eq!(p.own_address, None);
    }

    #[test]
    fn sent_folder_from_unknown_alias_is_outgoing() {
        let raw = b"From: Sales <sales@example.com>\r\nTo: client@corp.sk\r\n\r\nOffer attached";
        let p = parse(raw, &own(), true, 0).unwrap();
        assert!(p.outgoing);
        assert_eq!(chat_key("acc", &p.others), "acc|client@corp.sk");
    }

    #[test]
    fn html_only_and_attachments() {
        let raw = b"From: Shop <news@shop.sk>\r\nTo: me@example.com\r\nSubject: Order\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Your order <b>#42</b> shipped.</p>\r\n--b\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=\"invoice.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0xLjQK\r\n--b--\r\n";
        let p = parse(raw, &own(), false, 7).unwrap();
        assert!(p.body.contains("Your order #42 shipped."), "{:?}", p.body);
        assert_eq!(p.attachments.len(), 1);
        assert_eq!(p.attachments[0].name, "invoice.pdf");
        let (name, bytes) = attachment_file(raw, &p.attachments[0].id).unwrap();
        assert_eq!(name, "invoice.pdf");
        assert!(bytes.starts_with(b"%PDF"));
        assert_eq!(p.sent_at, 7, "no Date header falls back");
        assert!(p.html.unwrap().contains("<b>#42</b>"));
    }

    #[test]
    fn inline_cid_image_becomes_data_url_not_attachment() {
        let raw = b"From: a@x.sk\r\nTo: me@example.com\r\nMIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=r\r\n\r\n--r\r\nContent-Type: text/html\r\n\r\n<img src=\"cid:logo@x\"> Hi\r\n--r\r\nContent-Type: image/png\r\nContent-ID: <logo@x>\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n--r--\r\n";
        let p = parse(raw, &own(), false, 0).unwrap();
        let html = p.html.unwrap();
        assert!(
            html.contains("data:image/png;base64,iVBORw0KGgo="),
            "{html}"
        );
        assert!(p.attachments.is_empty());
    }

    #[test]
    fn keeps_calendar_invitation() {
        let raw = b"From: a@x.sk\r\nTo: me@example.com\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nSee invite\r\n--b\r\nContent-Type: text/calendar; method=REQUEST\r\n\r\nBEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nSUMMARY:Thesis\r\nDTSTART:20261103T130000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n--b--\r\n";
        let p = parse(raw, &own(), false, 0).unwrap();
        assert!(p.ics.unwrap().contains("SUMMARY:Thesis"));
        assert!(
            p.attachments.is_empty(),
            "the invitation shows as an event, not a file"
        );
    }

    #[test]
    fn html_only_mail_gets_clean_text_and_names_lose_quotes() {
        let raw = "From: 'Дія' <noreply@diia.gov.ua>\r\nTo: me@example.com\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<!DOCTYPE html><html><head><style>p{color:red}</style><script type=\"application/ld+json\">{\"@context\":\"http://schema.org\"}</script></head><body><p>ФОП&nbsp;припинено</p><p>Деталі &amp; дата: 13.10</p></body></html>";
        let p = parse(raw.as_bytes(), &own(), false, 0).unwrap();
        assert_eq!(p.from.name, "Дія");
        assert_eq!(p.body, "ФОП припинено\nДеталі & дата: 13.10");
        assert!(!p.body.contains("schema.org"));
    }

    #[test]
    fn text_part_that_is_really_html_is_converted() {
        let raw = b"From: a@x.sk\r\nTo: me@example.com\r\nContent-Type: text/plain\r\n\r\n<!DOCTYPE html><html><body><b>Hi</b> there</body></html>";
        assert_eq!(parse(raw, &own(), false, 0).unwrap().body, "Hi there");
    }

    #[test]
    fn leading_json_ld_in_text_part_is_dropped() {
        let raw = b"From: a@x.sk\r\nTo: me@example.com\r\n\r\n{ \"@context\": \"http://schema.org\", \"name\": \"x}\" }\r\n\r\nPlease confirm your account.";
        assert_eq!(
            parse(raw, &own(), false, 0).unwrap().body,
            "Please confirm your account."
        );
    }

    #[test]
    fn decodes_named_and_accent_entities() {
        assert_eq!(
            html_to_text(
                "&#271;akujeme za Va&scaron;u objedn&aacute;vku &ndash; &copy;&nbsp;2026 &amp;times; &zwnj;ok &bogus;"
            ),
            "ďakujeme za Vašu objednávku – © 2026 &times; ok &bogus;"
        );
        assert_eq!(html_to_text("&Scaron;&#x17E;"), "Šž");
    }

    fn automated(headers: &str, from: &str) -> bool {
        let raw = format!("From: {from}\r\nTo: me@example.com\r\n{headers}\r\nhi");
        parse(raw.as_bytes(), &own(), false, 0).unwrap().automated
    }

    #[test]
    fn detects_automated_mail_by_headers() {
        let person = "Martin <martin@studio.sk>";
        assert!(!automated("", person));
        assert!(automated(
            "List-Unsubscribe: <mailto:u@shop.sk>\r\n",
            person
        ));
        assert!(automated(
            "List-Id: Team <team.lists.university.sk>\r\n",
            person
        ));
        assert!(automated("Precedence: bulk\r\n", person));
        assert!(automated("precedence: List\r\n", person));
        assert!(!automated("Precedence: first-class\r\n", person));
        assert!(automated("Auto-Submitted: auto-generated\r\n", person));
        assert!(automated("Auto-Submitted: auto-replied\r\n", person));
        assert!(!automated("Auto-Submitted: no\r\n", person));
    }

    #[test]
    fn detects_automated_mail_by_sender() {
        for from in [
            "noreply@diia.gov.ua",
            "no-reply@accounts.google.com",
            "No_Reply@bank.sk",
            "donotreply@university.sk",
            "do-not-reply@shop.sk",
            "MAILER-DAEMON@mx.example.com",
            "notifications@github.com",
            "notification+abc@facebookmail.com",
            "newsletter@shop.sk",
            "noreply-orders@shop.sk",
        ] {
            assert!(automated("", from), "{from}");
        }
        for from in [
            "info@studio.sk",
            "support@shop.sk",
            "reply@x.sk",
            "noreplyman@x.sk",
            "nora@x.sk",
        ] {
            assert!(!automated("", from), "{from}");
        }
    }

    #[test]
    fn own_mail_is_never_automated() {
        let raw = b"From: me@example.com\r\nTo: noreply@shop.sk\r\nList-Id: x\r\n\r\nhi";
        assert!(!parse(raw, &own(), false, 0).unwrap().automated);
    }

    #[test]
    fn decodes_entities_in_text_parts() {
        let body = |text: &str| {
            let raw = format!("From: a@x.sk\r\nTo: me@example.com\r\n\r\n{text}");
            parse(raw.as_bytes(), &own(), false, 0).unwrap().body
        };
        assert_eq!(
            body("Tom &amp; Jerry&NBSP;&#8211; tickets &zwnj;&nbsp;&zwnj;&nbsp;\r\nSee you"),
            "Tom & Jerry – tickets\nSee you"
        );
        // Without a real entity, ampersands are left as written.
        assert_eq!(
            body("R&D; and AT&T &bogus; stay\r\n  indented"),
            "R&D; and AT&T &bogus; stay\n  indented"
        );
    }

    #[test]
    fn plain_text_mail_has_no_html() {
        let raw = b"From: a@x.sk\r\nTo: me@example.com\r\n\r\nJust text";
        assert!(parse(raw, &own(), false, 0).unwrap().html.is_none());
    }
}
