// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Named hooks and the format `index-v1`, the webhook of the Pebble app for
//! the Index ring (ADR 0010, ADR 0022).
//!
//! Taken from catastr (`catastr-core/src/hook.rs`, `index_webhook.rs`) as a
//! template, not a dependency: it belongs to neither API.
//!
//! The app signs `"v1\n" + timestamp + "\n" + delivery + "\n" + trigger +
//! "\n" + ("1" | "0") + "\n"` followed by the raw multipart body with
//! HMAC-SHA256, keyed with the UTF-8 bytes of the secret as entered, and
//! sends the lowercase hex in `X-Index-Signature`. The timestamp is in
//! seconds, `recordedAt` in milliseconds.
//!
//! Pure: the caller hands in the headers, the body and the time. Every
//! refusal is a 4xx, because the app retries 408, 429 and 5xx and gives up
//! on any other — a forged request must not be sent again ten times.

use hmac::{Hmac, Mac};
use sha2::Sha256;

/// The longest hook name: it is a path segment typed into a phone.
pub const HOOK_NAME_LIMIT: usize = 64;

/// A hook name that cannot be one.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "{0:?} is not a hook name: 1 to 64 characters of a-z, 0-9, `-` and `_`, starting with a letter or digit"
)]
pub struct InvalidHookName(pub String);

/// Checks a hook name: lower-case ASCII, because it is part of a URL typed
/// into the Pebble app.
///
/// # Errors
///
/// [`InvalidHookName`] for anything else.
pub fn validate_hook_name(name: &str) -> Result<(), InvalidHookName> {
    let allowed = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_';
    let starts_well = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    if name.len() <= HOOK_NAME_LIMIT && starts_well && name.chars().all(allowed) {
        Ok(())
    } else {
        Err(InvalidHookName(name.to_string()))
    }
}

/// The system named with a delivery id from the ring: the id is the Pebble
/// app's (ADR 0022 §1).
pub const INDEX_RING_SYSTEM: &str = "pebble-index";

/// The protocol version this format reads (`X-Index-Webhook-Version`).
pub const INDEX_WEBHOOK_VERSION: &str = "1";

/// Header names, lower case as `http` hands them out.
pub mod index_header {
    pub const VERSION: &str = "x-index-webhook-version";
    pub const TRIGGER: &str = "x-index-trigger";
    pub const TEST: &str = "x-index-test";
    pub const DELIVERY: &str = "x-index-delivery";
    pub const SIGNATURE: &str = "x-index-signature";
    pub const TIMESTAMP: &str = "x-index-timestamp";
}

/// Multipart field names.
pub mod index_field {
    pub const AUDIO: &str = "audio";
    pub const TRANSCRIPTION: &str = "transcription";
    pub const RECORDED_AT: &str = "recordedAt";
}

/// How far a request's timestamp may lie from the server's clock: five
/// minutes, as the app's documentation advises.
pub const INDEX_MAX_SKEW_SECONDS: i64 = 300;

/// The largest request body taken. A recording is 16 kHz AAC, a few
/// hundred KiB a minute; the audio is read and dropped (ADR 0010 §4).
pub const INDEX_BODY_LIMIT: usize = 64 << 20;

/// The longest trigger taken. The app sends a dozen characters.
pub const INDEX_TRIGGER_LIMIT: usize = 64;

/// The longest delivery id taken. The app sends a file id or
/// `text-<recordingId>`.
pub const DELIVERY_ID_LIMIT: usize = 128;

/// The headers a signed request carries, as the caller found them.
#[derive(Debug, Clone, Copy, Default)]
pub struct IndexHeaders<'a> {
    pub version: Option<&'a str>,
    pub timestamp: Option<&'a str>,
    pub delivery: Option<&'a str>,
    pub trigger: Option<&'a str>,
    pub test: Option<&'a str>,
    pub signature: Option<&'a str>,
}

/// What a request that passed says about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexDelivery {
    pub delivery: String,
    pub trigger: String,
    pub test: bool,
}

/// Why a request was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IndexRefusal {
    #[error("unsupported webhook version {0:?}; this hook reads version 1")]
    Version(String),
    #[error("no {0} header; turn on \"Sign requests\" for this gesture in the Pebble app")]
    Missing(&'static str),
    #[error("the timestamp {0:?} is not Unix seconds")]
    Timestamp(String),
    #[error("the request was signed {0} s away from the server's clock (at most 300)")]
    Expired(i64),
    #[error("the signature does not match; is the hook's secret entered in the Pebble app?")]
    Signature,
    #[error("{0}")]
    Header(String),
}

impl IndexRefusal {
    /// The HTTP status: 400 for a request this format cannot read, 401 for
    /// one it cannot trust. Both are final for the app.
    #[must_use]
    pub fn status(&self) -> u16 {
        match self {
            IndexRefusal::Version(_) | IndexRefusal::Timestamp(_) | IndexRefusal::Header(_) => 400,
            IndexRefusal::Missing(_) | IndexRefusal::Expired(_) | IndexRefusal::Signature => 401,
        }
    }
}

/// The bytes the app signs.
#[must_use]
pub fn index_signature_input(
    timestamp: i64,
    delivery: &str,
    trigger: &str,
    test: bool,
    body: &[u8],
) -> Vec<u8> {
    let prefix = format!(
        "v{INDEX_WEBHOOK_VERSION}\n{timestamp}\n{delivery}\n{trigger}\n{}\n",
        if test { '1' } else { '0' }
    );
    let mut input = Vec::with_capacity(prefix.len() + body.len());
    input.extend_from_slice(prefix.as_bytes());
    input.extend_from_slice(body);
    input
}

/// Signs as the app does: the lowercase hex of the HMAC. For tests, which
/// stand in for the app.
#[must_use]
pub fn index_signature(
    secret: &str,
    timestamp: i64,
    delivery: &str,
    trigger: &str,
    test: bool,
    body: &[u8],
) -> String {
    let mut mac = new_mac(secret);
    mac.update(&index_signature_input(
        timestamp, delivery, trigger, test, body,
    ));
    to_hex(&mac.finalize().into_bytes())
}

/// Checks a request: version, timestamp within five minutes of `now`
/// (Unix seconds), the delivery id and trigger readable, the signature in
/// constant time.
///
/// # Errors
///
/// [`IndexRefusal`] with the reason and, through [`IndexRefusal::status`],
/// the status to answer with.
pub fn verify_index_request(
    secret: &str,
    headers: &IndexHeaders<'_>,
    body: &[u8],
    now: i64,
) -> Result<IndexDelivery, IndexRefusal> {
    let version = headers
        .version
        .ok_or(IndexRefusal::Missing("X-Index-Webhook-Version"))?;
    if version.trim() != INDEX_WEBHOOK_VERSION {
        return Err(IndexRefusal::Version(version.to_string()));
    }
    let signature = headers
        .signature
        .ok_or(IndexRefusal::Missing("X-Index-Signature"))?;
    let stamp = headers
        .timestamp
        .ok_or(IndexRefusal::Missing("X-Index-Timestamp"))?;
    let timestamp: i64 = stamp
        .trim()
        .parse()
        .map_err(|_| IndexRefusal::Timestamp(stamp.to_string()))?;
    let skew = now.saturating_sub(timestamp);
    if skew.saturating_abs() > INDEX_MAX_SKEW_SECONDS {
        return Err(IndexRefusal::Expired(skew));
    }
    let delivery = headers
        .delivery
        .ok_or(IndexRefusal::Missing("X-Index-Delivery"))?;
    let trigger = headers
        .trigger
        .ok_or(IndexRefusal::Missing("X-Index-Trigger"))?;
    let test = headers.test == Some("true");

    let given = from_hex(signature.trim()).ok_or(IndexRefusal::Signature)?;
    let mut mac = new_mac(secret);
    mac.update(&index_signature_input(
        timestamp, delivery, trigger, test, body,
    ));
    // `verify_slice` compares in constant time.
    mac.verify_slice(&given)
        .map_err(|_| IndexRefusal::Signature)?;

    // Checked after the signature, so an unsigned request learns nothing
    // about what would have been taken.
    if delivery.is_empty()
        || delivery.len() > DELIVERY_ID_LIMIT
        || !delivery.chars().all(|c| c.is_ascii_graphic())
    {
        return Err(IndexRefusal::Header(
            "X-Index-Delivery: not a delivery id: 1 to 128 printable ASCII characters".into(),
        ));
    }
    if trigger.is_empty()
        || trigger.len() > INDEX_TRIGGER_LIMIT
        || !trigger.chars().all(|c| c.is_ascii_graphic())
    {
        return Err(IndexRefusal::Header(format!(
            "X-Index-Trigger: not a gesture name: {trigger:?}"
        )));
    }
    Ok(IndexDelivery {
        delivery: delivery.to_string(),
        trigger: trigger.to_string(),
        test,
    })
}

/// The longest title made from the ring's text, in characters (ADR 0010
/// §3).
pub const RING_TITLE_LIMIT: usize = 80;

/// The title of a task from the ring: the first line with text, cut at a
/// word boundary to at most [`RING_TITLE_LIMIT`] characters, `…` marking
/// the cut. A single word longer than that is cut inside it. `None` for a
/// text without words.
#[must_use]
pub fn ring_title(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= RING_TITLE_LIMIT {
        return Some(line);
    }
    // Room for the mark.
    let room: String = line.chars().take(RING_TITLE_LIMIT - 1).collect();
    let next = line.chars().nth(RING_TITLE_LIMIT - 1);
    let kept = if next == Some(' ') {
        room.as_str()
    } else {
        match room.rfind(' ') {
            Some(at) => &room[..at],
            None => room.as_str(),
        }
    };
    let kept = kept.trim_end_matches([',', ';', ':', '-', ' ']);
    Some(format!("{kept}…"))
}

/// The title of a task from the ring that came without text (ADR 0010 §5),
/// `when` already in the account's zone and format.
#[must_use]
pub fn ring_title_without_text(when: &str) -> String {
    format!("Aufnahme vom Ring, {when}")
}

/// How a task from the ring came, besides the hook: `via=tasker` for the
/// broadcast to the app (ADR 0032 §2).
pub const RING_VIA_TASKER: &str = "tasker";

/// The task a recording from the ring becomes, by the hook or through the
/// app (ADR 0010 §3, §5, §8; ADR 0032 §2): into the inbox, the title from
/// the first line or, without text, from the time in `zone`, the whole
/// text as note, `created` the time of the recording, no quick capture.
/// `via` adds the property of the same name.
#[must_use]
pub fn ring_task(
    id: crate::api::Id,
    text: &str,
    recorded: jiff::Timestamp,
    now: jiff::Timestamp,
    zone: &jiff::tz::TimeZone,
    via: Option<&str>,
) -> crate::api::Task {
    let title = ring_title(text).unwrap_or_else(|| {
        let when = recorded.to_zoned(zone.clone()).strftime("%d.%m.%Y %H:%M");
        ring_title_without_text(&when.to_string())
    });
    // Where it came from (ADR 0022 §2, ADR 0023 §5).
    let mut properties =
        std::collections::BTreeMap::from([("source".to_string(), INDEX_RING_SOURCE.to_string())]);
    if let Some(via) = via {
        properties.insert("via".into(), via.into());
    }
    let note = text.trim();
    crate::api::Task {
        created_by: None,
        habit: None,
        id,
        created: recorded,
        modified: now,
        title,
        note: (!note.is_empty()).then(|| note.to_string()),
        project: None,
        parent: None,
        schedule: crate::api::Schedule::default(),
        flagged: false,
        estimate: None,
        priority: None,
        completed_at: None,
        dropped_at: None,
        repeat_of: None,
        sequential: false,
        complete_with_last_child: false,
        tags: Vec::new(),
        properties,
        position: 0,
    }
}

/// The value of `source` on a task from the ring.
pub const INDEX_RING_SOURCE: &str = "index-ring";

#[allow(clippy::expect_used, reason = "HMAC takes keys of every length")]
fn new_mac(secret: &str) -> Hmac<Sha256> {
    <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts keys of every length")
}

fn to_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        // Writing to a String cannot fail.
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(text.get(at..at + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests may unwrap")]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    const SECRET: &str = "0123456789abcdef0123456789abcdef";
    const NOW: i64 = 1_700_000_000;
    const BODY: &[u8] =
        b"--b\r\nContent-Disposition: form-data; name=\"transcription\"\r\n\r\nMilch\r\n--b--\r\n";

    fn headers<'a>(signature: &'a str, test: Option<&'a str>) -> IndexHeaders<'a> {
        IndexHeaders {
            version: Some("1"),
            timestamp: Some("1700000000"),
            delivery: Some("delivery-123"),
            trigger: Some("double-click-hold"),
            test,
            signature: Some(signature),
        }
    }

    fn sign(test: bool) -> String {
        index_signature(SECRET, NOW, "delivery-123", "double-click-hold", test, BODY)
    }

    #[test]
    fn hmac_matches_rfc_4231() {
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&[0x0b; 20]).unwrap();
        mac.update(b"Hi There");
        assert_eq!(
            to_hex(&mac.finalize().into_bytes()),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn a_signed_request_passes() {
        let signature = sign(false);
        let delivery = verify_index_request(SECRET, &headers(&signature, None), BODY, NOW).unwrap();
        assert_eq!(
            delivery,
            IndexDelivery {
                delivery: "delivery-123".into(),
                trigger: "double-click-hold".into(),
                test: false,
            }
        );
    }

    #[test]
    fn a_test_event_is_signed_as_one() {
        let signature = sign(true);
        let delivery =
            verify_index_request(SECRET, &headers(&signature, Some("true")), BODY, NOW).unwrap();
        assert!(delivery.test);
        // The flag is part of what is signed: dropping it breaks the signature.
        assert_eq!(
            verify_index_request(SECRET, &headers(&signature, None), BODY, NOW),
            Err(IndexRefusal::Signature)
        );
    }

    #[test]
    fn a_wrong_secret_or_body_is_refused() {
        let signature = sign(false);
        let refused = verify_index_request("other", &headers(&signature, None), BODY, NOW);
        assert_eq!(refused, Err(IndexRefusal::Signature));
        let refused = verify_index_request(SECRET, &headers(&signature, None), b"x", NOW);
        assert_eq!(refused, Err(IndexRefusal::Signature));
        let refused = verify_index_request(SECRET, &headers("zz", None), BODY, NOW);
        assert_eq!(refused.unwrap_err().status(), 401);
    }

    #[test]
    fn an_old_or_future_request_is_refused() {
        let signature = sign(false);
        let late = verify_index_request(SECRET, &headers(&signature, None), BODY, NOW + 301);
        assert_eq!(late, Err(IndexRefusal::Expired(301)));
        let early = verify_index_request(SECRET, &headers(&signature, None), BODY, NOW - 301);
        assert_eq!(early, Err(IndexRefusal::Expired(-301)));
        assert!(verify_index_request(SECRET, &headers(&signature, None), BODY, NOW + 300).is_ok());
    }

    #[test]
    fn missing_headers_and_versions_are_refused_with_4xx() {
        let signature = sign(false);
        let mut h = headers(&signature, None);
        h.version = Some("2");
        let refusal = verify_index_request(SECRET, &h, BODY, NOW).unwrap_err();
        assert_eq!(refusal.status(), 400);
        let mut h = headers(&signature, None);
        h.signature = None;
        let refusal = verify_index_request(SECRET, &h, BODY, NOW).unwrap_err();
        assert_eq!(refusal, IndexRefusal::Missing("X-Index-Signature"));
        assert_eq!(refusal.status(), 401);
        let mut h = headers(&signature, None);
        h.timestamp = Some("yesterday");
        assert_eq!(
            verify_index_request(SECRET, &h, BODY, NOW)
                .unwrap_err()
                .status(),
            400
        );
    }

    #[test]
    fn hook_names_are_url_segments_in_lower_case() {
        for good in ["ring", "ring-2", "a_b", "0"] {
            assert!(validate_hook_name(good).is_ok(), "{good}");
        }
        for bad in ["", "Ring", "-ring", "ring/x", "ring x", &"a".repeat(65)] {
            assert!(validate_hook_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_title_is_the_first_line_cut_at_a_word() {
        assert_eq!(
            ring_title("\n  Milch   kaufen \nund Brot").as_deref(),
            Some("Milch kaufen")
        );
        assert_eq!(ring_title(" \n\t"), None);
        let long = "wort ".repeat(30);
        let title = ring_title(&long).unwrap();
        assert!(title.chars().count() <= RING_TITLE_LIMIT, "{title}");
        assert!(title.ends_with("wort…"), "{title}");
        let word = "x".repeat(100);
        assert_eq!(ring_title(&word).unwrap().chars().count(), RING_TITLE_LIMIT);
    }

    fn at(s: &str) -> jiff::Timestamp {
        s.parse().unwrap()
    }

    #[test]
    fn a_ring_task_lands_in_the_inbox_with_its_text_as_note() {
        let zone = jiff::tz::TimeZone::get("Europe/Berlin").unwrap();
        let id = crate::api::Id::new();
        let (recorded, now) = (at("2026-11-08T07:15:00Z"), at("2026-11-08T09:00:00Z"));
        let task = ring_task(id, "  Milch kaufen\nund Brot  ", recorded, now, &zone, None);
        assert_eq!(task.id, id);
        assert_eq!(task.title, "Milch kaufen");
        assert_eq!(task.note.as_deref(), Some("Milch kaufen\nund Brot"));
        assert_eq!((task.created, task.modified), (recorded, now));
        assert_eq!(task.project, None);
        assert!(task.tags.is_empty());
        assert_eq!(
            task.properties,
            BTreeMap::from([("source".into(), "index-ring".into())])
        );
    }

    #[test]
    fn a_ring_task_without_text_is_named_after_its_time_in_the_zone() {
        let zone = jiff::tz::TimeZone::get("Europe/Berlin").unwrap();
        let recorded = at("2026-11-08T07:15:00Z");
        let task = ring_task(
            crate::api::Id::new(),
            " \n ",
            recorded,
            recorded,
            &zone,
            None,
        );
        assert_eq!(task.title, "Aufnahme vom Ring, 08.11.2026 08:15");
        assert_eq!(task.note, None);
    }

    #[test]
    fn a_ring_task_through_tasker_says_so_and_is_not_parsed() {
        let zone = jiff::tz::TimeZone::UTC;
        let now = at("2026-11-08T07:15:00Z");
        let long = format!("morgen #einkauf {}", "wort ".repeat(30));
        let task = ring_task(
            crate::api::Id::new(),
            &long,
            now,
            now,
            &zone,
            Some(RING_VIA_TASKER),
        );
        assert_eq!(
            task.properties.get("via").map(String::as_str),
            Some("tasker")
        );
        assert_eq!(
            task.properties.get("source").map(String::as_str),
            Some("index-ring")
        );
        // No quick capture (ADR 0010 §8): no date, no tag, the words stay.
        assert!(task.title.starts_with("morgen #einkauf"), "{}", task.title);
        assert!(task.title.ends_with('…'));
        assert_eq!(task.schedule, crate::api::Schedule::default());
        assert!(task.tags.is_empty());
    }
}
