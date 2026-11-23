// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later OR MIT OR Apache-2.0

//! The objects of the model as whole states, the way `GET` returns them and
//! a cache keeps them (ADR 0006, ADR 0014 §8).
//!
//! Empty optional fields are left out when writing; a reader takes a missing
//! field and `null` alike. Unknown fields are skipped, never refused
//! (ADR 0003 §2). The field stamps of ADR 0004 are not part of these types.

use std::collections::BTreeMap;

use jiff::{Span, Timestamp};
use serde::{Deserialize, Serialize};

use crate::{Id, Repeat, When};

/// A length of time in ISO 8601, like `P1W` or `PT30M`.
///
/// A newtype because [`Span`] compares by value only field by field, and the
/// model types want `PartialEq` for tests and caches.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Period(pub Span);

impl PartialEq for Period {
    fn eq(&self, other: &Self) -> bool {
        self.0.fieldwise() == other.0.fieldwise()
    }
}

impl Eq for Period {}

/// A folder of projects (ADR 0006).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub id: Id,
    pub created: Timestamp,
    pub modified: Timestamp,
    pub title: String,
    /// The folder above; none at the top.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Id>,
    pub status: FolderStatus,
    /// An emoji in place of the initials (ADR 0059); none = no icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Shown only in a scope: true when `own_scope` is set. Derived by
    /// the server for older readers; read only (ADR 0094 §6).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub scope_only: bool,
    /// The scope it is shown in, and no other; none = everywhere
    /// (ADR 0094 §1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own_scope: Option<Id>,
    #[serde(default)]
    pub position: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FolderStatus {
    Active,
    /// Makes everything below invisible.
    Dropped,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    /// Read, never written: the server refuses it.
    #[serde(other)]
    Unknown,
}

/// The dates and repetition a task and a project share (ADR 0006, ADR 0007).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schedule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub defer: Option<When>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<When>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planned: Option<When>,
    /// How long before `due` it counts as due soon; none = the account's
    /// default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_warning: Option<Period>,
    /// With a `planned` that has a time: a block in the calendar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planned_duration: Option<Period>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<Repeat>,
}

/// A tag on a task or project: an element of a set, with its own position
/// (ADR 0004, Konsequenzen).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tagging {
    pub tag: Id,
    #[serde(default)]
    pub position: i64,
}

/// A project (ADR 0006): a kind, a status, a review, and everything a task
/// has.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: Id,
    pub created: Timestamp,
    pub modified: Timestamp,
    /// Who made it, set by the server from the session (ADR 0039 §1); none
    /// for what was made before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<Author>,
    pub title: String,
    /// An emoji before the title (ADR 0059); none = no icon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Shown only in a scope: true when `own_scope` is set. Derived by
    /// the server for older readers; read only (ADR 0094 §6).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub scope_only: bool,
    /// The scope it is shown in, and no other; none = everywhere
    /// (ADR 0094 §1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own_scope: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// None = at the top.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<Id>,
    /// The exclusive tag group whose children are the states here
    /// (ADR 0024 §3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<Id>,
    pub kind: ProjectKind,
    pub status: ProjectStatus,
    #[serde(flatten)]
    pub schedule: Schedule,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_interval: Option<Period>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_review: Option<jiff::civil::Date>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_reviewed: Option<Timestamp>,
    #[serde(default)]
    pub flagged: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate: Option<Period>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropped_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tagging>,
    /// Free keys and values, each written on its own as
    /// `properties.<key>` (ADR 0023).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, String>,
    #[serde(default)]
    pub position: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectKind {
    Parallel,
    /// Only the first open action is available.
    Sequential,
    /// A list of loose actions, never stuck.
    SingleActions,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    /// Read, never written: the server refuses it.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectStatus {
    Active,
    OnHold,
    Completed,
    Dropped,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    /// Read, never written: the server refuses it.
    #[serde(other)]
    Unknown,
}

/// A task (ADR 0006). With children it is an action group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: Id,
    pub created: Timestamp,
    pub modified: Timestamp,
    /// Who made it, set by the server from the session (ADR 0039 §1); none
    /// for what was made before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<Author>,
    pub title: String,
    /// Text with Org markup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// None = the inbox.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<Id>,
    /// The action group this task is in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Id>,
    #[serde(flatten)]
    pub schedule: Schedule,
    #[serde(default)]
    pub flagged: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimate: Option<Period>,
    /// None = no priority; it sorts after `C` (ADR 0056).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<Priority>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dropped_at: Option<Timestamp>,
    /// The task before this one in a repetition chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat_of: Option<Id>,
    /// As an action group: only the first open child is available.
    #[serde(default)]
    pub sequential: bool,
    /// As an action group: completed with its last child.
    #[serde(default)]
    pub complete_with_last_child: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Tagging>,
    /// Free keys and values, each written on its own as
    /// `properties.<key>` (ADR 0023).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub properties: BTreeMap<String, String>,
    #[serde(default)]
    pub position: i64,
    /// Set, the task is a habit (ADR 0091 §1). Changed as a whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub habit: Option<Habit>,
}

/// What makes a task a habit (ADR 0091 §1): its kind and its goal,
/// `times` per `per`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Habit {
    #[serde(default)]
    pub kind: HabitKind,
    /// How often per `per`; at least 1. Not read for `avoid`.
    #[serde(default = "one")]
    pub times: u32,
    #[serde(default)]
    pub per: HabitPer,
    /// A word for the amount, like `Glas`; without one every entry counts 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

impl Default for Habit {
    fn default() -> Self {
        Self {
            kind: HabitKind::Build,
            times: 1,
            per: HabitPer::Day,
            unit: None,
        }
    }
}

const fn one() -> u32 {
    1
}

/// Whether a habit is something to do or something to leave (ADR 0091 §1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HabitKind {
    /// Done is entered; a day without an entry is not done.
    #[default]
    Build,
    /// Only a relapse (`fail`) is entered; a day without one is done.
    Avoid,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    #[serde(other)]
    Unknown,
}

/// The period a habit's goal counts in (ADR 0091 §1): the account's day, a
/// week from Monday, a calendar month.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HabitPer {
    #[default]
    Day,
    Week,
    Month,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    #[serde(other)]
    Unknown,
}

/// One entry for a habit (ADR 0091 §2): every tap is one, a day's amount is
/// their sum. It does not change; a wrong one is deleted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HabitEntry {
    pub id: Id,
    pub created: Timestamp,
    pub modified: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<Author>,
    /// The task that is the habit.
    pub habit: Id,
    /// The account's day it counts for; never after today.
    pub day: jiff::civil::Date,
    pub status: HabitEntryStatus,
    /// Only for `done`; none counts 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<u32>,
    /// Only for `pause`: its last day, for a pause over several days.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<jiff::civil::Date>,
}

/// What an entry says about its day (ADR 0091 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HabitEntryStatus {
    Done,
    /// Not done, said on purpose; for `avoid` a relapse.
    Fail,
    /// Holiday, illness: the streak does not break.
    Pause,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    #[serde(other)]
    Unknown,
}

/// The priority of a task (ADR 0056), highest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Priority {
    A,
    B,
    C,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    /// Read, never written: the server refuses it.
    #[serde(other, rename = "unknown")]
    Unknown,
}

/// A tag (ADR 0006). With children and `exclusive` it is a tag group: setting
/// one child removes its siblings from the same task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tag {
    pub id: Id,
    pub created: Timestamp,
    pub modified: Timestamp,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<Id>,
    pub status: TagStatus,
    /// A group whose children are the states of a workflow (ADR 0024 §1).
    #[serde(default)]
    pub exclusive: bool,
    /// Shown only in a scope: true when `own_scope` is set. Derived by
    /// the server for older readers; read only (ADR 0094 §6).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub scope_only: bool,
    /// The scope it is shown in, and no other; none = everywhere
    /// (ADR 0094 §1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own_scope: Option<Id>,
    /// One character to pick this state by (ADR 0024 §4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Clients ask for a note on entering this state (ADR 0024 §6).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ask_note: bool,
    /// The workflow of a project that names none (ADR 0024 §3).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub default_workflow: bool,
    #[serde(default)]
    pub position: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagStatus {
    Active,
    /// What carries it is not available (someday).
    OnHold,
    Dropped,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    /// Read, never written: the server refuses it.
    #[serde(other)]
    Unknown,
}

/// A link from a task or project to a URI (ADR 0006, ADR 0012).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub id: Id,
    pub created: Timestamp,
    pub modified: Timestamp,
    /// The task or project the link belongs to.
    pub owner: Id,
    /// `catastr:<uuid>`, `samu:<uuid>`, `mid:…`, `https:…`. May be
    /// unresolved.
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub kind: LinkKind,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// A reference.
    #[default]
    Ref,
    /// The owner is blocked while the target task is open.
    BlockedBy,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    /// Read, never written: the server refuses it.
    #[serde(other)]
    Unknown,
}

/// Time spent on a task or project (ADR 0006). At most one runs per account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Clock {
    pub id: Id,
    pub created: Timestamp,
    pub modified: Timestamp,
    /// The task or project.
    pub owner: Id,
    pub start: Timestamp,
    /// None = running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<Timestamp>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A reminder on a task or project (ADR 0030): at an instant, or a length of
/// time before one of its dates. A due date reminds without one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reminder {
    pub id: Id,
    pub created: Timestamp,
    pub modified: Timestamp,
    /// The task or project.
    pub owner: Id,
    pub anchor: ReminderAnchor,
    /// The instant; only with `anchor = at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Timestamp>,
    /// How long before the anchor; none = at the anchor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<Period>,
    /// The time of day in the account's zone; only with `anchor = daily`
    /// (ADR 0093).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<jiff::civil::Time>,
}

/// What a reminder is relative to (ADR 0030 §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReminderAnchor {
    /// The reminder's own `at`.
    At,
    Due,
    Planned,
    Defer,
    /// Every day at the reminder's `time`, while its owner is open; a habit
    /// only while the day's or period's goal is open (ADR 0093).
    Daily,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    /// Read, never written: the server refuses it. It reminds of nothing.
    #[serde(other)]
    Unknown,
}

/// A lens over every view (ADR 0035): what it names, and whether the views
/// show only that or everything else. Chosen on each device, never stored
/// with the account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    pub id: Id,
    pub created: Timestamp,
    pub modified: Timestamp,
    pub title: String,
    pub mode: ScopeMode,
    /// Folders, projects and tags, each with everything below it. A set:
    /// changed with `add` and `remove` on `members`, one element at a time.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<Id>,
    /// The inbox as a member (ADR 0043 §1): with `only` the views show it
    /// only when this is set, with `except` they leave it out then.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub inbox: bool,
    /// Chips of the search language that a task must meet as well
    /// (ADR 0043 §2, §3). Changed as a whole.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filter: Vec<ScopeChip>,
    /// What a task captured while the scope is chosen gets, when its line
    /// names none (ADR 0043 §6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture: Option<ScopeCapture>,
    /// When a device that follows the schedule chooses this scope
    /// (ADR 0043 §7). Changed as a whole.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub schedule: Vec<ScopeWindow>,
    /// The calendars whose appointments the views show; none means all
    /// (ADR 0047 §12). Changed as a whole.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calendars: Vec<Id>,
    /// The colour the views show while it is chosen (ADR 0060); none =
    /// neutral.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ScopeColor>,
    /// How the views show what the scope holds (ADR 0072); none = as
    /// always. Changed as a whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<ScopeLayout>,
    #[serde(default)]
    pub position: i64,
}

/// The colours of a scope (ADR 0060 §1). The shades live in the views.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeColor {
    Violet,
    Blue,
    Teal,
    Green,
    Yellow,
    Orange,
    Red,
    Pink,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    /// Read, never written: the server refuses it.
    #[serde(other)]
    Unknown,
}

/// One chip of a scope's filter (ADR 0043 §2), in the language of the
/// search (ADR 0042 §2). For `project`, `tag`, `folder` and `state` the
/// value is an id (`inbox` for the inbox as a project); for the others the
/// word, like `7t`, `woche`, `markiert` or `<15m`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeChip {
    pub kind: ScopeChipKind,
    pub value: String,
    /// Negated: the task must not meet it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub not: bool,
    /// The OR group it belongs to (ADR 0121 §2): a task meets a filter
    /// when it meets every chip kind of one group. The numbers only order
    /// the groups; a writer counts from 0. A reader that does not know the
    /// field reads all chips as one group and finds less, never more.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub group: u32,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if"
)]
const fn is_zero(value: &u32) -> bool {
    *value == 0
}

/// What a chip asks about (ADR 0042 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeChipKind {
    Project,
    Tag,
    Folder,
    /// A state of a workflow (ADR 0024), by the id of its tag.
    State,
    Due,
    Planned,
    Defer,
    Completed,
    Created,
    Modified,
    /// `ist:` – `markiert`, `verfügbar`, `wartet`, …
    Is,
    Estimate,
    /// A property, own or inherited (ADR 0054 §8): `key` has it, `key=value`
    /// has it with this value, case aside.
    Property,
    /// The priority (ADR 0056 §4): `a`, `b`, `c`, `mit`, `ohne`.
    Priority,
    /// One task by its id: the whole UUID or its last 8 characters, the
    /// short id, in lower case. Chips of this kind are alternatives, so
    /// several name a list of tasks.
    Id,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    /// Read, never written: the server refuses it. Such a chip meets every
    /// task.
    #[serde(other)]
    Unknown,
}

/// How the views show a scope (ADR 0072 §1). A part a reader does not know
/// is passed over, and that part looks as always.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeLayout {
    /// The inspector (ADR 0072 §2); none = everything shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inspector: Option<InspectorLayout>,
}

/// Which parts of the inspector are shown; the rest is folded under one
/// line that names what has a value (ADR 0072 §2, §3).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectorLayout {
    /// In the inspector's own order; this only chooses.
    #[serde(default)]
    pub shown: Vec<InspectorPart>,
    /// How the note shows; none = `default`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<NoteMode>,
}

/// A part of the inspector a scope can show or fold (ADR 0072 §2). Place,
/// title and the actions are always there and not among them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectorPart {
    Properties,
    Availability,
    State,
    Project,
    Priority,
    Defer,
    Due,
    Planned,
    Repeat,
    /// A habit's streak and history (ADR 0091 §6).
    Series,
    Estimate,
    Tags,
    Note,
    /// The items of a checklist (ADR 0141).
    Checklist,
    Links,
    Reminders,
    Clock,
    Comments,
    Attachments,
    Logbook,
    /// A part from a newer samu (ADR 0015). Read, never written.
    #[serde(other)]
    Unknown,
}

/// How the note shows in the inspector (ADR 0072 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoteMode {
    /// Rendered while it has text, a field when empty (ADR 0058).
    Default,
    /// Rendered; empty, a quiet line to start one.
    Rendered,
    /// A mode from a newer samu (ADR 0015). Read, never written.
    #[serde(other)]
    Unknown,
}

/// A capture template (ADR 0073): a line of quick capture holding only
/// marks, filled in where the typed line names nothing, and reminders for
/// the new task. Called as `/title` in the line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Template {
    pub id: Id,
    pub created: Timestamp,
    pub modified: Timestamp,
    /// The name, and the call: letters, digits, `-` and `_`.
    pub title: String,
    /// Quick capture without a title, like `#Gewohnheiten @habit p:heute
    /// 21:00 every+:1d`; relative dates count from the moment of capture.
    #[serde(default)]
    pub text: String,
    /// Reminders the new task gets (ADR 0030). Changed as a whole.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reminders: Vec<TemplateReminder>,
    #[serde(default)]
    pub position: i64,
    /// The key that calls it after the capture prefix, like `h` for
    /// `C h` (ADR 0084): one letter or digit, `h` and `H` two keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

/// A reminder of a template: an anchor of the new task, and how long
/// before it (ADR 0073 §1), or a time of day (ADR 0093); never an instant
/// of its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateReminder {
    pub anchor: ReminderAnchor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<Period>,
    /// Only with `anchor = daily`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<jiff::civil::Time>,
}

/// What a scope gives a task captured while it is chosen (ADR 0043 §6).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeCapture {
    /// The project, when the line names none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<Id>,
    /// The tags, when the line names none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Id>,
}

/// A window of a scope's schedule (ADR 0043 §7): on these days, from
/// `from` to `to` in the account's zone. When `to` is not after `from` the
/// window ends on the next day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeWindow {
    pub days: Vec<Weekday>,
    pub from: jiff::civil::Time,
    pub to: jiff::civil::Time,
}

/// A day of the week.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Weekday {
    Mo,
    Tu,
    We,
    Th,
    Fr,
    Sa,
    Su,
    /// A value from a newer samu (ADR 0015). Read, never written.
    #[serde(other)]
    Unknown,
}

/// How a scope reads its members (ADR 0035 §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeMode {
    /// Only what a member covers.
    Only,
    /// Everything a member does not cover.
    Except,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    /// Read, never written: the server refuses it. Such a scope hides
    /// nothing.
    #[serde(other)]
    Unknown,
}

/// Who made an object: the device of the session and whether a person or an
/// agent sits behind it (ADR 0039 §1). Set by the server from the session,
/// never taken from a client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Author {
    pub device: String,
    pub kind: AuthorKind,
}

/// The kind of a session and of whoever writes through it (ADR 0039 §2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorKind {
    #[default]
    Human,
    Agent,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    /// Read, never written: the server refuses it.
    #[serde(other)]
    Unknown,
}

/// The settings of an account (ADR 0045 §4): one object per account, made
/// by the server, changed with `set` per field like any object, offline
/// too. Never created or deleted by a client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSettings {
    pub id: Id,
    pub created: Timestamp,
    pub modified: Timestamp,
    /// How long before `due` a task counts as due soon (ADR 0017 §2).
    pub due_soon: Period,
    /// When a reminder on a date without a time goes off (ADR 0030 §3).
    pub remind_time: jiff::civil::Time,
    /// How many days Forecast shows after today; the rest is „Später“.
    #[serde(default = "seven")]
    pub forecast_days: u32,
    /// Forecast has a section of flagged available tasks after today.
    #[serde(default = "yes")]
    pub forecast_flagged: bool,
    /// The tasks with this tag, or one below it, stand under today.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forecast_tag: Option<Id>,
    /// A task stands on the day its defer date makes it available.
    #[serde(default = "yes")]
    pub forecast_defer: bool,
    /// What was completed today stays under today.
    #[serde(default)]
    pub forecast_done: bool,
    /// Each day shows the appointments of the calendars (ADR 0047 §10).
    #[serde(default = "yes")]
    pub forecast_events: bool,
    /// Where a property's value leads (ADR 0054 §3): key → template with
    /// `{value}`, set the value URL-encoded. Each written on its own as
    /// `property_links.<key>`, like properties (ADR 0023).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub property_links: BTreeMap<String, String>,
    /// The writable calendar a new appointment goes into (ADR 0068 §10).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_calendar: Option<Id>,
    /// The web's own key bindings (ADR 0079 §2): action id → binding, or
    /// null for no key. Only what differs from the defaults; written as a
    /// whole.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub keys: BTreeMap<String, Option<String>>,
    /// What the report counts as worked on (ADR 0080 §1); none = `events`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_worked: Option<crate::ReportWorked>,
    /// Since samu-api 0.1.64: how a group that a search, a tag or a
    /// perspective finds starts when its children are no hits themselves
    /// (ADR 0021 §6); none = `open`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit_children: Option<HitChildren>,
}

/// How a group that is a hit starts when its children are none (ADR 0021
/// §6), a setting of the account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HitChildren {
    /// Unfolded: the children stand below it, as context.
    #[default]
    Open,
    /// Folded: the arrow brings them.
    Folded,
    /// A value from a newer samu (ADR 0015). Read, never written.
    #[serde(other)]
    Unknown,
}

const fn seven() -> u32 {
    7
}

const fn yes() -> bool {
    true
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's skip_serializing_if"
)]
const fn is_yes(value: &bool) -> bool {
    *value
}

/// A comment on a task or project (ADR 0039 §3). Changed or deleted only
/// from a session of the kind that wrote it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    pub id: Id,
    pub created: Timestamp,
    pub modified: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<Author>,
    /// The task or project.
    pub owner: Id,
    /// Text like a note.
    pub body: String,
}

/// One item of a task's checklist (ADR 0141): a line to tick off, not a
/// task of its own. Turned into a subtask and back by a client, with
/// ordinary changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub id: Id,
    pub created: Timestamp,
    pub modified: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<Author>,
    /// The task.
    pub owner: Id,
    /// One line of plain text.
    pub text: String,
    /// Ticked off.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub checked: bool,
    /// Order among the items of the task.
    pub position: i64,
}

/// A file on a task, a project or a comment (ADR 0039 §5). The file itself
/// is a blob, named by its SHA-256 and fetched with `GET /blobs/{sha256}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: Id,
    pub created: Timestamp,
    pub modified: Timestamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by: Option<Author>,
    /// The task, project or comment.
    pub owner: Id,
    /// The SHA-256 of the file, 64 lowercase hex digits.
    pub blob: String,
    /// The file name to show and to save as.
    pub name: String,
    /// Like `image/jpeg` or `audio/webm`.
    pub media_type: String,
    /// In bytes.
    pub size: u64,
    /// What is said in an audio file (ADR 0140 §2); absent until a
    /// transcript service has answered. The empty text when nothing was
    /// said. Only the server writes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript: Option<String>,
    /// Whether a transcript is on its way (ADR 0140 §3). A client may
    /// create an attachment with `pending`, which asks for the transcript;
    /// everything else is the server's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_state: Option<TranscriptState>,
    /// Why the last try failed, with `transcript_state` `failed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_error: Option<String>,
}

/// Where the transcript of an attachment stands while there is none
/// (ADR 0140 §3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TranscriptState {
    /// Asked for; the server is at it or will be.
    Pending,
    /// The service refused or could not be reached; asking again tries
    /// again.
    Failed,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    /// Read, never written.
    #[serde(other)]
    Unknown,
}

/// `PUT /blobs/{sha256}` answers with this (ADR 0039 §4), the same whether
/// the file was new or there already.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blob {
    /// The SHA-256 of the file, 64 lowercase hex digits.
    pub hash: String,
    /// In bytes.
    pub size: u64,
    /// The type it was first uploaded with, like `image/jpeg`.
    pub media_type: String,
}

/// A perspective (ADR 0090): a saved view of the search, its filter, its
/// arrangement and its presentation, shown inside the chosen scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Perspective {
    pub id: Id,
    pub created: Timestamp,
    pub modified: Timestamp,
    /// The name; unique without regard to case among live perspectives.
    pub title: String,
    /// One emoji, as for a folder or project (ADR 0059).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Free words of the search, each in title, note or id (ADR 0042 §2).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// Chips as in a scope's filter (ADR 0043 §2): projects, tags, folders
    /// and states as ids. Changed as a whole.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chips: Vec<ScopeChip>,
    #[serde(default)]
    pub filter: PerspectiveFilter,
    /// Grouping and sorting; none = by project, in its own order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrange: Option<PerspectiveArrange>,
    #[serde(default)]
    pub view: PerspectiveView,
    /// The board's columns (ADR 0061); none = from the default workflow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<PerspectiveBoard>,
    /// The table's columns (ADR 0115 §7); none = the default ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<PerspectiveTable>,
    /// Since samu-api 0.1.54: the blocks of a block perspective (ADR 0116
    /// §1), changed as a whole. Not empty, the perspective shows these one
    /// below the other, and its search, filter, arrangement and
    /// presentation do not count; an older reader shows it as a plain
    /// perspective with no search, which `/available` answers with all
    /// blocks together.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<PerspectiveBlock>,
    /// Since samu-api 0.1.54: a task that several blocks match stands only
    /// in the first (ADR 0116 §3); false, in every one, as in org.
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub dedupe: bool,
    #[serde(default)]
    pub position: i64,
}

/// One block of a block perspective (ADR 0116 §2, ADR 0143 §1): another
/// perspective, a fixed view or a search of its own, exactly one of the
/// three.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerspectiveBlock {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perspective: Option<Id>,
    /// A fixed view by name: `inbox`, `today` or `flagged`. A plain string,
    /// so a view from a newer samu survives an older client; a reader
    /// shows a block it does not know as missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<String>,
    /// Since samu-api 0.1.65: a search of the block's own (ADR 0143 §1),
    /// shown as a plain perspective with these fields would show it. An
    /// older reader sees a block of no form it knows and shows it as
    /// missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query: Option<BlockQuery>,
    /// A heading of its own; none = the name of the perspective or view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// The search of a block of its own (ADR 0143 §1): what a plain
/// perspective holds, with the same defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockQuery {
    /// Free words of the search, as [`Perspective::text`].
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chips: Vec<ScopeChip>,
    #[serde(default)]
    pub filter: PerspectiveFilter,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrange: Option<PerspectiveArrange>,
    /// A board is shown as a list in a block (ADR 0116 §5); kept so that
    /// nothing is lost between a perspective and a block.
    #[serde(default)]
    pub view: PerspectiveView,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<PerspectiveBoard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table: Option<PerspectiveTable>,
}

impl BlockQuery {
    /// The search of a plain perspective, for a block that is detached
    /// from it (ADR 0143 §4).
    #[must_use]
    pub fn of(perspective: &Perspective) -> Self {
        Self {
            text: perspective.text.clone(),
            chips: perspective.chips.clone(),
            filter: perspective.filter,
            arrange: perspective.arrange.clone(),
            view: perspective.view,
            board: perspective.board.clone(),
            table: perspective.table.clone(),
        }
    }

    /// A plain perspective with this search, the other fields as given:
    /// what the block shows, or what "Save as perspective" makes of it
    /// (ADR 0143 §2, §4).
    #[must_use]
    pub fn into_perspective(self, id: Id, created: Timestamp, title: String) -> Perspective {
        Perspective {
            id,
            created,
            modified: created,
            title,
            icon: None,
            text: self.text,
            chips: self.chips,
            filter: self.filter,
            arrange: self.arrange,
            view: self.view,
            board: self.board,
            table: self.table,
            blocks: Vec::new(),
            dedupe: true,
            position: 0,
        }
    }
}

/// Available, remaining or all (ADR 0020 §1, ADR 0090 §1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerspectiveFilter {
    #[default]
    Available,
    Remaining,
    All,
    /// A value from a newer samu (ADR 0015): read as `available`, never
    /// written.
    #[serde(other)]
    Unknown,
}

/// How a perspective groups and sorts (ADR 0042 §5–6, ADR 0064).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerspectiveArrange {
    #[serde(default)]
    pub group: PerspectiveGroup,
    #[serde(default)]
    pub sort: PerspectiveSort,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub desc: bool,
    /// The second key (ADR 0064 §2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub then: Option<PerspectiveSort>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub then_desc: bool,
}

/// What a list groups by (ADR 0042 §5, ADR 0064 §3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerspectiveGroup {
    #[default]
    Project,
    Folder,
    Tag,
    State,
    Priority,
    Date,
    None,
    /// A value from a newer samu (ADR 0015): read as `project`, never
    /// written.
    #[serde(other)]
    Unknown,
}

/// What a list sorts by (ADR 0042 §6, ADR 0056).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerspectiveSort {
    /// Its own order.
    #[default]
    Position,
    Due,
    Planned,
    Defer,
    Title,
    Created,
    Modified,
    Estimate,
    Priority,
    /// A value from a newer samu (ADR 0015): read as `position`, never
    /// written.
    #[serde(other)]
    Unknown,
}

/// List, board or table (ADR 0061 §1, ADR 0115 §1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerspectiveView {
    #[default]
    List,
    Board,
    /// Since samu-api 0.1.53; an older reader shows a list.
    Table,
    /// A value from a newer samu (ADR 0015): read as `list`, never written.
    #[serde(other)]
    Unknown,
}

/// The columns of a perspective's board (ADR 0061 §2–5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerspectiveBoard {
    #[serde(default)]
    pub source: BoardSource,
    /// The workflow the columns come from; none = the default one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<Id>,
    /// The tags that are columns, in order, with `source: tags`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<Id>,
    /// A column for what has none of them.
    #[serde(default = "yes")]
    pub none: bool,
    /// A column for what was completed in the last 7 days.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub done: bool,
    /// The most cards per column, by column: `none`, `done` or a tag's id.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub wip: BTreeMap<String, u32>,
}

impl Default for PerspectiveBoard {
    fn default() -> Self {
        Self {
            source: BoardSource::default(),
            workflow: None,
            tags: Vec::new(),
            none: true,
            done: false,
            wip: BTreeMap::new(),
        }
    }
}

/// The columns of a perspective's table (ADR 0115 §2, §7).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerspectiveTable {
    /// The shown columns after the title, in order, by name: `state`,
    /// `priority`, `project`, `tags`, `defer`, `due`, `planned`, `created`,
    /// `modified`, `completed`, `estimate`, `worked`, `remaining`,
    /// `blocked_by`, or `prop:<key>` for a property. Plain strings, so a
    /// column from a newer samu survives an older client; a reader skips
    /// the names it does not know.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<String>,
}

/// Where a board's columns come from (ADR 0061 §2).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoardSource {
    #[default]
    Workflow,
    Tags,
    /// A value from a newer samu (ADR 0015): read as `workflow`, never
    /// written.
    #[serde(other)]
    Unknown,
}
