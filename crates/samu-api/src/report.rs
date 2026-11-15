//! The clock report, as `GET /clocks/report` returns it (ADR 0046 §5), and
//! the report of done, worked on and time, as `GET /report` returns it
//! (ADR 0080).

use serde::{Deserialize, Serialize};

use jiff::Timestamp;
use jiff::civil::Date;

use crate::{Id, ObjectKind, Period};

/// The time clocked in a span of days, grouped one way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockReport {
    /// The first day, in the account's zone.
    pub from: Date,
    /// The last day, included.
    pub to: Date,
    pub by: ClockReportBy,
    /// Every second in the span once, however many rows it counts in.
    pub seconds: i64,
    #[serde(default)]
    pub rows: Vec<ClockReportRow>,
}

/// What the rows of a report are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClockReportBy {
    /// The task or project a clock is on.
    #[default]
    Task,
    Project,
    Folder,
    Tag,
    /// The day in the account's zone.
    Day,
    /// A value from a newer samu (ADR 0015). Read, never asked for.
    #[serde(other)]
    Unknown,
}

/// One row: what it is and how long.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClockReportRow {
    /// The task, project, folder or tag; none for the inbox, no folder or no
    /// tag, and for a day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    /// The day, for `by = day`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<Date>,
    /// The title as it is now; empty where there is no object.
    #[serde(default)]
    pub title: String,
    pub seconds: i64,
    /// The task's estimate, for `by = task`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate: Option<Period>,
}

/// What `GET /report` answers (ADR 0080 §4): the parts asked for over a
/// span of days, each filtered alike.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    /// The first day, in the account's zone.
    pub from: Date,
    /// The last day, included.
    pub to: Date,
    pub by: ClockReportBy,
    /// Completed (and with `dropped` dropped) tasks and projects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done: Option<ReportList>,
    /// Tasks and projects something happened to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worked: Option<ReportList>,
    /// The time clocked, as `GET /clocks/report` with the same filter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<ClockReport>,
}

/// The rows of one part, latest first, and with `by` other than `task`
/// their groups.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportList {
    #[serde(default)]
    pub rows: Vec<ReportRow>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<ReportGroup>,
}

/// A task or project in a report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportRow {
    pub id: Id,
    /// `task` or `project`.
    pub kind: ObjectKind,
    pub title: String,
    /// The project it is in; none for the inbox and for a project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<Id>,
    /// The folder of its project, or of the project itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<Id>,
    /// Completed (or dropped) at; in `worked` the last event in the span.
    pub at: Timestamp,
    /// What happened, with how often; in `done` empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<ReportReasonCount>,
    /// Time clocked on it in the span.
    #[serde(default)]
    pub seconds: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate: Option<Period>,
    /// Dropped rather than completed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dropped: bool,
}

/// One reason and how often it happened in the span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportReasonCount {
    pub reason: ReportReason,
    pub count: u32,
}

/// Why a task counts as worked on (ADR 0080 §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportReason {
    /// Time clocked in the span; `count` is the number of clocks.
    Clocked,
    /// Its state changed.
    State,
    Completed,
    Reopened,
    Dropped,
    Commented,
    /// One of its children was completed.
    ChildCompleted,
    /// A field changed (setting `report_worked = fields`).
    Changed,
    /// A reason from a newer samu (ADR 0015).
    #[serde(other)]
    Unknown,
}

/// A group of rows with `by` other than `task`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportGroup {
    /// The project, folder or tag; none for the inbox, no folder or no tag,
    /// and for a day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<Date>,
    #[serde(default)]
    pub title: String,
    /// The rows' time, each clock once.
    #[serde(default)]
    pub seconds: i64,
    /// The ids of its rows, in the order of `rows`; with `by = tag` a row
    /// may be in several groups.
    #[serde(default)]
    pub rows: Vec<Id>,
}

/// A part of a report, as `parts` asks for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportPart {
    Done,
    Worked,
    Time,
    /// A part from a newer samu (ADR 0015). Read, never asked for.
    #[serde(other)]
    Unknown,
}

/// What counts as worked on (ADR 0080 §1), a setting of the account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportWorked {
    /// Clocks, state changes, completion, comments, a child completed.
    #[default]
    Events,
    /// The events and every change of a field.
    Fields,
    /// A value from a newer samu (ADR 0015). Read, never written.
    #[serde(other)]
    Unknown,
}
