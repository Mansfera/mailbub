//! Guess IMAP/SMTP servers for an address: known providers first, then MX records
//! and common host names, probed locally (the address never leaves this machine).

use futures_util::future::join_all;
use hickory_resolver::TokioResolver;
use hickory_resolver::proto::rr::RData;

use crate::connect::{probe_plain_smtp, probe_tls};
use crate::model::{Discovered, Provider, Security, Server};

fn server(host: &str, port: u16, security: Security) -> Server {
    Server {
        host: host.into(),
        port,
        security,
    }
}

pub fn gmail() -> (Server, Server) {
    (
        server("imap.gmail.com", 993, Security::Tls),
        server("smtp.gmail.com", 465, Security::Tls),
    )
}

pub fn microsoft() -> (Server, Server) {
    (
        server("outlook.office365.com", 993, Security::Tls),
        server("smtp.office365.com", 587, Security::Starttls),
    )
}

pub(crate) fn known(provider: Provider) -> Discovered {
    let (imap, smtp) = match provider {
        Provider::Gmail => gmail(),
        _ => microsoft(),
    };
    Discovered {
        provider,
        imap: Some(imap),
        smtp: Some(smtp),
    }
}

pub(crate) fn provider_for_domain(domain: &str) -> Option<Provider> {
    match domain {
        "gmail.com" | "googlemail.com" => Some(Provider::Gmail),
        "outlook.com" | "hotmail.com" | "live.com" | "msn.com" => Some(Provider::Microsoft),
        _ => None,
    }
}

fn provider_for_mx(mx: &str) -> Option<Provider> {
    if mx.ends_with(".mail.protection.outlook.com") {
        Some(Provider::Microsoft)
    } else if mx.ends_with(".google.com") || mx.ends_with(".googlemail.com") {
        Some(Provider::Gmail)
    } else {
        None
    }
}

async fn mx_hosts(domain: &str) -> Vec<String> {
    let Some(resolver) = TokioResolver::builder_tokio()
        .ok()
        .and_then(|b| b.build().ok())
    else {
        return vec![];
    };
    let Ok(Ok(lookup)) = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        resolver.mx_lookup(format!("{domain}.")),
    )
    .await
    else {
        return vec![];
    };
    let mut mx: Vec<(u16, String)> = lookup
        .answers()
        .iter()
        .filter_map(|r| match &r.data {
            RData::MX(mx) => Some((
                mx.preference,
                mx.exchange.to_utf8().trim_end_matches('.').to_lowercase(),
            )),
            _ => None,
        })
        .collect();
    mx.sort();
    mx.into_iter().map(|(_, h)| h).collect()
}

pub(crate) async fn find_imap(candidates: &[String]) -> Option<Server> {
    let found = join_all(candidates.iter().map(|h| probe_tls(h, 993, "* OK"))).await;
    candidates
        .iter()
        .zip(found)
        .find(|(_, ok)| *ok)
        .map(|(h, _)| server(h, 993, Security::Tls))
}

pub(crate) async fn find_smtp(candidates: &[String]) -> Option<Server> {
    let tls = join_all(candidates.iter().map(|h| probe_tls(h, 465, "220"))).await;
    if let Some((h, _)) = candidates.iter().zip(tls).find(|(_, ok)| *ok) {
        return Some(server(h, 465, Security::Tls));
    }
    let plain = join_all(candidates.iter().map(|h| probe_plain_smtp(h, 587))).await;
    candidates
        .iter()
        .zip(plain)
        .find(|(_, ok)| *ok)
        .map(|(h, _)| server(h, 587, Security::Starttls))
}

pub async fn discover(email: &str) -> Discovered {
    let domain = email.rsplit('@').next().unwrap_or_default().to_lowercase();
    if let Some(p) = provider_for_domain(&domain) {
        return known(p);
    }

    let mx = mx_hosts(&domain).await;
    if let Some(p) = mx.iter().find_map(|h| provider_for_mx(h)) {
        return known(p);
    }

    let mut imap_hosts = vec![format!("imap.{domain}"), format!("mail.{domain}")];
    imap_hosts.extend(mx.iter().cloned());
    let mut smtp_hosts = vec![format!("smtp.{domain}"), format!("mail.{domain}")];
    smtp_hosts.extend(mx.iter().cloned());

    let (imap, smtp) = tokio::join!(find_imap(&imap_hosts), find_smtp(&smtp_hosts));
    Discovered {
        provider: Provider::Imap,
        imap,
        smtp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_hosted_providers_by_mx() {
        assert_eq!(
            provider_for_mx("contoso-com.mail.protection.outlook.com"),
            Some(Provider::Microsoft)
        );
        assert_eq!(provider_for_mx("aspmx.l.google.com"), Some(Provider::Gmail));
        assert_eq!(provider_for_mx("mail.example.org"), None);
    }

    #[test]
    fn recognises_consumer_domains() {
        assert_eq!(provider_for_domain("gmail.com"), Some(Provider::Gmail));
        assert_eq!(
            provider_for_domain("hotmail.com"),
            Some(Provider::Microsoft)
        );
        assert_eq!(provider_for_domain("example.org"), None);
    }
}

#[cfg(test)]
mod network_tests {
    use super::*;

    // Hits real DNS and servers; run with `cargo test -- --ignored`.
    #[tokio::test]
    #[ignore]
    async fn finds_imap_and_smtp_by_host_name() {
        let d = discover("someone@fastmail.com").await;
        assert_eq!(d.provider, Provider::Imap);
        assert_eq!(d.imap.unwrap().host, "imap.fastmail.com");
        assert_eq!(d.smtp.unwrap().host, "smtp.fastmail.com");
    }

    #[tokio::test]
    #[ignore]
    async fn routes_m365_tenant_to_microsoft() {
        assert_eq!(
            discover("someone@microsoft.com").await.provider,
            Provider::Microsoft
        );
    }
}
