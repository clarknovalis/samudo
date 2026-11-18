// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Android app's way into samu (ADR 0028).
//!
//! A thin layer over `samu-core` and `samu-client`: it keeps the mirror of
//! the server and the outbox in a small database ([`cache`]), shows the
//! mirror with the outbox laid over it (`samu_core::overlay`), computes
//! availability there with `samu-core`, and syncs: push the outbox, then
//! pull what changed. Everything the app does is an ordinary change through
//! `POST /changes`; there is no function only the app has.
//!
//! Reasons and warnings cross as codes; the app words them, as the web UI
//! does (ADR 0027 §4). All calls block; the app makes them off the main
//! thread.

mod arrange;
mod bar;
mod bulk;
mod clocking;
mod close;
mod edit;
mod habits;
mod line;
mod notes;
mod props;
mod remind;
mod search;
mod views;

pub use arrange::Arrange;
pub use bar::{BarEntry, BarItem};
pub use clocking::{ClockRow, RunningClock};
pub use close::{Closed, FieldBack};
pub use edit::{RepeatEdit, TagOption, TaskEdit};
pub use habits::{HabitDayRow, HabitDetail, HabitEntryRow, HabitLine, HabitRow};
pub use line::{EditChip, EditPreview};
pub use notes::{AttachmentRow, CommentRow};
pub use remind::{AlarmRow, Anchor, Notice, ReminderRow};
pub use search::{SearchChip, SearchOffer, SearchSplit};
pub use views::{PerspectiveRow, ProjectView};

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use samu_api::{
    Calendar, CalendarKind, Change, DeleteRequest, DeleteResponse, FolderStatus, Hlc, Id,
    InspectorPart, Link, LinkKind, Login, NoteMode, ObjectKind, PairRequest, Project,
    ProjectStatus, Reason, Scope, ScopeMode, SessionInfo, Tag, TagStatus, Task, TaskFate, When,
};
use samu_client::Client;
use samu_core::availability::{DueState, Evaluation, Settings, evaluate};
use samu_core::blockers;
use samu_core::capture::{
    Known, Meaning, Warning, apply_template, completed_at, parse, place_clocks, resolve_in_scope,
    template_reminders,
};
use samu_core::clock::next_stamp;
use samu_core::forecast;
use samu_core::hook::{RING_VIA_TASKER, ring_task};
use samu_core::overlay::{self, Objects};
use samu_core::pairing::PairingUri;
use samu_core::scope::Lens;
use samu_core::share::shared;
use samu_core::workflow::placement;
use serde_json::{Value, json};

use samu_replica::{Cache, CacheError};

uniffi::setup_scaffolding!();

/// How long a request may take before the phone gives up and tries later.
const TIMEOUT: Duration = Duration::from_secs(20);

/// What went wrong, as the app shows it. The text is `reason`, not
/// `message`, which would collide with `Throwable.message` in Kotlin.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum MobileError {
    /// The phone's database could not be opened or read.
    #[error("{reason}")]
    Store { reason: String },
    /// Not signed in, or the session is gone: back to the sign-in screen.
    /// The outbox stays.
    #[error("{reason}")]
    SignedOut { reason: String },
    /// The server could not be reached; nothing is lost, the next sync tries
    /// again.
    #[error("{reason}")]
    Offline { reason: String },
    /// The server refused, with its message.
    #[error("{reason}")]
    Refused { reason: String },
    /// Something typed that cannot be used.
    #[error("{reason}")]
    Input { reason: String },
}

impl From<samu_replica::Error> for MobileError {
    fn from(error: samu_replica::Error) -> Self {
        use samu_replica::Error as E;
        match error {
            E::Store(reason) => Self::Store { reason },
            E::SignedOut(reason) => Self::SignedOut { reason },
            E::Offline(reason) => Self::Offline { reason },
            E::Refused(reason) => Self::Refused { reason },
        }
    }
}

impl From<CacheError> for MobileError {
    fn from(error: CacheError) -> Self {
        Self::Store {
            reason: error.to_string(),
        }
    }
}

impl From<samu_client::Error> for MobileError {
    fn from(error: samu_client::Error) -> Self {
        use samu_client::Error as E;
        match error {
            E::Unauthorized(reason) => Self::SignedOut { reason },
            E::NoServer(reason) => Self::Offline { reason },
            E::Io(e) => Self::Offline {
                reason: e.to_string(),
            },
            other => Self::Refused {
                reason: other.to_string(),
            },
        }
    }
}

fn input(reason: impl Into<String>) -> MobileError {
    MobileError::Input {
        reason: reason.into(),
    }
}

/// How the app stands: signed in or not, and how far the sync is.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Status {
    pub signed_in: bool,
    pub server: Option<String>,
    pub account: Option<String>,
    pub device: Option<String>,
    /// Changes not yet confirmed by the server (ADR 0020 §1).
    pub outbox: u32,
    /// When the last sync ended, RFC 3339.
    pub last_sync: Option<String>,
    /// What the last sync or a refused change said; none when all is well.
    pub last_error: Option<String>,
    /// The zone "today" is computed in (IANA).
    pub zone: String,
    /// False when the server did not say its zone and the phone's is used
    /// (ADR 0028 §6).
    pub zone_from_server: bool,
}

/// Counts for the launcher (see [`Samu::glance`]).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Glance {
    /// Open tasks in the inbox.
    pub inbox: u32,
    /// Of `today_open`, the overdue ones.
    pub overdue: u32,
    /// What "Heute" lists: overdue, today and flagged, each once.
    pub today_open: u32,
    /// Completed today, in the account's zone.
    pub today_done: u32,
}

/// What one sync did.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SyncReport {
    /// Changes the server took.
    pub pushed: u32,
    /// Changes the server refused; they left the outbox.
    pub rejected: u32,
    /// Objects that came in.
    pub pulled: u32,
    /// The mirror was loaded anew.
    pub reset: bool,
}

/// Whether a due date is near (ADR 0017 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Due {
    None,
    Soon,
    Overdue,
}

/// Why a task is not available, as a code and what it names: a task's or
/// project's title, a tag, a date (ADR 0020 §2).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReasonRow {
    pub code: String,
    pub subject: Option<String>,
}

/// One task as a list shows it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TaskRow {
    pub id: String,
    pub title: String,
    pub project_id: Option<String>,
    pub project: Option<String>,
    /// Where it lives (ADR 0026 §7): `Projekt · Ordner/Unterordner`; none in
    /// the inbox.
    pub place: Option<String>,
    pub flagged: bool,
    pub done: bool,
    pub dropped: bool,
    pub available: bool,
    /// `2026-11-20` or `2026-11-20 09:00`.
    pub due: Option<String>,
    pub due_state: Due,
    pub defer: Option<String>,
    pub planned: Option<String>,
    /// The estimate as written, `PT1H30M` (ADR 0053 §3).
    pub estimate: Option<String>,
    /// `A`, `B` or `C` (ADR 0056); none without one.
    pub priority: Option<String>,
    /// The note is not empty (ADR 0053 §5).
    pub has_note: bool,
    /// An open task waits for it: 🚧 before the title.
    pub blocks: bool,
    /// The first reason it is not available.
    pub reason: Option<ReasonRow>,
    /// Tags other than the workflow state, as paths.
    pub tags: Vec<String>,
    /// Its state in the workflow (ADR 0024), by title.
    pub state: Option<String>,
    /// It has children: an action group (ADR 0021).
    pub group: bool,
    /// How deep under other tasks in this list.
    pub depth: u32,
    /// Waiting in the outbox: not yet on the server.
    pub pending: bool,
    /// In Forecast: why it stands there (ADR 0045), `due`, `planned`,
    /// `defer`, `tag`, `done`, `flagged`; none elsewhere.
    pub why: Option<String>,
    /// The date that reason names, `2026-11-13`.
    pub why_date: Option<String>,
    /// A habit (ADR 0092 §2): ✓ or +1 instead of the circle.
    pub habit: Option<HabitRow>,
}

/// A run of rows under one heading: a project, or the inbox (no id).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Section {
    pub project_id: Option<String>,
    pub title: Option<String>,
    /// The project's folder with its whole path, `Privat/Wohnung`.
    pub folder: Option<String>,
    pub rows: Vec<TaskRow>,
}

/// A section of Forecast (ADR 0045): `overdue`, `day`, `flagged` or
/// `later`, in the order samu-core puts them.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Day {
    pub kind: String,
    /// `2026-11-08` for a day; none for the others.
    pub date: Option<String>,
    /// The day is today in the account's zone, so the app need not know it.
    pub today: bool,
    pub rows: Vec<TaskRow>,
    /// The appointments of the day, before the tasks (ADR 0047 §10).
    pub appointments: Vec<AppointmentRow>,
}

/// An appointment as Forecast shows it, its times already in the account's
/// zone so the app need not know zones.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AppointmentRow {
    pub id: String,
    pub title: String,
    /// Over whole days; then `start` and `end` are empty.
    pub all_day: bool,
    /// `09:00` in the account's zone; an appointment over several days
    /// shows the same start on each of them.
    pub start: Option<String>,
    pub end: Option<String>,
    pub location: Option<String>,
    pub note: Option<String>,
    /// Where „open in the calendar“ leads (ADR 0047 §9).
    pub link: Option<String>,
    /// The calendar's colour, like `#3b82f6`.
    pub color: Option<String>,
    pub calendar: String,
    pub tentative: bool,
    /// The appointment's UID, to find it in the phone's calendar provider
    /// (ADR 0049 §2).
    pub uid: String,
    /// The day it starts, `2026-11-20`, in the account's zone.
    pub date: String,
    /// Start and end in milliseconds since the epoch, as Android's
    /// calendar intents take them; for whole days UTC midnight, as the
    /// provider keeps them (ADR 0049 §2).
    pub begin_ms: i64,
    pub end_ms: i64,
    /// The reminders its VALARMs carry, for the sheet (ADR 0101 §9),
    /// also when this phone does not remind of appointments.
    pub alarms: Vec<AlarmRow>,
}

/// What the calendar view shows (ADR 0047 §11, ADR 0049): a day or a week,
/// with the days to step to, so the app need not count dates.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CalendarPage {
    pub days: Vec<CalendarDay>,
    /// The first day shown one step back and forward, and today.
    pub previous: String,
    pub next: String,
    pub today: String,
}

/// One day of the calendar view.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct CalendarDay {
    /// `2026-11-20`.
    pub date: String,
    pub today: bool,
    /// Its midnight in the account's zone, in milliseconds since the epoch:
    /// a minute of the grid is this plus the minute (ADR 0049 §4).
    pub start_ms: i64,
    /// The minute of the day it is now, on today only: the line.
    pub now: Option<u32>,
    /// Appointments over whole days, for the band; a long one on each of
    /// its days.
    pub all_day: Vec<AppointmentRow>,
    /// Appointments and tasks at a time, laid out.
    pub blocks: Vec<Block>,
}

/// A block of the hour grid: from `top` to `bottom` in minutes of the day,
/// in lane `lane` of `lanes` side by side. Either an appointment or a task.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Block {
    pub top: u32,
    pub bottom: u32,
    pub lane: u32,
    pub lanes: u32,
    /// `09:00`, `bis 10:30` when it began a day before, or `ganzer Tag`.
    pub label: String,
    pub appointment: Option<AppointmentRow>,
    pub task: Option<TaskRow>,
}

/// A calendar for the settings screen (ADR 0047 §8, §13): what the phone
/// may change offline (title, colour, on/off) and what the server said.
/// The source never comes to the phone.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CalendarRow {
    pub id: String,
    pub title: String,
    /// `ics` or `caldav`.
    pub kind: String,
    pub color: Option<String>,
    pub enabled: bool,
    /// The host of the source; none without a source.
    pub host: Option<String>,
    /// What went wrong the last time; none when it went well.
    pub error: Option<String>,
    /// When a fetch last changed something, RFC 3339.
    pub fetched_at: Option<String>,
    /// Appointments the phone has of it.
    pub appointments: u32,
}

/// The account's Forecast settings (ADR 0045 §4), for the settings screen.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ForecastSettings {
    /// The settings object has come from the server; until then these are
    /// the defaults and cannot be changed.
    pub known: bool,
    pub days: u32,
    pub flagged: bool,
    pub tag: Option<String>,
    pub tag_path: Option<String>,
    pub defer: bool,
    pub done: bool,
    /// Appointments of the calendars in Forecast (ADR 0047 §10).
    pub events: bool,
}

/// A tag to filter by, with the open tasks it would show.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TagChip {
    pub id: String,
    /// `Status/Wartet`.
    pub path: String,
    /// Open tasks with this tag or one below it.
    pub count: u32,
}

/// A folder or an open project in the tab „Projekte“, as a tree
/// (ADR 0085 §1).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TreeRow {
    pub id: String,
    /// `folder` or `project`.
    pub kind: String,
    pub title: String,
    /// An emoji before the title (ADR 0059).
    pub icon: Option<String>,
    pub depth: u32,
    /// Available tasks: a project's own, a folder's the sum below it
    /// (ADR 0085 §7).
    pub available: u32,
    /// A project available but without a next action (ADR 0006).
    pub stuck: bool,
    /// A project due for review (ADR 0088 §7).
    pub review_due: bool,
    pub on_hold: bool,
    /// Shown only in its own scope (ADR 0094).
    pub scope_only: bool,
    /// The title of that scope, for the mark „nur in …“ (ADR 0094 §8).
    pub own_scope_title: Option<String>,
    /// A folder folded on this phone; what is below it is left out.
    pub folded: bool,
    /// A shown sibling of the same kind before it, or after it: what „Nach
    /// oben“ and „Nach unten“ pass (ADR 0085 §4).
    pub can_up: bool,
    pub can_down: bool,
}

/// A capture template (ADR 0073): `/title` in the line calls it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TemplateRow {
    pub id: String,
    pub title: String,
}

/// A scope (ADR 0035), as the menu and the settings show it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ScopeRow {
    pub id: String,
    pub title: String,
    /// The views show only what the members cover; else everything else.
    pub only: bool,
    /// Chosen on this phone.
    pub chosen: bool,
    /// The inbox is a member (ADR 0043 §1).
    pub inbox: bool,
    /// `violet`, `blue`, … (ADR 0060); none = neutral, or a colour from a
    /// newer samu.
    pub color: Option<String>,
}

/// What a task captured in a scope gets when its line names none
/// (ADR 0043 §6), by id.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ScopeCaptureRow {
    pub project: Option<String>,
    pub tags: Vec<String>,
}

/// What can be a member of a scope, in the order of the settings: folders
/// with their projects as a tree, then the tags as a tree.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MemberRow {
    pub id: String,
    pub title: String,
    /// `folder`, `project` or `tag`.
    pub kind: String,
    pub depth: u32,
    pub member: bool,
    /// Shown only in its own scope (ADR 0094): a mark in the tree.
    pub scope_only: bool,
    /// The title of that scope, for the mark „nur in …“ (ADR 0094 §8).
    pub own_scope_title: Option<String>,
}

/// A folder, project or tag in the settings (ADR 0036 §8).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PlaceRow {
    pub id: String,
    pub title: String,
    /// `folder`, `project` or `tag`.
    pub kind: String,
    pub depth: u32,
    /// What a project is beyond the plain: `Einzelaufgaben · ruht`.
    pub meta: String,
    /// A completed or dropped project.
    pub closed: bool,
    /// Shown only in its own scope (ADR 0094).
    pub scope_only: bool,
    /// The title of that scope, for the mark „nur in …“ (ADR 0094 §8).
    pub own_scope_title: Option<String>,
}

/// One folder, project or tag in full, for its own screen (ADR 0037 §6).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PlaceDetail {
    pub id: String,
    pub title: String,
    /// `folder`, `project` or `tag`.
    pub kind: String,
    /// The folder or tag above; none at the top.
    pub place: Option<String>,
    pub place_title: Option<String>,
    /// Only for a project: `parallel`, `sequential`, `single_actions`.
    pub project_kind: Option<String>,
    /// Only for a project: `active`, `on_hold`, `completed`, `dropped`.
    pub status: Option<String>,
    /// Only for a project: its exclusive tag group; none is the default.
    pub workflow: Option<String>,
    /// Only for a project: `P1W`, `P2W`, `P1M`, `P3M`, or none.
    pub review: Option<String>,
    /// Only for a project: its open tasks.
    pub open_tasks: u32,
    /// A folder's or project's emoji (ADR 0059).
    pub icon: Option<String>,
    /// Shown only in its own scope (ADR 0094).
    pub scope_only: bool,
    /// „Nur im Scope:“ (ADR 0094 §1): the scope's id, none for everywhere.
    pub own_scope: Option<String>,
    /// Its title; none also when the scope does not exist (§5).
    pub own_scope_title: Option<String>,
}

/// A scope's inspector layout (ADR 0072 §2) for its settings: the parts
/// it shows by key, in the inspector's order, and the note's mode.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct InspectorLayoutRow {
    pub shown: Vec<String>,
    pub note_rendered: bool,
}

/// How the task page looks in the chosen scope (ADR 0072 §2–§4).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TaskLayout {
    /// The parts to show by key; none = all of them (unfolded).
    pub shown: Option<Vec<String>>,
    /// Folded parts with a value, by key in the inspector's order: the
    /// line under the parts names them first (§3).
    pub valued: Vec<String>,
    /// How many folded parts have no value.
    pub rest: u32,
    pub note_rendered: bool,
    /// Unfolded on this phone, for this scope (§4).
    pub unfolded: bool,
}

/// The inspector's parts in its order (ADR 0072 §2), as on the wire.
const INSPECTOR_PARTS: [&str; 19] = [
    "properties",
    "availability",
    "state",
    "project",
    "priority",
    "defer",
    "due",
    "planned",
    "repeat",
    "series",
    "estimate",
    "tags",
    "note",
    "links",
    "reminders",
    "clock",
    "comments",
    "attachments",
    "logbook",
];

fn part_key(part: InspectorPart) -> Option<&'static str> {
    let key = serde_json::to_value(part).ok()?;
    INSPECTOR_PARTS.iter().copied().find(|k| key == *k)
}

/// The own scope of a folder, project or tag (ADR 0094 §1).
fn own_scope(objects: &Objects, id: Id) -> Option<Id> {
    objects
        .folders
        .iter()
        .find(|f| f.id == id)
        .map(|f| f.own_scope)
        .or_else(|| {
            objects
                .projects
                .iter()
                .find(|p| p.id == id)
                .map(|p| p.own_scope)
        })
        .or_else(|| {
            objects
                .tags
                .iter()
                .find(|t| t.id == id)
                .map(|t| t.own_scope)
        })
        .flatten()
}

/// Whether a folder, project or tag is shown only in its own scope.
fn is_scope_only(objects: &Objects, id: Id) -> bool {
    own_scope(objects, id).is_some()
}

/// The title of the scope `own` names, if that scope exists (ADR 0094 §5).
fn scope_title(objects: &Objects, own: Option<Id>) -> Option<String> {
    let own = own?;
    objects
        .scopes
        .iter()
        .find(|s| s.id == own)
        .map(|s| s.title.clone())
}

/// The title of the own scope of a folder, project or tag.
fn own_scope_title(objects: &Objects, id: Id) -> Option<String> {
    scope_title(objects, own_scope(objects, id))
}

/// The palette of scopes on the wire (ADR 0060 §1).
const SCOPE_COLORS: [&str; 8] = [
    "violet", "blue", "teal", "green", "yellow", "orange", "red", "pink",
];

/// A scope's colour as the app takes it; none for one from a newer samu.
fn scope_color(color: Option<samu_api::ScopeColor>) -> Option<String> {
    color
        .filter(|c| *c != samu_api::ScopeColor::Unknown)
        .and_then(|c| serde_json::to_value(c).ok())
        .and_then(|v| v.as_str().map(str::to_owned))
}

/// A priority as the app shows it; none for one from a newer samu.
pub(crate) fn priority_text(priority: samu_api::Priority) -> Option<String> {
    match priority {
        samu_api::Priority::A => Some("A".into()),
        samu_api::Priority::B => Some("B".into()),
        samu_api::Priority::C => Some("C".into()),
        samu_api::Priority::Unknown => None,
    }
}

/// What a delete did, or would do (ADR 0036 §1).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DeleteCount {
    pub deleted: u64,
    pub moved: u64,
    pub untagged: u64,
}

/// One task in full, for its own screen.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TaskDetail {
    pub row: TaskRow,
    pub reasons: Vec<ReasonRow>,
    pub note: Option<String>,
    pub estimate: Option<String>,
    /// When the task was captured (a1f43aed), milliseconds since the epoch.
    pub created_ms: i64,
}

/// A task another one waits for (ae5a3206), or one it could wait for.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BlockerRow {
    /// The `blocked_by` link; none for a task the picker offers.
    pub link_id: Option<String>,
    pub task_id: String,
    /// The task's title, or the link's address when the task is not here.
    pub title: String,
    /// `Projekt · Ordner`; none in the inbox.
    pub place: Option<String>,
    pub done: bool,
}

/// One recognised piece of a quick-capture line (ADR 0011, ADR 0027).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Chip {
    /// `project`, `tag`, `new_tag`, `flag`, `priority`, `defer`, `due`, `planned`,
    /// `estimate`, `repeat`, `note`.
    pub kind: String,
    /// A new tag shows its whole path, `Ort/Garten`.
    pub value: String,
    /// Where the line says it, to take it out with a tap; empty when the
    /// value comes from the view or the scope, not from the line.
    pub spans: Vec<Span>,
}

/// A stretch of the line in UTF-16 code units, as Compose counts them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

/// What to offer for the `#…` or `@…` word at the cursor: open projects or
/// tags by path, those that start with what is typed first (as the web
/// does, ADR 0027). `start..end` is the whole word, in UTF-16 code units.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Offer {
    pub start: u32,
    pub end: u32,
    /// `#` or `@`.
    pub sign: String,
    pub items: Vec<String>,
}

/// A warning of the parser, as a code and the text it is about.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct WarningRow {
    pub code: String,
    pub text: String,
}

/// What a quick-capture line would make, computed on the phone.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CapturePreview {
    pub title: String,
    pub project: Option<String>,
    pub chips: Vec<Chip>,
    pub warnings: Vec<WarningRow>,
}

/// What a text shared to the app makes (ADR 0013), before anything is
/// typed: shown in the sheet, handed back to [`Samu::capture_shared`].
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SharedDraft {
    pub title: String,
    pub note: Option<String>,
    /// `catastr:`, `mid:` and `https://` addresses, each once.
    pub links: Vec<String>,
}

/// The phone's samu: one database in the app's private storage.
#[derive(uniffi::Object)]
pub struct Samu {
    cache: Mutex<Cache>,
    /// Where the database lies, and beside it the files (`blobs/`).
    dir: PathBuf,
    /// The language of the interface, as the app resolved it (ADR 0105
    /// §2): the words of the search field are shown in it.
    lang: Mutex<samu_core::query::Lang>,
}

impl Samu {
    fn cache(&self) -> Result<MutexGuard<'_, Cache>, MobileError> {
        self.cache.lock().map_err(|_| MobileError::Store {
            reason: "the database lock is poisoned".into(),
        })
    }

    fn client(&self) -> Result<Client, MobileError> {
        let cache = self.cache()?;
        let server = cache
            .meta("server")?
            .ok_or_else(|| MobileError::SignedOut {
                reason: "not signed in".into(),
            })?;
        let token = cache.meta("token")?.ok_or_else(|| MobileError::SignedOut {
            reason: "not signed in".into(),
        })?;
        Ok(Client::new(&server, Some(token))?.with_timeout(TIMEOUT))
    }

    fn remember_session(&self, session: &SessionInfo) -> Result<(), MobileError> {
        Ok(samu_replica::remember_session(&*self.cache()?, session)?)
    }

    /// Keeps a new session: only the token, never a password. Another
    /// server or account forgets the mirror and the outbox of the old one.
    fn signed_in(&self, server: &str, session: &SessionInfo) -> Result<Status, MobileError> {
        let token = session.token.clone().ok_or_else(|| MobileError::Refused {
            reason: "the server sent no token".into(),
        })?;
        {
            let mut cache = self.cache()?;
            let same = cache.meta("server")?.as_deref() == Some(server)
                && cache.meta("account")?.as_deref() == Some(session.account.as_str());
            if !same {
                // Another account would take the outbox with the mirror
                // (ADR 0107 §6): refuse, and end the new session again. A
                // folder that never held an account keeps its outbox for
                // the first one.
                let before = cache.meta("account")?;
                let waiting = cache.outbox_len()?;
                if before.is_some() && waiting > 0 {
                    if let Ok(client) = Client::new(server, Some(token.clone())) {
                        let _ = client.with_timeout(TIMEOUT).call("POST", "/logout", None);
                    }
                    return Err(MobileError::Refused {
                        reason: format!(
                            "{waiting} changes still wait in the outbox of {}; add the other account as a new one",
                            before.unwrap_or_default()
                        ),
                    });
                }
                if before.is_some() {
                    cache.clear(true)?;
                }
                // The files of the other account go with its mirror.
                let _ = std::fs::remove_dir_all(self.dir.join("blobs"));
            }
            cache.set_meta("server", Some(server))?;
            cache.set_meta("account", Some(&session.account))?;
            cache.set_meta("device", Some(&session.device))?;
            cache.set_meta("token", Some(&token))?;
            cache.set_meta("last_error", None)?;
        }
        self.remember_session(session)?;
        self.status()
    }

    /// Settings from what the server said, else the phone's zone.
    fn settings(cache: &Cache) -> Result<(Settings, bool), MobileError> {
        Ok(samu_replica::settings(cache)?)
    }

    /// The scope chosen on this phone (ADR 0035 §4), if it is still there.
    /// While the phone follows the schedule, the last event wins: a choice
    /// by hand (or by Tasker) since the schedule's last edge, else the
    /// schedule's scope (ADR 0043 §7, §8).
    fn chosen<'a>(cache: &Cache, objects: &'a Objects) -> Result<Option<&'a Scope>, MobileError> {
        let id = if cache.meta("scope_auto")?.as_deref() == Some("1") {
            let (settings, _) = Self::settings(cache)?;
            let schedule =
                samu_core::scope::schedule_now(&objects.scopes, Timestamp::now(), &settings.zone);
            let hand = match (cache.meta("scope_hand")?, cache.meta("scope_hand_at")?) {
                (Some(id), Some(at)) => at
                    .parse::<Timestamp>()
                    .ok()
                    .map(|at| (id.parse::<Id>().ok(), at)),
                _ => None,
            };
            samu_core::scope::follow(hand, &schedule).map(|id| id.to_string())
        } else {
            cache.meta("scope")?
        };
        let Some(id) = id else {
            return Ok(None);
        };
        Ok(objects.scopes.iter().find(|s| s.id.to_string() == id))
    }

    /// The tasks the chosen scope shows, its filter counted from `now` in
    /// the account's zone (ADR 0043 §3); all of them without one.
    fn visible(
        cache: &Cache,
        objects: &Objects,
        evaluation: &Evaluation,
        now: Timestamp,
        settings: &Settings,
    ) -> Result<Vec<Task>, MobileError> {
        let snapshot = objects.snapshot();
        let lens = Lens::new(
            Self::chosen(cache, objects)?,
            &snapshot,
            now,
            &settings.zone,
        );
        Ok(objects
            .tasks
            .iter()
            .filter(|t| {
                evaluation
                    .tasks
                    .get(&t.id)
                    .is_some_and(|state| lens.task(t, state))
            })
            .cloned()
            .collect())
    }

    /// The folders folded on this phone (ADR 0085 §7).
    fn folded(cache: &Cache) -> Result<HashSet<Id>, MobileError> {
        Ok(cache
            .meta("folded_places")?
            .and_then(|s| serde_json::from_str::<Vec<Id>>(&s).ok())
            .unwrap_or_default()
            .into_iter()
            .collect())
    }

    /// The active folders and the open projects the chosen scope shows
    /// (ADR 0085 §1); below a dropped folder nothing.
    fn shown_places(cache: &Cache, objects: &Objects) -> Result<HashSet<Id>, MobileError> {
        let (settings, _) = Self::settings(cache)?;
        let snapshot = objects.snapshot();
        let lens = Lens::new(
            Self::chosen(cache, objects)?,
            &snapshot,
            Timestamp::now(),
            &settings.zone,
        );
        let dropped: HashSet<Id> = objects
            .folders
            .iter()
            .filter(|f| f.status != FolderStatus::Active)
            .map(|f| f.id)
            .collect();
        // A folder lies below a dropped one when one on its way up is.
        let below_dropped = |mut folder: Option<Id>| {
            let mut seen = HashSet::new();
            while let Some(id) = folder {
                if dropped.contains(&id) {
                    return true;
                }
                if !seen.insert(id) {
                    return false;
                }
                folder = objects
                    .folders
                    .iter()
                    .find(|f| f.id == id)
                    .and_then(|f| f.parent);
            }
            false
        };
        let folders = objects
            .folders
            .iter()
            .filter(|f| !below_dropped(Some(f.id)) && lens.folder(f))
            .map(|f| f.id);
        let projects = objects
            .projects
            .iter()
            .filter(|p| matches!(p.status, ProjectStatus::Active | ProjectStatus::OnHold))
            .filter(|p| !below_dropped(p.folder) && lens.project(p))
            .map(|p| p.id);
        Ok(folders.chain(projects).collect())
    }

    /// The mirror with the outbox laid over it, and which objects the
    /// outbox touches.
    fn view(cache: &Cache) -> Result<(Objects, Vec<Id>), MobileError> {
        Ok(samu_replica::view(cache)?)
    }

    /// Stamps and queues changes made on the phone, after checking they lay
    /// over what is there.
    fn queue(
        &self,
        make: impl FnOnce(&Objects, &mut dyn FnMut() -> Hlc) -> Result<Vec<Change>, MobileError>,
    ) -> Result<(), MobileError> {
        self.queue_numbered(make).map(|_| ())
    }

    /// `queue`, with the numbers the changes got in the outbox, for
    /// `withdraw` (ADR 0104 §3).
    fn queue_numbered(
        &self,
        make: impl FnOnce(&Objects, &mut dyn FnMut() -> Hlc) -> Result<Vec<Change>, MobileError>,
    ) -> Result<Vec<i64>, MobileError> {
        let mut cache = self.cache()?;
        let device = cache.meta("device")?.unwrap_or_else(|| "android".into());
        let mut own: Option<Hlc> = cache
            .meta("hlc")?
            .and_then(|s| serde_json::from_str(&s).ok());
        let (mut objects, _) = Self::view(&cache)?;
        let changes = {
            let mut stamp = || {
                let next = next_stamp(&device, own.as_ref(), None, Timestamp::now());
                own = Some(next.clone());
                next
            };
            make(&objects, &mut stamp)?
        };
        for change in &changes {
            overlay::apply(&mut objects, change).map_err(|e| input(e.to_string()))?;
        }
        let numbers = cache.push(&changes)?;
        if let Some(own) = own {
            cache.set_meta(
                "hlc",
                Some(&serde_json::to_string(&own).map_err(|e| input(e.to_string()))?),
            )?;
        }
        Ok(numbers)
    }

    /// `completed_at` or `dropped_at` now, and the end of a clock running
    /// on the task, as the server will (ADR 0046 §3).
    fn close_task(&self, id: &str, field: &str) -> Result<Vec<i64>, MobileError> {
        let id = parse_id(id)?;
        self.queue_numbered(|objects, stamp| {
            if !objects.tasks.iter().any(|t| t.id == id) {
                return Err(input(format!("no task {id}")));
            }
            Ok(clocking::close(objects, id, field, Timestamp::now(), stamp))
        })
    }

    fn set_field(&self, id: &str, field: &str, value: Value) -> Result<(), MobileError> {
        self.set_field_numbered(id, field, value).map(|_| ())
    }

    fn set_field_numbered(
        &self,
        id: &str,
        field: &str,
        value: Value,
    ) -> Result<Vec<i64>, MobileError> {
        let id = parse_id(id)?;
        self.queue_numbered(|objects, stamp| {
            if !objects.tasks.iter().any(|t| t.id == id) {
                return Err(input(format!("no task {id}")));
            }
            Ok(vec![Change::Set {
                object: id,
                field: field.into(),
                value,
                note: None,
                hlc: stamp(),
            }])
        })
    }

    /// `id` and every folder or tag below it, as strings.
    fn subtree(&self, id: Id) -> Result<HashSet<String>, MobileError> {
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let parents: Vec<(Id, Option<Id>)> = objects
            .folders
            .iter()
            .map(|f| (f.id, f.parent))
            .chain(objects.tags.iter().map(|t| (t.id, t.parent)))
            .collect();
        let mut below = HashSet::from([id]);
        let mut grew = true;
        while grew {
            grew = false;
            for (child, parent) in &parents {
                if parent.is_some_and(|p| below.contains(&p)) && below.insert(*child) {
                    grew = true;
                }
            }
        }
        Ok(below.into_iter().map(|id| id.to_string()).collect())
    }

    /// Fields of a project that is there, through the outbox.
    fn project_sets(&self, id: &str, fields: Vec<(&str, Value)>) -> Result<(), MobileError> {
        let id = parse_id(id)?;
        self.queue(|objects, stamp| {
            if !objects.projects.iter().any(|p| p.id == id) {
                return Err(input(format!("no project {id}")));
            }
            Ok(fields
                .into_iter()
                .map(|(field, value)| Change::Set {
                    object: id,
                    field: field.into(),
                    value,
                    note: None,
                    hlc: stamp(),
                })
                .collect())
        })
    }

    /// One change to a scope that is there, through the outbox.
    fn scope_change(
        &self,
        id: &str,
        make: impl FnOnce(Id, Hlc) -> Change,
    ) -> Result<(), MobileError> {
        let id = parse_id(id)?;
        self.queue(|objects, stamp| {
            if !objects.scopes.iter().any(|s| s.id == id) {
                return Err(input(format!("no scope {id}")));
            }
            Ok(vec![make(id, stamp())])
        })
    }

    /// Sections by project of the tasks `pick` takes, and only those with
    /// `tag` or a tag below it, when one is given.
    fn rows(
        &self,
        tag: Option<&str>,
        pick: impl Fn(&Task, &samu_core::availability::TaskState) -> bool,
    ) -> Result<Vec<Section>, MobileError> {
        let cache = self.cache()?;
        let (settings, _) = Self::settings(&cache)?;
        let (objects, touched) = Self::view(&cache)?;
        let now = Timestamp::now();
        let evaluation = evaluate(&objects.snapshot(), now, &settings);
        let names = Names::new(&objects);
        let wanted = tag_filter(tag, &objects.tags)?;
        let visible = Self::visible(&cache, &objects, &evaluation, now, &settings)?;
        let mut projects: Vec<Option<&Project>> = vec![None];
        let mut open: Vec<&Project> = objects
            .projects
            .iter()
            .filter(|p| matches!(p.status, ProjectStatus::Active | ProjectStatus::OnHold))
            .collect();
        open.sort_by_key(|p| (p.position, p.id));
        projects.extend(open.into_iter().map(Some));
        let mut sections = Vec::new();
        for project in projects {
            let pid = project.map(|p| p.id);
            let mut tasks: Vec<&Task> = visible
                .iter()
                .filter(|t| has_tag(t, wanted.as_ref()))
                .filter(|t| {
                    evaluation.tasks.get(&t.id).is_some_and(|s| {
                        // An orphan belongs to the inbox (ADR 0036 §6).
                        let home = if s.inbox { None } else { t.project };
                        home == pid && pick(t, s)
                    })
                })
                .collect();
            if tasks.is_empty() {
                continue;
            }
            tasks.sort_by_key(|t| (t.position, t.id));
            let rows = tree(&tasks)
                .into_iter()
                .map(|(task, depth)| {
                    row(
                        task,
                        depth,
                        &objects,
                        &evaluation,
                        &names,
                        &touched,
                        &settings.zone,
                    )
                })
                .collect();
            sections.push(Section {
                project_id: pid.map(|id| id.to_string()),
                title: project.map(|p| p.title.clone()),
                folder: project.and_then(|p| p.folder).map(|f| names.folder_path(f)),
                rows,
            });
        }
        Ok(sections)
    }

    /// The tasks `pick` takes as flat rows, with the task beside each, in
    /// the order of [`Self::rows`] but without the tree: what a list sorts
    /// and groups itself (ADR 0102 §4). Beside each its home project; none
    /// in the inbox, orphans among it (ADR 0036 §6).
    fn picked(
        &self,
        tag: Option<&str>,
        pick: impl Fn(&Task, &samu_core::availability::TaskState) -> bool,
    ) -> Result<Vec<(Task, Option<Id>, TaskRow)>, MobileError> {
        let cache = self.cache()?;
        let (settings, _) = Self::settings(&cache)?;
        let (objects, touched) = Self::view(&cache)?;
        let now = Timestamp::now();
        let evaluation = evaluate(&objects.snapshot(), now, &settings);
        let names = Names::new(&objects);
        let wanted = tag_filter(tag, &objects.tags)?;
        let visible = Self::visible(&cache, &objects, &evaluation, now, &settings)?;
        let place = |t: &Task| {
            let home = evaluation
                .tasks
                .get(&t.id)
                .and_then(|s| if s.inbox { None } else { t.project });
            home.and_then(|p| objects.projects.iter().find(|q| q.id == p))
                .map_or((false, 0, None), |p| (true, p.position, Some(p.id)))
        };
        let mut tasks: Vec<&Task> = visible
            .iter()
            .filter(|t| has_tag(t, wanted.as_ref()))
            .filter(|t| evaluation.tasks.get(&t.id).is_some_and(|s| pick(t, s)))
            .collect();
        tasks.sort_by_key(|t| (place(t), t.position, t.id));
        Ok(tasks
            .into_iter()
            .map(|task| {
                let row = row(
                    task,
                    0,
                    &objects,
                    &evaluation,
                    &names,
                    &touched,
                    &settings.zone,
                );
                (task.clone(), place(task).2, row)
            })
            .collect())
    }
}

fn named(title: &str) -> Result<String, MobileError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(input("a title, please"));
    }
    Ok(title.to_owned())
}

fn after_last(positions: impl Iterator<Item = i64>) -> i64 {
    positions.max().map_or(0, |p| p.saturating_add(1))
}

/// The German or the English text, as the interface is (ADR 0105): the
/// few words samu-mobile makes itself, beside the app's own table.
pub(crate) fn say(
    lang: samu_core::query::Lang,
    de: &'static str,
    en: &'static str,
) -> &'static str {
    match lang {
        samu_core::query::Lang::De => de,
        samu_core::query::Lang::En => en,
    }
}

/// `Einzelaufgaben · ruht`: what a project is beyond the plain.
fn project_meta(project: &Project, lang: samu_core::query::Lang) -> String {
    let mut parts = Vec::new();
    match project.kind {
        samu_api::ProjectKind::SingleActions => {
            parts.push(say(lang, "Einzelaufgaben", "single actions"))
        }
        samu_api::ProjectKind::Sequential => parts.push(say(lang, "sequenziell", "sequential")),
        _ => {}
    }
    match project.status {
        ProjectStatus::OnHold => parts.push(say(lang, "ruht", "on hold")),
        ProjectStatus::Completed => parts.push(say(lang, "erledigt", "completed")),
        ProjectStatus::Dropped => parts.push(say(lang, "verworfen", "dropped")),
        _ => {}
    }
    parts.join(" · ")
}

/// What the tab „Projekte“ draws its tree from (ADR 0085).
struct Tree<'a> {
    objects: &'a Objects,
    shown: &'a HashSet<Id>,
    folded: &'a HashSet<Id>,
    available: &'a dyn Fn(Id) -> u32,
    stuck: &'a dyn Fn(Id) -> bool,
    review_due: &'a dyn Fn(Id) -> bool,
}

impl Tree<'_> {
    fn folders(&self, parent: Option<Id>) -> Vec<&samu_api::Folder> {
        let mut here: Vec<_> = self
            .objects
            .folders
            .iter()
            .filter(|f| f.parent == parent && self.shown.contains(&f.id))
            .collect();
        here.sort_by_key(|f| (f.position, f.id));
        here
    }

    fn projects(&self, folder: Option<Id>) -> Vec<&Project> {
        let mut here: Vec<_> = self
            .objects
            .projects
            .iter()
            .filter(|p| p.folder == folder && self.shown.contains(&p.id))
            .collect();
        here.sort_by_key(|p| (p.position, p.id));
        here
    }

    /// The available tasks of the shown projects below a folder.
    fn total(&self, folder: Id, seen: &mut HashSet<Id>) -> u32 {
        if !seen.insert(folder) {
            return 0;
        }
        let own: u32 = self
            .projects(Some(folder))
            .iter()
            .map(|p| (self.available)(p.id))
            .sum();
        self.folders(Some(folder))
            .iter()
            .fold(own, |sum, f| sum.saturating_add(self.total(f.id, seen)))
    }

    fn walk(&self, parent: Option<Id>, depth: u32, seen: &mut HashSet<Id>, out: &mut Vec<TreeRow>) {
        let folders = self.folders(parent);
        let last = folders.len().saturating_sub(1);
        for (i, folder) in folders.iter().enumerate() {
            if !seen.insert(folder.id) {
                continue;
            }
            let folded = self.folded.contains(&folder.id);
            out.push(TreeRow {
                id: folder.id.to_string(),
                kind: "folder".into(),
                title: folder.title.clone(),
                icon: folder.icon.clone(),
                depth,
                available: self.total(folder.id, &mut HashSet::new()),
                stuck: false,
                review_due: false,
                on_hold: false,
                scope_only: folder.own_scope.is_some(),
                own_scope_title: scope_title(self.objects, folder.own_scope),
                folded,
                can_up: i > 0,
                can_down: i < last,
            });
            if !folded {
                self.walk(Some(folder.id), depth + 1, seen, out);
            }
        }
        let projects = self.projects(parent);
        let last = projects.len().saturating_sub(1);
        for (i, project) in projects.iter().enumerate() {
            out.push(TreeRow {
                id: project.id.to_string(),
                kind: "project".into(),
                title: project.title.clone(),
                icon: project.icon.clone(),
                depth,
                available: (self.available)(project.id),
                stuck: (self.stuck)(project.id),
                review_due: (self.review_due)(project.id),
                on_hold: project.status == ProjectStatus::OnHold,
                scope_only: project.own_scope.is_some(),
                own_scope_title: scope_title(self.objects, project.own_scope),
                folded: false,
                can_up: i > 0,
                can_down: i < last,
            });
        }
    }
}

/// Folders with their open projects as a tree, then the tags as a tree,
/// dropped ones left out: what scopes and the settings list (ADR 0035,
/// ADR 0036 §8).
fn place_tree(objects: &Objects, closed: bool) -> Vec<(Id, String, &'static str, u32)> {
    let mut tree = Vec::new();
    folders(objects, None, 0, closed, &mut HashSet::new(), &mut tree);
    tags(objects, None, 0, &mut HashSet::new(), &mut tree);
    tree
}

fn folders(
    objects: &Objects,
    parent: Option<Id>,
    depth: u32,
    closed: bool,
    seen: &mut HashSet<Id>,
    out: &mut Vec<(Id, String, &'static str, u32)>,
) {
    let mut here: Vec<_> = objects
        .folders
        .iter()
        .filter(|f| f.parent == parent)
        .collect();
    here.sort_by_key(|f| (f.position, f.id));
    for folder in here {
        if !seen.insert(folder.id) {
            continue;
        }
        out.push((folder.id, folder.title.clone(), "folder", depth));
        folders(objects, Some(folder.id), depth + 1, closed, seen, out);
    }
    let mut projects: Vec<_> = objects
        .projects
        .iter()
        .filter(|p| p.folder == parent)
        .filter(|p| closed || matches!(p.status, ProjectStatus::Active | ProjectStatus::OnHold))
        .collect();
    projects.sort_by_key(|p| (p.position, p.id));
    for project in projects {
        out.push((project.id, project.title.clone(), "project", depth));
    }
}
fn tags(
    objects: &Objects,
    parent: Option<Id>,
    depth: u32,
    seen: &mut HashSet<Id>,
    out: &mut Vec<(Id, String, &'static str, u32)>,
) {
    let mut here: Vec<_> = objects
        .tags
        .iter()
        .filter(|t| t.parent == parent && t.status != TagStatus::Dropped)
        .collect();
    here.sort_by_key(|t| (t.position, t.id));
    for tag in here {
        if !seen.insert(tag.id) {
            continue;
        }
        out.push((tag.id, tag.title.clone(), "tag", depth));
        tags(objects, Some(tag.id), depth + 1, seen, out);
    }
}

#[uniffi::export]
impl Samu {
    /// Opens (or makes) the database in `dir`, the app's private storage.
    #[uniffi::constructor]
    pub fn open(dir: String) -> Result<Arc<Self>, MobileError> {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).map_err(|e| MobileError::Store {
            reason: e.to_string(),
        })?;
        let cache = Cache::open(&dir.join("samu.db"))?;
        Ok(Arc::new(Self {
            cache: Mutex::new(cache),
            dir,
            lang: Mutex::new(samu_core::query::Lang::default()),
        }))
    }

    /// Signs in with account and password (ADR 0028 §8); keeps only the
    /// session token. Signing in to another server or account forgets the
    /// mirror of the old one; with changes in its outbox it is refused
    /// (ADR 0107 §6).
    pub fn login(
        &self,
        server: String,
        account: String,
        password: String,
        device: String,
    ) -> Result<Status, MobileError> {
        let server = server.trim().trim_end_matches('/').to_string();
        let device = device.trim().to_string();
        let client = Client::new(&server, None)?.with_timeout(TIMEOUT);
        let body = serde_json::to_value(Login {
            account: account.trim().to_string(),
            password,
            device: device.clone(),
            kind: None,
            only: None,
        })
        .map_err(|e| input(e.to_string()))?;
        let answer = client.call("POST", "/login", Some(&body))?;
        let session: SessionInfo =
            serde_json::from_value(answer.json()?).map_err(|e| MobileError::Refused {
                reason: e.to_string(),
            })?;
        self.signed_in(&server, &session)
    }

    /// Signs in with a pairing code from the web UI's QR code (ADR 0031):
    /// the server, account and device come with it. Otherwise as `login`.
    pub fn pair(&self, uri: String) -> Result<Status, MobileError> {
        let uri = PairingUri::parse(&uri).map_err(|e| input(e.to_string()))?;
        let client = Client::new(&uri.server, None)?.with_timeout(TIMEOUT);
        let body = serde_json::to_value(PairRequest { code: uri.code })
            .map_err(|e| input(e.to_string()))?;
        let answer = client.call("POST", "/pair", Some(&body))?;
        let session: SessionInfo =
            serde_json::from_value(answer.json()?).map_err(|e| MobileError::Refused {
                reason: e.to_string(),
            })?;
        self.signed_in(&uri.server, &session)
    }

    /// Forgets the session; mirror and outbox stay for the next sign-in to
    /// the same account.
    pub fn logout(&self) -> Result<(), MobileError> {
        if let Ok(client) = self.client() {
            let _ = client.call("POST", "/logout", None);
        }
        self.cache()?.set_meta("token", None)?;
        Ok(())
    }

    pub fn status(&self) -> Result<Status, MobileError> {
        let cache = self.cache()?;
        let (settings, zone_from_server) = Self::settings(&cache)?;
        Ok(Status {
            signed_in: cache.meta("token")?.is_some(),
            server: cache.meta("server")?,
            account: cache.meta("account")?,
            device: cache.meta("device")?,
            outbox: cache.outbox_len()?,
            last_sync: cache.meta("last_sync")?,
            last_error: cache.meta("last_error")?,
            zone: settings.zone.iana_name().unwrap_or("system").to_string(),
            zone_from_server,
        })
    }

    /// Push the outbox, then pull what changed (ADR 0028 §5). A refused
    /// change leaves the outbox and is remembered in `last_error`. Without a
    /// server nothing is lost.
    pub fn sync(&self) -> Result<SyncReport, MobileError> {
        let client = self.client()?;
        let blobs = self.blobs();
        let read = |hash: &str| std::fs::read(blobs.join(hash)).ok();
        let report = samu_replica::sync(&self.cache, &client, &read)?;
        Ok(SyncReport {
            pushed: report.pushed,
            rejected: report.rejected,
            pulled: report.pulled,
            reset: report.reset,
        })
    }

    /// Open tasks in the inbox, orphans among them (ADR 0036 §6). `tag` (an id) narrows every list to
    /// that tag and the tags below it.
    pub fn inbox(&self, tag: Option<String>) -> Result<Vec<Section>, MobileError> {
        self.listed(
            "inbox",
            tag.as_deref(),
            |t, s| s.inbox && is_open(t),
            |home| home.is_none(),
        )
    }

    /// Open and available tasks, by project (ADR 0020 §1).
    pub fn available(&self, tag: Option<String>) -> Result<Vec<Section>, MobileError> {
        self.listed(
            "available",
            tag.as_deref(),
            |t, s| is_open(t) && s.available,
            |_| true,
        )
    }

    /// Every open task of one project, available or not.
    pub fn project(&self, id: String, tag: Option<String>) -> Result<Vec<Section>, MobileError> {
        let id = parse_id(&id)?;
        self.listed(
            &format!("project:{id}"),
            tag.as_deref(),
            |t, _| t.project == Some(id) && is_open(t),
            |home| home == Some(id),
        )
    }

    /// Open tasks with every word of `text` in title or note, any case, as
    /// `samu list --search` finds them. No words, no hits.
    pub fn search(&self, text: String, tag: Option<String>) -> Result<Vec<Section>, MobileError> {
        if text.trim().is_empty() {
            return Ok(Vec::new());
        }
        // The search language of the web (ADR 0042 §2, a1f43aed): chips
        // from samu-core, the other words free text; what is closed only
        // when a chip asks for it (§4).
        // Read and let go of the lock: `rows` takes it again.
        let (settings, objects) = {
            let cache = self.cache()?;
            (Self::settings(&cache)?.0, Self::view(&cache)?.0)
        };
        let snapshot = objects.snapshot();
        let (chips, free) = samu_core::query::split_search(&text, &snapshot);
        let filter =
            samu_core::query::Filter::new(&chips, &snapshot, Timestamp::now(), &settings.zone);
        let closed = samu_core::query::asks_for_closed(&chips);
        self.listed(
            "search",
            tag.as_deref(),
            |t, s| {
                (closed || is_open(t))
                    && filter.matches(t, s)
                    && samu_core::query::text_matches(&free, t)
            },
            |_| true,
        )
    }

    /// Forecast as samu-core computes it (ADR 0045), with the account's
    /// settings as the phone has them (offline changes too), through the
    /// scope chosen here and, if given, only tasks with `tag` or below.
    /// Flat: a group's children stand on their own.
    pub fn forecast(&self, tag: Option<String>) -> Result<Vec<Day>, MobileError> {
        let cache = self.cache()?;
        let (settings, _) = Self::settings(&cache)?;
        let (objects, touched) = Self::view(&cache)?;
        let now = Timestamp::now();
        let evaluation = evaluate(&objects.snapshot(), now, &settings);
        let names = Names::new(&objects);
        let wanted = tag_filter(tag.as_deref(), &objects.tags)?;
        let today = now.to_zoned(settings.zone.clone()).date();
        let visible: HashSet<Id> = Self::visible(&cache, &objects, &evaluation, now, &settings)?
            .iter()
            .map(|t| t.id)
            .collect();
        let options = objects
            .settings
            .as_ref()
            .map(forecast::Options::from)
            .unwrap_or_default();
        let sections = forecast::forecast(
            &objects.snapshot(),
            &evaluation,
            &options,
            now,
            &settings.zone,
            |t, _| visible.contains(&t.id) && has_tag(t, wanted.as_ref()),
        );
        // The appointments (ADR 0047 §10), through the chosen scope's
        // calendars (§12), computed here as on the server.
        let horizon = forecast::horizon(today, &options);
        let instances = if options.events {
            let only = Self::chosen(&cache, &objects)?
                .map(|s| s.calendars.clone())
                .unwrap_or_default();
            samu_core::calendar::occurrences(
                &objects.appointments,
                &objects.calendars,
                &only,
                today,
                horizon,
                &settings.zone,
            )
        } else {
            Vec::new()
        };
        let parts =
            forecast::with_appointments(sections, &instances, today, horizon, &settings.zone);
        Ok(parts
            .into_iter()
            .map(|part| {
                let (section, rows) = (part.section, part.rows);
                let appointments = part
                    .appointments
                    .iter()
                    .map(|a| appointment_row(a, &objects, &settings.zone))
                    .collect();
                let (kind, date) = match section {
                    forecast::Section::Overdue => ("overdue", None),
                    forecast::Section::Day(d) => ("day", Some(d)),
                    forecast::Section::Flagged => ("flagged", None),
                    forecast::Section::Later => ("later", None),
                };
                Day {
                    appointments,
                    kind: kind.into(),
                    date: date.map(|d| d.to_string()),
                    today: date == Some(today),
                    rows: rows
                        .into_iter()
                        .map(|r| {
                            let mut out = row(
                                r.task,
                                0,
                                &objects,
                                &evaluation,
                                &names,
                                &touched,
                                &settings.zone,
                            );
                            out.why = Some(why_word(r.why).into());
                            out.why_date = r.date.map(|d| d.to_string());
                            out
                        })
                        .collect(),
                }
            })
            .collect())
    }

    /// The calendar view (ADR 0047 §11): the day `day` (today without one)
    /// or, with `week`, the week from Monday it is in. Appointments through
    /// the chosen scope's calendars, unfolded by samu-core as on the
    /// server; tasks of the scope planned at a time as blocks lasting
    /// their planned duration, else their estimate, else 30 minutes.
    /// Cancelled appointments and dropped tasks stand nowhere.
    pub fn calendar(&self, day: Option<String>, week: bool) -> Result<CalendarPage, MobileError> {
        use jiff::ToSpan;
        let cache = self.cache()?;
        let (settings, _) = Self::settings(&cache)?;
        let (objects, touched) = Self::view(&cache)?;
        let zone = settings.zone.clone();
        let now = Timestamp::now();
        let today = now.to_zoned(zone.clone()).date();
        let shown = match day {
            Some(text) => text
                .trim()
                .parse::<jiff::civil::Date>()
                .map_err(|e| input(format!("day: {e}")))?,
            None => today,
        };
        let first = if week {
            shown
                .checked_sub(i64::from(shown.weekday().to_monday_zero_offset()).days())
                .map_err(|e| input(e.to_string()))?
        } else {
            shown
        };
        let count: i64 = if week { 7 } else { 1 };
        let step = |n: i64| {
            first
                .checked_add(n.days())
                .map_err(|e| input(e.to_string()))
        };
        let last = step(count - 1)?;
        let evaluation = evaluate(&objects.snapshot(), now, &settings);
        let names = Names::new(&objects);
        let only = Self::chosen(&cache, &objects)?
            .map(|s| s.calendars.clone())
            .unwrap_or_default();
        let instances: Vec<_> = samu_core::calendar::occurrences(
            &objects.appointments,
            &objects.calendars,
            &only,
            first,
            last,
            &zone,
        )
        .into_iter()
        .filter(|a| a.status != samu_api::AppointmentStatus::Cancelled)
        .collect();
        let tasks = Self::visible(&cache, &objects, &evaluation, now, &settings)?;

        // Everything at a time as a span of instants, with what it is.
        let mut timed: Vec<(Timestamp, Timestamp, Shown)> = Vec::new();
        let mut whole: Vec<(jiff::civil::Date, jiff::civil::Date, AppointmentRow)> = Vec::new();
        for a in &instances {
            let row = appointment_row(a, &objects, &zone);
            match (&a.start, &a.end) {
                (When::Date(start), end) => {
                    // The end is excluded (ADR 0047 §4).
                    let end = match end {
                        Some(When::Date(e)) if e > start => e.yesterday().unwrap_or(*start),
                        _ => *start,
                    };
                    whole.push((*start, end, row));
                }
                (When::At(start), end) => {
                    let begin = start.timestamp();
                    let end = match end {
                        Some(When::At(e)) if e.timestamp() > begin => e.timestamp(),
                        _ => begin,
                    };
                    timed.push((begin, end, Shown::Appointment(row)));
                }
                _ => {}
            }
        }
        for task in tasks.iter().filter(|t| t.dropped_at.is_none()) {
            let Some(When::At(at)) = &task.schedule.planned else {
                continue;
            };
            let minutes = [&task.schedule.planned_duration, &task.estimate]
                .into_iter()
                .flatten()
                .find_map(period_minutes)
                .unwrap_or(TASK_MINUTES);
            let begin = at.timestamp();
            let end = begin
                .checked_add(jiff::SignedDuration::from_mins(minutes))
                .unwrap_or(begin);
            let row = row(
                task,
                0,
                &objects,
                &evaluation,
                &names,
                &touched,
                &settings.zone,
            );
            timed.push((begin, end, Shown::Task(row)));
        }

        let mut days = Vec::new();
        for n in 0..count {
            let date = step(n)?;
            let midnight = date
                .to_zoned(zone.clone())
                .map_err(|e| input(e.to_string()))?;
            let next = date
                .tomorrow()
                .and_then(|d| d.to_zoned(zone.clone()))
                .map_err(|e| input(e.to_string()))?;
            let (from, to) = (midnight.timestamp(), next.timestamp());
            let minute_of = |t: Timestamp| {
                let z = t.to_zoned(zone.clone());
                u32::try_from(z.hour()).unwrap_or(0) * 60 + u32::try_from(z.minute()).unwrap_or(0)
            };
            let mut parts = Vec::new();
            for (begin, end, what) in &timed {
                let zero = begin == end;
                if *begin >= to || (if zero { *begin < from } else { *end <= from }) {
                    continue;
                }
                let continued = *begin < from;
                let top = if continued { 0 } else { minute_of(*begin) };
                let bottom = if *end >= to {
                    DAY_MINUTES
                } else {
                    minute_of(*end)
                };
                let label = if !continued {
                    clock_words(*begin, &zone)
                } else if bottom >= DAY_MINUTES {
                    "ganzer Tag".to_string()
                } else {
                    format!("bis {}", clock_words(*end, &zone))
                };
                parts.push(Part {
                    top,
                    bottom: bottom.max(top + SHORTEST),
                    lane: 0,
                    lanes: 1,
                    label,
                    what: what.clone(),
                });
            }
            let blocks = lanes(parts)
                .into_iter()
                .map(|p| {
                    let (appointment, task) = match p.what {
                        Shown::Appointment(a) => (Some(a), None),
                        Shown::Task(t) => (None, Some(t)),
                    };
                    Block {
                        top: p.top,
                        bottom: p.bottom,
                        lane: p.lane,
                        lanes: p.lanes,
                        label: p.label,
                        appointment,
                        task,
                    }
                })
                .collect();
            days.push(CalendarDay {
                date: date.to_string(),
                today: date == today,
                start_ms: from.as_millisecond(),
                now: (date == today).then(|| minute_of(now)),
                all_day: whole
                    .iter()
                    .filter(|(s, e, _)| *s <= date && date <= *e)
                    .map(|(_, _, r)| r.clone())
                    .collect(),
                blocks,
            });
        }
        let stride = if week { 7 } else { 1 };
        Ok(CalendarPage {
            days,
            previous: step(-stride)?.to_string(),
            next: step(stride)?.to_string(),
            today: today.to_string(),
        })
    }

    /// What the widget shows (ADR 0028 §9; 08.11.2026): Forecast's
    /// overdue, today and flagged sections (ADR 0045), each task once and
    /// open; flagged ones always, whatever the setting says. Flat, no tag
    /// filter; through the scope chosen here (ADR 0035).
    pub fn today(&self) -> Result<Vec<TaskRow>, MobileError> {
        let cache = self.cache()?;
        let (settings, _) = Self::settings(&cache)?;
        let (objects, touched) = Self::view(&cache)?;
        let now = Timestamp::now();
        let evaluation = evaluate(&objects.snapshot(), now, &settings);
        let names = Names::new(&objects);
        let today = now.to_zoned(settings.zone.clone()).date();
        let visible: HashSet<Id> = Self::visible(&cache, &objects, &evaluation, now, &settings)?
            .iter()
            .map(|t| t.id)
            .collect();
        let mut options = objects
            .settings
            .as_ref()
            .map(forecast::Options::from)
            .unwrap_or_default();
        options.flagged = true;
        options.done = false;
        let sections = forecast::forecast(
            &objects.snapshot(),
            &evaluation,
            &options,
            now,
            &settings.zone,
            |t, _| visible.contains(&t.id),
        );
        let mut seen = HashSet::new();
        Ok(sections
            .into_iter()
            .filter(|(s, _)| {
                matches!(s, forecast::Section::Overdue | forecast::Section::Flagged)
                    || *s == forecast::Section::Day(today)
            })
            .flat_map(|(_, rows)| rows)
            .filter(|r| is_open(r.task) && seen.insert(r.task.id))
            .map(|r| {
                row(
                    r.task,
                    0,
                    &objects,
                    &evaluation,
                    &names,
                    &touched,
                    &settings.zone,
                )
            })
            .collect())
    }

    /// What the launcher shows around its clock: the inbox, and how far
    /// "today" is. `today_open` counts what [`Samu::today`] lists;
    /// `today_done` the tasks completed today, so that open and done make
    /// the day's whole. Through the chosen scope, like `today`.
    pub fn glance(&self) -> Result<Glance, MobileError> {
        let inbox: usize = self.inbox(None)?.iter().map(|s| s.rows.len()).sum();
        let cache = self.cache()?;
        let (settings, _) = Self::settings(&cache)?;
        let (objects, _) = Self::view(&cache)?;
        let now = Timestamp::now();
        let evaluation = evaluate(&objects.snapshot(), now, &settings);
        let today = now.to_zoned(settings.zone.clone()).date();
        let visible = Self::visible(&cache, &objects, &evaluation, now, &settings)?;
        let today_done = visible
            .iter()
            .filter(|t| {
                t.completed_at
                    .is_some_and(|at| at.to_zoned(settings.zone.clone()).date() == today)
            })
            .count();
        let ids: HashSet<Id> = visible.iter().map(|t| t.id).collect();
        let mut options = objects
            .settings
            .as_ref()
            .map(forecast::Options::from)
            .unwrap_or_default();
        options.flagged = true;
        options.done = false;
        let sections = forecast::forecast(
            &objects.snapshot(),
            &evaluation,
            &options,
            now,
            &settings.zone,
            |t, _| ids.contains(&t.id),
        );
        let mut seen = HashSet::new();
        let (mut overdue, mut today_open) = (0usize, 0usize);
        for (section, rows) in sections {
            let counted = matches!(
                section,
                forecast::Section::Overdue | forecast::Section::Flagged
            ) || section == forecast::Section::Day(today);
            if !counted {
                continue;
            }
            for row in rows {
                if is_open(row.task) && seen.insert(row.task.id) {
                    today_open += 1;
                    if section == forecast::Section::Overdue {
                        overdue += 1;
                    }
                }
            }
        }
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        Ok(Glance {
            inbox: count(inbox),
            overdue: count(overdue),
            today_open: count(today_open),
            today_done: count(today_done),
        })
    }

    /// The account's Forecast settings (ADR 0045 §4) as the phone has them;
    /// the defaults until the first sync brings the object.
    pub fn forecast_settings(&self) -> Result<ForecastSettings, MobileError> {
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let names = Names::new(&objects);
        let o = objects
            .settings
            .as_ref()
            .map(forecast::Options::from)
            .unwrap_or_default();
        Ok(ForecastSettings {
            known: objects.settings.is_some(),
            days: o.days,
            flagged: o.flagged,
            tag: o.tag.map(|t| t.to_string()),
            tag_path: o.tag.map(|t| names.tag_path(t)),
            defer: o.defer,
            done: o.done,
            events: o.events,
        })
    }

    /// Sets one Forecast setting through the outbox, offline too:
    /// `days` (a number), `flagged`, `defer`, `done` (`on`/`off`), `tag` (an
    /// id, or empty for none).
    pub fn set_forecast_setting(&self, key: String, value: String) -> Result<(), MobileError> {
        let (field, json) = match key.as_str() {
            "days" => {
                let days: u32 = value
                    .trim()
                    .parse()
                    .ok()
                    .filter(|d| (1..=366).contains(d))
                    .ok_or_else(|| input("days: a number from 1 to 366"))?;
                ("forecast_days", json!(days))
            }
            "flagged" | "defer" | "done" | "events" => {
                let on = match value.as_str() {
                    "on" | "true" => true,
                    "off" | "false" => false,
                    _ => return Err(input(format!("{key}: on or off"))),
                };
                (
                    match key.as_str() {
                        "flagged" => "forecast_flagged",
                        "defer" => "forecast_defer",
                        "events" => "forecast_events",
                        _ => "forecast_done",
                    },
                    json!(on),
                )
            }
            "tag" if value.trim().is_empty() => ("forecast_tag", Value::Null),
            "tag" => ("forecast_tag", json!(parse_id(value.trim())?)),
            _ => return Err(input(format!("no forecast setting {key:?}"))),
        };
        self.queue(|objects, stamp| {
            let Some(settings) = &objects.settings else {
                return Err(input("the settings come with the next sync"));
            };
            if field == "forecast_tag"
                && let Some(tag) = json.as_str()
                && !objects.tags.iter().any(|t| t.id.to_string() == tag)
            {
                return Err(input(format!("no tag {tag}")));
            }
            Ok(vec![Change::Set {
                object: settings.id,
                field: field.into(),
                value: json.clone(),
                note: None,
                hlc: stamp(),
            }])
        })
    }

    /// Tags that open tasks carry, by path, with how many each would show;
    /// the states of a workflow among them. Dropped tags are left out.
    pub fn tags(&self) -> Result<Vec<TagChip>, MobileError> {
        let cache = self.cache()?;
        let (settings, _) = Self::settings(&cache)?;
        let (objects, _) = Self::view(&cache)?;
        let names = Names::new(&objects);
        let now = Timestamp::now();
        let evaluation = evaluate(&objects.snapshot(), now, &settings);
        let visible = Self::visible(&cache, &objects, &evaluation, now, &settings)?;
        let open: Vec<&Task> = visible.iter().filter(|t| is_open(t)).collect();
        let mut chips: Vec<TagChip> = objects
            .tags
            .iter()
            .filter(|tag| tag.status != TagStatus::Dropped)
            .filter_map(|tag| {
                let below = tag_and_below(tag.id, &objects.tags);
                let count = open.iter().filter(|t| has_tag(t, Some(&below))).count();
                (count > 0).then(|| TagChip {
                    id: tag.id.to_string(),
                    path: names.tag_path(tag.id),
                    count: u32::try_from(count).unwrap_or(u32::MAX),
                })
            })
            .collect();
        chips.sort_by_key(|c| c.path.to_lowercase());
        Ok(chips)
    }

    /// The tab „Projekte“ as a tree (ADR 0085 §1): under each parent first
    /// the folders, then the projects, by position, as the chosen scope
    /// shows them; below a folded folder nothing, its sum on it.
    pub fn project_tree(&self) -> Result<Vec<TreeRow>, MobileError> {
        let cache = self.cache()?;
        let (settings, _) = Self::settings(&cache)?;
        let (objects, _) = Self::view(&cache)?;
        let shown = Self::shown_places(&cache, &objects)?;
        let folded = Self::folded(&cache)?;
        let evaluation = evaluate(&objects.snapshot(), Timestamp::now(), &settings);
        let available = |project: Id| {
            let n = objects
                .tasks
                .iter()
                .filter(|t| t.project == Some(project) && is_open(t))
                .filter(|t| evaluation.tasks.get(&t.id).is_some_and(|s| s.available))
                .count();
            u32::try_from(n).unwrap_or(u32::MAX)
        };
        let tree = Tree {
            objects: &objects,
            shown: &shown,
            folded: &folded,
            available: &available,
            stuck: &|project| evaluation.projects.get(&project).is_some_and(|s| s.stuck),
            review_due: &|project| {
                evaluation
                    .projects
                    .get(&project)
                    .is_some_and(|s| s.review_due)
            },
        };
        let mut rows = Vec::new();
        tree.walk(None, 0, &mut HashSet::new(), &mut rows);
        Ok(rows)
    }

    /// Folds or unfolds a folder in the tab „Projekte“, on this phone only
    /// (ADR 0085 §7).
    pub fn set_folded(&self, id: String, folded: bool) -> Result<(), MobileError> {
        let id = parse_id(&id)?;
        let cache = self.cache()?;
        let mut set = Self::folded(&cache)?;
        if folded {
            set.insert(id);
        } else {
            set.remove(&id);
        }
        let mut list: Vec<Id> = set.into_iter().collect();
        list.sort();
        let text = serde_json::to_string(&list).map_err(|e| input(e.to_string()))?;
        cache.set_meta("folded_places", (!list.is_empty()).then_some(text.as_str()))?;
        Ok(())
    }

    /// Moves a folder or project past its next shown sibling of the same
    /// kind, up or down, and counts all siblings anew in one write, those
    /// the scope hides too (ADR 0085 §4).
    pub fn step_place(&self, id: String, up: bool) -> Result<(), MobileError> {
        let id = parse_id(&id)?;
        let shown = {
            let cache = self.cache()?;
            let (objects, _) = Self::view(&cache)?;
            Self::shown_places(&cache, &objects)?
        };
        self.queue(|objects, stamp| {
            let mut siblings: Vec<(Id, i64)> =
                if let Some(project) = objects.projects.iter().find(|p| p.id == id) {
                    objects
                        .projects
                        .iter()
                        .filter(|p| p.folder == project.folder)
                        .map(|p| (p.id, p.position))
                        .collect()
                } else if let Some(folder) = objects.folders.iter().find(|f| f.id == id) {
                    objects
                        .folders
                        .iter()
                        .filter(|f| f.parent == folder.parent)
                        .map(|f| (f.id, f.position))
                        .collect()
                } else {
                    return Err(input(format!("no folder or project {id}")));
                };
            siblings.sort_by_key(|&(id, position)| (position, id));
            let at = siblings
                .iter()
                .position(|&(s, _)| s == id)
                .ok_or_else(|| input(format!("no folder or project {id}")))?;
            let passed = if up {
                (0..at).rev().find(|&i| shown.contains(&siblings[i].0))
            } else {
                (at + 1..siblings.len()).find(|&i| shown.contains(&siblings[i].0))
            };
            let Some(passed) = passed else {
                return Err(input(if up {
                    "already the first"
                } else {
                    "already the last"
                }));
            };
            let moved = siblings.remove(at);
            // Down, the one passed slid one up by the removal: inserting at
            // its old index puts the moved one right after it.
            siblings.insert(passed, moved);
            Ok(siblings
                .into_iter()
                .zip(0i64..)
                .filter(|&((_, old), new)| old != new)
                .map(|((object, _), new)| Change::Set {
                    object,
                    field: "position".into(),
                    value: json!(new),
                    note: None,
                    hlc: stamp(),
                })
                .collect())
        })
    }

    pub fn task(&self, id: String) -> Result<TaskDetail, MobileError> {
        let id = parse_id(&id)?;
        let cache = self.cache()?;
        let (settings, _) = Self::settings(&cache)?;
        let (objects, touched) = Self::view(&cache)?;
        let evaluation = evaluate(&objects.snapshot(), Timestamp::now(), &settings);
        let names = Names::new(&objects);
        let task = objects
            .tasks
            .iter()
            .find(|t| t.id == id)
            .ok_or_else(|| input(format!("no task {id}")))?;
        let reasons = evaluation
            .tasks
            .get(&id)
            .map(|s| {
                s.reasons
                    .iter()
                    .map(|r| reason_row(&r.into(), &names))
                    .collect()
            })
            .unwrap_or_default();
        Ok(TaskDetail {
            row: row(
                task,
                0,
                &objects,
                &evaluation,
                &names,
                &touched,
                &settings.zone,
            ),
            reasons,
            note: task.note.clone(),
            estimate: task.estimate.map(|p| p.0.to_string()),
            created_ms: task.created.as_millisecond(),
        })
    }

    /// The tasks `id` waits for (ae5a3206), open or done, in the order
    /// the links were made.
    pub fn blockers(&self, id: String) -> Result<Vec<BlockerRow>, MobileError> {
        let owner = parse_id(&id)?;
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let names = Names::new(&objects);
        let mut found = blockers::of(&objects.links, owner);
        found.sort_by_key(|(l, _)| (l.created, l.id));
        Ok(found
            .into_iter()
            .map(
                |(link, target)| match objects.tasks.iter().find(|t| t.id == target) {
                    Some(task) => blocker_row(task, Some(link.id), &objects, &names),
                    None => BlockerRow {
                        link_id: Some(link.id.to_string()),
                        task_id: target.to_string(),
                        title: link.uri.clone(),
                        place: None,
                        done: false,
                    },
                },
            )
            .collect())
    }

    /// The open tasks that wait for `id` (53a0d8e9, „Hält auf“), each with
    /// its `blocked_by` link, by title.
    pub fn held_up(&self, id: String) -> Result<Vec<BlockerRow>, MobileError> {
        let target = parse_id(&id)?;
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let names = Names::new(&objects);
        let mut rows: Vec<BlockerRow> = objects
            .links
            .iter()
            .filter(|l| l.kind == LinkKind::BlockedBy && Id::from_uri(&l.uri) == Some(target))
            .filter_map(|l| {
                objects
                    .tasks
                    .iter()
                    .find(|t| t.id == l.owner && t.completed_at.is_none() && t.dropped_at.is_none())
                    .map(|t| blocker_row(t, Some(l.id), &objects, &names))
            })
            .collect();
        rows.sort_by_key(|r| r.title.to_lowercase());
        Ok(rows)
    }

    /// The open tasks `id` may wait for, as samu-core decides (not
    /// itself, not twice, not in a circle), whose title holds every word
    /// of `query`; by title, at most 100.
    pub fn blocker_targets(
        &self,
        id: String,
        query: String,
    ) -> Result<Vec<BlockerRow>, MobileError> {
        let owner = parse_id(&id)?;
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let names = Names::new(&objects);
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let mut rows: Vec<BlockerRow> = objects
            .tasks
            .iter()
            .filter(|t| t.completed_at.is_none() && t.dropped_at.is_none())
            .filter(|t| {
                let title = t.title.to_lowercase();
                words.iter().all(|w| title.contains(w.as_str()))
            })
            .filter(|t| blockers::check(&objects.tasks, &objects.links, owner, t.id).is_ok())
            .map(|t| blocker_row(t, None, &objects, &names))
            .collect();
        rows.sort_by_key(|r| r.title.to_lowercase());
        rows.truncate(100);
        Ok(rows)
    }

    /// `id` waits until `target` is done: a `blocked_by` link through the
    /// outbox. Refused for itself, twice or in a circle.
    pub fn add_blocker(&self, id: String, target: String) -> Result<(), MobileError> {
        let owner = parse_id(&id)?;
        let target = parse_id(&target)?;
        self.queue(|objects, stamp| {
            blockers::check(&objects.tasks, &objects.links, owner, target)
                .map_err(|e| input(e.to_string()))?;
            let link = blockers::link(owner, target, Timestamp::now());
            Ok(vec![Change::Create {
                object: link.id,
                kind: ObjectKind::Link,
                value: serde_json::to_value(&link).map_err(|e| input(e.to_string()))?,
                hlc: stamp(),
            }])
        })
    }

    /// Takes a blocker away: its link is deleted.
    pub fn remove_blocker(&self, link_id: String) -> Result<(), MobileError> {
        let link = parse_id(&link_id)?;
        self.queue(|objects, stamp| {
            if !objects.links.iter().any(|l| l.id == link) {
                return Err(input(format!("no link {link}")));
            }
            Ok(vec![Change::Delete {
                object: link,
                hlc: stamp(),
            }])
        })
    }

    /// Complete now; a clock running on it stops (ADR 0046 §3).
    /// The numbers it got in the outbox, for `withdraw` (ADR 0104 §3).
    pub fn complete(&self, id: String) -> Result<Vec<i64>, MobileError> {
        self.close_task(&id, "completed_at")
    }

    /// Open again, from completed or dropped.
    pub fn reopen(&self, id: String) -> Result<(), MobileError> {
        let task_id = parse_id(&id)?;
        self.queue(|objects, stamp| {
            let task = objects
                .tasks
                .iter()
                .find(|t| t.id == task_id)
                .ok_or_else(|| input(format!("no task {task_id}")))?;
            let field = if task.dropped_at.is_some() {
                "dropped_at"
            } else {
                "completed_at"
            };
            Ok(vec![Change::Set {
                object: task_id,
                field: field.into(),
                value: Value::Null,
                note: None,
                hlc: stamp(),
            }])
        })
    }

    /// The numbers it got in the outbox, for `withdraw` (ADR 0104 §3).
    pub fn set_flag(&self, id: String, flagged: bool) -> Result<Vec<i64>, MobileError> {
        self.set_field_numbered(&id, "flagged", json!(flagged))
    }

    /// Takes changes made here back out of the outbox before they are
    /// sent, all or none (ADR 0104 §3). `false` when one has left or is on
    /// its way: then only a change against it undoes it.
    pub fn withdraw(&self, numbers: Vec<i64>) -> Result<bool, MobileError> {
        Ok(self.cache()?.withdraw(&numbers)?)
    }

    /// What a quick-capture line would make (ADR 0011), computed here and
    /// offline; nothing is written.
    pub fn preview(
        &self,
        text: String,
        project: Option<String>,
    ) -> Result<CapturePreview, MobileError> {
        let cache = self.cache()?;
        let (settings, _) = Self::settings(&cache)?;
        let (objects, _) = Self::view(&cache)?;
        let fallback = project.as_deref().map(parse_id).transpose()?;
        let now = Timestamp::now();
        let mut draft = parse(&text, now, &settings.zone);
        // Line, then template, then scope (ADR 0073 §4), as the server.
        let template = apply_template(&mut draft, &objects.templates, None, now, &settings.zone);
        let scope = Self::chosen(&cache, &objects)?;
        let resolved = resolve_in_scope(
            draft,
            Known {
                projects: &objects.projects,
                tags: &objects.tags,
                tasks: &objects.tasks,
            },
            fallback,
            scope,
            Timestamp::now(),
            &mut Id::new,
        );
        let names = Names::new(&objects);
        let task = &resolved.task;
        let marks = &resolved.draft.marks;
        let span = |mark: &samu_core::capture::Mark| Span {
            start: utf16_at(&text, mark.start),
            end: utf16_at(&text, mark.end),
        };
        let spans_of = |meaning: Meaning| -> Vec<Span> {
            marks
                .iter()
                .filter(|m| m.meaning == meaning)
                .map(span)
                .collect()
        };
        // The tag each `@name` mark means, through the names that resolved.
        let tag_spans = |tag: Id| -> Vec<Span> {
            marks
                .iter()
                .filter(|m| m.meaning == Meaning::Tag)
                .filter(|m| {
                    let name = text
                        .get(m.start..m.end)
                        .and_then(|word| word.strip_prefix('@'))
                        .map(unquote)
                        .unwrap_or_default();
                    resolved
                        .named
                        .iter()
                        .any(|(n, id)| *n == name && *id == tag)
                })
                .map(span)
                .collect()
        };
        let mut chips = Vec::new();
        let mut chip = |kind: &str, value: String, spans: Vec<Span>| {
            chips.push(Chip {
                kind: kind.into(),
                value,
                spans,
            })
        };
        if let Some(t) = template {
            chip("template", t.title.clone(), spans_of(Meaning::Template));
        }
        if let Some(p) = task.project {
            chip("project", names.project(p), spans_of(Meaning::Project));
        }
        let new: Vec<Id> = resolved.new_tags.iter().map(|t| t.id).collect();
        for tag in &resolved.tags {
            if !new.contains(tag) {
                chip("tag", names.tag_path(*tag), tag_spans(*tag));
            }
        }
        // A new tag by its whole path, its new parents included.
        let mut all_tags = objects.tags.clone();
        all_tags.extend(resolved.new_tags.iter().cloned());
        for tag in resolved.tags.iter().filter(|t| new.contains(t)) {
            chip("new_tag", tag_path_in(&all_tags, *tag), tag_spans(*tag));
        }
        if task.flagged {
            chip("flag", String::new(), spans_of(Meaning::Flag));
        }
        if let Some(priority) = task.priority.and_then(priority_text) {
            chip("priority", priority, spans_of(Meaning::Priority));
        }
        for (kind, when, meaning) in [
            ("defer", &task.schedule.defer, Meaning::Defer),
            ("due", &task.schedule.due, Meaning::Due),
            ("planned", &task.schedule.planned, Meaning::Planned),
        ] {
            if let Some(when) = when {
                chip(kind, when_text(when), spans_of(meaning));
            }
        }
        if let Some(estimate) = task.estimate {
            chip(
                "estimate",
                estimate.0.to_string(),
                spans_of(Meaning::Estimate),
            );
        }
        if let Some(repeat) = &task.schedule.repeat {
            chip("repeat", repeat.rule.clone(), spans_of(Meaning::Repeat));
        }
        if task.note.is_some() {
            chip("note", String::new(), spans_of(Meaning::Note));
        }
        Ok(CapturePreview {
            title: task.title.clone(),
            project: task.project.map(|p| names.project(p)),
            chips,
            warnings: resolved.draft.warnings.iter().map(warning_row).collect(),
        })
    }

    /// What to offer for the `#…` or `@…` word that the cursor (UTF-16)
    /// stands in, from the first sign on; none outside such a word, when
    /// nothing fits, or when exactly the one name is typed already.
    pub fn capture_offer(&self, text: String, cursor: u32) -> Result<Option<Offer>, MobileError> {
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let at = byte_at(&text, cursor);
        let before = &text[..at];
        let start = before
            .char_indices()
            .rev()
            .find(|(_, c)| c.is_whitespace())
            .map_or(0, |(i, c)| i + c.len_utf8());
        let word = &before[start..];
        let Some(sign) = word.chars().next().filter(|c| *c == '#' || *c == '@') else {
            return Ok(None);
        };
        if word[1..].starts_with('"') {
            return Ok(None);
        }
        let end = text[at..]
            .char_indices()
            .find(|(_, c)| c.is_whitespace())
            .map_or(text.len(), |(i, _)| at + i);
        let query = word[1..].to_lowercase();
        let names: Vec<String> = if sign == '#' {
            objects
                .projects
                .iter()
                .filter(|p| matches!(p.status, ProjectStatus::Active | ProjectStatus::OnHold))
                .map(|p| p.title.clone())
                .collect()
        } else {
            let names = Names::new(&objects);
            objects
                .tags
                .iter()
                .filter(|t| t.status != TagStatus::Dropped)
                .map(|t| names.tag_path(t.id))
                .collect()
        };
        let (mut starts, mut inside): (Vec<String>, Vec<String>) = names
            .into_iter()
            .filter(|n| n.to_lowercase().contains(&query))
            .partition(|n| n.to_lowercase().starts_with(&query));
        starts.sort();
        inside.sort();
        let items: Vec<String> = starts.into_iter().chain(inside).take(8).collect();
        if items.is_empty() || (items.len() == 1 && items[0].to_lowercase() == query) {
            return Ok(None);
        }
        Ok(Some(Offer {
            start: utf16_at(&text, start),
            end: utf16_at(&text, end),
            sign: sign.to_string(),
            items,
        }))
    }

    /// Makes a task from a quick-capture line, offline (ADR 0028 §7): new
    /// tags, the task, then each tag, as `POST /capture` would. Answers the
    /// new task's id.
    pub fn capture(&self, text: String, project: Option<String>) -> Result<String, MobileError> {
        if text.trim().is_empty() {
            return Err(input("nothing to capture"));
        }
        self.make(&text, project.as_deref(), None)
    }

    /// Makes a task as the last child of the open task `parent`, in its
    /// project, offline (ADR 0062 §2–3, ADR 0102 §7); a project the line
    /// names wins and the task goes there without a parent. The rule is
    /// `samu_core::views::capture`'s, as on the server. Answers the new
    /// task's id.
    pub fn capture_under(&self, text: String, parent: String) -> Result<String, MobileError> {
        if text.trim().is_empty() {
            return Err(input("nothing to capture"));
        }
        let parent = parse_id(&parent)?;
        let (settings, scope) = {
            let cache = self.cache()?;
            let (objects, _) = Self::view(&cache)?;
            let scope = Self::chosen(&cache, &objects)?.map(|s| s.id);
            (Self::settings(&cache)?.0, scope)
        };
        let request = samu_api::CaptureRequest {
            text,
            project: None,
            parent: Some(parent),
            scope,
            template: None,
            task: None,
            preview: false,
        };
        let mut made = None;
        self.queue(|objects, stamp| {
            let placeholder = stamp();
            let capture = samu_core::views::capture(
                objects,
                &settings,
                &request,
                Timestamp::now(),
                &placeholder,
                &mut Id::new,
            )
            .map_err(|e| input(e.to_string()))?;
            made = Some(capture.answer.task.id);
            Ok(capture
                .changes
                .into_iter()
                .map(|change| samu_core::clock::restamp(change, stamp()))
                .collect())
        })?;
        made.map(|id| id.to_string())
            .ok_or_else(|| input("nothing captured"))
    }

    /// The account's capture templates (ADR 0073), by name, for the
    /// buttons in the capture sheet.
    pub fn templates(&self) -> Result<Vec<TemplateRow>, MobileError> {
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let mut rows: Vec<TemplateRow> = objects
            .templates
            .iter()
            .map(|t| TemplateRow {
                id: t.id.to_string(),
                title: t.title.clone(),
            })
            .collect();
        rows.sort_by_key(|t| t.title.to_lowercase());
        Ok(rows)
    }

    /// Takes a shared text apart (ADR 0013 §2); nothing is written.
    pub fn share_draft(&self, subject: Option<String>, text: String) -> SharedDraft {
        let s = shared(subject.as_deref(), &text);
        SharedDraft {
            title: s.title,
            note: s.note,
            links: s.links,
        }
    }

    /// Makes a task from a shared text, offline: `title` as it stands, then
    /// what `extra` says as quick capture (project, tags, dates, flag), its
    /// other words after the title; the typed note before the shared one;
    /// each address a link of kind `ref` (ADR 0013).
    pub fn capture_shared(
        &self,
        title: String,
        extra: String,
        note: Option<String>,
        links: Vec<String>,
        project: Option<String>,
    ) -> Result<String, MobileError> {
        if title.trim().is_empty() {
            return Err(input("a shared task needs a title"));
        }
        let shared = Shared {
            title: title.trim().to_string(),
            note: note.filter(|n| !n.trim().is_empty()),
            links,
        };
        self.make(&extra, project.as_deref(), Some(&shared))
    }

    /// Files a recording from the ring that Tasker handed over, offline
    /// (ADR 0032): the task the hook would make, from `samu_core`, with
    /// `via=tasker`; `recorded_at_ms` is the time of the recording in Unix
    /// milliseconds, else now. Signed out it refuses, so the app can show
    /// the text instead of losing it.
    pub fn capture_ring(
        &self,
        text: String,
        recorded_at_ms: Option<i64>,
    ) -> Result<String, MobileError> {
        let zone = {
            let cache = self.cache()?;
            if cache.meta("token")?.is_none() {
                return Err(MobileError::SignedOut {
                    reason: "not signed in: the recording was not filed".into(),
                });
            }
            Self::settings(&cache)?.0.zone
        };
        let now = Timestamp::now();
        let recorded = match recorded_at_ms {
            Some(ms) => Timestamp::from_millisecond(ms)
                .map_err(|e| input(format!("recorded_at {ms}: {e}")))?,
            None => now,
        };
        let task = ring_task(
            Id::new(),
            &text,
            recorded,
            now,
            &zone,
            Some(RING_VIA_TASKER),
        );
        let id = task.id;
        let value = serde_json::to_value(&task).map_err(|e| input(e.to_string()))?;
        self.queue(|_, stamp| {
            Ok(vec![Change::Create {
                object: id,
                kind: ObjectKind::Task,
                value,
                hlc: stamp(),
            }])
        })?;
        Ok(id.to_string())
    }

    /// Every scope in order, with the one chosen here marked (ADR 0035).
    pub fn scopes(&self) -> Result<Vec<ScopeRow>, MobileError> {
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let chosen = Self::chosen(&cache, &objects)?.map(|s| s.id);
        let mut scopes: Vec<&Scope> = objects.scopes.iter().collect();
        scopes.sort_by_key(|s| (s.position, s.id));
        Ok(scopes
            .into_iter()
            .map(|s| ScopeRow {
                id: s.id.to_string(),
                title: s.title.clone(),
                only: s.mode == ScopeMode::Only,
                chosen: chosen == Some(s.id),
                inbox: s.inbox,
                color: scope_color(s.color),
            })
            .collect())
    }

    /// The scope chosen on this phone, if it is still there; one that is
    /// gone counts as none (ADR 0035 §4).
    pub fn selected_scope(&self) -> Result<Option<ScopeRow>, MobileError> {
        Ok(self.scopes()?.into_iter().find(|s| s.chosen))
    }

    /// Chooses a scope on this phone, or none. Never synced.
    pub fn select_scope(&self, id: Option<String>) -> Result<(), MobileError> {
        if let Some(id) = &id {
            parse_id(id)?;
        }
        let cache = self.cache()?;
        if cache.meta("scope_auto")?.as_deref() == Some("1") {
            // Following the schedule, a choice by hand is an event that
            // holds until the next edge (ADR 0043 §7); "none" is one too.
            cache.set_meta("scope_hand", Some(id.as_deref().unwrap_or("none")))?;
            cache.set_meta("scope_hand_at", Some(&Timestamp::now().to_string()))?;
        } else {
            cache.set_meta("scope", id.as_deref())?;
        }
        Ok(())
    }

    /// Whether this phone follows the scopes' schedule (ADR 0043 §7).
    /// Details as a second line (ADR 0053), this phone only.
    pub fn details(&self) -> Result<bool, MobileError> {
        Ok(self.cache()?.meta("details")?.as_deref() == Some("1"))
    }

    pub fn set_details(&self, on: bool) -> Result<(), MobileError> {
        self.cache()?.set_meta("details", on.then_some("1"))?;
        Ok(())
    }

    pub fn scope_auto(&self) -> Result<bool, MobileError> {
        Ok(self.cache()?.meta("scope_auto")?.as_deref() == Some("1"))
    }

    /// Follow the schedule, or stop; either way a choice by hand is
    /// forgotten.
    pub fn set_scope_auto(&self, on: bool) -> Result<(), MobileError> {
        let cache = self.cache()?;
        cache.set_meta("scope_auto", on.then_some("1"))?;
        cache.set_meta("scope_hand", None)?;
        cache.set_meta("scope_hand_at", None)?;
        Ok(())
    }

    /// What Tasker asks for (ADR 0043 §8): a scope by title or id, empty
    /// for none, `auto` to follow the schedule. Answers what holds now, as
    /// a word for a message; an unknown scope changes nothing.
    pub fn choose_scope_by_word(&self, word: String) -> Result<String, MobileError> {
        let word = word.trim();
        if word.eq_ignore_ascii_case("auto") {
            self.set_scope_auto(true)?;
            return Ok(self.selected_scope()?.map_or_else(
                || "kein Scope (Zeitplan)".into(),
                |s| format!("{} (Zeitplan)", s.title),
            ));
        }
        if word.is_empty() {
            self.select_scope(None)?;
            return Ok("kein Scope".into());
        }
        let scopes = self.scopes()?;
        let found = scopes
            .iter()
            .find(|s| s.id == word || s.title.eq_ignore_ascii_case(word))
            .ok_or_else(|| input(format!("no scope {word:?}")))?;
        self.select_scope(Some(found.id.clone()))?;
        Ok(found.title.clone())
    }

    /// Open tasks outside the chosen scope that are overdue or due soon:
    /// the hint in the header (ADR 0035 §6); without a scope, what is
    /// scope_only and due (ADR 0069 §7).
    pub fn outside_due(&self) -> Result<u32, MobileError> {
        let cache = self.cache()?;
        let (settings, _) = Self::settings(&cache)?;
        let (objects, _) = Self::view(&cache)?;
        let snapshot = objects.snapshot();
        let now = Timestamp::now();
        let lens = Lens::new(
            Self::chosen(&cache, &objects)?,
            &snapshot,
            now,
            &settings.zone,
        );
        let evaluation = evaluate(&snapshot, now, &settings);
        let count = objects
            .tasks
            .iter()
            .filter(|t| {
                evaluation
                    .tasks
                    .get(&t.id)
                    .is_some_and(|state| lens.due_outside(t, state))
            })
            .count();
        Ok(u32::try_from(count).unwrap_or(u32::MAX))
    }

    /// What a scope can name, with its members ticked: folders and their
    /// open projects as a tree, then the tags as a tree, dropped ones left
    /// out.
    pub fn scope_members(&self, scope: String) -> Result<Vec<MemberRow>, MobileError> {
        let id = parse_id(&scope)?;
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let scope = objects
            .scopes
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| input(format!("no scope {id}")))?;
        let member = |id: Id| scope.members.contains(&id);
        let mut rows = Vec::new();
        let row = |id: Id, title: &str, kind: &str, depth: u32| MemberRow {
            id: id.to_string(),
            title: title.to_owned(),
            kind: kind.into(),
            depth,
            member: member(id),
            scope_only: is_scope_only(&objects, id),
            own_scope_title: own_scope_title(&objects, id),
        };
        for (id, title, kind, depth) in place_tree(&objects, false) {
            rows.push(row(id, &title, kind, depth));
        }
        Ok(rows)
    }

    /// Makes a scope through the outbox and answers its id.
    pub fn create_scope(&self, title: String, only: bool) -> Result<String, MobileError> {
        let title = title.trim().to_owned();
        if title.is_empty() {
            return Err(input("a scope needs a name"));
        }
        let id = Id::new();
        self.queue(|objects, stamp| {
            let now = Timestamp::now().to_string();
            let value = json!({
                "id": id,
                "created": now,
                "modified": now,
                "title": title,
                "mode": if only { "only" } else { "except" },
                "position": objects.scopes.len(),
            });
            Ok(vec![Change::Create {
                object: id,
                kind: ObjectKind::Scope,
                value,
                hlc: stamp(),
            }])
        })?;
        Ok(id.to_string())
    }

    pub fn rename_scope(&self, id: String, title: String) -> Result<(), MobileError> {
        let title = title.trim().to_owned();
        if title.is_empty() {
            return Err(input("a scope needs a name"));
        }
        self.scope_change(&id, |scope, hlc| Change::Set {
            object: scope,
            field: "title".into(),
            value: json!(title),
            note: None,
            hlc,
        })
    }

    /// A scope's colour from the palette, or none (ADR 0060).
    pub fn set_scope_color(&self, id: String, color: Option<String>) -> Result<(), MobileError> {
        let value = match color.as_deref() {
            None => Value::Null,
            Some(c) if SCOPE_COLORS.contains(&c) => json!(c),
            Some(other) => return Err(input(format!("no colour {other:?}"))),
        };
        self.scope_change(&id, |scope, hlc| Change::Set {
            object: scope,
            field: "color".into(),
            value,
            note: None,
            hlc,
        })
    }

    /// A scope's inspector layout (ADR 0072 §2); none = everything shown.
    pub fn scope_layout(&self, id: String) -> Result<Option<InspectorLayoutRow>, MobileError> {
        let id = parse_id(&id)?;
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let scope = objects
            .scopes
            .iter()
            .find(|s| s.id == id)
            .ok_or_else(|| input(format!("no scope {id}")))?;
        Ok(scope
            .layout
            .as_ref()
            .and_then(|l| l.inspector.as_ref())
            .map(|i| InspectorLayoutRow {
                shown: i
                    .shown
                    .iter()
                    .filter_map(|&p| part_key(p))
                    .map(Into::into)
                    .collect(),
                note_rendered: i.note == Some(NoteMode::Rendered),
            }))
    }

    /// Sets the inspector layout of a scope as a whole (ADR 0072 §1, §5);
    /// none shows everything again. The shown parts keep the inspector's
    /// order.
    pub fn set_scope_layout(
        &self,
        id: String,
        layout: Option<InspectorLayoutRow>,
    ) -> Result<(), MobileError> {
        let value = match layout {
            None => Value::Null,
            Some(layout) => {
                if let Some(bad) = layout
                    .shown
                    .iter()
                    .find(|k| !INSPECTOR_PARTS.contains(&k.as_str()))
                {
                    return Err(input(format!("no part of the inspector {bad:?}")));
                }
                let shown: Vec<&str> = INSPECTOR_PARTS
                    .iter()
                    .copied()
                    .filter(|k| layout.shown.iter().any(|s| s == k))
                    .collect();
                let note = if layout.note_rendered {
                    "rendered"
                } else {
                    "default"
                };
                json!({"inspector": {"shown": shown, "note": note}})
            }
        };
        self.scope_change(&id, |scope, hlc| Change::Set {
            object: scope,
            field: "layout".into(),
            value,
            note: None,
            hlc,
        })
    }

    /// How the task page looks in the scope chosen here (ADR 0072 §2–§4);
    /// none without a scope or without a layout: everything shown.
    pub fn task_layout(&self, id: String) -> Result<Option<TaskLayout>, MobileError> {
        let detail = self.task(id.clone())?;
        let task_id = parse_id(&id)?;
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let Some(scope) = Self::chosen(&cache, &objects)? else {
            return Ok(None);
        };
        let Some(inspector) = scope.layout.as_ref().and_then(|l| l.inspector.as_ref()) else {
            return Ok(None);
        };
        let note_rendered = inspector.note == Some(NoteMode::Rendered);
        let unfolded = cache
            .meta(&format!("inspector_open.{}", scope.id))?
            .as_deref()
            == Some("1");
        if unfolded {
            return Ok(Some(TaskLayout {
                shown: None,
                valued: Vec::new(),
                rest: 0,
                note_rendered,
                unfolded,
            }));
        }
        let shown: Vec<&str> = inspector
            .shown
            .iter()
            .filter_map(|&p| part_key(p))
            .collect();
        let task = objects.tasks.iter().find(|t| t.id == task_id);
        let row = &detail.row;
        // Whether a part has a value, as `PARTS` in web/lib/layout.js.
        let has = |key: &str| match key {
            "properties" => task.is_some_and(|t| !t.properties.is_empty()),
            "availability" => !row.available,
            "state" => row.state.is_some(),
            "project" => row.project_id.is_some(),
            "priority" => row.priority.is_some(),
            "defer" => row.defer.is_some(),
            "due" => row.due.is_some(),
            "planned" => row.planned.is_some(),
            "repeat" => task.is_some_and(|t| t.schedule.repeat.is_some()),
            "series" => task.is_some_and(|t| t.habit.is_some()),
            "estimate" => row.estimate.is_some(),
            "tags" => !row.tags.is_empty(),
            "note" => row.has_note,
            "links" => objects.links.iter().any(|l| l.owner == task_id),
            "reminders" => objects.reminders.iter().any(|r| r.owner == task_id),
            "clock" => objects.clocks.iter().any(|c| c.owner == task_id),
            "comments" => objects.comments.iter().any(|c| c.owner == task_id),
            "attachments" => objects.attachments.iter().any(|a| a.owner == task_id),
            _ => false,
        };
        let folded: Vec<&str> = INSPECTOR_PARTS
            .iter()
            .copied()
            .filter(|k| !shown.contains(k))
            .collect();
        let valued: Vec<String> = folded
            .iter()
            .copied()
            .filter(|k| has(k))
            .map(Into::into)
            .collect();
        let rest = u32::try_from(folded.len() - valued.len()).unwrap_or(u32::MAX);
        Ok(Some(TaskLayout {
            shown: Some(shown.into_iter().map(Into::into).collect()),
            valued,
            rest,
            note_rendered,
            unfolded,
        }))
    }

    /// Unfolds or folds the task page in the scope chosen here; kept on
    /// this phone only (ADR 0072 §4). Without a scope nothing is folded.
    pub fn set_inspector_unfolded(&self, on: bool) -> Result<(), MobileError> {
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        if let Some(scope) = Self::chosen(&cache, &objects)? {
            cache.set_meta(&format!("inspector_open.{}", scope.id), on.then_some("1"))?;
        }
        Ok(())
    }

    pub fn set_scope_mode(&self, id: String, only: bool) -> Result<(), MobileError> {
        self.scope_change(&id, |scope, hlc| Change::Set {
            object: scope,
            field: "mode".into(),
            value: json!(if only { "only" } else { "except" }),
            note: None,
            hlc,
        })
    }

    /// A scope's filter in the search language (ADR 0043 §2), as one text
    /// with names; empty when it has none.
    pub fn scope_filter(&self, id: String) -> Result<String, MobileError> {
        let wanted = parse_id(&id)?;
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let scope = objects
            .scopes
            .iter()
            .find(|s| s.id == wanted)
            .ok_or_else(|| input(format!("no scope {wanted}")))?;
        Ok(samu_core::query::words_in(
            &scope.filter,
            &objects.snapshot(),
            self.ui_lang(),
        ))
    }

    /// Sets a scope's filter from a text in the search language; an empty
    /// text clears it. A word that is no chip is an input error and
    /// changes nothing.
    pub fn set_scope_filter(&self, id: String, text: String) -> Result<(), MobileError> {
        // The cache is let go before the change takes it again.
        let chips = {
            let cache = self.cache()?;
            let (objects, _) = Self::view(&cache)?;
            samu_core::query::parse(&text, &objects.snapshot()).map_err(|e| input(e.to_string()))?
        };
        let value = serde_json::to_value(&chips).map_err(|e| input(e.to_string()))?;
        self.scope_change(&id, |scope, hlc| Change::Set {
            object: scope,
            field: "filter".into(),
            value,
            note: None,
            hlc,
        })
    }

    /// A scope's schedule as text, `mo,di 08:00-17:00; sa 22:00-02:00`
    /// (ADR 0043 §7).
    pub fn scope_schedule(&self, id: String) -> Result<String, MobileError> {
        let wanted = parse_id(&id)?;
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let scope = objects
            .scopes
            .iter()
            .find(|s| s.id == wanted)
            .ok_or_else(|| input(format!("no scope {wanted}")))?;
        Ok(samu_core::scope::windows_text(&scope.schedule))
    }

    /// Sets a scope's schedule from text; an empty text clears it.
    pub fn set_scope_schedule(&self, id: String, text: String) -> Result<(), MobileError> {
        let windows = samu_core::scope::parse_windows(&text).map_err(|e| input(e.to_string()))?;
        let value = json!(windows);
        self.scope_change(&id, |scope, hlc| Change::Set {
            object: scope,
            field: "schedule".into(),
            value,
            note: None,
            hlc,
        })
    }

    /// What a capture in this scope gets (ADR 0043 §6).
    pub fn scope_capture(&self, id: String) -> Result<ScopeCaptureRow, MobileError> {
        let wanted = parse_id(&id)?;
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let scope = objects
            .scopes
            .iter()
            .find(|s| s.id == wanted)
            .ok_or_else(|| input(format!("no scope {wanted}")))?;
        let capture = scope.capture.clone().unwrap_or_default();
        Ok(ScopeCaptureRow {
            project: capture.project.map(|p| p.to_string()),
            tags: capture.tags.iter().map(ToString::to_string).collect(),
        })
    }

    /// Sets what a capture in this scope gets; no project and no tags
    /// clears it.
    pub fn set_scope_capture(
        &self,
        id: String,
        project: Option<String>,
        tags: Vec<String>,
    ) -> Result<(), MobileError> {
        let project = project.as_deref().map(parse_id).transpose()?;
        let tags = tags
            .iter()
            .map(|t| parse_id(t))
            .collect::<Result<Vec<_>, _>>()?;
        let value = if project.is_none() && tags.is_empty() {
            Value::Null
        } else {
            json!(samu_api::ScopeCapture { project, tags })
        };
        self.scope_change(&id, |scope, hlc| Change::Set {
            object: scope,
            field: "capture".into(),
            value,
            note: None,
            hlc,
        })
    }

    /// Ticks the inbox as a member of a scope, or unticks it (ADR 0043 §1).
    pub fn set_scope_inbox(&self, id: String, on: bool) -> Result<(), MobileError> {
        self.scope_change(&id, |scope, hlc| Change::Set {
            object: scope,
            field: "inbox".into(),
            value: json!(on),
            note: None,
            hlc,
        })
    }

    /// Ticks a folder, project or tag as a member of a scope, or unticks it.
    pub fn set_scope_member(
        &self,
        id: String,
        member: String,
        on: bool,
    ) -> Result<(), MobileError> {
        let element = parse_id(&member)?;
        self.scope_change(&id, |scope, hlc| {
            if on {
                Change::Add {
                    object: scope,
                    field: "members".into(),
                    element,
                    position: 0,
                    note: None,
                    hlc,
                }
            } else {
                Change::Remove {
                    object: scope,
                    field: "members".into(),
                    element,
                    note: None,
                    hlc,
                }
            }
        })
    }

    /// Folders and their open projects as a tree, then the tags (ADR 0036
    /// §8).
    pub fn places(&self, closed: bool) -> Result<Vec<PlaceRow>, MobileError> {
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        Ok(place_tree(&objects, closed)
            .into_iter()
            .map(|(id, title, kind, depth)| {
                let project = objects.projects.iter().find(|p| p.id == id);
                PlaceRow {
                    id: id.to_string(),
                    title,
                    kind: kind.into(),
                    depth,
                    meta: project
                        .map(|p| project_meta(p, self.ui_lang()))
                        .unwrap_or_default(),
                    closed: project.is_some_and(|p| {
                        matches!(p.status, ProjectStatus::Completed | ProjectStatus::Dropped)
                    }),
                    scope_only: is_scope_only(&objects, id),
                    own_scope_title: own_scope_title(&objects, id),
                }
            })
            .collect())
    }

    /// One folder, project or tag in full (ADR 0037 §6).
    pub fn place_detail(&self, id: String) -> Result<PlaceDetail, MobileError> {
        let id = parse_id(&id)?;
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let folder_title = |f: Option<Id>| {
            f.and_then(|f| objects.folders.iter().find(|x| x.id == f))
                .map(|f| f.title.clone())
        };
        let mut detail = PlaceDetail {
            id: id.to_string(),
            title: String::new(),
            kind: String::new(),
            place: None,
            place_title: None,
            project_kind: None,
            status: None,
            workflow: None,
            review: None,
            open_tasks: 0,
            icon: None,
            scope_only: is_scope_only(&objects, id),
            own_scope: own_scope(&objects, id).map(|s| s.to_string()),
            own_scope_title: own_scope_title(&objects, id),
        };
        if let Some(p) = objects.projects.iter().find(|p| p.id == id) {
            let wire = |v: Value| v.as_str().map(str::to_owned);
            detail.title = p.title.clone();
            detail.icon = p.icon.clone();
            detail.kind = "project".into();
            detail.place = p.folder.map(|f| f.to_string());
            detail.place_title = folder_title(p.folder);
            detail.project_kind = serde_json::to_value(p.kind).ok().and_then(wire);
            detail.status = serde_json::to_value(p.status).ok().and_then(wire);
            detail.workflow = p.workflow.map(|w| w.to_string());
            detail.review = p
                .review_interval
                .and_then(|r| serde_json::to_value(r).ok())
                .and_then(wire);
            let open = objects
                .tasks
                .iter()
                .filter(|t| t.project == Some(id) && is_open(t))
                .count();
            detail.open_tasks = u32::try_from(open).unwrap_or(u32::MAX);
        } else if let Some(f) = objects.folders.iter().find(|f| f.id == id) {
            detail.title = f.title.clone();
            detail.icon = f.icon.clone();
            detail.kind = "folder".into();
            detail.place = f.parent.map(|p| p.to_string());
            detail.place_title = folder_title(f.parent);
        } else if let Some(t) = objects.tags.iter().find(|t| t.id == id) {
            detail.title = t.title.clone();
            detail.kind = "tag".into();
            detail.place = t.parent.map(|p| p.to_string());
            detail.place_title = t
                .parent
                .and_then(|p| objects.tags.iter().find(|x| x.id == p))
                .map(|t| t.title.clone());
        } else {
            return Err(input(format!("no folder, project or tag {id}")));
        }
        Ok(detail)
    }

    /// Where an object may go (ADR 0037 §1/§2): folders for a project or a
    /// folder, tags for a tag, never the object itself or what is below it.
    pub fn place_targets(&self, id: String) -> Result<Vec<PlaceRow>, MobileError> {
        let id = parse_id(&id)?;
        let tag = self
            .cache()
            .and_then(|cache| Ok(Self::view(&cache)?.0))?
            .tags
            .iter()
            .any(|t| t.id == id);
        let below = self.subtree(id)?;
        Ok(self
            .places(false)?
            .into_iter()
            .filter(|p| {
                if tag {
                    p.kind == "tag"
                } else {
                    p.kind == "folder"
                }
            })
            .filter(|p| !below.contains(&p.id))
            .collect())
    }

    /// The exclusive tag groups a project can follow (ADR 0024 §3).
    pub fn workflows(&self) -> Result<Vec<PlaceRow>, MobileError> {
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let mut groups: Vec<&Tag> = objects.tags.iter().filter(|t| t.exclusive).collect();
        groups.sort_by_key(|t| (t.position, t.id));
        Ok(groups
            .into_iter()
            .map(|t| PlaceRow {
                id: t.id.to_string(),
                title: t.title.clone(),
                kind: "tag".into(),
                depth: 0,
                meta: String::new(),
                closed: false,
                scope_only: t.own_scope.is_some(),
                own_scope_title: scope_title(&objects, t.own_scope),
            })
            .collect())
    }

    /// Moves a folder, project or tag into `inside` (a folder, or a tag for
    /// a tag), or to the top, last there, through the outbox (ADR 0037 §1).
    /// Never below itself.
    pub fn move_place(&self, id: String, inside: Option<String>) -> Result<(), MobileError> {
        let id = parse_id(&id)?;
        let inside = inside.as_deref().map(parse_id).transpose()?;
        if let Some(inside) = inside
            && self.subtree(id)?.contains(&inside.to_string())
        {
            return Err(input("not below itself"));
        }
        self.queue(|objects, stamp| {
            let (field, position) = if objects.projects.iter().any(|p| p.id == id) {
                if inside.is_some_and(|f| !objects.folders.iter().any(|x| x.id == f)) {
                    return Err(input("a project goes into a folder"));
                }
                let siblings = objects
                    .projects
                    .iter()
                    .filter(|p| p.folder == inside && p.id != id);
                ("folder", after_last(siblings.map(|p| p.position)))
            } else if objects.folders.iter().any(|f| f.id == id) {
                if inside.is_some_and(|f| !objects.folders.iter().any(|x| x.id == f)) {
                    return Err(input("a folder goes into a folder"));
                }
                let siblings = objects
                    .folders
                    .iter()
                    .filter(|f| f.parent == inside && f.id != id);
                ("parent", after_last(siblings.map(|f| f.position)))
            } else if objects.tags.iter().any(|t| t.id == id) {
                if inside.is_some_and(|t| !objects.tags.iter().any(|x| x.id == t)) {
                    return Err(input("a tag goes under a tag"));
                }
                let siblings = objects
                    .tags
                    .iter()
                    .filter(|t| t.parent == inside && t.id != id);
                ("parent", after_last(siblings.map(|t| t.position)))
            } else {
                return Err(input(format!("no folder, project or tag {id}")));
            };
            Ok(vec![
                Change::Set {
                    object: id,
                    field: field.into(),
                    value: inside.map_or(Value::Null, |i| json!(i)),
                    note: None,
                    hlc: stamp(),
                },
                Change::Set {
                    object: id,
                    field: "position".into(),
                    value: json!(position),
                    note: None,
                    hlc: stamp(),
                },
            ])
        })
    }

    /// `parallel`, `sequential` or `single_actions` (ADR 0037 §3).
    pub fn set_project_kind(&self, id: String, kind: String) -> Result<(), MobileError> {
        if !matches!(kind.as_str(), "parallel" | "sequential" | "single_actions") {
            return Err(input(format!("no kind {kind:?}")));
        }
        self.project_sets(&id, vec![("kind", json!(kind))])
    }

    /// `active`, `on_hold`, `completed` or `dropped`; done and dropped carry
    /// their moment, back to open clears both (ADR 0037 §3).
    pub fn set_project_status(&self, id: String, status: String) -> Result<(), MobileError> {
        let now = json!(Timestamp::now().to_string());
        let (completed, dropped) = match status.as_str() {
            "completed" => (now, Value::Null),
            "dropped" => (Value::Null, now),
            "active" | "on_hold" => (Value::Null, Value::Null),
            other => return Err(input(format!("no status {other:?}"))),
        };
        self.project_sets(
            &id,
            vec![
                ("status", json!(status)),
                ("completed_at", completed),
                ("dropped_at", dropped),
            ],
        )
    }

    /// An exclusive tag group, or none for the default (ADR 0024 §3).
    pub fn set_project_workflow(
        &self,
        id: String,
        workflow: Option<String>,
    ) -> Result<(), MobileError> {
        let workflow = workflow.as_deref().map(parse_id).transpose()?;
        self.project_sets(
            &id,
            vec![("workflow", workflow.map_or(Value::Null, |w| json!(w)))],
        )
    }

    /// `P1W`, `P2W`, `P1M`, `P3M`, or none.
    pub fn set_project_review(
        &self,
        id: String,
        review: Option<String>,
    ) -> Result<(), MobileError> {
        if review
            .as_deref()
            .is_some_and(|r| !matches!(r, "P1W" | "P2W" | "P1M" | "P3M"))
        {
            return Err(input(format!("no review interval {review:?}")));
        }
        self.project_sets(
            &id,
            vec![("review_interval", review.map_or(Value::Null, |r| json!(r)))],
        )
    }

    /// Makes a project through the outbox, last at its level; answers its
    /// id.
    pub fn create_project(
        &self,
        title: String,
        folder: Option<String>,
    ) -> Result<String, MobileError> {
        let title = named(&title)?;
        let folder = folder.as_deref().map(parse_id).transpose()?;
        let id = Id::new();
        self.queue(|objects, stamp| {
            if let Some(folder) = folder
                && !objects.folders.iter().any(|f| f.id == folder)
            {
                return Err(input(format!("no folder {folder}")));
            }
            let now = Timestamp::now().to_string();
            let mut value = json!({
                "id": id,
                "created": now,
                "modified": now,
                "title": title,
                "kind": "parallel",
                "status": "active",
                "flagged": false,
                "position": after_last(
                    objects.projects.iter().filter(|p| p.folder == folder).map(|p| p.position)
                ),
            });
            if let Some(folder) = folder {
                value["folder"] = json!(folder);
            }
            Ok(vec![Change::Create {
                object: id,
                kind: ObjectKind::Project,
                value,
                hlc: stamp(),
            }])
        })?;
        Ok(id.to_string())
    }

    /// Makes a folder through the outbox, last at its level.
    pub fn create_folder(
        &self,
        title: String,
        parent: Option<String>,
    ) -> Result<String, MobileError> {
        let title = named(&title)?;
        let parent = parent.as_deref().map(parse_id).transpose()?;
        let id = Id::new();
        self.queue(|objects, stamp| {
            if let Some(parent) = parent
                && !objects.folders.iter().any(|f| f.id == parent)
            {
                return Err(input(format!("no folder {parent}")));
            }
            let now = Timestamp::now().to_string();
            let mut value = json!({
                "id": id,
                "created": now,
                "modified": now,
                "title": title,
                "status": "active",
                "position": after_last(
                    objects.folders.iter().filter(|f| f.parent == parent).map(|f| f.position)
                ),
            });
            if let Some(parent) = parent {
                value["parent"] = json!(parent);
            }
            Ok(vec![Change::Create {
                object: id,
                kind: ObjectKind::Folder,
                value,
                hlc: stamp(),
            }])
        })?;
        Ok(id.to_string())
    }

    /// Makes a tag through the outbox, last at its level.
    pub fn create_tag(&self, title: String, parent: Option<String>) -> Result<String, MobileError> {
        let title = named(&title)?;
        samu_core::capture::check_tag_title(&title).map_err(input)?;
        let parent = parent.as_deref().map(parse_id).transpose()?;
        let id = Id::new();
        self.queue(|objects, stamp| {
            if let Some(parent) = parent
                && !objects.tags.iter().any(|t| t.id == parent)
            {
                return Err(input(format!("no tag {parent}")));
            }
            let now = Timestamp::now().to_string();
            let mut value = json!({
                "id": id,
                "created": now,
                "modified": now,
                "title": title,
                "status": "active",
                "exclusive": false,
                "ask_note": false,
                "default_workflow": false,
                "position": after_last(
                    objects.tags.iter().filter(|t| t.parent == parent).map(|t| t.position)
                ),
            });
            if let Some(parent) = parent {
                value["parent"] = json!(parent);
            }
            Ok(vec![Change::Create {
                object: id,
                kind: ObjectKind::Tag,
                value,
                hlc: stamp(),
            }])
        })?;
        Ok(id.to_string())
    }

    /// The tag a tag picker means by `path` (`Büro` or `Ort/Büro`), made
    /// through the outbox with whatever step is missing; an existing one
    /// is only answered. Steps match by whole title, ignoring case, as
    /// `samu-core` decides; a workflow or its state is refused.
    pub fn create_tag_path(&self, path: String) -> Result<String, MobileError> {
        let mut found = None;
        self.queue(|objects, stamp| {
            let (id, made) = samu_core::capture::tag_at_path(
                &path,
                &objects.tags,
                Timestamp::now(),
                &mut Id::new,
            )
            .map_err(input)?;
            found = Some(id);
            made.into_iter()
                .map(|tag| {
                    let value = serde_json::to_value(&tag).map_err(|e| input(e.to_string()))?;
                    Ok(Change::Create {
                        object: tag.id,
                        kind: ObjectKind::Tag,
                        value,
                        hlc: stamp(),
                    })
                })
                .collect()
        })?;
        found
            .map(|id| id.to_string())
            .ok_or_else(|| input("no tag"))
    }

    /// Renames a folder, project or tag through the outbox.
    /// A folder's or project's emoji (ADR 0059); none or blank takes it
    /// off. More than 32 bytes is an input error, as on the server.
    pub fn set_place_icon(&self, id: String, icon: Option<String>) -> Result<(), MobileError> {
        let icon = icon.map(|i| i.trim().to_owned()).filter(|i| !i.is_empty());
        if icon.as_ref().is_some_and(|i| i.len() > 32) {
            return Err(input("an icon is an emoji, at most 32 bytes"));
        }
        let id = parse_id(&id)?;
        self.queue(|objects, stamp| {
            let known = objects.folders.iter().any(|f| f.id == id)
                || objects.projects.iter().any(|p| p.id == id);
            if !known {
                return Err(input(format!("no folder or project {id}")));
            }
            Ok(vec![Change::Set {
                object: id,
                field: "icon".into(),
                value: icon.map_or(Value::Null, |i| json!(i)),
                note: None,
                hlc: stamp(),
            }])
        })
    }

    /// „Nur im Scope:“ (ADR 0094 §1): shown only in `scope`, or with none
    /// everywhere, through the outbox.
    pub fn set_own_scope(&self, id: String, scope: Option<String>) -> Result<(), MobileError> {
        let id = parse_id(&id)?;
        let scope = scope.as_deref().map(parse_id).transpose()?;
        self.queue(|objects, stamp| {
            let known = objects.folders.iter().any(|f| f.id == id)
                || objects.projects.iter().any(|p| p.id == id)
                || objects.tags.iter().any(|t| t.id == id);
            if !known {
                return Err(input(format!("no folder, project or tag {id}")));
            }
            if let Some(scope) = scope
                && !objects.scopes.iter().any(|s| s.id == scope)
            {
                return Err(input(format!("no scope {scope}")));
            }
            Ok(vec![Change::Set {
                object: id,
                field: "own_scope".into(),
                value: scope.map_or(Value::Null, |s| json!(s.to_string())),
                note: None,
                hlc: stamp(),
            }])
        })
    }

    /// „Geprüft“ (ADR 0088 §3, §7): `last_reviewed` now and `next_review`
    /// cleared, through the outbox, also offline.
    pub fn mark_reviewed(&self, id: String) -> Result<(), MobileError> {
        let id = parse_id(&id)?;
        self.queue(|objects, stamp| {
            if !objects.projects.iter().any(|p| p.id == id) {
                return Err(input(format!("no project {id}")));
            }
            Ok(vec![
                Change::Set {
                    object: id,
                    field: "last_reviewed".into(),
                    value: json!(Timestamp::now().to_string()),
                    note: None,
                    hlc: stamp(),
                },
                Change::Set {
                    object: id,
                    field: "next_review".into(),
                    value: Value::Null,
                    note: None,
                    hlc: stamp(),
                },
            ])
        })
    }

    pub fn rename_place(&self, id: String, title: String) -> Result<(), MobileError> {
        let title = named(&title)?;
        let id = parse_id(&id)?;
        self.queue(|objects, stamp| {
            let known = objects.folders.iter().any(|f| f.id == id)
                || objects.projects.iter().any(|p| p.id == id)
                || objects.tags.iter().any(|t| t.id == id);
            if !known {
                return Err(input(format!("no folder, project or tag {id}")));
            }
            if objects.tags.iter().any(|t| t.id == id) {
                samu_core::capture::check_tag_title(&title).map_err(input)?;
            }
            Ok(vec![Change::Set {
                object: id,
                field: "title".into(),
                value: json!(title),
                note: None,
                hlc: stamp(),
            }])
        })
    }

    /// Deletes a folder, project or tag on the server, which composes it
    /// (ADR 0036 §1, §7): online only, never through the outbox. For a
    /// project, `tasks` is `inbox`, `delete` or `project` with `target`.
    /// With `preview`, the counts and nothing written. Otherwise the outbox
    /// goes first and the mirror follows after.
    pub fn delete_place(
        &self,
        id: String,
        tasks: Option<String>,
        target: Option<String>,
        preview: bool,
    ) -> Result<DeleteCount, MobileError> {
        let id = parse_id(&id)?;
        let tasks = match tasks.as_deref() {
            None => None,
            Some("inbox") => Some(TaskFate::Inbox),
            Some("delete") => Some(TaskFate::Delete),
            Some("project") => {
                let target = target.ok_or_else(|| input("which project?"))?;
                Some(TaskFate::Project(parse_id(&target)?))
            }
            Some(other) => return Err(input(format!("no choice {other:?} for the tasks"))),
        };
        if !preview {
            self.sync()?;
        }
        let body = serde_json::to_value(DeleteRequest { tasks, preview })
            .map_err(|e| input(e.to_string()))?;
        let client = self.client()?;
        let answer = self.signed_out_on_401(client.call(
            "POST",
            &format!("/objects/{id}/delete"),
            Some(&body),
        ))?;
        let done: DeleteResponse =
            serde_json::from_value(answer.json()?).map_err(|e| MobileError::Refused {
                reason: e.to_string(),
            })?;
        if !preview {
            self.sync()?;
        }
        Ok(DeleteCount {
            deleted: done.deleted,
            moved: done.moved,
            untagged: done.untagged,
        })
    }

    /// Deletes a scope; chosen here, it is chosen no more.
    pub fn delete_scope(&self, id: String) -> Result<(), MobileError> {
        self.scope_change(&id, |scope, hlc| Change::Delete { object: scope, hlc })?;
        let cache = self.cache()?;
        if cache.meta("scope")?.as_deref() == Some(id.as_str()) {
            cache.set_meta("scope", None)?;
        }
        Ok(())
    }

    /// The calendars by position (ADR 0047 §13), as the phone has them.
    pub fn calendars(&self) -> Result<Vec<CalendarRow>, MobileError> {
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let mut all: Vec<&Calendar> = objects.calendars.iter().collect();
        all.sort_by(|a, b| a.position.cmp(&b.position).then(a.title.cmp(&b.title)));
        Ok(all
            .into_iter()
            .map(|c| CalendarRow {
                id: c.id.to_string(),
                title: c.title.clone(),
                kind: match c.kind {
                    CalendarKind::Ics => "ics",
                    CalendarKind::Caldav => "caldav",
                    CalendarKind::Unknown => "?",
                }
                .into(),
                color: c.color.clone(),
                enabled: c.enabled,
                host: c.host.clone(),
                error: c.error.clone(),
                fetched_at: c.fetched_at.map(|t| t.to_string()),
                appointments: objects
                    .appointments
                    .iter()
                    .filter(|a| a.calendar == c.id)
                    .count() as u32,
            })
            .collect())
    }

    /// Sets one field of a calendar through the outbox, offline too:
    /// `title`, `color` (`#rrggbb`, or empty for none), `enabled`
    /// (`on`/`off`). The rest belongs to the web settings and the CLI.
    pub fn set_calendar(&self, id: String, key: String, value: String) -> Result<(), MobileError> {
        let id = parse_id(&id)?;
        let json = match key.as_str() {
            "title" if value.trim().is_empty() => return Err(input("a calendar needs a name")),
            "title" => json!(value.trim()),
            "color" if value.trim().is_empty() => Value::Null,
            "color" => {
                let c = value.trim();
                let hex = c.len() == 7
                    && c.starts_with('#')
                    && c[1..].chars().all(|ch| ch.is_ascii_hexdigit());
                if !hex {
                    return Err(input(format!("color: #rrggbb, not {c:?}")));
                }
                json!(c.to_ascii_lowercase())
            }
            "enabled" => json!(match value.as_str() {
                "on" | "true" => true,
                "off" | "false" => false,
                _ => return Err(input("enabled: on or off")),
            }),
            _ => return Err(input(format!("no calendar field {key:?}"))),
        };
        self.queue(|objects, stamp| {
            if !objects.calendars.iter().any(|c| c.id == id) {
                return Err(input(format!("no calendar {id}")));
            }
            Ok(vec![Change::Set {
                object: id,
                field: key.clone(),
                value: json.clone(),
                note: None,
                hlc: stamp(),
            }])
        })
    }

    /// Fetches a calendar now (`POST /calendars/{id}/refresh`, ADR 0047 §7),
    /// online only: the outbox goes first, the new appointments follow with
    /// a sync. What went wrong comes back; none when it went well.
    pub fn refresh_calendar(&self, id: String) -> Result<Option<String>, MobileError> {
        let id = parse_id(&id)?;
        self.sync()?;
        let client = self.client()?;
        let answer =
            self.signed_out_on_401(client.call("POST", &format!("/calendars/{id}/refresh"), None))?;
        let calendar: Calendar =
            serde_json::from_value(answer.json()?).map_err(|e| MobileError::Refused {
                reason: e.to_string(),
            })?;
        self.sync()?;
        Ok(calendar.error)
    }

    pub fn outbox_count(&self) -> Result<u32, MobileError> {
        Ok(self.cache()?.outbox_len()?)
    }
}

/// The shared part of a task made from a shared text.
struct Shared {
    title: String,
    note: Option<String>,
    links: Vec<String>,
}

impl Samu {
    /// Queues what a capture makes: new tags, the task, each tag, and for a
    /// shared text its links.
    fn make(
        &self,
        text: &str,
        project: Option<&str>,
        shared: Option<&Shared>,
    ) -> Result<String, MobileError> {
        let fallback = project.map(parse_id).transpose()?;
        let (zone, chosen) = {
            let cache = self.cache()?;
            let (objects, _) = Self::view(&cache)?;
            let chosen = Self::chosen(&cache, &objects)?.map(|s| s.id.to_string());
            (Self::settings(&cache)?.0.zone, chosen)
        };
        let mut made = None;
        self.queue(|objects, stamp| {
            let now = Timestamp::now();
            // The scope chosen here fills in what the line names not
            // (ADR 0043 §6).
            let scope = chosen
                .as_deref()
                .and_then(|id| objects.scopes.iter().find(|s| s.id.to_string() == id));
            // Line, then template, then scope (ADR 0073 §4), as the server.
            let mut draft = parse(text, now, &zone);
            let template = apply_template(&mut draft, &objects.templates, None, now, &zone);
            let mut resolved = resolve_in_scope(
                draft,
                Known {
                    projects: &objects.projects,
                    tags: &objects.tags,
                    tasks: &objects.tasks,
                },
                fallback,
                scope,
                now,
                &mut Id::new,
            );
            if let Some(shared) = shared {
                let task = &mut resolved.task;
                let rest = task.title.trim();
                task.title = if rest.is_empty() {
                    shared.title.clone()
                } else {
                    format!("{} {rest}", shared.title)
                };
                let notes: Vec<&str> = [task.note.as_deref(), shared.note.as_deref()]
                    .into_iter()
                    .flatten()
                    .collect();
                task.note = (!notes.is_empty()).then(|| notes.join("\n\n"));
            }
            let value = |v: serde_json::Result<Value>| v.map_err(|e| input(e.to_string()));
            let mut changes = Vec::new();
            for tag in &resolved.new_tags {
                changes.push(Change::Create {
                    object: tag.id,
                    kind: ObjectKind::Tag,
                    value: value(serde_json::to_value(tag))?,
                    hlc: stamp(),
                });
            }
            changes.push(Change::Create {
                object: resolved.task.id,
                kind: ObjectKind::Task,
                value: value(serde_json::to_value(&resolved.task))?,
                hlc: stamp(),
            });
            for (&tag, position) in resolved.tags.iter().zip(0..) {
                changes.push(Change::Add {
                    object: resolved.task.id,
                    field: "tags".into(),
                    element: tag,
                    position,
                    note: None,
                    hlc: stamp(),
                });
            }
            // The template's reminders come with the task (ADR 0073 §1).
            for reminder in template_reminders(template, resolved.task.id, now, &mut Id::new) {
                changes.push(Change::Create {
                    object: reminder.id,
                    kind: ObjectKind::Reminder,
                    value: value(serde_json::to_value(&reminder))?,
                    hlc: stamp(),
                });
            }
            for uri in shared.map(|s| s.links.as_slice()).unwrap_or_default() {
                let link = Link {
                    id: Id::new(),
                    created: now,
                    modified: now,
                    owner: resolved.task.id,
                    uri: uri.clone(),
                    label: None,
                    kind: LinkKind::Ref,
                };
                changes.push(Change::Create {
                    object: link.id,
                    kind: ObjectKind::Link,
                    value: value(serde_json::to_value(&link))?,
                    hlc: stamp(),
                });
            }
            // `z:` and `x:` as on the server (ADR 0081 §2, §4): the clocks,
            // then the completion.
            let (clocks, _) = place_clocks(
                &resolved.draft.clocks,
                &objects.clocks,
                resolved.task.id,
                &zone,
                now,
                &mut Id::new,
            );
            for clock in &clocks {
                changes.push(Change::Create {
                    object: clock.id,
                    kind: ObjectKind::Clock,
                    value: value(serde_json::to_value(clock))?,
                    hlc: stamp(),
                });
            }
            if let Some(day) = resolved.draft.completed {
                let all: Vec<_> = objects.clocks.iter().chain(&clocks).cloned().collect();
                changes.push(Change::Set {
                    object: resolved.task.id,
                    field: "completed_at".into(),
                    value: value(serde_json::to_value(completed_at(day, &all, &zone, now)))?,
                    note: None,
                    hlc: stamp(),
                });
            }
            made = Some(resolved.task.id);
            Ok(changes)
        })?;
        made.map(|id| id.to_string())
            .ok_or_else(|| input("nothing was made"))
    }

    /// A 401 forgets the token, so the app shows the sign-in; the outbox and
    /// the mirror stay.
    fn signed_out_on_401(
        &self,
        answer: samu_client::Result<samu_client::Answer>,
    ) -> Result<samu_client::Answer, MobileError> {
        match answer {
            Err(samu_client::Error::Unauthorized(reason)) => {
                self.cache()?.set_meta("token", None)?;
                Err(MobileError::SignedOut { reason })
            }
            Err(other) => {
                let error = MobileError::from(other);
                if let MobileError::Offline { .. } = error {
                } else if let Ok(cache) = self.cache() {
                    let _ = cache.set_meta("last_error", Some(&error.to_string()));
                }
                Err(error)
            }
            Ok(answer) => Ok(answer),
        }
    }
}

fn is_open(task: &Task) -> bool {
    task.completed_at.is_none() && task.dropped_at.is_none()
}

/// `id` and every tag below it; the loop ends because the set only grows.
fn tag_and_below(id: Id, tags: &[Tag]) -> HashSet<Id> {
    let mut out = HashSet::from([id]);
    loop {
        let before = out.len();
        for tag in tags {
            if tag.parent.is_some_and(|p| out.contains(&p)) {
                out.insert(tag.id);
            }
        }
        if out.len() == before {
            return out;
        }
    }
}

/// The tags a filter by `tag` (an id) lets through; none means no filter.
/// An occurrence as Forecast shows it on the phone.
pub(crate) fn appointment_row(
    instance: &samu_api::AppointmentInstance,
    objects: &Objects,
    zone: &TimeZone,
) -> AppointmentRow {
    let time = |when: Option<&When>| match when {
        Some(When::At(at)) => Some(
            at.with_time_zone(zone.clone())
                .strftime("%H:%M")
                .to_string(),
        ),
        _ => None,
    };
    // Whole days as UTC midnight, as Android's calendar provider keeps
    // them; times as their instant.
    let millis = |when: Option<&When>| match when {
        Some(When::At(at)) => Some(at.timestamp().as_millisecond()),
        Some(When::Date(d)) => d
            .to_zoned(TimeZone::UTC)
            .ok()
            .map(|z| z.timestamp().as_millisecond()),
        _ => None,
    };
    let date = match &instance.start {
        When::At(at) => at.with_time_zone(zone.clone()).date(),
        When::Date(d) => *d,
        When::Unknown => jiff::civil::Date::default(),
    };
    let begin_ms = millis(Some(&instance.start)).unwrap_or_default();
    let end_ms = millis(instance.end.as_ref())
        .filter(|end| *end >= begin_ms)
        .unwrap_or(match instance.start {
            When::Date(_) => begin_ms + 86_400_000,
            _ => begin_ms,
        });
    let calendar = objects.calendars.iter().find(|c| c.id == instance.calendar);
    let appointment = objects
        .appointments
        .iter()
        .find(|a| a.id == instance.appointment);
    AppointmentRow {
        id: instance.appointment.to_string(),
        title: instance.title.clone(),
        all_day: matches!(instance.start, When::Date(_)),
        start: time(Some(&instance.start)),
        end: time(instance.end.as_ref()),
        location: instance.location.clone(),
        note: instance.note.clone(),
        link: instance.link.clone(),
        color: calendar.and_then(|c| c.color.clone()),
        calendar: calendar.map(|c| c.title.clone()).unwrap_or_default(),
        tentative: instance.status == samu_api::AppointmentStatus::Tentative,
        uid: appointment.map(|a| a.uid.clone()).unwrap_or_default(),
        date: date.to_string(),
        begin_ms,
        end_ms,
        alarms: appointment
            .map(|a| a.alarms.iter().filter_map(remind::alarm_row).collect())
            .unwrap_or_default(),
    }
}

/// Minutes of a day.
const DAY_MINUTES: u32 = 24 * 60;
/// A block is at least this long, so a short one stays readable and
/// counts as overlapping what it covers on screen (as the web).
const SHORTEST: u32 = 25;
/// What a task planned at a time lasts without a planned duration or an
/// estimate (ADR 0047 §11).
const TASK_MINUTES: i64 = 30;

/// What a block of the calendar view shows.
#[derive(Debug, Clone)]
#[allow(
    clippy::large_enum_variant,
    reason = "a few blocks per day, built once and laid out"
)]
enum Shown {
    Appointment(AppointmentRow),
    Task(TaskRow),
}

/// A block on its day before it is laid out.
#[derive(Debug, Clone)]
struct Part {
    top: u32,
    bottom: u32,
    lane: u32,
    lanes: u32,
    label: String,
    what: Shown,
}

/// Side by side where blocks overlap, as the web does it: each gets a
/// lane, and every block of a run of overlapping ones shares the run's
/// number of lanes. Sorted by top, the longer first.
fn lanes(mut parts: Vec<Part>) -> Vec<Part> {
    parts.sort_by(|a, b| a.top.cmp(&b.top).then(b.bottom.cmp(&a.bottom)));
    let mut run_start = 0;
    let mut run_end = 0;
    let mut ends: Vec<u32> = Vec::new();
    for i in 0..parts.len() {
        if i > run_start && parts[i].top >= run_end {
            let n = u32::try_from(ends.len()).unwrap_or(1);
            parts[run_start..i].iter_mut().for_each(|p| p.lanes = n);
            run_start = i;
            ends.clear();
        }
        let top = parts[i].top;
        let lane = match ends.iter().position(|end| *end <= top) {
            Some(lane) => {
                ends[lane] = parts[i].bottom;
                lane
            }
            None => {
                ends.push(parts[i].bottom);
                ends.len() - 1
            }
        };
        parts[i].lane = u32::try_from(lane).unwrap_or(0);
        run_end = if i == run_start {
            parts[i].bottom
        } else {
            run_end.max(parts[i].bottom)
        };
    }
    let n = u32::try_from(ends.len()).unwrap_or(1);
    let total = parts.len();
    parts[run_start..total].iter_mut().for_each(|p| p.lanes = n);
    parts
}

/// `09:00` in the zone.
fn clock_words(at: Timestamp, zone: &TimeZone) -> String {
    at.to_zoned(zone.clone()).strftime("%H:%M").to_string()
}

/// A period in whole minutes, a day counted as 24 hours; none for nothing
/// or less than a minute.
fn period_minutes(period: &samu_api::Period) -> Option<i64> {
    let total = period
        .0
        .total((
            jiff::Unit::Minute,
            jiff::SpanRelativeTo::days_are_24_hours(),
        ))
        .ok()?;
    // The float is a count of minutes, small enough to be exact.
    #[allow(clippy::cast_possible_truncation)]
    let minutes = total.round() as i64;
    (minutes > 0).then_some(minutes)
}

fn tag_filter(tag: Option<&str>, tags: &[Tag]) -> Result<Option<HashSet<Id>>, MobileError> {
    tag.map(|t| Ok(tag_and_below(parse_id(t)?, tags)))
        .transpose()
}

/// The task carries one of `wanted` itself; no filter lets all through.
fn has_tag(task: &Task, wanted: Option<&HashSet<Id>>) -> bool {
    wanted.is_none_or(|set| task.tags.iter().any(|t| set.contains(&t.tag)))
}

fn parse_id(text: &str) -> Result<Id, MobileError> {
    text.parse()
        .map_err(|_| input(format!("{text:?} is no id")))
}

/// `2026-11-20` or `2026-11-20 09:00` in the time's own zone.
pub(crate) fn when_text(when: &When) -> String {
    let value = serde_json::to_value(when).unwrap_or(Value::Null);
    if let Some(date) = value["date"].as_str() {
        return date.to_string();
    }
    if let Some(at) = value["at"].as_str() {
        return at.get(..16).unwrap_or(at).replace('T', " ");
    }
    String::new()
}

/// Tasks in tree order with their depth among the given ones; a task whose
/// parent is not among them stands at the top.
fn tree<'a>(tasks: &[&'a Task]) -> Vec<(&'a Task, u32)> {
    let ids: Vec<Id> = tasks.iter().map(|t| t.id).collect();
    let mut out = Vec::new();
    fn walk<'a>(
        parent: Option<Id>,
        depth: u32,
        tasks: &[&'a Task],
        ids: &[Id],
        out: &mut Vec<(&'a Task, u32)>,
        seen: &mut Vec<Id>,
    ) {
        for task in tasks {
            let own_parent = task.parent.filter(|p| ids.contains(p));
            if own_parent == parent && !seen.contains(&task.id) {
                seen.push(task.id);
                out.push((*task, depth));
                walk(Some(task.id), depth + 1, tasks, ids, out, seen);
            }
        }
    }
    let mut seen = Vec::new();
    walk(None, 0, tasks, &ids, &mut out, &mut seen);
    out
}

fn row(
    task: &Task,
    depth: u32,
    objects: &Objects,
    evaluation: &Evaluation,
    names: &Names,
    touched: &[Id],
    zone: &TimeZone,
) -> TaskRow {
    let state = evaluation.tasks.get(&task.id);
    let project = task
        .project
        .and_then(|p| objects.projects.iter().find(|x| x.id == p));
    let placed = placement(&task.tags, project, &objects.tags);
    TaskRow {
        id: task.id.to_string(),
        title: task.title.clone(),
        project_id: task.project.map(|p| p.to_string()),
        project: project.map(|p| p.title.clone()),
        place: project.map(|p| names.place(p)),
        flagged: task.flagged,
        done: task.completed_at.is_some(),
        dropped: task.dropped_at.is_some(),
        available: state.is_some_and(|s| s.available),
        due: task.schedule.due.as_ref().map(when_text),
        due_state: match state.map(|s| s.due) {
            Some(DueState::Overdue) => Due::Overdue,
            Some(DueState::Soon) => Due::Soon,
            _ => Due::None,
        },
        defer: task.schedule.defer.as_ref().map(when_text),
        planned: task.schedule.planned.as_ref().map(when_text),
        estimate: task.estimate.as_ref().map(|p| p.0.to_string()),
        priority: task.priority.and_then(priority_text),
        has_note: task.note.as_deref().is_some_and(|n| !n.trim().is_empty()),
        blocks: blockers::holds_up(&objects.tasks, &objects.links, task.id),
        reason: state
            .and_then(|s| s.reasons.first())
            .map(|r| reason_row(&r.into(), names)),
        tags: task
            .tags
            .iter()
            .filter(|t| Some(t.tag) != placed.state)
            .map(|t| names.tag_path(t.tag))
            .collect(),
        state: placed.state.map(|s| names.tag_path(s)),
        group: objects.tasks.iter().any(|t| t.parent == Some(task.id)),
        depth,
        pending: touched.contains(&task.id),
        why: None,
        why_date: None,
        habit: habits::row_of(task, objects, zone, Timestamp::now()),
    }
}

fn why_word(why: forecast::Why) -> &'static str {
    match why {
        forecast::Why::Due => "due",
        forecast::Why::Planned => "planned",
        forecast::Why::Defer => "defer",
        forecast::Why::Tag => "tag",
        forecast::Why::Done => "done",
        forecast::Why::Flagged => "flagged",
    }
}

fn reason_row(reason: &Reason, names: &Names) -> ReasonRow {
    let code = serde_json::to_value(reason)
        .ok()
        .and_then(|v| v["reason"].as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".into());
    let subject = match reason {
        Reason::Deferred { until } => Some(when_text(until)),
        Reason::ProjectNotActive { project } => Some(names.project(*project)),
        Reason::TagNotActive { tag } => Some(names.tag_path(*tag)),
        Reason::WaitingInSequence { first } => Some(names.task(*first)),
        Reason::BlockedBy { task } => Some(names.task(*task)),
        _ => None,
    };
    ReasonRow { code, subject }
}

fn warning_row(warning: &Warning) -> WarningRow {
    let meaning = |m: &Meaning| format!("{m:?}").to_lowercase();
    let (code, text) = match warning {
        Warning::UnknownDate { meaning: m, text } => {
            (format!("unknown_date_{}", meaning(m)), text.clone())
        }
        Warning::UnknownEstimate { text } => ("unknown_estimate".into(), text.clone()),
        Warning::UnknownRepeat { text } => ("unknown_repeat".into(), text.clone()),
        Warning::Twice { meaning: m, text } => (format!("twice_{}", meaning(m)), text.clone()),
        Warning::UnknownProject { name } => ("unknown_project".into(), name.clone()),
        Warning::UnknownTag { name } => ("unknown_tag".into(), name.clone()),
        Warning::UnknownTemplate { name } => ("unknown_template".into(), name.clone()),
        Warning::FreeText { text } => ("free_text".into(), text.clone()),
        Warning::ClockInFuture { text } => ("clock_in_future".into(), text.clone()),
        Warning::UnknownClock { text } => ("unknown_clock".into(), text.clone()),
        Warning::UnknownHabit { text } => ("unknown_habit".into(), text.clone()),
    };
    WarningRow { code, text }
}

/// Titles by id, for rows and reasons.
fn blocker_row(task: &Task, link: Option<Id>, objects: &Objects, names: &Names) -> BlockerRow {
    BlockerRow {
        link_id: link.map(|l| l.to_string()),
        task_id: task.id.to_string(),
        title: task.title.clone(),
        place: task
            .project
            .and_then(|p| objects.projects.iter().find(|x| x.id == p))
            .map(|p| names.place(p)),
        done: task.completed_at.is_some() || task.dropped_at.is_some(),
    }
}

struct Names<'a> {
    objects: &'a Objects,
}

impl<'a> Names<'a> {
    fn new(objects: &'a Objects) -> Self {
        Self { objects }
    }

    fn project(&self, id: Id) -> String {
        self.objects
            .projects
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.title.clone())
            .unwrap_or_default()
    }

    fn task(&self, id: Id) -> String {
        self.objects
            .tasks
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.title.clone())
            .unwrap_or_default()
    }

    /// A folder's path from the top, `Privat/Wohnung`.
    fn folder_path(&self, id: Id) -> String {
        let mut parts = Vec::new();
        let mut at = self.objects.folders.iter().find(|f| f.id == id);
        while let Some(folder) = at {
            parts.push(folder.title.clone());
            // A cycle in broken data must not loop forever (ADR 0017 §9).
            if parts.len() > 16 {
                break;
            }
            at = folder
                .parent
                .and_then(|p| self.objects.folders.iter().find(|f| f.id == p));
        }
        parts.reverse();
        parts.join("/")
    }

    /// Where a project's tasks live (ADR 0026 §7): `Projekt · Ordner/Pfad`.
    fn place(&self, project: &Project) -> String {
        match project.folder {
            Some(f) => format!("{} · {}", project.title, self.folder_path(f)),
            None => project.title.clone(),
        }
    }

    fn tag_path(&self, id: Id) -> String {
        tag_path_in(&self.objects.tags, id)
    }
}

/// A tag's path from the top among `tags`, `Ort/Büro`.
fn tag_path_in(tags: &[Tag], id: Id) -> String {
    let mut parts = Vec::new();
    let mut at = tags.iter().find(|t| t.id == id);
    while let Some(tag) = at {
        parts.push(tag.title.clone());
        if parts.len() > 16 {
            break;
        }
        at = tag.parent.and_then(|p| tags.iter().find(|t| t.id == p));
    }
    parts.reverse();
    parts.join("/")
}

/// A byte offset of `text` in UTF-16 code units.
fn utf16_at(text: &str, byte: usize) -> u32 {
    let byte = byte.min(text.len());
    let units: usize = text
        .char_indices()
        .take_while(|(i, _)| *i < byte)
        .map(|(_, c)| c.len_utf16())
        .sum();
    u32::try_from(units).unwrap_or(u32::MAX)
}

/// A UTF-16 offset as a byte offset of `text`, on a char boundary.
fn byte_at(text: &str, utf16: u32) -> usize {
    let mut units = 0_u32;
    for (i, c) in text.char_indices() {
        if units >= utf16 {
            return i;
        }
        units += u32::try_from(c.len_utf16()).unwrap_or(2);
    }
    text.len()
}

/// `"Ort Büro"` → `Ort Büro`, as the parser reads a quoted name.
fn unquote(name: &str) -> String {
    match name.strip_prefix('"') {
        Some(inner) => inner.strip_suffix('"').unwrap_or(inner).to_owned(),
        None => name.to_owned(),
    }
}
