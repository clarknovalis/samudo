// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! One running clock per account, and a clock stops with its task
//! (ADR 0046 §2-§4). Synthetic data only.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

use std::path::PathBuf;

use jiff::Timestamp;
use samu_store::core::api::{Author, AuthorKind, Change, ChangeOutcome, Hlc, Id, ObjectKind};
use samu_store::{AccountStore, Object};
use serde_json::{Value, json};

fn person(device: &str) -> Author {
    Author {
        device: device.into(),
        kind: AuthorKind::Human,
    }
}

fn now() -> Timestamp {
    "2026-11-09T12:00:00Z".parse().unwrap()
}

fn apply(store: &mut AccountStore, change: &Change, device: &str) -> ChangeOutcome {
    store.apply(change, &person(device), now()).unwrap()
}

fn create(store: &mut AccountStore, kind: ObjectKind, value: Value, hlc: Hlc) -> ChangeOutcome {
    let object = value["id"].as_str().unwrap().parse().unwrap();
    let device = hlc.device.clone();
    apply(
        store,
        &Change::Create {
            object,
            kind,
            value,
            hlc,
        },
        &device,
    )
}

fn set(store: &mut AccountStore, id: Id, field: &str, value: Value, hlc: Hlc) -> ChangeOutcome {
    let device = hlc.device.clone();
    apply(
        store,
        &Change::Set {
            object: id,
            field: field.into(),
            value,
            note: None,
            hlc,
        },
        &device,
    )
}

fn applied(outcome: &ChangeOutcome) -> bool {
    matches!(outcome, ChangeOutcome::Applied { .. })
}

fn task(store: &mut AccountStore, title: &str) -> Id {
    let id = Id::new();
    let value = json!({
        "id": id.to_string(), "created": "2026-11-09T07:00:00Z",
        "modified": "2026-11-09T07:00:00Z", "title": title, "flagged": false,
        "sequential": false, "complete_with_last_child": false, "position": 0
    });
    assert!(applied(&create(
        store,
        ObjectKind::Task,
        value,
        Hlc::new(1_000, 0, "laptop")
    )));
    id
}

fn clock_value(id: Id, owner: Id, start: &str, end: Option<&str>) -> Value {
    let mut value = json!({
        "id": id.to_string(), "created": start, "modified": start,
        "owner": owner.to_string(), "start": start
    });
    if let Some(end) = end {
        value["end"] = json!(end);
    }
    value
}

fn start(store: &mut AccountStore, owner: Id, at: &str, hlc: Hlc) -> Id {
    let id = Id::new();
    let outcome = create(
        store,
        ObjectKind::Clock,
        clock_value(id, owner, at, None),
        hlc,
    );
    assert!(applied(&outcome), "{outcome:?}");
    id
}

fn end_of(store: &AccountStore, id: Id) -> Option<Timestamp> {
    match store.get(id).unwrap() {
        Some(Object::Clock(c)) => c.end,
        other => panic!("not a clock: {other:?}"),
    }
}

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Asked of the database itself: how many clocks run, and the ends.
fn ask(path: &std::path::Path) -> (i64, Vec<Option<String>>) {
    let conn = rusqlite::Connection::open(path).unwrap();
    let running = conn
        .query_row(
            "SELECT count(*) FROM clock c JOIN object o ON o.id = c.object \
             WHERE o.deleted_at IS NULL AND c.end_at IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let mut stmt = conn
        .prepare("SELECT end_at FROM clock ORDER BY start_at")
        .unwrap();
    let ends = stmt
        .query_map([], |row| row.get::<_, Option<String>>(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    (running, ends)
}

#[test]
fn a_newer_clock_from_another_device_ends_the_older_in_the_database() {
    let dir = Dir(std::env::temp_dir().join(format!("samu-store-clock-{}", Id::new())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let path = dir.0.join("account.db");
    let mut store = AccountStore::open(&path).unwrap();
    let a = task(&mut store, "A");
    let b = task(&mut store, "B");

    let first = start(
        &mut store,
        a,
        "2026-11-09T08:00:00Z",
        Hlc::new(2_000, 0, "laptop"),
    );
    // The probe that must find something: one clock runs.
    assert_eq!(ask(&path).0, 1);
    let seq_before = store.seq().unwrap();
    let second = start(
        &mut store,
        b,
        "2026-11-09T09:00:00Z",
        Hlc::new(2_001, 0, "pixel"),
    );
    assert_eq!(
        ask(&path),
        (1, vec![Some("2026-11-09T09:00:00Z".into()), None])
    );
    assert_eq!(end_of(&store, second), None);
    // The end is a change of its own: the first clock is changed after the
    // second was made, so every device pulls it.
    let changed = store.changed_after(seq_before).unwrap();
    assert!(changed.iter().any(|c| c.id == first));
    assert!(changed.iter().any(|c| c.id == second));
}

#[test]
fn an_older_clock_arriving_late_ends_itself() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let a = task(&mut store, "A");
    let b = task(&mut store, "B");
    let newer = start(
        &mut store,
        a,
        "2026-11-09T09:00:00Z",
        Hlc::new(2_000, 0, "laptop"),
    );
    // Started offline on the phone before, sent only now.
    let older = start(
        &mut store,
        b,
        "2026-11-09T08:00:00Z",
        Hlc::new(2_001, 0, "pixel"),
    );
    assert_eq!(end_of(&store, newer), None);
    assert_eq!(
        end_of(&store, older),
        Some("2026-11-09T09:00:00Z".parse().unwrap())
    );
    // Its end is stamped after the change that caused it: the phone's own
    // change from before cannot undo it.
    let undo = set(
        &mut store,
        older,
        "end",
        Value::Null,
        Hlc::new(2_001, 0, "pixel"),
    );
    assert!(!matches!(undo, ChangeOutcome::Rejected { .. }));
    assert!(end_of(&store, older).is_some());
}

#[test]
fn restarting_a_stopped_clock_ends_the_running_one() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let a = task(&mut store, "A");
    let id = Id::new();
    assert!(applied(&create(
        &mut store,
        ObjectKind::Clock,
        clock_value(id, a, "2026-11-09T10:00:00Z", Some("2026-11-09T10:30:00Z")),
        Hlc::new(2_000, 0, "laptop"),
    )));
    let running = start(
        &mut store,
        a,
        "2026-11-09T08:00:00Z",
        Hlc::new(2_001, 0, "laptop"),
    );
    assert_eq!(end_of(&store, running), None);
    // Open the later one again: it is the newer and wins.
    assert!(applied(&set(
        &mut store,
        id,
        "end",
        Value::Null,
        Hlc::new(2_002, 0, "laptop")
    )));
    assert_eq!(end_of(&store, id), None);
    assert_eq!(
        end_of(&store, running),
        Some("2026-11-09T10:00:00Z".parse().unwrap())
    );
}

#[test]
fn completing_or_dropping_stops_the_clock_on_it() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let a = task(&mut store, "A");
    let b = task(&mut store, "B");
    let on_a = start(
        &mut store,
        a,
        "2026-11-09T08:00:00Z",
        Hlc::new(2_000, 0, "laptop"),
    );
    // Completing another task leaves it running.
    assert!(applied(&set(
        &mut store,
        b,
        "completed_at",
        json!("2026-11-09T08:30:00Z"),
        Hlc::new(2_001, 0, "laptop")
    )));
    assert_eq!(end_of(&store, on_a), None);
    assert!(applied(&set(
        &mut store,
        a,
        "completed_at",
        json!("2026-11-09T09:15:00Z"),
        Hlc::new(2_002, 0, "laptop")
    )));
    assert_eq!(
        end_of(&store, on_a),
        Some("2026-11-09T09:15:00Z".parse().unwrap())
    );
    // Opening it again starts nothing.
    assert!(applied(&set(
        &mut store,
        a,
        "completed_at",
        Value::Null,
        Hlc::new(2_003, 0, "laptop")
    )));
    assert!(end_of(&store, on_a).is_some());

    let c = task(&mut store, "C");
    let on_c = start(
        &mut store,
        c,
        "2026-11-09T10:00:00Z",
        Hlc::new(2_004, 0, "laptop"),
    );
    assert!(applied(&set(
        &mut store,
        c,
        "dropped_at",
        json!("2026-11-09T10:05:00Z"),
        Hlc::new(2_005, 0, "laptop")
    )));
    assert_eq!(
        end_of(&store, on_c),
        Some("2026-11-09T10:05:00Z".parse().unwrap())
    );
}

#[test]
fn a_clock_cannot_end_before_it_starts() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let a = task(&mut store, "A");
    let id = Id::new();
    let outcome = create(
        &mut store,
        ObjectKind::Clock,
        clock_value(id, a, "2026-11-09T10:00:00Z", Some("2026-11-09T09:00:00Z")),
        Hlc::new(2_000, 0, "laptop"),
    );
    assert!(
        matches!(outcome, ChangeOutcome::Rejected { .. }),
        "{outcome:?}"
    );
    let running = start(
        &mut store,
        a,
        "2026-11-09T10:00:00Z",
        Hlc::new(2_001, 0, "laptop"),
    );
    let outcome = set(
        &mut store,
        running,
        "end",
        json!("2026-11-09T09:59:59Z"),
        Hlc::new(2_002, 0, "laptop"),
    );
    assert!(
        matches!(outcome, ChangeOutcome::Rejected { .. }),
        "{outcome:?}"
    );
    assert_eq!(end_of(&store, running), None);
}
