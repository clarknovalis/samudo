// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later OR MIT OR Apache-2.0

//! A task with its computed availability, as `GET /available` returns it
//! (ADR 0017 §10, ADR 0019 §5), and a project with its own, as
//! `GET /available/projects` returns it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use jiff::Timestamp;

use crate::{Id, Project, Task, When};

/// One task and what samu-core computed for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskStatus {
    pub task: Task,
    pub available: bool,
    /// Every reason it is not available; empty when it is.
    #[serde(default)]
    pub reasons: Vec<Reason>,
    #[serde(default)]
    pub due: DueState,
    /// Planned for today or earlier, and open.
    #[serde(default)]
    pub today: bool,
    /// The workflow it follows: the exclusive tag group of its project, or
    /// the default (ADR 0024 §3); its children are the states to offer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<Id>,
    /// Its state in the workflow of its project (ADR 0024, ADR 0025 §3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<Id>,
    /// The state cycling takes it to; none means out of every state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_state: Option<Id>,
    /// For an action group: what a folded row shows (ADR 0021 §1/§3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<GroupSummary>,
    /// Only with `scope=`: outside the scope, but open and overdue or due
    /// soon. Counted for a hint, shown in no view (ADR 0035 §6).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub outside: bool,
    /// In the inbox: no project, or one that is not there (ADR 0036 §6).
    /// Group by this, not by `task.project`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub inbox: bool,
    /// The properties it inherits from its action groups and project and
    /// does not have itself (ADR 0054 §7).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inherited: BTreeMap<String, Inherited>,
    /// For an open task that repeats: what completing it now would make
    /// (ADR 0007 §3, ADR 0033). Computed by samu-core.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_repeat: Option<NextRepeat>,
}

/// A [`TaskStatus`] without the task: its `id` and what samu-core computed,
/// for a client that has the tasks from `GET /changes` already
/// (`GET /available?bare=true`, ADR 0065). The fields are those of
/// `TaskStatus`; one added there is added here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BareStatus {
    pub id: Id,
    pub available: bool,
    #[serde(default)]
    pub reasons: Vec<Reason>,
    #[serde(default)]
    pub due: DueState,
    #[serde(default)]
    pub today: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_state: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<GroupSummary>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub outside: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub inbox: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub inherited: BTreeMap<String, Inherited>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_repeat: Option<NextRepeat>,
}

impl From<TaskStatus> for BareStatus {
    fn from(status: TaskStatus) -> Self {
        // Taken apart by name, so a field added to `TaskStatus` does not
        // build until it is here too.
        let TaskStatus {
            task,
            available,
            reasons,
            due,
            today,
            workflow,
            state,
            next_state,
            group,
            outside,
            inbox,
            inherited,
            next_repeat,
        } = status;
        Self {
            id: task.id,
            available,
            reasons,
            due,
            today,
            workflow,
            state,
            next_state,
            group,
            outside,
            inbox,
            inherited,
            next_repeat,
        }
    }
}

/// The dates the next task of a repetition would get if this one were
/// completed now; `ended` when the rule makes none.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NextRepeat {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defer: Option<When>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<When>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planned: Option<When>,
    /// `UNTIL` or `COUNT` is reached: completing makes no next task.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ended: bool,
}

/// A property a task inherits, and from where (ADR 0054 §7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inherited {
    pub value: String,
    /// The action group or project it comes from.
    pub from: Id,
}

/// One project and what samu-core computed for it (ADR 0006, ADR 0020 §1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectAvailability {
    pub project: Project,
    pub available: bool,
    /// Every reason it is not available; empty when it is.
    #[serde(default)]
    pub reasons: Vec<Reason>,
    /// Available, but with neither an available nor a deferred open action:
    /// it needs a next action (the view Stuck).
    #[serde(default)]
    pub stuck: bool,
    /// The date of the next review, computed by samu-core from
    /// `next_review`, or `last_reviewed`/`created` plus `review_interval`;
    /// none without an interval (ADR 0088 §1, §4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<jiff::civil::Date>,
    /// Today has reached `review` (the view Review).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub review_due: bool,
}

/// An action group at a glance (ADR 0021): its direct children, open and
/// done, how many are available, and for a sequential one the next action.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupSummary {
    pub open: u32,
    pub done: u32,
    pub available: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<Id>,
}

/// Why a task is not available (ADR 0006, ADR 0017), tagged by `reason`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum Reason {
    Completed,
    Dropped,
    Deferred {
        until: When,
    },
    ProjectNotActive {
        project: Id,
    },
    FolderDropped {
        folder: Id,
    },
    TagNotActive {
        tag: Id,
    },
    WaitingInSequence {
        first: Id,
    },
    HasOpenChildren,
    ContainerUnavailable {
        id: Id,
    },
    BlockedBy {
        task: Id,
    },
    UnknownValue,
    Cycle,
    /// A reason from a newer samu (ADR 0015).
    #[serde(other)]
    Unknown,
}

/// How close the due date is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DueState {
    #[default]
    None,
    Soon,
    Overdue,
    /// A state from a newer samu (ADR 0015).
    #[serde(other)]
    Unknown,
}

/// What the schedules of the scopes say now (ADR 0043 §7), as
/// `GET /scopes/scheduled` returns it. A device that follows the schedule
/// shows `scope`, unless it was chosen by hand at or after `since`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledScope {
    /// The scope whose window holds now; none when no window does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Id>,
    /// The last time a window began or ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<Timestamp>,
    /// The next time a window begins or ends: when to ask again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<Timestamp>,
}

/// One block of a perspective, as `GET /blocks` returns them in order
/// (ADR 0116 §4); a plain perspective is one section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockSection {
    /// The heading: the block's own, else the name of its perspective or
    /// view.
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// The block as the perspective holds it; none for a plain
    /// perspective.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<crate::PerspectiveBlock>,
    /// It points at a perspective that is gone, a block perspective or a
    /// view this server does not know, and shows nothing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub missing: bool,
    #[serde(default)]
    pub tasks: Vec<TaskStatus>,
}

/// One section of Forecast (ADR 0045), as `GET /forecast` returns them in
/// order: overdue, today, flagged, the next days, later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForecastSection {
    pub section: ForecastKind,
    /// The day, for `day`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<jiff::civil::Date>,
    #[serde(default)]
    pub rows: Vec<ForecastRow>,
    /// The appointments of the day, before the tasks (ADR 0047 §10): only
    /// in a `day`, whole days first, then by start.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub appointments: Vec<crate::AppointmentInstance>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForecastKind {
    Overdue,
    Day,
    Flagged,
    Later,
    /// A value from a newer samu (ADR 0015).
    #[serde(other)]
    Unknown,
}

/// A task in a section, and why it stands there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForecastRow {
    pub why: ForecastWhy,
    /// The date the reason names, where it names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<jiff::civil::Date>,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForecastWhy {
    Due,
    Planned,
    Defer,
    Tag,
    Done,
    Flagged,
    /// A value from a newer samu (ADR 0015).
    #[serde(other)]
    Unknown,
}

/// A habit and what samu-core computed for it (ADR 0091 §3), as
/// `GET /habits` and `GET /habits/{id}` return it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HabitStatus {
    /// The task; its `habit` is set.
    pub task: Task,
    /// The day the rest is computed for, in the account's zone.
    pub today: jiff::civil::Date,
    /// What today is.
    pub day: HabitDay,
    /// The period today is in, with what it has so far.
    pub period: HabitPeriod,
    /// Periods met in a row up to today; for `avoid` the whole days since
    /// the last relapse, pauses left out.
    pub streak: u32,
    /// The longest streak there ever was.
    pub longest: u32,
    /// Closed periods ending in the last 28 days, pauses left out.
    pub recent: HabitRatio,
    /// Closed periods ending this calendar year, pauses left out.
    pub year: HabitRatio,
    /// For `avoid`: the day of the last relapse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_fail: Option<jiff::civil::Date>,
    /// Only when asked with `from`: every day from then to today.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<HabitDayState>,
    /// Only when asked with `from`: the entries from then on, by day.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<crate::HabitEntry>,
}

/// What one day of a habit is (ADR 0091 §3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HabitDay {
    /// The day's goal is reached; for `avoid` a day without relapse.
    Done,
    /// Some amount, less than the day's goal.
    Partial,
    /// A `fail` entry; for `avoid` a relapse.
    Fail,
    Pause,
    /// Nothing entered; also every day before the habit began.
    #[default]
    None,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    #[serde(other)]
    Unknown,
}

/// One day of a habit's history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HabitDayState {
    pub day: jiff::civil::Date,
    pub state: HabitDay,
    /// The sum of the day's `done` amounts.
    #[serde(default)]
    pub amount: u32,
}

/// The period a day is in, and how far its goal is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HabitPeriod {
    pub from: jiff::civil::Date,
    pub to: jiff::civil::Date,
    /// The sum of the `done` amounts in it.
    #[serde(default)]
    pub amount: u32,
    /// The goal, the habit's `times`.
    pub target: u32,
    /// The goal is reached; for `avoid` no relapse in it.
    #[serde(default)]
    pub met: bool,
}

/// Periods met out of periods counted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HabitRatio {
    pub met: u32,
    pub of: u32,
}
