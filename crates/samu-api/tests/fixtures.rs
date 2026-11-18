// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later OR MIT OR Apache-2.0

//! The fixtures are the contract (ADR 0003 §2, ADR 0014 §11).
//!
//! Every file under `fixtures/<type>/` is read into its type and written
//! back, and the two must be the same JSON. catastr checks against the same
//! files. A type without a fixture, or a directory nobody checks, fails —
//! a check that passes on nothing checks nothing.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use samu_api::{
    AccountDelete, AccountLogEntry, AccountSettings, Alert, AnyObject, ApiError, Appointment,
    AppointmentAlarm, AppointmentEdit, AppointmentEditResponse, AppointmentInstance, Attachment,
    BareStatus, Blob, BlockSection, Calendar, CalendarImport, CalendarSource, CaptureRequest,
    CaptureResponse, Change, ChangeNotice, ChangeOutcome, ChangesPage, Clock, ClockReport,
    ClockReportRow, Comment, DeleteRequest, DeleteResponse, DemoAdded, DemoRequest,
    DiscoveredCalendar, Document, Export, ExportManifest, ExportedObject, Folder, ForecastSection,
    Grant, HabitEntry, HabitStatus, Hlc, HookChange, HookChangeResponse, HookDelivery, HookView,
    Link, LogEntry, Login, PairCode, PairCodeRequest, PairRequest, Perspective, Project,
    ProjectAvailability, Register, Registered, RegistrationInfo, Reminder, RenderRequest,
    RenderResponse, Report, ReportRow, ResetNotice, ScheduledScope, Scope, SessionInfo,
    SessionSummary, Tag, Task, TaskStatus, Template,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

type RoundTrip = fn(&Value) -> Result<Value, serde_json::Error>;

fn round_trip<T: DeserializeOwned + Serialize>(value: &Value) -> Result<Value, serde_json::Error> {
    let typed: T = serde_json::from_value(value.clone())?;
    serde_json::to_value(typed)
}

/// Every wire type and the directory of its fixtures.
const TYPES: &[(&str, RoundTrip)] = &[
    ("folder", round_trip::<Folder>),
    ("project", round_trip::<Project>),
    ("task", round_trip::<Task>),
    ("tag", round_trip::<Tag>),
    ("link", round_trip::<Link>),
    ("clock", round_trip::<Clock>),
    ("reminder", round_trip::<Reminder>),
    ("scope", round_trip::<Scope>),
    ("template", round_trip::<Template>),
    ("perspective", round_trip::<Perspective>),
    ("block_section", round_trip::<BlockSection>),
    ("habit_entry", round_trip::<HabitEntry>),
    ("habit_status", round_trip::<HabitStatus>),
    ("account_settings", round_trip::<AccountSettings>),
    ("calendar", round_trip::<Calendar>),
    ("calendar_source", round_trip::<CalendarSource>),
    ("discovered_calendar", round_trip::<DiscoveredCalendar>),
    ("calendar_import", round_trip::<CalendarImport>),
    ("appointment", round_trip::<Appointment>),
    ("appointment_alarm", round_trip::<AppointmentAlarm>),
    ("alert", round_trip::<Alert>),
    ("appointment_instance", round_trip::<AppointmentInstance>),
    ("appointment_edit", round_trip::<AppointmentEdit>),
    (
        "appointment_edit_response",
        round_trip::<AppointmentEditResponse>,
    ),
    ("forecast_section", round_trip::<ForecastSection>),
    ("clock_report", round_trip::<ClockReport>),
    ("clock_report_row", round_trip::<ClockReportRow>),
    ("report", round_trip::<Report>),
    ("report_row", round_trip::<ReportRow>),
    ("scheduled_scope", round_trip::<ScheduledScope>),
    ("comment", round_trip::<Comment>),
    ("attachment", round_trip::<Attachment>),
    ("blob", round_trip::<Blob>),
    ("change_notice", round_trip::<ChangeNotice>),
    ("reset_notice", round_trip::<ResetNotice>),
    ("hlc", round_trip::<Hlc>),
    ("change", round_trip::<Change>),
    ("change_outcome", round_trip::<ChangeOutcome>),
    ("login", round_trip::<Login>),
    ("session_info", round_trip::<SessionInfo>),
    ("grant", round_trip::<Grant>),
    ("session_summary", round_trip::<SessionSummary>),
    ("api_error", round_trip::<ApiError>),
    ("any_object", round_trip::<AnyObject>),
    ("changes_page", round_trip::<ChangesPage>),
    ("task_status", round_trip::<TaskStatus>),
    ("bare_status", round_trip::<BareStatus>),
    ("project_availability", round_trip::<ProjectAvailability>),
    ("capture_request", round_trip::<CaptureRequest>),
    ("capture_response", round_trip::<CaptureResponse>),
    ("hook_view", round_trip::<HookView>),
    ("hook_change", round_trip::<HookChange>),
    ("hook_change_response", round_trip::<HookChangeResponse>),
    ("hook_delivery", round_trip::<HookDelivery>),
    ("log_entry", round_trip::<LogEntry>),
    ("account_log_entry", round_trip::<AccountLogEntry>),
    ("pair_code_request", round_trip::<PairCodeRequest>),
    ("pair_code", round_trip::<PairCode>),
    ("pair_request", round_trip::<PairRequest>),
    ("registration_info", round_trip::<RegistrationInfo>),
    ("register", round_trip::<Register>),
    ("registered", round_trip::<Registered>),
    ("account_delete", round_trip::<AccountDelete>),
    ("demo_request", round_trip::<DemoRequest>),
    ("demo_added", round_trip::<DemoAdded>),
    ("delete_request", round_trip::<DeleteRequest>),
    ("delete_response", round_trip::<DeleteResponse>),
    ("document", round_trip::<Document>),
    ("render_request", round_trip::<RenderRequest>),
    ("render_response", round_trip::<RenderResponse>),
    ("export", round_trip::<Export>),
    ("export_manifest", round_trip::<ExportManifest>),
    ("exported_object", round_trip::<ExportedObject>),
];

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn files_in(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    files
}

fn read(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap();
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn every_fixture_reads_and_writes_back_unchanged() {
    let mut checked = 0;
    for (name, round_trip) in TYPES {
        let files = files_in(&fixture_root().join(name));
        assert!(!files.is_empty(), "no fixture for {name}");
        for path in files {
            let original = read(&path);
            let written = round_trip(&original)
                .unwrap_or_else(|e| panic!("{}: does not read: {e}", path.display()));
            assert_eq!(
                written,
                original,
                "{} changes on the way back",
                path.display()
            );
            checked += 1;
        }
    }
    assert!(checked >= TYPES.len());
}

#[test]
fn every_fixture_directory_belongs_to_a_type() {
    let known: BTreeSet<&str> = TYPES.iter().map(|(name, _)| *name).collect();
    for entry in fs::read_dir(fixture_root()).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().into_string().unwrap();
        assert!(
            known.contains(name.as_str()),
            "fixtures/{name} is checked by nothing"
        );
    }
}

/// ADR 0003 §2: samu may add a field without breaking a reader.
#[test]
fn unknown_fields_are_skipped() {
    for (name, round_trip) in TYPES {
        let path = &files_in(&fixture_root().join(name))[0];
        let original = read(path);
        let mut extended = original.clone();
        extended
            .as_object_mut()
            .unwrap()
            .insert("from_the_future".into(), Value::Bool(true));
        let written = round_trip(&extended)
            .unwrap_or_else(|e| panic!("{}: an unknown field breaks it: {e}", path.display()));
        assert_eq!(written, original, "{}", path.display());
    }
}

/// A reader takes `null` for a field that is usually left out.
#[test]
fn null_reads_like_a_missing_field() {
    let path = fixture_root().join("task/inbox.json");
    let original = read(&path);
    let mut with_null = original.clone();
    let object = with_null.as_object_mut().unwrap();
    for field in ["note", "project", "due", "repeat", "completed_at"] {
        object.insert(field.into(), Value::Null);
    }
    assert_eq!(round_trip::<Task>(&with_null).unwrap(), original);
}

/// ADR 0015: a value from a newer samu reads as `unknown` instead of
/// breaking the reader — for every enum on the wire.
#[test]
fn unknown_enum_values_read_as_unknown() {
    let cases: &[(&str, &[&str])] = &[
        ("folder/full.json", &["status"]),
        ("project/full.json", &["kind"]),
        ("project/full.json", &["status"]),
        ("project/full.json", &["repeat", "mode"]),
        ("tag/full.json", &["status"]),
        ("link/ref.json", &["kind"]),
        ("reminder/at.json", &["anchor"]),
        ("scope/only.json", &["mode"]),
        ("scope/second_round.json", &["filter", "0", "kind"]),
        ("forecast_section/tomorrow.json", &["section"]),
        ("forecast_section/tomorrow.json", &["rows", "0", "why"]),
        ("scope/second_round.json", &["schedule", "0", "days", "0"]),
        ("comment/by_agent.json", &["created_by", "kind"]),
        ("task/by_agent.json", &["created_by", "kind"]),
        ("task/priority.json", &["priority"]),
        ("scope/color.json", &["color"]),
        ("scope/layout.json", &["layout", "inspector", "shown", "0"]),
        ("template/habit.json", &["reminders", "0", "anchor"]),
        ("capture_response/template.json", &["marks", "0", "meaning"]),
        ("any_object/template.json", &["kind"]),
        ("scope/layout.json", &["layout", "inspector", "note"]),
        ("capture_response/priority.json", &["marks", "0", "meaning"]),
        ("login/agent.json", &["kind"]),
        ("session_info/agent.json", &["kind"]),
        ("grant/rw.json", &["access"]),
        ("session_info/grant.json", &["grant", "access"]),
        ("session_summary/restricted.json", &["grant", "access"]),
        ("session_summary/current.json", &["kind"]),
        ("log_entry/by_agent.json", &["author_kind"]),
        ("log_entry/moved.json", &["kind"]),
        ("account_log_entry/moved.json", &["object_kind"]),
        ("any_object/comment.json", &["kind"]),
        ("any_object/scope.json", &["kind"]),
        ("task/full.json", &["repeat", "mode"]),
        ("change/create.json", &["kind"]),
        ("change/set.json", &["op"]),
        ("change_outcome/applied.json", &["outcome"]),
        ("api_error/unauthorized.json", &["error"]),
        ("any_object/task.json", &["kind"]),
        ("task_status/blocked.json", &["due"]),
        ("hook_view/ring.json", &["format"]),
        ("document/markdown.json", &["format"]),
        ("document/markdown.json", &["blocks", "0", "type"]),
        (
            "document/markdown.json",
            &["blocks", "0", "content", "0", "type"],
        ),
        (
            "document/markdown.json",
            &["blocks", "2", "content", "0", "class"],
        ),
        ("document/markdown.json", &["blocks", "3", "align", "1"]),
        ("render_request/two.json", &["texts", "1", "format"]),
    ];
    for (file, pointer) in cases {
        let dir = file.split('/').next().unwrap();
        let round_trip = TYPES.iter().find(|(name, _)| *name == dir).unwrap().1;
        let mut value = read(&fixture_root().join(file));
        // A JSON pointer, so a step may be an index into a list.
        let at = format!("/{}", pointer.join("/"));
        let slot = value.pointer_mut(&at).unwrap();
        assert!(slot.is_string(), "{file} {pointer:?} is not an enum");
        *slot = Value::String("from_the_future".into());
        let mut written = round_trip(&value)
            .unwrap_or_else(|e| panic!("{file} {pointer:?}: a new value breaks it: {e}"));
        let slot = written.pointer_mut(&at).unwrap();
        assert_eq!(*slot, Value::String("unknown".into()), "{file} {pointer:?}");
    }
}

/// ADR 0015: a date in a form from a newer samu reads as unknown too.
#[test]
fn an_unknown_date_form_reads_as_unknown() {
    let mut value = read(&fixture_root().join("task/full.json"));
    value["due"] = serde_json::json!({"week": "2026-W47"});
    let task: Task = serde_json::from_value(value).unwrap();
    assert_eq!(task.schedule.due, Some(samu_api::When::Unknown));
}

/// ADR 0015 inside a list: a reason from a newer samu, with fields of its
/// own, reads as `unknown`.
#[test]
fn an_unknown_reason_with_fields_reads_as_unknown() {
    let mut value = read(&fixture_root().join("task_status/blocked.json"));
    value["reasons"][0] = serde_json::json!({"reason": "waiting_for_weather", "sky": "grey"});
    let status: TaskStatus = serde_json::from_value(value).unwrap();
    assert_eq!(status.reasons[0], samu_api::Reason::Unknown);
}

/// A bare status is the status without its task (ADR 0065): every fixture
/// of the one gives the fixture of the same name of the other.
#[test]
fn a_bare_status_is_the_status_without_its_task() {
    let dir = fixture_root().join("task_status");
    let mut seen = 0;
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap();
        let status: TaskStatus = serde_json::from_value(read(&path)).unwrap();
        let bare = serde_json::to_value(BareStatus::from(status)).unwrap();
        let wanted = read(&fixture_root().join("bare_status").join(name));
        assert_eq!(bare, wanted, "{}", path.display());
        seen += 1;
    }
    assert!(seen >= 9, "only {seen} fixtures read");
}

/// ADR 0072 §1: a part of the layout from a newer samu is passed over, the
/// parts this reader knows still count.
#[test]
fn a_layout_part_from_a_newer_samu_is_passed_over() {
    let mut value = read(&fixture_root().join("scope/layout.json"));
    value["layout"]["sidebar"] = serde_json::json!({"step": "narrow"});
    let scope: Scope = serde_json::from_value(value).unwrap();
    let inspector = scope.layout.unwrap().inspector.unwrap();
    assert_eq!(inspector.shown.len(), 4);
    assert_eq!(inspector.note, Some(samu_api::NoteMode::Rendered));
}
