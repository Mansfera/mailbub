//! "Add to Calendar" (an .ics file opened in the default calendar app) and place lookup for
//! map previews (OpenStreetMap Nominatim: free, no key, light use only).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{AppError, ErrorCode, Result};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewEvent {
    pub title: String,
    /// Unix seconds.
    pub start: i64,
    pub end: Option<i64>,
    pub all_day: bool,
    pub location: Option<String>,
    pub notes: Option<String>,
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace("\r\n", "\\n")
        .replace('\n', "\\n")
}

/// UTC "YYYYMMDDTHHMMSSZ", or "YYYYMMDD" for all-day events.
fn stamp(unix: i64, date_only: bool) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    if date_only {
        format!("{y:04}{m:02}{d:02}")
    } else {
        format!(
            "{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z",
            secs / 3600,
            secs % 3600 / 60,
            secs % 60
        )
    }
}

pub fn to_ics(e: &NewEvent, now: i64) -> String {
    let end = e
        .end
        .filter(|end| *end > e.start)
        .unwrap_or(e.start + if e.all_day { 86_400 } else { 3600 });
    let (start_line, end_line) = if e.all_day {
        (
            format!("DTSTART;VALUE=DATE:{}", stamp(e.start, true)),
            format!("DTEND;VALUE=DATE:{}", stamp(end, true)),
        )
    } else {
        (
            format!("DTSTART:{}", stamp(e.start, false)),
            format!("DTEND:{}", stamp(end, false)),
        )
    };
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_owned(),
        "VERSION:2.0".to_owned(),
        "PRODID:-//mansfera//Mailbub//EN".to_owned(),
        "BEGIN:VEVENT".to_owned(),
        format!("UID:{}@mailbub", uuid::Uuid::new_v4()),
        format!("DTSTAMP:{}", stamp(now, false)),
        start_line,
        end_line,
        format!("SUMMARY:{}", escape(&e.title)),
    ];
    if let Some(l) = e.location.as_deref().filter(|l| !l.trim().is_empty()) {
        lines.push(format!("LOCATION:{}", escape(l)));
    }
    if let Some(n) = e.notes.as_deref().filter(|n| !n.trim().is_empty()) {
        lines.push(format!("DESCRIPTION:{}", escape(n)));
    }
    lines.extend(["END:VEVENT".to_owned(), "END:VCALENDAR".to_owned()]);
    lines.join("\r\n") + "\r\n"
}

/// Writes the event to a temporary .ics file and opens it; Calendar asks where to add it.
pub fn open_in_calendar(app: &tauri::AppHandle, dir: &std::path::Path, e: &NewEvent) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!("event-{}.ics", uuid::Uuid::new_v4().simple()));
    std::fs::write(&path, to_ics(e, crate::oauth::now()))?;
    crate::files::open_with_system(app, path.as_os_str())
}

#[derive(Debug, Clone, Serialize)]
pub struct Place {
    pub lat: f64,
    pub lon: f64,
    pub name: String,
}

#[derive(Deserialize)]
struct NominatimHit {
    lat: String,
    lon: String,
    display_name: String,
}

/// Results are cached for the session; Nominatim asks for at most one request per second.
#[derive(Default)]
pub struct Geocoder(Mutex<HashMap<String, Option<Place>>>);

impl Geocoder {
    pub async fn lookup(&self, query: &str) -> Result<Option<Place>> {
        let key = query.trim().to_lowercase();
        if let Some(hit) = self.0.lock().ok().and_then(|c| c.get(&key).cloned()) {
            return Ok(hit);
        }
        let hits: Vec<NominatimHit> = crate::connect::http_builder()
            .timeout(Duration::from_secs(10))
            .user_agent("Mailbub/0.1 (desktop mail client by mansfera)")
            .build()
            .map_err(|e| AppError::new(ErrorCode::Invalid, e.to_string()))?
            .get("https://nominatim.openstreetmap.org/search")
            .query(&[("format", "jsonv2"), ("limit", "1"), ("q", query.trim())])
            .send()
            .await
            .map_err(|e| AppError::new(ErrorCode::Invalid, format!("Map lookup failed: {e}")))?
            .json()
            .await
            .map_err(|e| AppError::new(ErrorCode::Invalid, format!("Map lookup failed: {e}")))?;
        let place = hits.into_iter().next().and_then(|h| {
            Some(Place {
                lat: h.lat.parse().ok()?,
                lon: h.lon.parse().ok()?,
                name: h.display_name,
            })
        });
        if let Ok(mut c) = self.0.lock() {
            c.insert(key, place.clone());
        }
        Ok(place)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_timed_and_all_day_events() {
        let e = NewEvent {
            title: "Thesis, chapter 2; review".into(),
            start: 1_793_710_800, // 2026-11-03 13:00 UTC
            end: None,
            all_day: false,
            location: Some("M-163\nFMFI".into()),
            notes: None,
        };
        let ics = to_ics(&e, 0);
        assert!(ics.contains("DTSTART:20261103T130000Z"));
        assert!(
            ics.contains("DTEND:20261103T140000Z"),
            "defaults to one hour"
        );
        assert!(ics.contains("SUMMARY:Thesis\\, chapter 2\\; review"));
        assert!(ics.contains("LOCATION:M-163\\nFMFI"));

        let day = NewEvent { all_day: true, ..e };
        let ics = to_ics(&day, 0);
        assert!(ics.contains("DTSTART;VALUE=DATE:20261103"));
        assert!(ics.contains("DTEND;VALUE=DATE:20261104"));
    }
}
