// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A delivery as the Pebble app sends it (format `index-v1`, ADR 0010), for
//! `samu hook send`, which stands in for the app.
//!
//! The command line depends on the wire types only, so the signing lives
//! here a second time; the server checks with `samu-core::hook`, and the
//! integration test sends from here to there, so the two cannot drift
//! apart unnoticed.

use hmac::{Hmac, Mac};
use sha2::Sha256;

const BOUNDARY: &str = "samu-hook-send-7d1c4b2a";

/// What one delivery carries.
#[derive(Debug, Clone)]
pub(crate) struct Delivery {
    pub(crate) delivery: String,
    pub(crate) text: Option<String>,
    /// Unix milliseconds of the recording.
    pub(crate) recorded_at: i64,
    pub(crate) test: bool,
}

/// The headers and the body of a signed request, `timestamp` in Unix
/// seconds.
pub(crate) fn request(
    secret: &str,
    delivery: &Delivery,
    timestamp: i64,
) -> (String, Vec<(&'static str, String)>, Vec<u8>) {
    let mut body = String::new();
    let mut part = |name: &str, value: &str| {
        body.push_str(&format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
        ));
    };
    if let Some(text) = &delivery.text {
        part("transcription", text);
    }
    part("recordedAt", &delivery.recorded_at.to_string());
    part("client", "ring");
    body.push_str(&format!("--{BOUNDARY}--\r\n"));
    let body = body.into_bytes();

    let trigger = if delivery.test {
        "test-event"
    } else {
        "double-click-hold"
    };
    let signature = sign(
        secret,
        timestamp,
        &delivery.delivery,
        trigger,
        delivery.test,
        &body,
    );
    let mut headers = vec![
        ("X-Index-Webhook-Version", "1".to_string()),
        ("X-Index-Timestamp", timestamp.to_string()),
        ("X-Index-Delivery", delivery.delivery.clone()),
        ("X-Index-Trigger", trigger.to_string()),
        ("X-Index-Signature", signature),
    ];
    if delivery.test {
        headers.push(("X-Index-Test", "true".to_string()));
    }
    (
        format!("multipart/form-data; boundary={BOUNDARY}"),
        headers,
        body,
    )
}

#[allow(clippy::expect_used, reason = "HMAC takes keys of every length")]
fn sign(
    secret: &str,
    timestamp: i64,
    delivery: &str,
    trigger: &str,
    test: bool,
    body: &[u8],
) -> String {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts keys of every length");
    mac.update(
        format!(
            "v1\n{timestamp}\n{delivery}\n{trigger}\n{}\n",
            if test { '1' } else { '0' }
        )
        .as_bytes(),
    );
    mac.update(body);
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_test_event_says_so_in_headers_and_signature() {
        let delivery = Delivery {
            delivery: "d".into(),
            text: None,
            recorded_at: 0,
            test: true,
        };
        let (_, headers, _) = request("s", &delivery, 1);
        assert!(headers.contains(&("X-Index-Test", "true".to_string())));
        assert!(headers.contains(&("X-Index-Trigger", "test-event".to_string())));
        let (_, plain, _) = request(
            "s",
            &Delivery {
                test: false,
                ..delivery
            },
            1,
        );
        let signature = |h: &[(&str, String)]| {
            h.iter()
                .find(|(n, _)| *n == "X-Index-Signature")
                .map(|(_, v)| v.clone())
        };
        assert_ne!(signature(&headers), signature(&plain));
    }
}
