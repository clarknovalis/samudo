// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Completing a repetition makes the next task (ADR 0007 §4, ADR 0033).
//! Synthetic data only.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

use std::path::PathBuf;

use jiff::Timestamp;
use samu_store::core::api::{
    Change, ChangeOutcome, Hlc, Id, LogKind, ObjectKind, ReminderAnchor, When,
};
use samu_store::{AccountStore, Object};
use serde_json::{Value, json};

/// The session most tests write through: a person on a laptop.
fn person() -> samu_store::core::api::Author {
    samu_store::core::api::Author {
        device: "laptop".into(),
        kind: samu_store::core::api::AuthorKind::Human,
    }
}

fn now() -> Timestamp {
    "2026-11-10T09:00:00Z".parse().unwrap()
}

fn stamp(wall: i64) -> Hlc {
    Hlc::new(wall, 0, "pixel")
}

fn apply(store: &mut AccountStore, change: &Change) -> ChangeOutcome {
    store.apply(change, &person(), now()).unwrap()
}

fn create(store: &mut AccountStore, kind: ObjectKind, value: Value, wall: i64) -> ChangeOutcome {
    let object = value["id"].as_str().unwrap().parse().unwrap();
    apply(
        store,
        &Change::Create {
            object,
            kind,
            value,
            hlc: stamp(wall),
        },
    )
}

fn set(store: &mut AccountStore, id: Id, field: &str, value: Value, wall: i64) -> ChangeOutcome {
    apply(
        store,
        &Change::Set {
            object: id,
            field: field.into(),
            value,
            note: None,
            hlc: stamp(wall),
        },
    )
}

fn complete(store: &mut AccountStore, id: Id, wall: i64) -> ChangeOutcome {
    set(
        store,
        id,
        "completed_at",
        json!("2026-11-10T08:30:00Z"),
        wall,
    )
}

/// A task due on 10.11. that repeats with `rule`; more fields from `extra`.
fn repeating(store: &mut AccountStore, rule: &str, extra: Value) -> Id {
    let id = Id::new();
    let mut value = json!({
        "id": id.to_string(),
        "created": "2026-11-01T08:00:00Z",
        "modified": "2026-11-01T08:00:00Z",
        "title": "Müll rausstellen",
        "due": {"date": "2026-11-10"},
        "repeat": {"rule": rule, "mode": "fixed"},
        "flagged": false,
        "sequential": false,
        "complete_with_last_child": false,
        "position": 4
    });
    if let (Value::Object(value), Value::Object(extra)) = (&mut value, extra) {
        value.extend(extra);
    }
    let outcome = create(store, ObjectKind::Task, value, 1_000);
    assert!(
        matches!(outcome, ChangeOutcome::Applied { .. }),
        "{outcome:?}"
    );
    id
}

fn tasks(store: &AccountStore) -> Vec<samu_store::core::api::Task> {
    store.snapshot().unwrap().tasks.clone()
}

fn next_of(store: &AccountStore, id: Id) -> Vec<samu_store::core::api::Task> {
    tasks(store)
        .into_iter()
        .filter(|t| t.repeat_of == Some(id))
        .collect()
}

fn last_note(store: &AccountStore, id: Id) -> Option<String> {
    let log = store.logbook(id).unwrap().unwrap();
    let last = log.last().unwrap();
    assert_eq!(last.kind, LogKind::Completed);
    last.note.clone()
}

#[test]
fn completing_makes_the_next_task_with_what_belongs_to_it() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let tag = Id::new();
    create(
        &mut store,
        ObjectKind::Tag,
        json!({
            "id": tag.to_string(),
            "created": "2026-11-01T08:00:00Z",
            "modified": "2026-11-01T08:00:00Z",
            "title": "Haus",
            "status": "active",
            "exclusive": false,
            "position": 0
        }),
        500,
    );
    let id = repeating(
        &mut store,
        "FREQ=WEEKLY",
        json!({
            "note": "gelbe Tonne",
            "flagged": true,
            "estimate": "PT5M",
            "defer": {"date": "2026-11-09"},
            "tags": [{"tag": tag.to_string(), "position": 2}],
            "properties": {"org_id": "abc-123", "source": "org-agenda-home"}
        }),
    );
    for (anchor, extra) in [
        ("due", json!({"before": "PT2H"})),
        ("at", json!({"at": "2026-11-09T18:00:00Z"})),
    ] {
        let mut reminder = json!({
            "id": Id::new().to_string(),
            "created": "2026-11-01T08:00:00Z",
            "modified": "2026-11-01T08:00:00Z",
            "owner": id.to_string(),
            "anchor": anchor
        });
        if let (Value::Object(r), Value::Object(e)) = (&mut reminder, extra) {
            r.extend(e);
        }
        create(&mut store, ObjectKind::Reminder, reminder, 1_100);
    }

    create(
        &mut store,
        ObjectKind::Link,
        json!({
            "id": Id::new().to_string(),
            "created": "2026-11-01T08:00:00Z",
            "modified": "2026-11-01T08:00:00Z",
            "owner": id.to_string(),
            "uri": "https://example.org/abfuhrkalender",
            "label": "Abfuhrkalender"
        }),
        1_200,
    );

    let outcome = complete(&mut store, id, 2_000);
    assert!(
        matches!(outcome, ChangeOutcome::Applied { .. }),
        "{outcome:?}"
    );

    let next = next_of(&store, id);
    assert_eq!(next.len(), 1);
    let next = &next[0];
    assert_ne!(next.id, id);
    assert_eq!(next.title, "Müll rausstellen");
    assert_eq!(next.note.as_deref(), Some("gelbe Tonne"));
    assert!(next.flagged);
    assert_eq!(next.completed_at, None);
    assert_eq!(
        next.schedule.due,
        Some(When::Date("2026-11-17".parse().unwrap()))
    );
    assert_eq!(
        next.schedule.defer,
        Some(When::Date("2026-11-16".parse().unwrap()))
    );
    assert_eq!(next.schedule.repeat.as_ref().unwrap().rule, "FREQ=WEEKLY");
    assert_eq!(next.position, 5);
    assert_eq!(next.tags.len(), 1);
    assert_eq!((next.tags[0].tag, next.tags[0].position), (tag, 2));
    assert_eq!(next.properties.get("org_id"), None, "org_id is unique");
    assert_eq!(
        next.properties.get("source").map(String::as_str),
        Some("org-agenda-home")
    );

    // The old one stays completed and keeps its repeat.
    let old = tasks(&store).into_iter().find(|t| t.id == id).unwrap();
    assert!(old.completed_at.is_some());
    assert!(old.schedule.repeat.is_some());

    // Only the reminder relative to due comes along.
    let reminders = store.snapshot().unwrap().reminders.clone();
    let copied: Vec<_> = reminders.iter().filter(|r| r.owner == next.id).collect();
    assert_eq!(copied.len(), 1);
    assert_eq!(serde_json::to_value(copied[0]).unwrap()["before"], "PT2H");
    assert_eq!(copied[0].anchor, ReminderAnchor::Due);

    // The link comes along (08.11.2026), the old one keeps its own.
    let links = store.snapshot().unwrap().links.clone();
    let copied: Vec<_> = links.iter().filter(|l| l.owner == next.id).collect();
    assert_eq!(copied.len(), 1);
    assert_eq!(copied[0].uri, "https://example.org/abfuhrkalender");
    assert_eq!(copied[0].label.as_deref(), Some("Abfuhrkalender"));
    assert_eq!(links.iter().filter(|l| l.owner == id).count(), 1);
    assert_eq!(last_note(&store, id), None);
}

fn item(store: &mut AccountStore, owner: Id, text: &str, checked: bool, position: i64) -> Id {
    let id = Id::new();
    let outcome = create(
        store,
        ObjectKind::ChecklistItem,
        json!({
            "id": id.to_string(),
            "created": "2026-11-01T08:00:00Z",
            "modified": "2026-11-01T08:00:00Z",
            "owner": owner.to_string(),
            "text": text,
            "checked": checked,
            "position": position
        }),
        1_300,
    );
    assert!(
        matches!(outcome, ChangeOutcome::Applied { .. }),
        "{outcome:?}"
    );
    id
}

/// The items of a task as (text, checked), in list order.
fn items_of(store: &AccountStore, owner: Id) -> Vec<(String, bool)> {
    let mut items: Vec<_> = store
        .snapshot()
        .unwrap()
        .checklist_items
        .iter()
        .filter(|i| i.owner == owner)
        .cloned()
        .collect();
    items.sort_by_key(|i| i.position);
    items.into_iter().map(|i| (i.text, i.checked)).collect()
}

/// A packing list was empty the next time (ADR 0141 §7).
#[test]
fn the_next_task_takes_the_checklist_along_unticked() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let id = repeating(&mut store, "FREQ=WEEKLY", json!({}));
    // Created out of order: the list order is the position.
    item(&mut store, id, "Badehose", false, 2);
    item(&mut store, id, "Handtuch", true, 1);
    let gone = item(&mut store, id, "Schnorchel", true, 3);
    apply(
        &mut store,
        &Change::Delete {
            object: gone,
            hlc: stamp(1_400),
        },
    );

    complete(&mut store, id, 2_000);

    let next = next_of(&store, id);
    assert_eq!(next.len(), 1);
    assert_eq!(
        items_of(&store, next[0].id),
        [("Handtuch".into(), false), ("Badehose".into(), false)]
    );
    // The completed task keeps its list as it was ticked.
    assert_eq!(
        items_of(&store, id),
        [("Handtuch".into(), true), ("Badehose".into(), false)]
    );
    assert_eq!(last_note(&store, id), None);
}

#[test]
fn never_a_second_next_task() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let id = repeating(&mut store, "FREQ=DAILY", json!({}));
    let done = Change::Set {
        object: id,
        field: "completed_at".into(),
        value: json!("2026-11-10T08:30:00Z"),
        note: None,
        hlc: stamp(2_000),
    };
    apply(&mut store, &done);
    // The same change again, as an outbox resends it.
    apply(&mut store, &done);
    assert_eq!(next_of(&store, id).len(), 1);

    // Opened and completed again.
    set(&mut store, id, "completed_at", Value::Null, 3_000);
    complete(&mut store, id, 4_000);
    assert_eq!(next_of(&store, id).len(), 1);

    // Not even when the next one was deleted.
    let next = next_of(&store, id)[0].id;
    apply(
        &mut store,
        &Change::Delete {
            object: next,
            hlc: stamp(5_000),
        },
    );
    set(&mut store, id, "completed_at", Value::Null, 6_000);
    complete(&mut store, id, 7_000);
    assert!(next_of(&store, id).is_empty());
    assert_eq!(tasks(&store).len(), 1);
}

#[test]
fn count_ends_the_chain_over_repeat_of() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let first = repeating(&mut store, "FREQ=YEARLY;COUNT=3", json!({}));
    complete(&mut store, first, 2_000);
    let second = next_of(&store, first)[0].id;
    complete(&mut store, second, 3_000);
    let third = next_of(&store, second)[0].id;
    complete(&mut store, third, 4_000);
    assert!(next_of(&store, third).is_empty());
    assert_eq!(tasks(&store).len(), 3);
    assert_eq!(
        last_note(&store, third).as_deref(),
        Some("the repetition has ended: no next task")
    );
}

#[test]
fn an_action_group_makes_no_next_task_yet() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let group = repeating(&mut store, "FREQ=WEEKLY", json!({}));
    let child = Id::new();
    create(
        &mut store,
        ObjectKind::Task,
        json!({
            "id": child.to_string(),
            "created": "2026-11-01T08:00:00Z",
            "modified": "2026-11-01T08:00:00Z",
            "title": "Tonne an die Straße",
            "parent": group.to_string(),
            "flagged": false,
            "sequential": false,
            "complete_with_last_child": false,
            "position": 0
        }),
        1_500,
    );
    complete(&mut store, group, 2_000);
    assert!(next_of(&store, group).is_empty());
    assert!(last_note(&store, group).unwrap().contains("action group"));
}

#[test]
fn dropping_makes_no_next_task() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let id = repeating(&mut store, "FREQ=WEEKLY", json!({}));
    set(
        &mut store,
        id,
        "dropped_at",
        json!("2026-11-10T08:30:00Z"),
        2_000,
    );
    assert!(next_of(&store, id).is_empty());
}

#[test]
fn a_rule_samu_cannot_compute_is_refused() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let id = Id::new();
    let outcome = create(
        &mut store,
        ObjectKind::Task,
        json!({
            "id": id.to_string(),
            "created": "2026-11-01T08:00:00Z",
            "modified": "2026-11-01T08:00:00Z",
            "title": "stündlich",
            "repeat": {"rule": "FREQ=HOURLY", "mode": "fixed"},
            "flagged": false,
            "sequential": false,
            "complete_with_last_child": false,
            "position": 0
        }),
        1_000,
    );
    assert!(
        matches!(outcome, ChangeOutcome::Rejected { .. }),
        "{outcome:?}"
    );

    let id = repeating(&mut store, "FREQ=WEEKLY", json!({}));
    let outcome = set(
        &mut store,
        id,
        "repeat",
        json!({"rule": "BYDAY=MO", "mode": "fixed"}),
        2_000,
    );
    assert!(
        matches!(outcome, ChangeOutcome::Rejected { .. }),
        "{outcome:?}"
    );
}

/// The database, asked directly: before completing no row has `repeat_of`,
/// after it exactly one, with the next due date in its columns.
#[test]
fn the_tables_hold_the_next_task() {
    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = Dir(std::env::temp_dir().join(format!("samu-store-repeat-{}", Id::new())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let path = dir.0.join("account.db");
    let ask = || {
        let conn = rusqlite::Connection::open(&path).unwrap();
        let mut stmt = conn
            .prepare("SELECT due_date FROM task WHERE repeat_of IS NOT NULL")
            .unwrap();
        stmt.query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };

    let id = {
        let mut store = AccountStore::open(&path).unwrap();
        repeating(&mut store, "FREQ=WEEKLY;BYDAY=TU", json!({}))
    };
    assert!(ask().is_empty());
    {
        let mut store = AccountStore::open(&path).unwrap();
        complete(&mut store, id, 2_000);
        assert!(matches!(store.get(id).unwrap(), Some(Object::Task(_))));
    }
    assert_eq!(ask(), ["2026-11-17"]);
}
