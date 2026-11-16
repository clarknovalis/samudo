// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later OR MIT OR Apache-2.0

//! Named hooks: a sender outside samu delivers to `POST /hooks/{name}`
//! (ADR 0010, ADR 0022).
//!
//! A hook belongs to one account and puts what it receives into that
//! account's inbox. Its secret is shown once, in the answer to the change
//! that made or renewed it, and is never part of a view.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::Id;

/// How a hook reads a request. A new sender is a new format, not a new
/// route (catastr ADR 0086 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HookFormat {
    /// The webhook of the Pebble app for the Index ring, version 1
    /// (`X-Index-Webhook-Version: 1`).
    #[serde(rename = "index-v1")]
    IndexV1,
    /// A format from a newer samu (ADR 0015). Read, never written.
    #[serde(other, rename = "unknown")]
    Unknown,
}

/// A hook as `GET /hooks` shows it: everything but the secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookView {
    pub name: String,
    pub format: HookFormat,
    pub created: Timestamp,
}

/// A change to the hooks of the signed-in account (`POST /hooks`).
///
/// One route with an action rather than `/hooks/{name}/remove`: every path
/// below `/hooks/` is a hook a sender delivers to (as in catastr).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum HookChange {
    /// Makes a hook; the answer carries its secret, this once.
    Add { name: String, format: HookFormat },
    /// Removes a hook. What it made stays.
    Remove { name: String },
    /// Gives a hook a new secret; the old one stops working at once.
    Secret { name: String },
}

/// What a change to the hooks did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookChangeResponse {
    /// The hook as it is now; absent after a removal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook: Option<HookView>,
    /// The secret to enter in the sender, shown once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
    #[serde(default)]
    pub removed: bool,
}

/// What a delivery to a hook did (`POST /hooks/{name}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookDelivery {
    /// The task it made, or found by its delivery id; absent for a test
    /// event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<Id>,
    /// Whether the task was made now rather than found.
    #[serde(default)]
    pub created: bool,
    /// A test event: checked and answered, nothing made.
    #[serde(default)]
    pub test: bool,
}
