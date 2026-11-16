// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later OR MIT OR Apache-2.0

//! Calendars read from elsewhere and their appointments (ADR 0047, 0048).
//!
//! A calendar is the user's: they add it, name it, colour it. Its source –
//! the address with its secret, a user name, a password – never travels in
//! these types; it is set once with `PUT /calendars/{id}/source` and stays
//! on the server. Appointments come from what the server fetched; in a
//! calendar the server may write to, a client changes them too, and the
//! server writes them back (ADR 0068).

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::{Id, Period, When};

/// A calendar samu reads (ADR 0047 §2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Calendar {
    pub id: Id,
    pub created: Timestamp,
    pub modified: Timestamp,
    pub title: String,
    pub kind: CalendarKind,
    /// A CSS colour, like `#3b82f6`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Fetched and shown only when set.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Minutes between two fetches, at least 5.
    #[serde(default = "fifteen")]
    pub refresh: u32,
    /// Where „open in the calendar“ leads for an appointment without a URL
    /// of its own: `{uid}`, `{date}` (`YYYY-MM-DD`), `{y}`, `{m}`, `{d}`
    /// are filled in (ADR 0047 §9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_template: Option<String>,
    #[serde(default)]
    pub position: i64,
    /// When a fetch last changed something: an appointment or `error`.
    /// A fetch that finds the calendar as it was writes nothing (ADR 0047
    /// §7), not even this. Server's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetched_at: Option<Timestamp>,
    /// What went wrong the last time; none when it went well. Server's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The host of the source, to tell sources apart without the secret.
    /// Server's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// The source lets samu write: CalDAV with `write`, `write-content` or
    /// `all` in `current-user-privilege-set` (ADR 0068 §1). Server's.
    #[serde(default, skip_serializing_if = "is_false")]
    pub writable: bool,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde passes a reference"
)]
const fn is_false(value: &bool) -> bool {
    !*value
}

const fn yes() -> bool {
    true
}

const fn fifteen() -> u32 {
    15
}

/// How the server reads a calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalendarKind {
    /// An iCalendar file at an address, `https://` or `webcal://`.
    Ics,
    /// A calendar collection on a CalDAV server (RFC 4791).
    Caldav,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    /// Read, never written: the server refuses it.
    #[serde(other)]
    Unknown,
}

/// Where a calendar comes from, the body of `PUT /calendars/{id}/source`.
/// Kept sealed on the server and never answered back (ADR 0047 §3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarSource {
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Take user and password from this CalDAV calendar's source on the
    /// same host instead, so they never leave the server (ADR 0100 §2).
    /// Copied when set, not linked; `username` and `password` are then
    /// left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials_from: Option<Id>,
}

/// The answer of `POST /calendars/{id}/import` (ADR 0100 §4): how many
/// resources went in new, how many replaced one with the same UID, and what
/// was left out or failed, each with its UID and why.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarImport {
    pub created: u32,
    pub replaced: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<ImportIssue>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed: Vec<ImportIssue>,
}

/// One part of an import that did not go in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportIssue {
    /// The UID, when it had one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uid: Option<String>,
    pub reason: String,
}

/// A calendar collection `POST /calendars/discover` found on a CalDAV
/// server (ADR 0047 §8). Nothing is made of it until a client adds a
/// calendar and sets its source to `url` with the same user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredCalendar {
    /// The address of the collection, absolute.
    pub url: String,
    /// Its display name, else the last part of its path.
    pub title: String,
    /// A CSS colour, like `#3b82f6`, when the server keeps one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// A calendar of this account has this address as its source already
    /// (ADR 0100 §1).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub known: bool,
}

/// An appointment as the server read it (ADR 0047 §4): a VEVENT, the main
/// one of a repetition or one instance of it that was changed. Changed by a
/// client only in a writable calendar and never when it is a `meeting`
/// (ADR 0068 §2, §3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Appointment {
    pub id: Id,
    pub created: Timestamp,
    pub modified: Timestamp,
    pub calendar: Id,
    /// The UID in the calendar.
    pub uid: String,
    /// The instance of a repetition this one replaces; none for the main
    /// one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurrence_id: Option<When>,
    pub title: String,
    /// A date for an appointment over whole days, an instant otherwise
    /// (ADR 0007 §1).
    pub start: When,
    /// Excluded, as in iCalendar: an appointment on 20 November alone ends
    /// on the 21st. None: it ends when it starts (a whole day for a date).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<When>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The URL the calendar gave it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default)]
    pub status: AppointmentStatus,
    /// Blocks time (TRANSP:OPAQUE); not when free.
    #[serde(default = "yes")]
    pub busy: bool,
    /// The RRULE without its prefix, like `FREQ=WEEKLY;BYDAY=MO`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rrule: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rdate: Vec<When>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exdate: Vec<When>,
    /// It has attendees: an invitation samu does not answer, so it stays
    /// read-only (ADR 0068 §2). Server's.
    #[serde(default, skip_serializing_if = "is_false")]
    pub meeting: bool,
    /// The reminders its VALARMs carry (ADR 0101 §2). Server's: read from
    /// the calendar, not changed over `/changes`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alarms: Vec<AppointmentAlarm>,
}

/// One reminder of an appointment, from a VALARM with `ACTION:DISPLAY` or
/// `AUDIO` (ADR 0101 §2): a length of time before its start or end, or a
/// fixed instant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppointmentAlarm {
    /// How long before `related`; negative after it, like `-PT5M`. None
    /// with `at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<Period>,
    /// What `before` is counted from; the start unless said.
    #[serde(default, skip_serializing_if = "AlarmRelated::is_start")]
    pub related: AlarmRelated,
    /// A fixed instant (`TRIGGER;VALUE=DATE-TIME`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<Timestamp>,
}

/// What a relative alarm is counted from (`TRIGGER;RELATED=…`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlarmRelated {
    #[default]
    Start,
    End,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    #[serde(other)]
    Unknown,
}

impl AlarmRelated {
    #[must_use]
    pub const fn is_start(&self) -> bool {
        matches!(self, Self::Start)
    }
}

/// One reminder that goes off, as `GET /alarms` lists them (ADR 0101 §4,
/// §5): of a task or project, or of one occurrence of an appointment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alert {
    pub at: Timestamp,
    pub title: String,
    /// The task or project; none for an appointment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<Id>,
    /// The appointment; none for a task or project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appointment: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar: Option<Id>,
    /// The start of the occurrence, as `GET /appointments` gives it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<When>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<When>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
}

/// The fields of an appointment a client may change (ADR 0068 §3).
pub const APPOINTMENT_FIELDS: &[&str] = &[
    "title", "start", "end", "location", "note", "busy", "rrule", "rdate", "exdate", "calendar",
];

/// Which occurrences of a repetition an edit is about (ADR 0068 §4).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditRange {
    /// The one occurrence `instance` names.
    #[default]
    This,
    /// That occurrence and every one after it.
    Following,
    /// The whole repetition.
    All,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    #[serde(other)]
    Unknown,
}

/// The body of `POST /appointments/{id}/edit` (ADR 0068 §5): an edit of an
/// appointment or of one occurrence of its repetition, turned into changes
/// by samu-core. Either `patch` or `delete`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppointmentEdit {
    /// The start of the occurrence, as `GET /appointments` gives it; none
    /// for the appointment as a whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<When>,
    #[serde(default)]
    pub range: EditRange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<AppointmentPatch>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub delete: bool,
}

/// The fields an edit sets; what is none stays. `location` and `note` are
/// cleared with an empty text, `end` and `rrule` with `clear`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppointmentPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<When>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<When>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub busy: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rrule: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar: Option<Id>,
    /// Fields cleared: `end`, `rrule`, `location`, `note`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clear: Vec<String>,
}

/// What `POST /appointments/{id}/edit` did: each change it made, with the
/// server's answer, in order.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AppointmentEditResponse {
    pub results: Vec<EditResult>,
}

/// One change of an edit and its outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EditResult {
    pub change: crate::Change,
    pub outcome: crate::ChangeOutcome,
}

/// STATUS of an appointment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppointmentStatus {
    #[default]
    Confirmed,
    Tentative,
    Cancelled,
    /// A value from a newer samu that this reader does not know (ADR 0015).
    #[serde(other)]
    Unknown,
}

/// One occurrence of an appointment, as `GET /appointments` lists them
/// (ADR 0047 §5, ADR 0048): repetitions unfolded by samu-core.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppointmentInstance {
    pub appointment: Id,
    pub calendar: Id,
    pub title: String,
    pub start: When,
    /// Excluded, like [`Appointment::end`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<When>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// The appointment's DESCRIPTION, to show when it is opened.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Where „open in the calendar“ leads (ADR 0047 §9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default)]
    pub status: AppointmentStatus,
    #[serde(default = "yes")]
    pub busy: bool,
}
