// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later OR MIT OR Apache-2.0

//! Deleting an object with what hangs on it (ADR 0036).

use serde::{Deserialize, Serialize};

use crate::Id;

/// `POST /objects/{id}/delete`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteRequest {
    /// Only for a project: where its tasks go. None means the inbox
    /// (ADR 0036 §2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tasks: Option<TaskFate>,
    /// Count what would happen; write nothing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub preview: bool,
}

/// Where the tasks of a deleted project go (ADR 0036 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskFate {
    /// Into the inbox, action groups whole.
    Inbox,
    /// Into another project, after its last tasks.
    Project(Id),
    /// Deleted along with the project.
    Delete,
    /// A choice from a newer samu (ADR 0015). The server rejects it.
    #[serde(other)]
    Unknown,
}

/// What `POST /objects/{id}/delete` did, or with `preview` would do.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteResponse {
    /// Objects that become tombstones, the object itself among them.
    pub deleted: u64,
    /// Tasks, projects and folders that move somewhere else.
    pub moved: u64,
    /// Tasks, projects and scopes a deleted tag is taken from.
    pub untagged: u64,
    /// The change number written up to; none with `preview`, or when the
    /// object was already deleted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
}
