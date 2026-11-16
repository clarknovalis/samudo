// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The URI in a pairing QR code (ADR 0031 §4):
//! `samu-pair:?server=<url>&account=<name>&device=<name>&code=<code>`.
//!
//! Made and read only here: the server makes it, the phone reads it.
//! Every value is percent-encoded; keys a newer samu adds are skipped.

use std::fmt;

/// The scheme of a [`PairingUri`].
pub const SCHEME: &str = "samu-pair";

/// What a phone needs to pair: where, as whom, under which name, and the
/// code to trade for a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingUri {
    /// `https://samu.example.org`, without a trailing slash.
    pub server: String,
    pub account: String,
    pub device: String,
    pub code: String,
}

/// Why a text is no pairing URI.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PairingUriError {
    #[error("not a samu pairing code (it does not start with {SCHEME}:?)")]
    Scheme,
    #[error("the pairing code lacks {0}")]
    Missing(&'static str),
    #[error("the server in the pairing code is not an http or https address")]
    Server,
    #[error("the pairing code is garbled: {0}")]
    Encoding(String),
}

impl fmt::Display for PairingUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{SCHEME}:?server={}&account={}&device={}&code={}",
            encode(&self.server),
            encode(&self.account),
            encode(&self.device),
            encode(&self.code)
        )
    }
}

impl PairingUri {
    /// Reads a scanned text; spaces around it are forgiven.
    pub fn parse(text: &str) -> Result<Self, PairingUriError> {
        let query = text
            .trim()
            .strip_prefix(SCHEME)
            .and_then(|rest| rest.strip_prefix(":?"))
            .ok_or(PairingUriError::Scheme)?;
        let (mut server, mut account, mut device, mut code) = (None, None, None, None);
        for pair in query.split('&').filter(|p| !p.is_empty()) {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            let value = decode(value)?;
            match key {
                "server" => server = Some(value),
                "account" => account = Some(value),
                "device" => device = Some(value),
                "code" => code = Some(value),
                _ => {}
            }
        }
        let wanted = |v: Option<String>, name| {
            v.filter(|v| !v.is_empty())
                .ok_or(PairingUriError::Missing(name))
        };
        let server = wanted(server, "server")?.trim_end_matches('/').to_string();
        if !(server.starts_with("https://") || server.starts_with("http://")) {
            return Err(PairingUriError::Server);
        }
        Ok(Self {
            server,
            account: wanted(account, "account")?,
            device: wanted(device, "device")?,
            code: wanted(code, "code")?,
        })
    }
}

/// Everything but the unreserved characters of RFC 3986 as `%XX`.
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn decode(text: &str) -> Result<String, PairingUriError> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text
                .get(i + 1..i + 3)
                .ok_or_else(|| PairingUriError::Encoding(format!("a lone % in {text:?}")))?;
            let byte = u8::from_str_radix(hex, 16)
                .map_err(|_| PairingUriError::Encoding(format!("%{hex} in {text:?}")))?;
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|e| PairingUriError::Encoding(e.to_string()))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests may unwrap")]

    use super::*;

    fn uri() -> PairingUri {
        PairingUri {
            server: "https://samu.example.org".into(),
            account: "privat".into(),
            device: "Pixel 9 Pro".into(),
            code: "3f9a0c4d5e6b7a8190a1b2c3d4e5f607".into(),
        }
    }

    #[test]
    fn written_and_read_back() {
        let text = uri().to_string();
        assert_eq!(
            text,
            "samu-pair:?server=https%3A%2F%2Fsamu.example.org&account=privat\
             &device=Pixel%209%20Pro&code=3f9a0c4d5e6b7a8190a1b2c3d4e5f607"
        );
        assert_eq!(PairingUri::parse(&text).unwrap(), uri());
        // As the fixture of samu-api has it.
        let fixture = "samu-pair:?server=https%3A%2F%2Fsamu.example.org&account=privat\
                       &device=pixel&code=3f9a0c4d5e6b7a8190a1b2c3d4e5f607";
        assert_eq!(PairingUri::parse(fixture).unwrap().device, "pixel");
    }

    #[test]
    fn umlauts_and_newer_keys_and_a_slash_at_the_end() {
        let mut u = uri();
        u.device = "Alices Tablet ä/ö".into();
        let text = format!("  {u}&colour=blue\n").replace(
            "server=https%3A%2F%2Fsamu.example.org",
            "server=https%3A%2F%2Fsamu.example.org%2F",
        );
        assert_eq!(PairingUri::parse(&text).unwrap(), u);
    }

    #[test]
    fn what_is_not_a_pairing_uri_says_why() {
        assert_eq!(
            PairingUri::parse("https://samu.example.org"),
            Err(PairingUriError::Scheme)
        );
        assert_eq!(
            PairingUri::parse("samu-pair:?server=https%3A%2F%2Fx&account=a&device=d"),
            Err(PairingUriError::Missing("code"))
        );
        assert_eq!(
            PairingUri::parse("samu-pair:?server=ftp%3A%2F%2Fx&account=a&device=d&code=c"),
            Err(PairingUriError::Server)
        );
        assert!(matches!(
            PairingUri::parse("samu-pair:?server=https%3A%2F%2Fx&account=a%&device=d&code=c"),
            Err(PairingUriError::Encoding(_))
        ));
        assert!(matches!(
            PairingUri::parse("samu-pair:?server=https%3A%2F%2Fx&account=%FF&device=d&code=c"),
            Err(PairingUriError::Encoding(_))
        ));
    }
}
