// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later OR MIT OR Apache-2.0

//! Pairing a phone by QR code (ADR 0031, catastr ADR 0081).

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// `POST /pair/code`: the name the phone will have, and the signed-in
/// account's password once more, so a stolen session alone is not enough.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairCodeRequest {
    pub device: String,
    pub password: String,
}

/// Without the password.
impl std::fmt::Debug for PairCodeRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairCodeRequest")
            .field("device", &self.device)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// A pairing code, as `POST /pair/code` answers: worth ten minutes and one
/// use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairCode {
    /// The secret itself; also inside `uri`.
    pub code: String,
    pub account: String,
    pub device: String,
    /// When it stops working, used or not.
    pub expires: Timestamp,
    /// `samu-pair:?server=…&account=…&device=…&code=…`, for the QR code.
    pub uri: String,
}

/// `POST /pair`: a code traded for a session, answered like `POST /login`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairRequest {
    pub code: String,
}

/// Without the code.
impl std::fmt::Debug for PairRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairRequest")
            .field("code", &"<redacted>")
            .finish()
    }
}
