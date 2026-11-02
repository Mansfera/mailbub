//! Leaving a mailing list: the `List-Unsubscribe` header (RFC 2369) and its one-click POST
//! (RFC 8058). The header is the same in every language, unlike the footer link's text.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, ErrorCode, Result};

/// Ways the sender offers to unsubscribe, from the message's headers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Unsubscribe {
    /// Web address (https/http): a page to open, or the one-click endpoint.
    pub url: Option<String>,
    pub mailto: Option<String>,
    /// `url` takes a POST and unsubscribes at once, without a page (RFC 8058).
    pub one_click: bool,
}

/// Reads `List-Unsubscribe: <mailto:…>, <https://…>` and `List-Unsubscribe-Post`.
pub fn from_headers(list: Option<&str>, post: Option<&str>) -> Option<Unsubscribe> {
    let mut url = None;
    let mut mailto = None;
    for item in list?.split('<').skip(1) {
        let Some((target, _)) = item.split_once('>') else {
            continue;
        };
        // Folded headers can leave line breaks inside the brackets.
        let target: String = target.split_whitespace().collect();
        let lower = target.to_ascii_lowercase();
        if lower.starts_with("https://") || lower.starts_with("http://") {
            url.get_or_insert(target);
        } else if lower.starts_with("mailto:") {
            mailto.get_or_insert(target);
        }
    }
    if url.is_none() && mailto.is_none() {
        return None;
    }
    let one_click = url
        .as_deref()
        .is_some_and(|u| u.to_ascii_lowercase().starts_with("https://"))
        && post.is_some_and(|p| {
            p.split_whitespace()
                .collect::<String>()
                .eq_ignore_ascii_case("List-Unsubscribe=One-Click")
        });
    Some(Unsubscribe {
        url,
        mailto,
        one_click,
    })
}

/// Sends the RFC 8058 one-click request. Only https, and nothing that identifies the user
/// beyond what the sender put in the address.
#[tauri::command]
pub async fn unsubscribe_one_click(url: String) -> Result<()> {
    if !url.trim().to_ascii_lowercase().starts_with("https://") {
        return Err(AppError::new(
            ErrorCode::Invalid,
            "One-click unsubscribe needs an https address",
        ));
    }
    let res = crate::connect::http_builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| AppError::new(ErrorCode::Invalid, e.to_string()))?
        .post(url.trim())
        .form(&[("List-Unsubscribe", "One-Click")])
        .send()
        .await
        .map_err(|e| {
            AppError::new(
                ErrorCode::Timeout,
                format!("Could not reach the sender: {e}"),
            )
        })?;
    if res.status().is_success() {
        Ok(())
    } else {
        Err(AppError::new(
            ErrorCode::Invalid,
            format!("The sender answered {}", res.status()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_web_and_mail_targets() {
        let u = from_headers(
            Some("<mailto:unsub-1@list.shop.sk?subject=unsubscribe>,\r\n <https://shop.sk/u?id=1>"),
            None,
        )
        .unwrap();
        assert_eq!(u.url.as_deref(), Some("https://shop.sk/u?id=1"));
        assert_eq!(
            u.mailto.as_deref(),
            Some("mailto:unsub-1@list.shop.sk?subject=unsubscribe")
        );
        assert!(!u.one_click);
    }

    #[test]
    fn one_click_needs_post_header_and_https() {
        let post = Some("List-Unsubscribe=One-Click");
        assert!(
            from_headers(Some("<https://x.sk/u>"), post)
                .unwrap()
                .one_click
        );
        assert!(
            !from_headers(Some("<http://x.sk/u>"), post)
                .unwrap()
                .one_click
        );
        assert!(
            !from_headers(Some("<mailto:u@x.sk>"), post)
                .unwrap()
                .one_click
        );
    }

    #[test]
    fn folded_url_and_junk() {
        let u = from_headers(Some("<https://x.sk/u?a=1\r\n &b=2>"), None).unwrap();
        assert_eq!(u.url.as_deref(), Some("https://x.sk/u?a=1&b=2"));
        assert_eq!(from_headers(Some("no brackets here"), None), None);
        assert_eq!(from_headers(Some("<javascript:alert(1)>"), None), None);
        assert_eq!(from_headers(None, None), None);
    }
}
