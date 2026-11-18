// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Clocking on the phone (ADR 0046 §2, §3, §8).
//!
//! Start and stop go into the outbox like every change. The rules the
//! server keeps — one running clock, and a clock stops with its task — are
//! applied here as well, offline, with the same functions from
//! `samu_core::clocking`; when the server has the changes it decides, and
//! the next pull brings its word.

use jiff::Timestamp;
use samu_api::{Change, Clock, Hlc, Id, ObjectKind};
use samu_core::clocking::{Ending, one_running, running, stop_on_close, total};
use samu_core::overlay::Objects;
use serde_json::json;

use crate::{MobileError, Samu, input, parse_id};

/// The running clock, for the app bar and the ongoing notification.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RunningClock {
    pub id: String,
    /// The task or project it is on.
    pub owner_id: String,
    pub title: String,
    /// When it began, milliseconds since the epoch.
    pub start_ms: i64,
}

/// One clock of a task, for its page.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ClockRow {
    pub id: String,
    pub start_ms: i64,
    /// None: it runs.
    pub end_ms: Option<i64>,
    pub seconds: i64,
}

/// The ends of `endings` as changes.
pub(crate) fn ended(endings: Vec<Ending>, stamp: &mut dyn FnMut() -> Hlc) -> Vec<Change> {
    endings
        .into_iter()
        .map(|e| Change::Set {
            object: e.clock,
            field: "end".into(),
            value: json!(e.end.to_string()),
            note: None,
            hlc: stamp(),
        })
        .collect()
}

fn title_of(objects: &Objects, owner: Id) -> String {
    objects
        .tasks
        .iter()
        .find(|t| t.id == owner)
        .map(|t| t.title.clone())
        .or_else(|| {
            objects
                .projects
                .iter()
                .find(|p| p.id == owner)
                .map(|p| p.title.clone())
        })
        .unwrap_or_default()
}

/// Completing or dropping `owner` at `at`, and ending its running clock
/// there (§3), as the server will.
pub(crate) fn close(
    objects: &Objects,
    owner: Id,
    field: &str,
    at: Timestamp,
    stamp: &mut dyn FnMut() -> Hlc,
) -> Vec<Change> {
    let mut changes = vec![Change::Set {
        object: owner,
        field: field.into(),
        value: json!(at.to_string()),
        note: None,
        hlc: stamp(),
    }];
    changes.extend(ended(stop_on_close(&objects.clocks, owner, at), stamp));
    changes
}

#[uniffi::export]
impl Samu {
    /// Start a clock on a task or project now; one running elsewhere ends
    /// where this one begins (§2).
    pub fn clock_in(&self, owner: String) -> Result<String, MobileError> {
        let owner = parse_id(&owner)?;
        let id = Id::new();
        self.queue(|objects, stamp| {
            let open_task = objects
                .tasks
                .iter()
                .any(|t| t.id == owner && t.completed_at.is_none() && t.dropped_at.is_none());
            if !open_task && !objects.projects.iter().any(|p| p.id == owner) {
                return Err(input(format!("no open task or project {owner}")));
            }
            let now = Timestamp::now();
            let clock = Clock {
                id,
                created: now,
                modified: now,
                owner,
                start: now,
                end: None,
                note: None,
            };
            let mut after = objects.clocks.clone();
            after.push(clock.clone());
            let mut changes = vec![Change::Create {
                object: id,
                kind: ObjectKind::Clock,
                value: serde_json::to_value(&clock).map_err(|e| input(e.to_string()))?,
                hlc: stamp(),
            }];
            changes.extend(ended(one_running(&after), stamp));
            Ok(changes)
        })?;
        Ok(id.to_string())
    }

    /// Stop the running clock now; nothing when none runs.
    pub fn clock_out(&self) -> Result<(), MobileError> {
        self.queue(|objects, stamp| {
            let Some(clock) = running(&objects.clocks) else {
                return Ok(Vec::new());
            };
            let end = Timestamp::now().max(clock.start);
            Ok(ended(
                vec![Ending {
                    clock: clock.id,
                    end,
                }],
                stamp,
            ))
        })
    }

    /// The running clock, as the outbox has it.
    pub fn running_clock(&self) -> Result<Option<RunningClock>, MobileError> {
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        Ok(running(&objects.clocks).map(|c| RunningClock {
            id: c.id.to_string(),
            owner_id: c.owner.to_string(),
            title: title_of(&objects, c.owner),
            start_ms: c.start.as_millisecond(),
        }))
    }

    /// The clocks of a task or project, newest first.
    pub fn clocks(&self, owner: String) -> Result<Vec<ClockRow>, MobileError> {
        let owner = parse_id(&owner)?;
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let now = Timestamp::now();
        let mut rows: Vec<&Clock> = objects.clocks.iter().filter(|c| c.owner == owner).collect();
        rows.sort_by_key(|c| std::cmp::Reverse((c.start, c.id)));
        Ok(rows
            .into_iter()
            .map(|c| ClockRow {
                id: c.id.to_string(),
                start_ms: c.start.as_millisecond(),
                end_ms: c.end.map(Timestamp::as_millisecond),
                seconds: (c.end.unwrap_or(now).as_second() - c.start.as_second()).max(0),
            })
            .collect())
    }

    /// Seconds clocked on a task or project, a running clock up to now.
    pub fn clocked_seconds(&self, owner: String) -> Result<i64, MobileError> {
        let owner = parse_id(&owner)?;
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        Ok(total(&objects.clocks, owner, Timestamp::now()))
    }
}
