//! Reads account settings (never passwords) from Apple Mail via AppleScript and turns them
//! into setup suggestions. Asking Mail needs one Automation consent instead of the
//! Full Disk Access that reading ~/Library/Accounts would.

use std::time::Duration;

use futures_util::future::join_all;
use tokio::process::Command;

use crate::discover::{self, find_imap, find_smtp, provider_for_domain};
use crate::error::{AppError, ErrorCode, Result};
use crate::model::{ImportCandidate, Provider, Security, Server};

const SCRIPT: &str = include_str!("../scripts/mail-accounts.applescript");
const FIELD_SEP: char = '\u{1f}';

/// One account as Mail describes it.
#[derive(Debug, Default, PartialEq)]
struct MailAccount {
    class: String,
    name: String,
    enabled: bool,
    emails: Vec<String>,
    full_name: String,
    user_name: String,
    imap_host: Option<String>,
    smtp_host: Option<String>,
}

fn non_empty(s: &str) -> Option<String> {
    let s = s.trim();
    (!s.is_empty() && s != "missing value").then(|| s.to_owned())
}

fn parse(output: &str) -> Vec<MailAccount> {
    output
        .lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split(FIELD_SEP).collect();
            let get = |i: usize| f.get(i).copied().unwrap_or_default();
            let emails: Vec<String> = get(3)
                .split(',')
                .filter_map(non_empty)
                .map(|e| e.to_lowercase())
                .collect();
            if emails.is_empty() {
                return None;
            }
            Some(MailAccount {
                class: get(0).trim().to_owned(),
                name: get(1).trim().to_owned(),
                enabled: get(2).trim() == "true",
                emails,
                full_name: get(4).trim().to_owned(),
                user_name: get(5).trim().to_owned(),
                imap_host: non_empty(get(6)),
                // Fields 7-8 are Mail's IMAP port/SSL; Mailbub re-probes for implicit TLS.
                smtp_host: non_empty(get(9)),
            })
        })
        .collect()
}

async fn run_script() -> Result<String> {
    let out = tokio::time::timeout(
        Duration::from_secs(60),
        Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(SCRIPT)
            .output(),
    )
    .await
    .map_err(|_| AppError::new(ErrorCode::MailUnavailable, "Mail did not answer in time"))?
    .map_err(|e| AppError::new(ErrorCode::MailUnavailable, e.to_string()))?;

    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    // -1743: the user (or a previous answer) denied Automation access to Mail.
    let code = if err.contains("-1743") {
        ErrorCode::MailNotAllowed
    } else {
        ErrorCode::MailUnavailable
    };
    Err(AppError::new(code, err.trim().to_owned()))
}

async fn resolve(acc: MailAccount, connected: &[String]) -> ImportCandidate {
    let email = acc.emails[0].clone();
    let domain = email.rsplit('@').next().unwrap_or_default().to_owned();
    let is_gmail = acc.imap_host.as_deref() == Some("imap.gmail.com")
        || provider_for_domain(&domain) == Some(Provider::Gmail);

    let (provider, imap, smtp) = if is_gmail {
        let d = discover::known(Provider::Gmail);
        (Provider::Gmail, d.imap, d.smtp)
    } else if let Some(host) = acc.imap_host.clone() {
        // Mail may use port 143 + STARTTLS; Mailbub needs implicit TLS, so probe 993.
        let smtp_hosts: Vec<String> = acc.smtp_host.iter().chain([&host]).cloned().collect();
        let imap_hosts = [host];
        let (imap, smtp) = tokio::join!(find_imap(&imap_hosts), find_smtp(&smtp_hosts));
        (Provider::Imap, imap, smtp)
    } else {
        // Exchange / Microsoft accounts carry no IMAP settings in Mail.
        let d = discover::discover(&email).await;
        (d.provider, d.imap, d.smtp)
    };

    let username = if acc.user_name.is_empty() || acc.user_name.eq_ignore_ascii_case(&email) {
        email.clone()
    } else {
        acc.user_name
    };
    ImportCandidate {
        name: if acc.name.is_empty() {
            domain
        } else {
            acc.name
        },
        already_connected: acc
            .emails
            .iter()
            .any(|e| connected.iter().any(|c| c.eq_ignore_ascii_case(e))),
        aliases: acc.emails[1..].to_vec(),
        email,
        username,
        display_name: acc.full_name,
        provider,
        imap,
        smtp: smtp.or_else(|| fallback_smtp(provider)),
    }
}

fn fallback_smtp(provider: Provider) -> Option<Server> {
    (provider == Provider::Gmail).then(|| Server {
        host: "smtp.gmail.com".into(),
        port: 465,
        security: Security::Tls,
    })
}

pub async fn list(connected: Vec<String>) -> Result<Vec<ImportCandidate>> {
    let accounts: Vec<MailAccount> = parse(&run_script().await?)
        .into_iter()
        .filter(|a| a.enabled && a.class != "pop account")
        .collect();
    Ok(join_all(accounts.into_iter().map(|a| resolve(a, &connected))).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(fields: &[&str]) -> String {
        fields.join(&FIELD_SEP.to_string())
    }

    #[test]
    fn parses_imap_and_exchange_rows() {
        let out = [
            line(&[
                "imap account",
                "Bridge",
                "true",
                "me@bridge.example",
                "Me",
                "me@bridge.example",
                "mail.bridge.example",
                "143",
                "true",
                "mail.bridge.example",
                "25",
                "true",
                "me@bridge.example",
            ]),
            line(&[
                "account",
                "Work",
                "true",
                "Me@Work.example,sales@work.example",
                "Ме",
                "login@work.example",
                "missing value",
                "0",
                "false",
                "",
                "",
                "",
                "",
            ]),
            line(&[
                "imap account",
                "Broken",
                "true",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
            ]),
        ]
        .join("\n");
        let parsed = parse(&out);
        assert_eq!(parsed.len(), 2, "rows without an address are skipped");
        assert_eq!(parsed[0].imap_host.as_deref(), Some("mail.bridge.example"));
        assert_eq!(parsed[0].smtp_host.as_deref(), Some("mail.bridge.example"));
        assert_eq!(parsed[1].imap_host, None);
        assert_eq!(parsed[1].emails, ["me@work.example", "sales@work.example"]);
        assert_eq!(parsed[1].user_name, "login@work.example");
        assert_eq!(parsed[1].full_name, "Ме");
    }

    #[tokio::test]
    async fn gmail_gets_presets_without_probing() {
        let acc = MailAccount {
            class: "imap account".into(),
            name: "Personal".into(),
            enabled: true,
            emails: vec!["me@gmail.com".into()],
            imap_host: Some("imap.gmail.com".into()),
            user_name: "me@gmail.com".into(),
            ..Default::default()
        };
        let c = resolve(acc, &["ME@gmail.com".into()]).await;
        assert_eq!(c.provider, Provider::Gmail);
        assert_eq!(c.smtp.unwrap().host, "smtp.gmail.com");
        assert!(c.already_connected);
        assert_eq!(c.username, "me@gmail.com");
    }

    // Talks to the real Mail app (asks for Automation access once).
    #[tokio::test]
    #[ignore]
    async fn reads_real_mail_accounts() {
        for c in list(vec![]).await.unwrap() {
            println!(
                "{:?} {} user={} imap={:?} smtp={:?}",
                c.provider,
                c.email,
                c.username,
                c.imap.map(|s| (s.host, s.port)),
                c.smtp.map(|s| (s.host, s.port))
            );
        }
    }
}
