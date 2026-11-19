// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Calendars and appointments in the store (ADR 0047, 0048): appointments
//! are the server's alone, the source is sealed, and a sync writes only what
//! changed. Asked of the database itself, not of what the store says.
//! Synthetic data only.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

use std::path::{Path, PathBuf};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use samu_store::core::api::{
    Author, AuthorKind, CalendarSource, Change, ChangeOutcome, Hlc, Id, ObjectKind,
};
use samu_store::core::calendar::read_ics;
use samu_store::{AccountStore, Fetched, Object, SecretKey, Synced};
use serde_json::{Value, json};

const GOOGLE: &str = include_str!("../../../testdata/calendar/google.ics");

fn person() -> Author {
    Author {
        device: "laptop".into(),
        kind: AuthorKind::Human,
    }
}

fn now() -> Timestamp {
    "2026-11-09T12:00:00Z".parse().unwrap()
}

fn later(minutes: i64) -> Timestamp {
    now()
        .checked_add(jiff::SignedDuration::from_mins(minutes))
        .unwrap()
}

fn key() -> SecretKey {
    SecretKey::new([7; 32])
}

fn apply(store: &mut AccountStore, change: &Change) -> ChangeOutcome {
    store.apply(change, &person(), now()).unwrap()
}

/// Where the stamps begin that must win over the settings' own, which
/// carry the real time of their making.
const AHEAD: i64 = 4_000_000_000_000;

/// `apply` when the clock says [`AHEAD`]: against `now()` such a stamp
/// would be from the future and capped.
fn apply_ahead(store: &mut AccountStore, change: &Change) -> ChangeOutcome {
    let at = Timestamp::from_millisecond(AHEAD).unwrap();
    store.apply(change, &person(), at).unwrap()
}

fn applied(outcome: &ChangeOutcome) -> bool {
    matches!(outcome, ChangeOutcome::Applied { .. })
}

fn rejected(outcome: &ChangeOutcome) -> bool {
    matches!(outcome, ChangeOutcome::Rejected { .. })
}

fn hlc(wall: i64) -> Hlc {
    Hlc::new(wall, 0, "laptop")
}

fn calendar_value(id: Id) -> Value {
    json!({
        "id": id.to_string(), "created": "2026-11-09T07:00:00Z",
        "modified": "2026-11-09T07:00:00Z", "title": "Familie", "kind": "ics",
        "enabled": true, "refresh": 15, "position": 0
    })
}

fn add_calendar(store: &mut AccountStore) -> Id {
    let id = Id::new();
    assert!(applied(&apply(
        store,
        &Change::Create {
            object: id,
            kind: ObjectKind::Calendar,
            value: calendar_value(id),
            hlc: hlc(1_000),
        }
    )));
    id
}

fn source() -> CalendarSource {
    CalendarSource {
        url: "https://calendar.example.org/ical/very-secret-token/basic.ics".into(),
        username: Some("alice".into()),
        password: Some("hunter2-app-password".into()),
        credentials_from: None,
    }
}

fn google() -> Fetched {
    Fetched::Read {
        read: read_ics(GOOGLE, &TimeZone::get("Europe/Berlin").unwrap()).unwrap(),
        etag: Some("\"v1\"".into()),
        last_modified: None,
    }
}

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn store() -> (Dir, PathBuf, AccountStore) {
    let dir = Dir(std::env::temp_dir().join(format!("samu-store-calendar-{}", Id::new())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let path = dir.0.join("account.db");
    let store = AccountStore::open(&path).unwrap();
    (dir, path, store)
}

fn count(path: &Path, sql: &str) -> i64 {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

const LIVE: &str = "SELECT count(*) FROM object WHERE kind = 'appointment' AND deleted_at IS NULL";

#[test]
fn a_client_writes_no_appointment_and_no_field_of_the_server() {
    let (_dir, _path, mut store) = store();
    let calendar = add_calendar(&mut store);
    store.sync_calendar(calendar, google(), now()).unwrap();
    let appointment = store.appointments().unwrap()[0].id;

    let made = Id::new();
    let mut value = serde_json::to_value(&store.appointments().unwrap()[0]).unwrap();
    value["id"] = json!(made.to_string());
    assert!(rejected(&apply(
        &mut store,
        &Change::Create {
            object: made,
            kind: ObjectKind::Appointment,
            value,
            hlc: hlc(2_000),
        }
    )));
    assert!(rejected(&apply(
        &mut store,
        &Change::Set {
            object: appointment,
            field: "title".into(),
            value: json!("mine"),
            note: None,
            hlc: hlc(2_000),
        }
    )));
    assert!(rejected(&apply(
        &mut store,
        &Change::Delete {
            object: appointment,
            hlc: hlc(2_000),
        }
    )));
    assert!(rejected(&apply(
        &mut store,
        &Change::Set {
            object: calendar,
            field: "error".into(),
            value: json!("made up"),
            note: None,
            hlc: hlc(2_000),
        }
    )));
    let other = Id::new();
    let mut value = calendar_value(other);
    value["host"] = json!("example.org");
    assert!(rejected(&apply(
        &mut store,
        &Change::Create {
            object: other,
            kind: ObjectKind::Calendar,
            value,
            hlc: hlc(2_000),
        }
    )));
    // What the user owns, they change.
    assert!(applied(&apply(
        &mut store,
        &Change::Set {
            object: calendar,
            field: "color".into(),
            value: json!("#3b82f6"),
            note: None,
            hlc: hlc(2_000),
        }
    )));
}

#[test]
fn the_source_is_sealed_and_opens_only_with_its_key() {
    let (_dir, path, mut store) = store();
    let calendar = add_calendar(&mut store);
    assert!(
        store
            .set_calendar_source(calendar, &source(), &key(), now())
            .unwrap()
            .is_ok()
    );
    // The probe must find the row before its absence of plain text counts.
    assert_eq!(count(&path, "SELECT count(*) FROM calendar_source"), 1);
    let conn = rusqlite::Connection::open(&path).unwrap();
    let sealed: Vec<u8> = conn
        .query_row("SELECT sealed FROM calendar_source", [], |row| row.get(0))
        .unwrap();
    let text = String::from_utf8_lossy(&sealed);
    for secret in [
        "very-secret-token",
        "hunter2",
        "alice",
        "calendar.example.org",
    ] {
        assert!(!text.contains(secret), "{secret} in the clear");
    }
    let opened = store.calendar_source(calendar, &key()).unwrap().unwrap();
    assert_eq!(opened.source, source());
    assert!(
        store
            .calendar_source(calendar, &SecretKey::new([8; 32]))
            .is_err()
    );
    let Some(Object::Calendar(c)) = store.get(calendar).unwrap() else {
        panic!("no calendar");
    };
    assert_eq!(c.host.as_deref(), Some("calendar.example.org"));
    // An address that is none is refused.
    let bad = CalendarSource {
        url: "file:///etc/passwd".into(),
        username: None,
        password: None,
        credentials_from: None,
    };
    assert!(
        store
            .set_calendar_source(calendar, &bad, &key(), now())
            .unwrap()
            .is_err()
    );
}

#[test]
fn a_calendar_is_due_at_once_then_after_its_refresh() {
    let (_dir, _path, mut store) = store();
    assert!(!store.has_calendar_work().unwrap(), "no calendar");
    let calendar = add_calendar(&mut store);
    assert!(store.due_calendars(now()).unwrap().is_empty(), "no source");
    assert!(!store.has_calendar_work().unwrap(), "no source");
    store
        .set_calendar_source(calendar, &source(), &key(), now())
        .unwrap()
        .unwrap();
    assert!(store.has_calendar_work().unwrap());
    assert_eq!(store.due_calendars(now()).unwrap(), vec![calendar]);
    store.sync_calendar(calendar, google(), now()).unwrap();
    assert!(store.due_calendars(later(14)).unwrap().is_empty());
    assert_eq!(store.due_calendars(later(15)).unwrap(), vec![calendar]);
    assert!(store.fetch_soon(calendar).unwrap());
    assert_eq!(store.due_calendars(now()).unwrap(), vec![calendar]);
}

#[test]
fn a_sync_writes_what_changed_and_nothing_when_nothing_did() {
    let (_dir, path, mut store) = store();
    let calendar = add_calendar(&mut store);
    let synced = store.sync_calendar(calendar, google(), now()).unwrap();
    // Seven with a UID, one of them long past.
    assert_eq!(
        synced,
        Some(Synced {
            created: 6,
            changed: 0,
            deleted: 0
        })
    );
    assert_eq!(count(&path, LIVE), 6);
    assert_eq!(
        count(
            &path,
            "SELECT count(*) FROM object WHERE kind = 'appointment' AND author_device IS NULL"
        ),
        6
    );
    assert_eq!(
        count(
            &path,
            "SELECT count(*) FROM stamp WHERE field = '@create' AND device = 'server' \
             AND object IN (SELECT id FROM object WHERE kind = 'appointment')"
        ),
        6
    );

    let seq = store.seq().unwrap();
    let again = store.sync_calendar(calendar, google(), later(15)).unwrap();
    assert_eq!(again, Some(Synced::default()));
    assert_eq!(
        store.seq().unwrap(),
        seq,
        "an unchanged calendar writes nothing"
    );
    let not_modified = store
        .sync_calendar(calendar, Fetched::NotModified, later(30))
        .unwrap();
    assert_eq!(not_modified, Some(Synced::default()));
    assert_eq!(store.seq().unwrap(), seq);

    // One title changed, one appointment gone.
    let changed = GOOGLE
        .replace("SUMMARY:Geburtstag Oma", "SUMMARY:Geburtstag Opa")
        .replace("UID:anruf@example.org", "UID:anders@example.org");
    let fetched = Fetched::Read {
        read: read_ics(&changed, &TimeZone::get("Europe/Berlin").unwrap()).unwrap(),
        etag: None,
        last_modified: None,
    };
    let synced = store.sync_calendar(calendar, fetched, later(45)).unwrap();
    assert_eq!(
        synced,
        Some(Synced {
            created: 1,
            changed: 1,
            deleted: 1
        })
    );
    assert_eq!(count(&path, LIVE), 6);
    assert_eq!(
        count(
            &path,
            "SELECT count(*) FROM stamp s JOIN appointment a ON a.object = s.object \
             WHERE s.field = 'title' AND s.device = 'server' AND a.title = 'Geburtstag Opa'"
        ),
        1
    );
    assert_eq!(
        count(
            &path,
            "SELECT count(*) FROM stamp WHERE field NOT LIKE '@%' AND object IN \
             (SELECT id FROM object WHERE kind = 'appointment')"
        ),
        1,
        "only the title was stamped"
    );
    assert_eq!(
        count(
            &path,
            "SELECT count(*) FROM appointment a JOIN object o ON o.id = a.object \
             WHERE a.uid = 'anruf@example.org' AND o.deleted_at IS NOT NULL"
        ),
        1
    );
}

#[test]
fn a_failure_keeps_the_appointments_and_says_why() {
    let (_dir, path, mut store) = store();
    let calendar = add_calendar(&mut store);
    store.sync_calendar(calendar, google(), now()).unwrap();
    store
        .sync_calendar(
            calendar,
            Fetched::Failed("401 Unauthorized".into()),
            later(15),
        )
        .unwrap();
    assert_eq!(count(&path, LIVE), 6);
    let error = |store: &AccountStore| match store.get(calendar).unwrap() {
        Some(Object::Calendar(c)) => (c.error, c.fetched_at),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        error(&store),
        (Some("401 Unauthorized".into()), Some(later(15)))
    );
    let seq = store.seq().unwrap();
    store
        .sync_calendar(
            calendar,
            Fetched::Failed("401 Unauthorized".into()),
            later(30),
        )
        .unwrap();
    assert_eq!(
        store.seq().unwrap(),
        seq,
        "the same error again is no change"
    );
    store.sync_calendar(calendar, google(), later(45)).unwrap();
    assert_eq!(error(&store), (None, Some(later(45))));
}

#[test]
fn deleting_a_calendar_takes_its_appointments_and_its_source() {
    let (_dir, path, mut store) = store();
    let calendar = add_calendar(&mut store);
    store
        .set_calendar_source(calendar, &source(), &key(), now())
        .unwrap()
        .unwrap();
    store.sync_calendar(calendar, google(), now()).unwrap();
    assert_eq!(count(&path, LIVE), 6);
    assert!(applied(&apply(
        &mut store,
        &Change::Delete {
            object: calendar,
            hlc: hlc(5_000),
        }
    )));
    assert_eq!(count(&path, LIVE), 0);
    assert_eq!(
        count(
            &path,
            "SELECT count(*) FROM object WHERE kind = 'appointment' AND deleted_at IS NOT NULL"
        ),
        6
    );
    assert_eq!(count(&path, "SELECT count(*) FROM calendar_source"), 0);
    assert_eq!(
        store.sync_calendar(calendar, google(), now()).unwrap(),
        None
    );
}

#[test]
fn the_snapshot_has_the_calendars_and_not_the_appointments() {
    let (_dir, _path, mut store) = store();
    let calendar = add_calendar(&mut store);
    store.sync_calendar(calendar, google(), now()).unwrap();
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.calendars.len(), 1);
    assert_eq!(store.appointments().unwrap().len(), 6);
    // They still go out over /changes.
    let changed = store.changed_after(0).unwrap();
    assert!(
        changed
            .iter()
            .filter(|c| matches!(store.get(c.id).unwrap(), Some(Object::Appointment(_))))
            .count()
            == 6
    );
}

#[test]
fn a_scope_names_only_live_calendars() {
    let (_dir, _path, mut store) = store();
    let calendar = add_calendar(&mut store);
    let scope = Id::new();
    let value = |calendars: Vec<String>| {
        json!({
            "id": scope.to_string(), "created": "2026-11-09T07:00:00Z",
            "modified": "2026-11-09T07:00:00Z", "title": "Dienst", "mode": "only",
            "calendars": calendars, "position": 0
        })
    };
    assert!(rejected(&apply(
        &mut store,
        &Change::Create {
            object: scope,
            kind: ObjectKind::Scope,
            value: value(vec![Id::new().to_string()]),
            hlc: hlc(3_000),
        }
    )));
    assert!(applied(&apply(
        &mut store,
        &Change::Create {
            object: scope,
            kind: ObjectKind::Scope,
            value: value(vec![calendar.to_string()]),
            hlc: hlc(3_000),
        }
    )));
    let Some(Object::Scope(s)) = store.get(scope).unwrap() else {
        panic!("no scope");
    };
    assert_eq!(s.calendars, vec![calendar]);
}

/// ADR 0047 §10: forecast_events is on for an account, set like any field
/// of the settings, and only to a boolean.
#[test]
fn forecast_events_is_on_and_set_like_any_setting() {
    let (_dir, path, mut store) = store();
    let settings = store.snapshot().unwrap().settings.clone().unwrap();
    assert!(settings.forecast_events);
    let set = |value: Value| Change::Set {
        object: settings.id,
        field: "forecast_events".into(),
        value,
        note: None,
        // After the stamp the server made the settings with.
        hlc: hlc(AHEAD),
    };
    assert!(rejected(&apply_ahead(&mut store, &set(json!("bald")))));
    assert!(applied(&apply_ahead(&mut store, &set(json!(false)))));
    assert!(
        !store
            .snapshot()
            .unwrap()
            .settings
            .clone()
            .unwrap()
            .forecast_events
    );
    assert_eq!(
        count(
            &path,
            "SELECT count(*) FROM settings WHERE forecast_events = 0"
        ),
        1
    );
}

/// ADR 0079 §2: the key bindings are one field of the settings, written as
/// a whole; a map of strings or null, nothing else.
#[test]
fn keys_are_set_as_a_whole_and_read_back() {
    let (_dir, path, mut store) = store();
    let settings = store.snapshot().unwrap().settings.clone().unwrap();
    assert!(settings.keys.is_empty());
    let set = |value: Value, ms: i64| Change::Set {
        object: settings.id,
        field: "keys".into(),
        value,
        note: None,
        hlc: hlc(AHEAD + ms),
    };
    assert!(rejected(&apply_ahead(
        &mut store,
        &set(json!({"complete": 5}), 0)
    )));
    assert!(rejected(&apply_ahead(&mut store, &set(json!("y"), 0))));
    let keys = json!({"complete": "y", "delete": null, "go.inbox": "g i"});
    assert!(applied(&apply_ahead(&mut store, &set(keys.clone(), 1))));
    let back = store.snapshot().unwrap().settings.clone().unwrap().keys;
    assert_eq!(serde_json::to_value(&back).unwrap(), keys);
    assert_eq!(
        count(
            &path,
            "SELECT count(*) FROM settings WHERE keys IS NOT NULL"
        ),
        1
    );
    // Back to the defaults: an empty map, not null.
    assert!(rejected(&apply_ahead(&mut store, &set(Value::Null, 2))));
    assert!(applied(&apply_ahead(&mut store, &set(json!({}), 3))));
    assert!(
        store
            .snapshot()
            .unwrap()
            .settings
            .clone()
            .unwrap()
            .keys
            .is_empty()
    );
}

/// ADR 0080 §1: what counts as worked on, `events` or `fields`; none reads
/// as `events`, a value from a newer samu is refused.
#[test]
fn report_worked_is_a_setting() {
    let (_dir, path, mut store) = store();
    let settings = store.snapshot().unwrap().settings.clone().unwrap();
    assert_eq!(settings.report_worked, None);
    let set = |value: Value, ms: i64| Change::Set {
        object: settings.id,
        field: "report_worked".into(),
        value,
        note: None,
        hlc: hlc(AHEAD + ms),
    };
    assert!(rejected(&apply_ahead(&mut store, &set(json!("später"), 0))));
    assert!(applied(&apply_ahead(&mut store, &set(json!("fields"), 1))));
    assert_eq!(
        store
            .snapshot()
            .unwrap()
            .settings
            .clone()
            .unwrap()
            .report_worked,
        Some(samu_store::core::api::ReportWorked::Fields)
    );
    assert_eq!(
        count(
            &path,
            "SELECT count(*) FROM settings WHERE report_worked = 'fields'"
        ),
        1
    );
    assert!(applied(&apply_ahead(&mut store, &set(Value::Null, 2))));
    assert_eq!(
        store
            .snapshot()
            .unwrap()
            .settings
            .clone()
            .unwrap()
            .report_worked,
        None
    );
}
