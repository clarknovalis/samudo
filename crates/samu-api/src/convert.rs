// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later OR MIT OR Apache-2.0

//! Turning a task into a project (ADR 0126): the task stays, with its id
//! and what hangs on it; a new project takes its title, note, dates and
//! children.

use serde::{Deserialize, Serialize};

use crate::Id;

/// `POST /objects/{id}/to-project`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToProjectRequest {
    /// The folder the project goes in; none means the folder of the task's
    /// project, or the top for a task in the inbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<Id>,
    /// Put before the title of the task that stays, like `Notes: `; none
    /// keeps the title. Not applied to a task with the property `source`,
    /// whose title an outside sync writes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
    /// Say what would happen; write nothing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub preview: bool,
}

/// What `POST /objects/{id}/to-project` did, or with `preview` would do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToProjectResponse {
    /// The new project; with `preview`, the id it would have had.
    pub project: Id,
    /// Tasks that move into it, the task itself among them.
    pub moved: u64,
    /// The change number written up to; none with `preview`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
}
