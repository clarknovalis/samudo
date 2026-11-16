// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later OR MIT OR Apache-2.0

//! What happened to a task or project, beyond the last state the stamps
//! keep (ADR 0006, ADR 0024 §5), and what a client did to an appointment
//! (ADR 0068 §3). Written by the server, read with
//! `GET /objects/{id}/logbook`; it does not travel with `/changes`.

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::Id;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub object: Id,
    pub at: Timestamp,
    pub kind: LogKind,
    /// The device of the change, from its stamp.
    pub device: String,
    /// Whether a person or an agent made the change, from the session that
    /// brought it (ADR 0039 §1); none for older entries. Not `kind`, which
    /// is the kind of entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author_kind: Option<crate::AuthorKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// For `state`: the state tag before, none if there was none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old: Option<Id>,
    /// For `state`: the state tag after, none if it was cleared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new: Option<Id>,
    /// For `changed`: the field, and its value before and after in the form
    /// the object has it; none when it was not set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_value: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_value: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogKind {
    /// The state in a workflow changed.
    State,
    Completed,
    Dropped,
    /// Completed or dropped no more.
    Reopened,
    /// An appointment made by a client (ADR 0068 §3).
    Created,
    /// A field of an appointment changed by a client.
    Changed,
    /// An appointment deleted by a client.
    Deleted,
    /// An iCalendar file imported into a calendar (ADR 0100 §4); `note`
    /// says the file and what came of it.
    Imported,
    #[serde(other)]
    Unknown,
}
