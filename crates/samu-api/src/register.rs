// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later OR MIT OR Apache-2.0

//! Making an account over HTTP, and deleting one's own (ADR 0099 §1–5).

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// Who may make an account with `POST /register` (ADR 0099 §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationMode {
    /// Nobody: accounts only with `samu-server account add`.
    Off,
    /// Anybody, a demo account with demo data that goes after a rest.
    Demo,
    /// Anybody, an ordinary account, demo data if asked.
    Open,
    /// A mode from a newer samu (ADR 0015).
    #[serde(other)]
    Unknown,
}

/// `GET /register`, without a session: what the sign-in page offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrationInfo {
    pub mode: RegistrationMode,
    /// The languages of the demo data, like `["de", "en"]`; empty with
    /// `off`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
}

/// `POST /register`, without a session (ADR 0099 §2).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Register {
    /// None in the mode `demo`: the server makes a name and a password.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// The language of the demo data. In the mode `demo` always some, by
    /// `Accept-Language` without it; in `open` none without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub demo: Option<String>,
    /// The device of the session the account starts with, as in `Login`.
    pub device: String,
}

/// Without the password.
impl std::fmt::Debug for Register {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Register")
            .field("name", &self.name)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("demo", &self.demo)
            .field("device", &self.device)
            .finish()
    }
}

/// The answer to `POST /register`: the account, signed in. A browser gets
/// the session's cookie too.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Registered {
    pub account: String,
    /// Only when the server made it; shown this once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    pub token: String,
    pub expires: Timestamp,
}

/// Without the password and the token.
impl std::fmt::Debug for Registered {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registered")
            .field("account", &self.account)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field("token", &"<redacted>")
            .field("expires", &self.expires)
            .finish()
    }
}

/// `POST /account/delete`: the password once more (ADR 0099 §5).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountDelete {
    pub password: String,
}

/// Without the password.
impl std::fmt::Debug for AccountDelete {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountDelete")
            .field("password", &"<redacted>")
            .finish()
    }
}

/// `POST /demo`: demo data into the account of the session (ADR 0099 §7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DemoRequest {
    /// The language of the demo data, one of `RegistrationInfo.languages`.
    pub lang: String,
}

/// The answer to `POST /demo`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DemoAdded {
    /// True when the account was empty and got everything; false when it
    /// got only what lies under the folder and the tag `Demo`.
    pub whole: bool,
    /// How many were made, by kind (`tasks`, `projects`, …).
    #[serde(default)]
    pub counts: std::collections::BTreeMap<String, usize>,
}
