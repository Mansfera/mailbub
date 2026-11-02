//! Profile pictures for the people we write with, served to the UI as `avatar://<address>`.
//! Tried in order: Gravatar for the address, the domain's BIMI brand logo, then the site's
//! touch icon or favicon for a company domain (never for public mailbox providers like
//! gmail.com). Results, misses included, are cached on disk so each address is looked up
//! rarely.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use hickory_resolver::TokioResolver;
use hickory_resolver::proto::rr::{RData, RecordType};
use sha2::{Digest, Sha256};

const HIT_TTL: Duration = Duration::from_secs(30 * 24 * 3600);
const MISS_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
const MAX_BYTES: usize = 512 * 1024;
/// 16px favicons are too blurry even as a small centred logo.
const MIN_ICON_PX: u32 = 32;

const PUBLIC_DOMAINS: &[&str] = &[
    "gmail.com",
    "googlemail.com",
    "outlook.com",
    "hotmail.com",
    "live.com",
    "msn.com",
    "yahoo.com",
    "ymail.com",
    "icloud.com",
    "me.com",
    "mac.com",
    "aol.com",
    "proton.me",
    "protonmail.com",
    "pm.me",
    "gmx.com",
    "gmx.net",
    "gmx.de",
    "web.de",
    "mail.com",
    "yandex.ru",
    "yandex.com",
    "mail.ru",
    "ukr.net",
    "i.ua",
    "meta.ua",
    "seznam.cz",
    "centrum.sk",
    "azet.sk",
    "zoznam.sk",
    "post.sk",
    "atlas.sk",
    "zoho.com",
    "fastmail.com",
    "tutanota.com",
    "hey.com",
];

/// Second-level labels under which organisations register (company.co.uk, firm.com.ua).
const SHARED_SLDS: &[&str] = &["co", "com", "org", "net", "ac", "gov", "edu", "ltd", "plc"];

#[derive(Default)]
pub struct Avatars {
    /// One lookup per address at a time; later requests wait and read the cache.
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

pub struct Image {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
}

impl Avatars {
    pub async fn get(&self, dir: &Path, address: &str) -> Option<Image> {
        let address = address.trim().to_lowercase();
        if !address.contains('@') {
            return None;
        }
        let path = dir.join(hex(&Sha256::digest(address.as_bytes())));
        if let Some(cached) = read_cache(&path) {
            return cached;
        }
        let lock = self
            .locks
            .lock()
            .ok()?
            .entry(address.clone())
            .or_default()
            .clone();
        let _guard = lock.lock().await;
        if let Some(cached) = read_cache(&path) {
            return cached;
        }
        let found = lookup(&address).await;
        let _ = std::fs::create_dir_all(dir);
        // An empty file records a miss.
        let _ = std::fs::write(&path, found.as_deref().unwrap_or_default());
        if let Ok(mut locks) = self.locks.lock() {
            locks.remove(&address);
        }
        found.and_then(|b| sniff(&b).map(|mime| Image { bytes: b, mime }))
    }
}

/// `None` when there is no fresh entry; `Some(None)` for a cached miss.
fn read_cache(path: &PathBuf) -> Option<Option<Image>> {
    let meta = std::fs::metadata(path).ok()?;
    let age = SystemTime::now()
        .duration_since(meta.modified().ok()?)
        .unwrap_or_default();
    let ttl = if meta.len() == 0 { MISS_TTL } else { HIT_TTL };
    if age > ttl {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    Some(sniff(&bytes).map(|mime| Image { bytes, mime }))
}

async fn lookup(address: &str) -> Option<Vec<u8>> {
    let http = crate::connect::http_builder()
        .timeout(Duration::from_secs(8))
        .build()
        .ok()?;
    let hash = hex(&Sha256::digest(address.as_bytes()));
    if let Some(b) = fetch(
        &http,
        &format!("https://gravatar.com/avatar/{hash}?s=160&d=404"),
    )
    .await
    {
        return Some(b);
    }
    let domain = address.rsplit('@').next()?;
    if PUBLIC_DOMAINS.contains(&domain) {
        return None;
    }
    let org = org_domain(domain);
    for d in [domain, org.as_str()] {
        if let Some(url) = bimi_logo(d).await
            && let Some(b) = fetch(&http, &url).await
        {
            return Some(b);
        }
        if d == org {
            break;
        }
    }
    let icons = [
        format!("https://{org}/apple-touch-icon.png"),
        format!("https://www.google.com/s2/favicons?domain={org}&sz=128"),
    ];
    for url in icons {
        if let Some(icon) = fetch(&http, &url).await
            && png_width(&icon).unwrap_or(MIN_ICON_PX) >= MIN_ICON_PX
        {
            return Some(icon);
        }
    }
    None
}

async fn fetch(http: &reqwest::Client, url: &str) -> Option<Vec<u8>> {
    let res = http.get(url).send().await.ok()?;
    if !res.status().is_success() {
        return None;
    }
    let bytes = res.bytes().await.ok()?;
    (bytes.len() <= MAX_BYTES && sniff(&bytes).is_some()).then(|| bytes.to_vec())
}

/// The logo URL from the domain's `default._bimi` TXT record.
async fn bimi_logo(domain: &str) -> Option<String> {
    let resolver = TokioResolver::builder_tokio().ok()?.build().ok()?;
    let lookup = tokio::time::timeout(
        Duration::from_secs(4),
        resolver.lookup(format!("default._bimi.{domain}."), RecordType::TXT),
    )
    .await
    .ok()?
    .ok()?;
    lookup.answers().iter().find_map(|r| match &r.data {
        RData::TXT(txt) => {
            let text: String = txt
                .txt_data
                .iter()
                .map(|s| String::from_utf8_lossy(s))
                .collect();
            bimi_url(&text)
        }
        _ => None,
    })
}

fn bimi_url(record: &str) -> Option<String> {
    if !record.trim_start().starts_with("v=BIMI1") {
        return None;
    }
    record
        .split(';')
        .filter_map(|tag| tag.trim().split_once('='))
        .find(|(k, _)| k.trim() == "l")
        .map(|(_, v)| v.trim().to_owned())
        .filter(|v| v.starts_with("https://"))
}

/// mail.news.company.co.uk -> company.co.uk
fn org_domain(domain: &str) -> String {
    let labels: Vec<&str> = domain.split('.').collect();
    let n = labels.len();
    let keep = if n >= 3 && labels[n - 1].len() == 2 && SHARED_SLDS.contains(&labels[n - 2]) {
        3
    } else {
        2
    };
    labels[n.saturating_sub(keep)..].join(".")
}

fn sniff(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(b"\x89PNG") {
        Some("image/png")
    } else if b.starts_with(b"\xFF\xD8\xFF") {
        Some("image/jpeg")
    } else if b.starts_with(b"GIF8") {
        Some("image/gif")
    } else if b.len() > 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some("image/webp")
    } else if b.starts_with(b"\0\0\x01\0") {
        Some("image/x-icon")
    } else {
        let head = String::from_utf8_lossy(&b[..b.len().min(512)]).to_lowercase();
        head.contains("<svg").then_some("image/svg+xml")
    }
}

fn png_width(b: &[u8]) -> Option<u32> {
    (b.starts_with(b"\x89PNG") && b.len() >= 24)
        .then(|| u32::from_be_bytes([b[16], b[17], b[18], b[19]]))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn org_domains() {
        assert_eq!(org_domain("skoda-auto.com"), "skoda-auto.com");
        assert_eq!(org_domain("em.news.company.com"), "company.com");
        assert_eq!(org_domain("mail.company.co.uk"), "company.co.uk");
        assert_eq!(org_domain("cs.university.sk"), "university.sk");
    }

    #[test]
    fn bimi_records() {
        assert_eq!(
            bimi_url("v=BIMI1; l=https://x.example/logo.svg; a=https://x.example/a.pem").as_deref(),
            Some("https://x.example/logo.svg")
        );
        assert_eq!(bimi_url("v=BIMI1; l=; a=;"), None);
        assert_eq!(bimi_url("v=spf1 -all"), None);
    }

    #[test]
    fn sniffs_images_only() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n"), Some("image/png"));
        assert_eq!(
            sniff(b"<?xml version=\"1.0\"?><svg>"),
            Some("image/svg+xml")
        );
        assert_eq!(sniff(b"<!doctype html><html>"), None);
    }
}
