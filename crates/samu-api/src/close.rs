// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later OR MIT OR Apache-2.0

//! Completing or dropping a task with what is still open below it
//! (ADR 0128): with it, lifted out, or not at all until asked.

use serde::{Deserialize, Serialize};

/// `POST /objects/{id}/close`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseRequest {
    /// Completed or dropped.
    pub field: CloseField,
    /// What happens to the open tasks below; `ask` writes nothing while
    /// there are any.
    #[serde(default, skip_serializing_if = "ChildFate::is_ask")]
    pub children: ChildFate,
    /// Why, for the logbook entry of the task itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Count what would happen; write nothing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub preview: bool,
}

/// Which way a task is closed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseField {
    /// `completed_at`.
    #[default]
    Completed,
    /// `dropped_at`.
    Dropped,
    /// A field from a newer samu (ADR 0015). The server rejects it.
    #[serde(other)]
    Unknown,
}

/// What happens to the open tasks below the one closed (ADR 0128 §1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildFate {
    /// Close nothing while any is open; the answer counts them.
    #[default]
    Ask,
    /// Close every open one below with the task, at the same instant.
    Close,
    /// Move the open direct children, with what is below them, one level
    /// up, right after the task (ADR 0128 §2).
    Lift,
    /// A choice from a newer samu (ADR 0015). The server rejects it.
    #[serde(other)]
    Unknown,
}

impl ChildFate {
    /// Whether this is the default, `ask`.
    #[must_use]
    pub fn is_ask(&self) -> bool {
        *self == Self::Ask
    }
}

/// What `POST /objects/{id}/close` did, or with `preview` would do.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloseResponse {
    /// Open tasks below the task, at any depth. With `ask` and more than
    /// none, nothing is written: the client asks and sends a choice.
    pub open: u64,
    /// Tasks that are closed, the task itself among them.
    pub closed: u64,
    /// Open direct children that move one level up.
    pub lifted: u64,
    /// The change number written up to; none with `preview`, when it
    /// waits for a choice, or when the task was closed already.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
}
