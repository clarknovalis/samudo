// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Appointments written by clients in a writable calendar (ADR 0068):
//! taken, queued for the writer, logged, left alone by a fetch while
//! queued, and settled with what the writer learns. Asked of the database
//! itself. Synthetic data only.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

use std::path::{Path, PathBuf};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use samu_store::core::api::{
    Appointment, Author, AuthorKind, CalendarSource, Change, ChangeOutcome, Hlc, Id, LogKind,
    ObjectKind,
};
use samu_store::core::calendar::{Read, read_ics};
use samu_store::{AccountStore, Fetched, Object, SecretKey};
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

fn berlin() -> TimeZone {
    TimeZone::get("Europe/Berlin").unwrap()
}

/// Where clients' stamps begin: after every stamp the server made,
/// including those from the real clock when the account was opened.
const BASE: i64 = 4_000_000_000_000;

/// A stamp `after_ms` milliseconds after [`BASE`].
fn hlc(after_ms: i64) -> Hlc {
    Hlc::new(BASE + after_ms, 0, "laptop")
}

/// When the clients' changes arrive: at [`BASE`], since a stamp further
/// ahead of the clock than the drift is capped.
fn arrival() -> Timestamp {
    Timestamp::from_millisecond(BASE).unwrap()
}

fn apply(store: &mut AccountStore, change: &Change) -> ChangeOutcome {
    store.apply(change, &person(), arrival()).unwrap()
}

fn applied(outcome: &ChangeOutcome) -> bool {
    matches!(outcome, ChangeOutcome::Applied { .. })
}

fn set(object: Id, field: &str, value: Value, after_ms: i64) -> Change {
    Change::Set {
        object,
        field: field.into(),
        value,
        note: None,
        hlc: hlc(after_ms),
    }
}

/// A CalDAV calendar with a source, writable or not.
fn add_calendar(store: &mut AccountStore, writable: bool) -> Id {
    let id = Id::new();
    let value = json!({
        "id": id.to_string(), "created": "2026-11-09T07:00:00Z",
        "modified": "2026-11-09T07:00:00Z", "title": "Kalender", "kind": "caldav",
        "enabled": true, "refresh": 15, "position": 0
    });
    assert!(applied(&apply(
        store,
        &Change::Create {
            object: id,
            kind: ObjectKind::Calendar,
            value,
            hlc: hlc(1),
        }
    )));
    let source = CalendarSource {
        url: format!("https://dav.example.org/cal/{id}/"),
        username: Some("alice".into()),
        password: Some("app-password".into()),
        credentials_from: None,
    };
    store
        .set_calendar_source(id, &source, &key(), now())
        .unwrap()
        .unwrap();
    store.set_writable(id, writable, now()).unwrap();
    id
}

/// The Google file as a CalDAV collection holds it: one resource per UID.
fn google_read(text: &str) -> Read {
    let mut read = read_ics(text, &berlin()).unwrap();
    for a in &mut read.appointments {
        a.href = Some(format!("/cal/{}.ics", a.uid));
        a.etag = Some("\"e1\"".into());
    }
    read
}

fn fetched(text: &str) -> Fetched {
    Fetched::Read {
        read: google_read(text),
        etag: Some("\"ctag1\"".into()),
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
    let dir = Dir(std::env::temp_dir().join(format!("samu-store-write-{}", Id::new())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let path = dir.0.join("account.db");
    let store = AccountStore::open(&path).unwrap();
    (dir, path, store)
}

fn query<T: rusqlite::types::FromSql>(path: &Path, sql: &str) -> T {
    let conn = rusqlite::Connection::open(path).unwrap();
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

fn find(store: &AccountStore, uid: &str) -> Appointment {
    store
        .appointments()
        .unwrap()
        .into_iter()
        .find(|a| a.uid == uid && a.recurrence_id.is_none())
        .unwrap()
}

fn get(store: &AccountStore, id: Id) -> Option<Appointment> {
    match store.get(id).unwrap() {
        Some(Object::Appointment(a)) => Some(a),
        _ => None,
    }
}

/// A calendar with the Google file in it, synced.
fn synced(writable: bool) -> (Dir, PathBuf, AccountStore, Id) {
    let (dir, path, mut store) = store();
    let calendar = add_calendar(&mut store, writable);
    store
        .sync_calendar(calendar, fetched(GOOGLE), now())
        .unwrap();
    (dir, path, store, calendar)
}

const QUEUED: &str = "SELECT count(*) FROM appointment_write";

#[test]
fn a_fetch_keeps_where_each_appointment_lives_and_whether_samu_may_write() {
    let (_dir, path, store, calendar) = synced(true);
    assert_eq!(
        query::<String>(
            &path,
            "SELECT href FROM appointment WHERE uid = 'zahnarzt@example.org'"
        ),
        "/cal/zahnarzt@example.org.ics"
    );
    let Some(Object::Calendar(c)) = store.get(calendar).unwrap() else {
        panic!()
    };
    assert!(c.writable);
    assert_eq!(query::<i64>(&path, QUEUED), 0, "a fetch queues nothing");
}

#[test]
fn a_change_in_a_writable_calendar_is_taken_queued_and_logged() {
    let (_dir, path, mut store, _) = synced(true);
    let dentist = find(&store, "zahnarzt@example.org");
    let outcome = apply(&mut store, &set(dentist.id, "title", json!("Zahnarzt"), 10));
    assert!(applied(&outcome), "{outcome:?}");
    assert_eq!(get(&store, dentist.id).unwrap().title, "Zahnarzt");
    assert_eq!(
        query::<String>(&path, "SELECT uid FROM appointment_write"),
        "zahnarzt@example.org"
    );
    assert_eq!(
        query::<String>(
            &path,
            "SELECT json_extract(fields, '$[0][1]') FROM appointment_write"
        ),
        "title"
    );
    let log = store.logbook(dentist.id).unwrap().unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].kind, LogKind::Changed);
    assert_eq!(log[0].field.as_deref(), Some("title"));
    assert_eq!(log[0].old_value, Some(json!("Zahnarzt, Kontrolle")));
    assert_eq!(log[0].new_value, Some(json!("Zahnarzt")));
    assert_eq!(log[0].device, "laptop");

    // A second change to the same resource is one entry, version 2.
    assert!(applied(&apply(
        &mut store,
        &set(dentist.id, "location", Value::Null, 20)
    )));
    assert_eq!(query::<i64>(&path, QUEUED), 1);
    assert_eq!(
        query::<i64>(&path, "SELECT version FROM appointment_write"),
        2
    );
}

#[test]
fn what_samu_may_not_write_is_refused() {
    let (_dir, path, mut store, calendar) = synced(false);
    let dentist = find(&store, "zahnarzt@example.org");
    let refused = apply(&mut store, &set(dentist.id, "title", json!("x"), 10));
    assert!(
        matches!(refused, ChangeOutcome::Rejected { .. }),
        "not writable"
    );

    store.set_writable(calendar, true, now()).unwrap();
    // An invitation stays read-only.
    let meeting = GOOGLE.replace(
        "UID:zahnarzt@example.org\r\n",
        "UID:zahnarzt@example.org\r\nATTENDEE:mailto:praxis@example.org\r\n",
    );
    let meeting = if meeting == GOOGLE {
        GOOGLE.replace(
            "UID:zahnarzt@example.org\n",
            "UID:zahnarzt@example.org\nATTENDEE:mailto:praxis@example.org\n",
        )
    } else {
        meeting
    };
    store
        .sync_calendar(calendar, fetched(&meeting), later(1))
        .unwrap();
    assert!(get(&store, dentist.id).unwrap().meeting);
    assert!(matches!(
        apply(&mut store, &set(dentist.id, "title", json!("x"), 10)),
        ChangeOutcome::Rejected { .. }
    ));
    assert!(matches!(
        apply(
            &mut store,
            &Change::Delete {
                object: dentist.id,
                hlc: hlc(10)
            }
        ),
        ChangeOutcome::Rejected { .. }
    ));
    // meeting and the stamps of the server are not a client's.
    let call = find(&store, "anruf@example.org");
    assert!(matches!(
        apply(&mut store, &set(call.id, "meeting", json!(true), 10)),
        ChangeOutcome::Rejected { .. }
    ));
    assert!(matches!(
        apply(&mut store, &set(call.id, "uid", json!("other"), 10)),
        ChangeOutcome::Rejected { .. }
    ));
    // A UID that is there already, an instance of a series that is not.
    let mut taken = call.clone();
    taken.id = Id::new();
    let create = |a: &Appointment| Change::Create {
        object: a.id,
        kind: ObjectKind::Appointment,
        value: serde_json::to_value(a).unwrap(),
        hlc: hlc(10),
    };
    assert!(matches!(
        apply(&mut store, &create(&taken)),
        ChangeOutcome::Rejected { .. }
    ));
    let mut orphan = call.clone();
    orphan.id = Id::new();
    orphan.uid = "nobody@example.org".into();
    orphan.recurrence_id = Some(orphan.start.clone());
    assert!(matches!(
        apply(&mut store, &create(&orphan)),
        ChangeOutcome::Rejected { .. }
    ));
    assert_eq!(query::<i64>(&path, QUEUED), 0, "nothing refused is queued");
}

#[test]
fn a_new_appointment_is_queued_and_the_writer_finishes_it() {
    let (_dir, path, mut store, calendar) = synced(true);
    let mut new = find(&store, "anruf@example.org");
    new.id = Id::new();
    new.uid = new.id.to_string();
    new.title = "Neu".into();
    new.url = None;
    let outcome = apply(
        &mut store,
        &Change::Create {
            object: new.id,
            kind: ObjectKind::Appointment,
            value: serde_json::to_value(&new).unwrap(),
            hlc: hlc(10),
        },
    );
    assert!(applied(&outcome), "{outcome:?}");
    assert_eq!(
        store.due_writes(arrival()).unwrap(),
        vec![(calendar, new.uid.clone())]
    );
    let job = store
        .write_job(calendar, &new.uid, &key())
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(job.appointments.len(), 1);
    assert_eq!(job.href, None, "never written");
    assert_eq!(job.fields[0].1, "@create");
    assert_eq!(job.source.source.username.as_deref(), Some("alice"));
    let log = store.logbook(new.id).unwrap().unwrap();
    assert_eq!(log[0].kind, LogKind::Created);

    store
        .wrote(
            calendar,
            &new.uid,
            job.version,
            Some("/cal/new.ics"),
            Some("\"n1\""),
        )
        .unwrap();
    assert_eq!(query::<i64>(&path, QUEUED), 0);
    assert_eq!(
        query::<String>(
            &path,
            &format!("SELECT etag FROM appointment WHERE uid = '{}'", new.uid)
        ),
        "\"n1\""
    );
}

#[test]
fn a_change_while_writing_stays_queued() {
    let (_dir, path, mut store, calendar) = synced(true);
    let dentist = find(&store, "zahnarzt@example.org");
    apply(&mut store, &set(dentist.id, "title", json!("A"), 10));
    let job = store
        .write_job(calendar, &dentist.uid, &key())
        .unwrap()
        .unwrap()
        .unwrap();
    apply(&mut store, &set(dentist.id, "title", json!("B"), 20));
    store
        .wrote(
            calendar,
            &dentist.uid,
            job.version,
            Some("/cal/zahnarzt@example.org.ics"),
            Some("\"e2\""),
        )
        .unwrap();
    assert_eq!(query::<i64>(&path, QUEUED), 1, "B is still to be written");
}

#[test]
fn a_fetch_leaves_a_queued_resource_alone() {
    let (_dir, _path, mut store, calendar) = synced(true);
    let dentist = find(&store, "zahnarzt@example.org");
    apply(&mut store, &set(dentist.id, "title", json!("Zahnarzt"), 10));
    store
        .sync_calendar(calendar, fetched(GOOGLE), later(15))
        .unwrap();
    assert_eq!(get(&store, dentist.id).unwrap().title, "Zahnarzt");
    // Once written, the fetch counts again.
    let job = store
        .write_job(calendar, &dentist.uid, &key())
        .unwrap()
        .unwrap()
        .unwrap();
    store
        .wrote(calendar, &dentist.uid, job.version, None, None)
        .unwrap();
    store
        .sync_calendar(calendar, fetched(GOOGLE), later(30))
        .unwrap();
    assert_eq!(
        get(&store, dentist.id).unwrap().title,
        "Zahnarzt, Kontrolle"
    );
}

#[test]
fn a_failed_write_is_tried_again_later_or_given_up_with_an_error() {
    let (_dir, path, mut store, calendar) = synced(true);
    let dentist = find(&store, "zahnarzt@example.org");
    apply(&mut store, &set(dentist.id, "title", json!("Zahnarzt"), 10));
    store
        .write_failed(calendar, &dentist.uid, "502 Bad Gateway", true, now())
        .unwrap();
    assert!(store.due_writes(now()).unwrap().is_empty());
    assert_eq!(store.due_writes(later(1)).unwrap().len(), 1);
    assert_eq!(
        query::<i64>(&path, "SELECT attempts FROM appointment_write"),
        1
    );

    store
        .write_failed(calendar, &dentist.uid, "403 Forbidden", false, later(2))
        .unwrap();
    assert_eq!(query::<i64>(&path, QUEUED), 0);
    let Some(Object::Calendar(c)) = store.get(calendar).unwrap() else {
        panic!()
    };
    assert_eq!(
        c.error.as_deref(),
        Some("could not write „Zahnarzt“: 403 Forbidden")
    );
    assert_eq!(
        query::<Option<String>>(&path, "SELECT etag FROM calendar_source"),
        None,
        "the next fetch reads all again"
    );
    store
        .sync_calendar(calendar, fetched(GOOGLE), later(15))
        .unwrap();
    assert_eq!(
        get(&store, dentist.id).unwrap().title,
        "Zahnarzt, Kontrolle"
    );
}

#[test]
fn a_crossed_write_takes_each_field_from_the_later_change() {
    let (_dir, _path, mut store, calendar) = synced(true);
    let dentist = find(&store, "zahnarzt@example.org");
    // samu: title at +10 s, location at +30 min.
    apply(&mut store, &set(dentist.id, "title", json!("samu"), 10_000));
    apply(
        &mut store,
        &set(dentist.id, "location", json!("hier"), 30 * 60_000),
    );
    // Elsewhere, at +10 min: both changed.
    let modified = Timestamp::from_millisecond(BASE + 10 * 60_000)
        .unwrap()
        .strftime("%Y%m%dT%H%M%SZ")
        .to_string();
    let theirs = GOOGLE
        .replace("SUMMARY:Zahnarzt\\, Kontrolle", "SUMMARY:drüben")
        .replace("LOCATION:Hauptstraße 1\\, Stadt", "LOCATION:dort")
        .replace(
            "UID:zahnarzt@example.org",
            &format!("UID:zahnarzt@example.org\r\nLAST-MODIFIED:{modified}"),
        );
    let mut remote = google_read(&theirs);
    remote.appointments.retain(|a| a.uid == dentist.uid);
    for a in &mut remote.appointments {
        a.etag = Some("\"e9\"".into());
    }
    store
        .crossed(calendar, &dentist.uid, &remote, later(40))
        .unwrap();
    let now_there = get(&store, dentist.id).unwrap();
    assert_eq!(
        now_there.title, "drüben",
        "theirs is later than samu's title"
    );
    assert_eq!(
        now_there.location.as_deref(),
        Some("hier"),
        "samu's location is later"
    );
    let job = store
        .write_job(calendar, &dentist.uid, &key())
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        job.etag.as_deref(),
        Some("\"e9\""),
        "the next PUT matches theirs"
    );
}

#[test]
fn a_resource_gone_elsewhere_takes_samus_change_with_it() {
    let (_dir, path, mut store, calendar) = synced(true);
    let choir = find(&store, "chor@example.org");
    apply(&mut store, &set(choir.id, "title", json!("Chorprobe"), 10));
    store.gone(calendar, &choir.uid, later(1)).unwrap();
    assert_eq!(
        query::<i64>(
            &path,
            "SELECT count(*) FROM appointment a JOIN object o ON o.id = a.object \
             WHERE a.uid = 'chor@example.org' AND o.deleted_at IS NULL"
        ),
        0
    );
    assert_eq!(query::<i64>(&path, QUEUED), 0);
}

#[test]
fn a_deleted_appointment_is_queued_with_where_it_was() {
    let (_dir, _path, mut store, calendar) = synced(true);
    let dentist = find(&store, "zahnarzt@example.org");
    assert!(applied(&apply(
        &mut store,
        &Change::Delete {
            object: dentist.id,
            hlc: hlc(10)
        }
    )));
    let job = store
        .write_job(calendar, &dentist.uid, &key())
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(job.appointments.is_empty(), "to be deleted");
    assert_eq!(job.href.as_deref(), Some("/cal/zahnarzt@example.org.ics"));
    assert_eq!(job.etag.as_deref(), Some("\"e1\""));
    assert_eq!(
        store.logbook(dentist.id).unwrap().unwrap()[0].kind,
        LogKind::Deleted
    );
}

#[test]
fn a_move_queues_the_new_calendar_and_remembers_the_old_resource() {
    let (_dir, path, mut store, calendar) = synced(true);
    let other = add_calendar(&mut store, true);
    let dentist = find(&store, "zahnarzt@example.org");
    apply(&mut store, &set(dentist.id, "title", json!("vorher"), 5));
    assert!(applied(&apply(
        &mut store,
        &set(dentist.id, "calendar", json!(other.to_string()), 10)
    )));
    assert_eq!(query::<i64>(&path, QUEUED), 1, "the old entry went along");
    let job = store
        .write_job(other, &dentist.uid, &key())
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(job.href, None, "new over there");
    let (from, href) = job.from.unwrap();
    assert_eq!(href, "/cal/zahnarzt@example.org.ics");
    assert!(from.source.url.contains(&calendar.to_string()));
    assert_eq!(job.fields.len(), 2, "both changes are pending");
}

#[test]
fn the_default_calendar_must_be_a_calendar() {
    let (_dir, _path, mut store, calendar) = synced(true);
    let settings = store.snapshot().unwrap().settings.clone().unwrap().id;
    assert!(applied(&apply(
        &mut store,
        &set(
            settings,
            "default_calendar",
            json!(calendar.to_string()),
            10
        )
    )));
    assert!(matches!(
        apply(
            &mut store,
            &set(
                settings,
                "default_calendar",
                json!(Id::new().to_string()),
                20
            )
        ),
        ChangeOutcome::Rejected { .. }
    ));
}
