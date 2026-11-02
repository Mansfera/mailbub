//! Splits a plain-text email body into the new text and the quoted/signature tail, so a
//! bubble shows only what the sender actually wrote this time.

use std::sync::LazyLock;

use regex::Regex;

/// "On <date>, <person> wrote:" in the languages these mailboxes see (en, sk, cs, uk, ru, de,
/// fr, es, it, pl). Gmail may wrap it over two lines, so up to one newline is allowed.
static ATTRIBUTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?im)^[ \t]*(on|dňa|dne|am|le|el|il|w dniu|у|в|о)\b[^\n]{0,200}(\n[^\n]{0,200})?\b(wrote|napísal|napísala|napsal|napsala|schrieb|a écrit|escribió|ha scritto|napisał|napisała|пише|написав|написала|писал|написал|написала)(\(a\))?\s*:\s*$",
    )
    .expect("valid regex")
});

/// Outlook-style forwarded/replied header blocks.
static OUTLOOK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?im)^[ \t]*(-{2,}\s*(original message|původní zpráva|pôvodná správa|ursprüngliche nachricht|вихідне повідомлення|исходное сообщение)\s*-{2,}|_{20,})\s*$",
    )
    .expect("valid regex")
});

/// "From: …" followed within two lines by "Sent:/Date:" (Outlook without the separator line).
static HEADER_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?im)^[ \t]*\*?(from|od|von|de|від|от)\*?:[^\n]+\n([^\n]*\n)?[ \t]*\*?(sent|date|odoslané|odesláno|gesendet|envoyé|надіслано|отправлено|datum|дата)\*?:",
    )
    .expect("valid regex")
});

/// Mobile client footers that add nothing in a chat view.
static DEVICE_FOOTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^[ \t]*(sent from my (iphone|ipad|android|phone)|get outlook for (ios|android)|odoslané z (iphonu|môjho iphonu)|надіслано з iphone)[^\n]*$")
        .expect("valid regex")
});

/// Returns (new text, stripped tail if any). Never returns empty new text when the original
/// had content: if everything looks quoted, the whole body is kept.
pub fn split(text: &str) -> (String, Option<String>) {
    let text = text.replace("\r\n", "\n");
    let mut cut = text.len();

    for re in [&*ATTRIBUTION, &*OUTLOOK, &*HEADER_BLOCK, &*DEVICE_FOOTER] {
        if let Some(m) = re.find(&text) {
            cut = cut.min(m.start());
        }
    }

    // A trailing block of "> " lines, possibly with blank lines in between.
    let mut offset = 0;
    let mut quote_start = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with('>') {
            quote_start.get_or_insert(offset);
        } else if !trimmed.trim().is_empty() {
            quote_start = None;
        }
        offset += line.len();
    }
    if let Some(start) = quote_start {
        cut = cut.min(start);
    }

    // Signature delimiter "-- " (RFC 3676); also accept "--" alone, which many clients send.
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let l = line.trim_end_matches('\n');
        if (l == "-- " || l == "--") && offset < cut {
            cut = offset;
            break;
        }
        offset += line.len();
    }

    let head = text[..cut].trim_end();
    let tail = text[cut..].trim();
    if head.trim().is_empty() {
        return (text.trim().to_owned(), None);
    }
    (
        trim_blank_lines(head),
        (!tail.is_empty()).then(|| tail.to_owned()),
    )
}

/// Collapses runs of 3+ blank lines that HTML-to-text conversion leaves behind.
fn trim_blank_lines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut blanks = 0;
    for line in s.trim().lines() {
        if line.trim().is_empty() {
            blanks += 1;
            if blanks > 1 {
                continue;
            }
        } else {
            blanks = 0;
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::split;

    #[test]
    fn gmail_reply_with_wrapped_attribution() {
        let (body, quoted) = split(
            "Sounds good, see you Tuesday.\n\nOn Mon, 26 Oct 2026 at 14:02, Peter Horváth <\npeter@example.sk> wrote:\n> Tuesday at 14:00 works.\n> Peter\n",
        );
        assert_eq!(body, "Sounds good, see you Tuesday.");
        assert!(quoted.unwrap().contains("Tuesday at 14:00"));
    }

    #[test]
    fn slovak_and_ukrainian_attributions() {
        let (b, _) =
            split("Ďakujem!\n\nDňa po 26. 10. 2026 o 9:12 Jana Mináriková napísal(a):\n> text");
        assert_eq!(b, "Ďakujem!");
        let (b, _) = split(
            "Так, буду.\n\nУ пн, 26 жовт. 2026 о 10:00 Олена <o@example.com> пише:\n> Ти приїдеш?",
        );
        assert_eq!(b, "Так, буду.");
    }

    #[test]
    fn outlook_header_block() {
        let (b, q) = split(
            "Please see attached.\n\nBest,\nAnna\n\nFrom: Alex <alex@example.uk>\nSent: Monday, October 26, 2026 9:00 AM\nTo: Anna\nSubject: Invoice\n\nHi Anna,",
        );
        assert_eq!(b, "Please see attached.\n\nBest,\nAnna");
        assert!(q.unwrap().starts_with("From: Alex"));
    }

    #[test]
    fn outlook_separator_line() {
        let (b, _) = split("Yes.\n\n-----Original Message-----\nFrom: x\nSent: y");
        assert_eq!(b, "Yes.");
        let (b, _) = split("Yes.\n________________________________\nOd: x\nOdoslané: y");
        assert_eq!(b, "Yes.");
    }

    #[test]
    fn signature_and_device_footer() {
        let (b, q) = split("Will do.\n\n-- \nAlex Novak\nMailbub");
        assert_eq!(b, "Will do.");
        assert_eq!(q.unwrap(), "-- \nAlex Novak\nMailbub");
        let (b, _) = split("On my way\n\nSent from my iPhone");
        assert_eq!(b, "On my way");
    }

    #[test]
    fn inline_quotes_followed_by_reply_are_kept() {
        let text = "> Can you add a baseline?\nYes, added in 2.3.\n";
        let (b, q) = split(text);
        assert_eq!(b, "> Can you add a baseline?\nYes, added in 2.3.");
        assert!(q.is_none());
    }

    #[test]
    fn fully_quoted_body_is_kept_whole() {
        let (b, q) = split("> only a quote\n> nothing else");
        assert_eq!(b, "> only a quote\n> nothing else");
        assert!(q.is_none());
    }

    #[test]
    fn plain_message_untouched_except_blank_runs() {
        let (b, q) = split("Hi,\n\n\n\nThe exam is on 23 December.\n");
        assert_eq!(b, "Hi,\n\nThe exam is on 23 December.");
        assert!(q.is_none());
    }
}
