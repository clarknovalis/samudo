// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later OR MIT OR Apache-2.0

//! What a client sends from its outbox, and what the server answers
//! (ADR 0004, ADR 0016).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Hlc, Id};

/// One change, tagged by `op` (ADR 0016 §2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Change {
    /// A new object, idempotent by its id. `value` is the whole object in the
    /// form `GET` returns it.
    Create {
        object: Id,
        kind: ObjectKind,
        value: Value,
        hlc: Hlc,
    },
    /// One field set to a value in the form the field has in the object;
    /// `null` clears it.
    Set {
        object: Id,
        field: String,
        value: Value,
        /// Why, for the logbook (ADR 0024 §5).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        hlc: Hlc,
    },
    /// An element into a set (`tags`), or an element at a new position.
    Add {
        object: Id,
        field: String,
        element: Id,
        #[serde(default)]
        position: i64,
        /// Why, for the logbook (ADR 0024 §5).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        hlc: Hlc,
    },
    /// An element out of a set.
    Remove {
        object: Id,
        field: String,
        element: Id,
        /// Why, for the logbook (ADR 0024 §5).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        hlc: Hlc,
    },
    /// The object becomes a tombstone (ADR 0004 §7).
    Delete { object: Id, hlc: Hlc },
    /// An operation from a newer samu (ADR 0015). The server rejects it.
    #[serde(other)]
    Unknown,
}

impl Change {
    /// The stamp of the change; none for an unknown one.
    #[must_use]
    pub fn hlc(&self) -> Option<&Hlc> {
        match self {
            Self::Create { hlc, .. }
            | Self::Set { hlc, .. }
            | Self::Add { hlc, .. }
            | Self::Remove { hlc, .. }
            | Self::Delete { hlc, .. } => Some(hlc),
            Self::Unknown => None,
        }
    }
}

/// What kind of object a `create` makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectKind {
    Folder,
    Project,
    Task,
    Tag,
    Link,
    Clock,
    /// ADR 0030.
    Reminder,
    /// ADR 0035.
    Scope,
    /// ADR 0039.
    Comment,
    /// ADR 0039.
    Attachment,
    /// ADR 0045: the account's settings, one object.
    Settings,
    /// ADR 0047: a calendar read from elsewhere.
    Calendar,
    /// ADR 0047, 0048: an appointment, written by the server alone.
    Appointment,
    /// ADR 0073: a capture template.
    Template,
    /// ADR 0090: a saved view.
    Perspective,
    /// ADR 0091: an entry for a habit.
    HabitEntry,
    /// A kind from a newer samu (ADR 0015). Read, never written.
    #[serde(other)]
    Unknown,
}

/// The server's answer to one change, tagged by `outcome` (ADR 0016 §3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ChangeOutcome {
    /// It holds now, at this change number.
    Applied { seq: u64 },
    /// A newer change to the same field already holds.
    Superseded { by: Hlc },
    /// Not applied, and never will be as it stands.
    Rejected { reason: String },
    /// An outcome from a newer samu (ADR 0015).
    #[serde(other)]
    Unknown,
}
