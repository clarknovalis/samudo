// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later OR MIT OR Apache-2.0

//! Wire types of the samu API.
//!
//! This crate is the contract between samu and its clients, catastr among
//! them (ADR 0003). Rules that hold here:
//!
//! - **serde types only.** No logic, no I/O, no async runtime, nothing from
//!   the samu workspace. It is published on crates.io as `samudo-api`, and
//!   other projects build it on its own.
//! - **Fixtures are the contract.** Every type has JSON examples under
//!   `tests/fixtures/`, checked in both directions. A fixture changes only
//!   with a new version of this crate.
//! - **Unknown fields are skipped, not rejected,** so samu can add a field
//!   without breaking a reader. Removing or reinterpreting one is a new major
//!   version.

// The example in the README, compiled with the doc tests.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct Readme;

mod calendar;
mod capture;
mod change;
mod close;
mod convert;
mod delete;
mod document;
mod events;
mod export;
mod hlc;
mod hook;
mod id;
mod logbook;
mod model;
mod page;
mod pair;
mod register;
mod report;
mod session;
mod status;
mod time;

pub use calendar::{
    APPOINTMENT_FIELDS, AlarmRelated, Alert, Appointment, AppointmentAlarm, AppointmentEdit,
    AppointmentEditResponse, AppointmentInstance, AppointmentPatch, AppointmentStatus, Calendar,
    CalendarImport, CalendarKind, CalendarSource, DiscoveredCalendar, EditRange, EditResult,
    ImportIssue,
};
pub use capture::{
    CaptureLinesRequest, CaptureLinesResponse, CaptureMark, CaptureMeaning, CaptureRequest,
    CaptureResponse, CaptureWarning,
};
pub use change::{Change, ChangeOutcome, ObjectKind};
pub use close::{ChildFate, CloseField, CloseRequest, CloseResponse};
pub use convert::{ToProjectRequest, ToProjectResponse};
pub use delete::{DeleteRequest, DeleteResponse, TaskFate};
pub use document::{
    Align, Block, Document, DocumentFormat, Inline, LinkClass, LinkedObject, ListItem, Property,
    RenderRequest, RenderResponse, RenderText, Span, TableRow,
};
pub use events::{ChangeNotice, ResetNotice};
pub use export::{
    EXPORT_BLOBS, EXPORT_FILES, EXPORT_FORMAT, EXPORT_KINDS, EXPORT_LOGBOOK, EXPORT_MANIFEST,
    EXPORT_README, Export, ExportError, ExportManifest, ExportedObject, MemberStamp, ObjectSync,
    PropertyStamp, SyncStamp, TagStamp, export_file, id_of, kind_of,
};
pub use hlc::Hlc;
pub use hook::{HookChange, HookChangeResponse, HookDelivery, HookFormat, HookView};
pub use id::Id;
pub use logbook::{AccountLogEntry, LogEntry, LogKind};
pub use model::{
    AccountSettings, Attachment, Author, AuthorKind, Blob, BlockQuery, BoardSource, ChecklistItem,
    Clock, Comment, Folder, FolderStatus, Habit, HabitEntry, HabitEntryStatus, HabitKind, HabitPer,
    HitChildren, InspectorLayout, InspectorPart, Link, LinkKind, NoteMode, Period, Perspective,
    PerspectiveArrange, PerspectiveBlock, PerspectiveBoard, PerspectiveFilter, PerspectiveGroup,
    PerspectiveSort, PerspectiveTable, PerspectiveView, Priority, Project, ProjectKind,
    ProjectStatus, Reminder, ReminderAnchor, Schedule, Scope, ScopeCapture, ScopeChip,
    ScopeChipKind, ScopeColor, ScopeLayout, ScopeMode, ScopeWindow, Tag, TagStatus, Tagging, Task,
    Template, TemplateReminder, TranscriptState, Weekday,
};
pub use page::{AnyObject, ChangesPage};
pub use pair::{PairCode, PairCodeRequest, PairRequest};
pub use register::{
    AccountDelete, DemoAdded, DemoRequest, Register, Registered, RegistrationInfo, RegistrationMode,
};
pub use report::{
    ClockReport, ClockReportBy, ClockReportRow, Report, ReportGroup, ReportList, ReportPart,
    ReportReason, ReportReasonCount, ReportRow, ReportWorked,
};
pub use session::{
    Access, ApiError, ErrorCode, Grant, Login, ROUTES, ReplicaState, Route, SessionInfo,
    SessionSummary,
};
pub use status::{
    BareStatus, BlockSection, DueState, ForecastKind, ForecastRow, ForecastSection, ForecastWhy,
    GroupSummary, HabitDay, HabitDayState, HabitPeriod, HabitRatio, HabitStatus, Inherited,
    NextRepeat, ProjectAvailability, Reason, ScheduledScope, TaskStatus,
};
pub use time::{Repeat, RepeatMode, When};

/// The version of this crate, which is the version of the wire format.
///
/// A server reports it so a client can tell which contract it speaks.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The URI scheme of samu objects: `samu:<uuid>` (ADR 0001 §2).
pub const URI_SCHEME: &str = "samu";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_the_crate_version() {
        assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
        assert!(!VERSION.is_empty());
    }
}
