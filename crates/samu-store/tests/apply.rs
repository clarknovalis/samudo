//! Changes from an outbox against a real account database (ADR 0016,
//! ADR 0018). Synthetic data only: the fixtures of samu-api and made-up ids.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

use std::path::{Path, PathBuf};

use jiff::Timestamp;
use samu_store::core::api::{Change, ChangeOutcome, Hlc, Id, LogKind, ObjectKind, ReminderAnchor};
use samu_store::core::availability::{Reason, evaluate};
use samu_store::{AccountStore, Delivered, Object};
use serde_json::{Value, json};

fn fixture(path: &str) -> Value {
    let full = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../samu-api/tests/fixtures")
        .join(path);
    serde_json::from_str(&std::fs::read_to_string(&full).unwrap()).unwrap()
}

/// The session most tests write through: a person on a laptop.
fn person() -> samu_store::core::api::Author {
    samu_store::core::api::Author {
        device: "laptop".into(),
        kind: samu_store::core::api::AuthorKind::Human,
    }
}

fn now() -> Timestamp {
    "2026-11-07T09:00:00Z".parse().unwrap()
}

fn stamp(wall: i64, device: &str) -> Hlc {
    Hlc::new(wall, 0, device)
}

fn id_of(value: &Value) -> Id {
    value["id"].as_str().unwrap().parse().unwrap()
}

fn create(store: &mut AccountStore, kind: ObjectKind, value: Value, hlc: Hlc) -> ChangeOutcome {
    let change = Change::Create {
        object: id_of(&value),
        kind,
        value,
        hlc,
    };
    store.apply(&change, &person(), now()).unwrap()
}

fn set(store: &mut AccountStore, id: Id, field: &str, value: Value, hlc: Hlc) -> ChangeOutcome {
    let change = Change::Set {
        object: id,
        field: field.into(),
        value,
        note: None,
        hlc,
    };
    store.apply(&change, &person(), now()).unwrap()
}

fn is_applied(outcome: &ChangeOutcome) -> bool {
    matches!(outcome, ChangeOutcome::Applied { .. })
}

fn is_rejected(outcome: &ChangeOutcome) -> bool {
    matches!(outcome, ChangeOutcome::Rejected { .. })
}

fn new_task(title: &str) -> Value {
    let id = Id::new();
    json!({
        "id": id.to_string(),
        "created": "2026-11-07T08:00:00Z",
        "modified": "2026-11-07T08:00:00Z",
        "title": title,
        "flagged": false,
        "sequential": false,
        "complete_with_last_child": false,
        "position": 0
    })
}

fn new_tag(title: &str) -> Value {
    let id = Id::new();
    json!({
        "id": id.to_string(),
        "created": "2026-11-07T08:00:00Z",
        "modified": "2026-11-07T08:00:00Z",
        "title": title,
        "status": "active",
        "exclusive": false,
        "position": 0
    })
}

fn task_of(store: &AccountStore, id: Id) -> samu_store::core::api::Task {
    match store.get(id).unwrap() {
        Some(Object::Task(task)) => task,
        other => panic!("not a task: {other:?}"),
    }
}

/// ADR 0091 §1, §2: a habit's goal is checked; an entry needs a habit, a
/// day not after today, an amount only when done and an end only on a
/// pause; it never changes; deleting the habit takes its entries along.
#[test]
fn habits_and_their_entries_are_checked() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let plain = new_task("Kein Habit");
    let plain_id = id_of(&plain);
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Task,
        plain,
        stamp(1, "a")
    )));
    let mut habit = new_task("Wasser");
    habit["habit"] = json!({"kind": "build", "times": 8, "per": "day", "unit": "Glas"});
    let habit_id = id_of(&habit);
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Task,
        habit,
        stamp(2, "a")
    )));
    assert_eq!(task_of(&store, habit_id).habit.unwrap().times, 8);

    for bad in [
        json!({"kind": "build", "times": 0, "per": "day"}),
        json!({"kind": "sometimes", "per": "day"}),
        json!({"kind": "build", "per": "fortnight"}),
        json!({"kind": "build", "unit": " "}),
    ] {
        let outcome = set(&mut store, habit_id, "habit", bad.clone(), stamp(3, "a"));
        assert!(is_rejected(&outcome), "{bad}");
    }
    assert!(is_applied(&set(
        &mut store,
        habit_id,
        "habit",
        json!({"kind": "build", "times": 3, "per": "week"}),
        stamp(4, "a")
    )));

    let entry = |habit: Id, day: &str, more: Value| {
        let mut v = json!({
            "id": Id::new().to_string(),
            "created": "2026-11-07T08:00:00Z",
            "modified": "2026-11-07T08:00:00Z",
            "habit": habit.to_string(),
            "day": day,
            "status": "done",
        });
        for (k, x) in more.as_object().unwrap() {
            v[k] = x.clone();
        }
        v
    };
    let refused = [
        entry(plain_id, "2026-11-06", json!({})),
        entry(Id::new(), "2026-11-06", json!({})),
        entry(habit_id, "2026-11-08", json!({})),
        entry(
            habit_id,
            "2026-11-06",
            json!({"status": "fail", "amount": 2}),
        ),
        entry(habit_id, "2026-11-06", json!({"amount": 0})),
        entry(habit_id, "2026-11-06", json!({"until": "2026-11-07"})),
        entry(
            habit_id,
            "2026-11-06",
            json!({"status": "pause", "until": "2026-11-05"}),
        ),
        entry(habit_id, "2026-11-06", json!({"status": "maybe"})),
    ];
    for value in refused {
        let outcome = create(
            &mut store,
            ObjectKind::HabitEntry,
            value.clone(),
            stamp(5, "a"),
        );
        assert!(is_rejected(&outcome), "{value}");
    }
    let done = entry(habit_id, "2026-11-07", json!({"amount": 2}));
    let done_id = id_of(&done);
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::HabitEntry,
        done,
        stamp(6, "a")
    )));
    let pause = entry(
        habit_id,
        "2026-11-01",
        json!({"status": "pause", "until": "2026-11-03"}),
    );
    let pause_id = id_of(&pause);
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::HabitEntry,
        pause,
        stamp(7, "a")
    )));
    assert!(is_rejected(&set(
        &mut store,
        done_id,
        "amount",
        json!(3),
        stamp(8, "a")
    )));

    // Asked of the database: two live rows, then none once the task is gone.
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.habit_entries.len(), 2);
    let gone = Change::Delete {
        object: habit_id,
        hlc: stamp(9, "a"),
    };
    assert!(is_applied(&store.apply(&gone, &person(), now()).unwrap()));
    assert!(store.get(done_id).unwrap().is_none());
    assert!(store.get(pause_id).unwrap().is_none());
    assert!(store.snapshot().unwrap().habit_entries.is_empty());
}

/// ADR 0056: the priority is a field of its own, set and taken away; a
/// value the server does not know is refused, not stored.
#[test]
fn a_priority_is_set_cleared_and_checked() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let task = new_task("Steuer");
    let id = id_of(&task);
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Task,
        task,
        stamp(1_000, "laptop")
    )));
    assert_eq!(task_of(&store, id).priority, None);
    assert!(is_applied(&set(
        &mut store,
        id,
        "priority",
        json!("A"),
        stamp(1_001, "laptop")
    )));
    assert_eq!(
        task_of(&store, id).priority,
        Some(samu_store::core::api::Priority::A)
    );
    for (i, bad) in [json!("unknown"), json!("D"), json!(1)]
        .into_iter()
        .enumerate()
    {
        let wall = 1_002 + i64::try_from(i).unwrap();
        let outcome = set(
            &mut store,
            id,
            "priority",
            bad.clone(),
            stamp(wall, "laptop"),
        );
        assert!(is_rejected(&outcome), "{bad}: {outcome:?}");
    }
    assert_eq!(
        task_of(&store, id).priority,
        Some(samu_store::core::api::Priority::A)
    );
    assert!(is_applied(&set(
        &mut store,
        id,
        "priority",
        Value::Null,
        stamp(1_010, "laptop")
    )));
    assert_eq!(task_of(&store, id).priority, None);
}

/// ADR 0073 §1: a template's name is callable and unique ignoring case,
/// its text holds only marks, its reminders sit on a date of the task.
#[test]
fn a_template_is_checked_on_create_and_set() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let mut wall = 1_000;
    let mut next = || {
        wall += 1;
        stamp(wall, "laptop")
    };
    let template = |title: &str, text: &str, reminders: Value| {
        json!({
            "id": Id::new().to_string(),
            "created": "2026-11-11T08:00:00Z",
            "modified": "2026-11-11T08:00:00Z",
            "title": title,
            "text": text,
            "reminders": reminders,
            "position": 0
        })
    };
    let habit = template(
        "habit",
        "#Gewohnheiten @habit p:heute 21:00 every+:1d",
        json!([{"anchor": "planned"}]),
    );
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Template,
        habit.clone(),
        next()
    )));
    for bad in [
        template("Habit", "", json!([])),
        template("zwei worte", "", json!([])),
        template("x", "Rauchfrei #Haus", json!([])),
        template("y", "#Haus /habit", json!([])),
        template(
            "z",
            "",
            json!([{"anchor": "at", "at": "2026-11-11T08:00:00Z"}]),
        ),
        template("w", "", json!([{"anchor": "unknown"}])),
        // ADR 0093 §5: daily needs its time, and only daily has one.
        template("d", "", json!([{"anchor": "daily"}])),
        template("e", "", json!([{"anchor": "due", "time": "20:00:00"}])),
        template(
            "f",
            "",
            json!([{"anchor": "daily", "time": "20:00:00", "before": "PT1H"}]),
        ),
    ] {
        let outcome = create(&mut store, ObjectKind::Template, bad.clone(), next());
        assert!(is_rejected(&outcome), "{bad}: {outcome:?}");
    }
    let id = id_of(&habit);
    assert!(is_rejected(&set(
        &mut store,
        id,
        "text",
        json!("Text #x"),
        next()
    )));
    assert!(is_applied(&set(
        &mut store,
        id,
        "text",
        json!("@habit every+:1d"),
        next()
    )));
    // Its own name again, in another case, is no clash.
    assert!(is_applied(&set(
        &mut store,
        id,
        "title",
        json!("Habit"),
        next()
    )));
    match store.get(id).unwrap() {
        Some(Object::Template(t)) => {
            assert_eq!(t.title, "Habit");
            assert_eq!(t.text, "@habit every+:1d");
            assert_eq!(t.reminders.len(), 1);
        }
        other => panic!("not a template: {other:?}"),
    }
}

/// ADR 0090 §1: a perspective's name is unique ignoring case, its chips
/// name live objects in words samu-core reads, and a value from a newer
/// samu is not written.
#[test]
fn a_perspective_is_checked_on_create_and_set() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let mut wall = 1_000;
    let mut next = || {
        wall += 1;
        stamp(wall, "laptop")
    };
    let tag = new_tag("Büro");
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Tag,
        tag.clone(),
        next()
    )));
    let perspective = |title: &str, more: Value| {
        let mut value = json!({
            "id": Id::new().to_string(),
            "created": "2026-11-12T08:00:00Z",
            "modified": "2026-11-12T08:00:00Z",
            "title": title,
            "position": 0
        });
        value
            .as_object_mut()
            .unwrap()
            .extend(more.as_object().unwrap().clone());
        value
    };
    let office = perspective(
        "Büro",
        json!({
            "icon": "🏢",
            "chips": [{"kind": "tag", "value": id_of(&tag).to_string()}],
            "arrange": {"group": "none", "sort": "estimate"},
            "view": "board",
        }),
    );
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Perspective,
        office.clone(),
        next()
    )));
    for bad in [
        perspective("BÜRO", json!({})),
        perspective(" ", json!({})),
        perspective("a", json!({"icon": ""})),
        perspective(
            "b",
            json!({"chips": [{"kind": "tag", "value": Id::new().to_string()}]}),
        ),
        perspective(
            "c",
            json!({"chips": [{"kind": "due", "value": "irgendwann"}]}),
        ),
        perspective("d", json!({"filter": "someday"})),
        perspective("e", json!({"view": "gallery"})),
        perspective("f", json!({"arrange": {"group": "weather"}})),
        perspective(
            "g",
            json!({"board": {"source": "tags", "tags": [Id::new().to_string()]}}),
        ),
    ] {
        let outcome = create(&mut store, ObjectKind::Perspective, bad.clone(), next());
        assert!(is_rejected(&outcome), "{bad}: {outcome:?}");
    }
    let id = id_of(&office);
    assert!(is_rejected(&set(
        &mut store,
        id,
        "chips",
        json!([{"kind": "is", "value": "nie"}]),
        next()
    )));
    assert!(is_applied(&set(
        &mut store,
        id,
        "chips",
        json!([{"kind": "estimate", "value": "<15m"}]),
        next()
    )));
    assert!(is_applied(&set(
        &mut store,
        id,
        "filter",
        json!("all"),
        next()
    )));
    assert!(is_applied(&set(
        &mut store,
        id,
        "title",
        json!("büro"),
        next()
    )));
    match store.get(id).unwrap() {
        Some(Object::Perspective(p)) => {
            assert_eq!(p.title, "büro");
            assert_eq!(p.chips.len(), 1);
            assert_eq!(p.filter, samu_store::core::api::PerspectiveFilter::All);
            assert_eq!(p.view, samu_store::core::api::PerspectiveView::Board);
            assert_eq!(p.icon.as_deref(), Some("🏢"));
        }
        other => panic!("not a perspective: {other:?}"),
    }
}

/// ADR 0115 §2, §7: a table's columns are kept with the perspective, known
/// names only, each once.
#[test]
fn a_perspective_keeps_its_table_columns() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let mut wall = 1_000;
    let mut next = || {
        wall += 1;
        stamp(wall, "laptop")
    };
    let id = Id::new();
    let effort = json!({
        "id": id.to_string(),
        "created": "2026-11-16T08:00:00Z",
        "modified": "2026-11-16T08:00:00Z",
        "title": "Aufwand",
        "position": 0,
        "view": "table",
        "table": {"columns": ["estimate", "worked", "prop:jira"]}
    });
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Perspective,
        effort,
        next()
    )));
    for bad in [
        json!({"columns": ["ist"]}),
        json!({"columns": ["prop:"]}),
        json!({"columns": ["due", "due"]}),
        json!({"columns": ["title"]}),
    ] {
        let outcome = set(&mut store, id, "table", bad.clone(), next());
        assert!(is_rejected(&outcome), "{bad}: {outcome:?}");
    }
    assert!(is_applied(&set(
        &mut store,
        id,
        "table",
        json!({"columns": ["state", "remaining", "prop:kunde"]}),
        next()
    )));
    match store.get(id).unwrap() {
        Some(Object::Perspective(p)) => {
            assert_eq!(p.view, samu_store::core::api::PerspectiveView::Table);
            let columns = p.table.map(|t| t.columns).unwrap_or_default();
            assert_eq!(columns, ["state", "remaining", "prop:kunde"]);
        }
        other => panic!("not a perspective: {other:?}"),
    }
    assert!(is_applied(&set(
        &mut store,
        id,
        "table",
        Value::Null,
        next()
    )));
    match store.get(id).unwrap() {
        Some(Object::Perspective(p)) => assert_eq!(p.table, None),
        other => panic!("not a perspective: {other:?}"),
    }
}

/// ADR 0116 §1-2: a block perspective keeps its blocks and `dedupe`;
/// a block shows one live plain perspective or a known view, never itself
/// or another block perspective, and a perspective shown as a block gets
/// no blocks.
#[test]
fn a_block_perspective_keeps_its_blocks() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let mut wall = 1_000;
    let mut next = || {
        wall += 1;
        stamp(wall, "laptop")
    };
    let plain = |title: &str| {
        json!({
            "id": Id::new().to_string(),
            "created": "2026-11-16T08:00:00Z",
            "modified": "2026-11-16T08:00:00Z",
            "title": title,
            "position": 0
        })
    };
    let office = plain("Büro");
    let waiting = plain("Wartet");
    let morning = plain("Morgen");
    for p in [&office, &waiting, &morning] {
        assert!(is_applied(&create(
            &mut store,
            ObjectKind::Perspective,
            p.clone(),
            next()
        )));
    }
    let [office, waiting, morning] = [&office, &waiting, &morning].map(id_of);
    let blocks = json!([
        {"view": "today"},
        {"perspective": office.to_string(), "title": "Kurz im Büro"},
        {"perspective": waiting.to_string()}
    ]);
    assert!(is_applied(&set(
        &mut store,
        morning,
        "blocks",
        blocks,
        next()
    )));
    match store.get(morning).unwrap() {
        Some(Object::Perspective(p)) => {
            assert_eq!(p.blocks.len(), 3);
            assert_eq!(p.blocks[1].title.as_deref(), Some("Kurz im Büro"));
            assert!(p.dedupe, "the default");
        }
        other => panic!("not a perspective: {other:?}"),
    }
    assert!(is_applied(&set(
        &mut store,
        morning,
        "dedupe",
        json!(false),
        next()
    )));
    match store.get(morning).unwrap() {
        Some(Object::Perspective(p)) => assert!(!p.dedupe),
        other => panic!("not a perspective: {other:?}"),
    }
    for bad in [
        json!([{"view": "forecast"}]),
        json!([{"view": "today", "perspective": office.to_string()}]),
        json!([{}]),
        json!([{"view": "today", "title": " "}]),
        json!([{"perspective": morning.to_string()}]),
        json!([{"perspective": Id::new().to_string()}]),
    ] {
        let outcome = set(&mut store, morning, "blocks", bad.clone(), next());
        assert!(is_rejected(&outcome), "{bad}: {outcome:?}");
    }
    // A block of Morgen gets no blocks, and Morgen is no block.
    let outcome = set(
        &mut store,
        office,
        "blocks",
        json!([{"view": "inbox"}]),
        next(),
    );
    assert!(is_rejected(&outcome), "{outcome:?}");
    let outcome = set(
        &mut store,
        waiting,
        "blocks",
        json!([{"view": "inbox"}, {"perspective": office.to_string()}]),
        next(),
    );
    assert!(is_rejected(&outcome), "{outcome:?}");
    // Without blocks, Morgen is plain again and may be a block.
    assert!(is_applied(&set(
        &mut store,
        morning,
        "blocks",
        json!([]),
        next()
    )));
    assert!(is_applied(&set(
        &mut store,
        waiting,
        "blocks",
        json!([{"perspective": morning.to_string()}]),
        next()
    )));
}

/// ADR 0084 §1: a template's key is one letter or digit, no other live
/// template's; `h` and `H` are two keys, and a deleted template frees its.
#[test]
fn a_template_key_is_checked_and_unique() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let mut wall = 1_000;
    let mut next = || {
        wall += 1;
        stamp(wall, "laptop")
    };
    let template = |title: &str, key: Value| {
        json!({
            "id": Id::new().to_string(),
            "created": "2026-11-12T08:00:00Z",
            "modified": "2026-11-12T08:00:00Z",
            "title": title,
            "text": "",
            "position": 0,
            "key": key
        })
    };
    let habit = template("habit", json!("h"));
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Template,
        habit.clone(),
        next()
    )));
    for bad in [
        template("x", json!("h")),
        template("y", json!("hh")),
        template("z", json!("-")),
        template("w", json!("")),
    ] {
        let outcome = create(&mut store, ObjectKind::Template, bad.clone(), next());
        assert!(is_rejected(&outcome), "{bad}: {outcome:?}");
    }
    let inbox = template("inbox", json!("H"));
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Template,
        inbox.clone(),
        next()
    )));
    let plain = template("plain", Value::Null);
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Template,
        plain.clone(),
        next()
    )));
    // Setting a taken key is refused; its own key again is no clash.
    assert!(is_rejected(&set(
        &mut store,
        id_of(&plain),
        "key",
        json!("h"),
        next()
    )));
    assert!(is_applied(&set(
        &mut store,
        id_of(&habit),
        "key",
        json!("h"),
        next()
    )));
    match store.get(id_of(&plain)).unwrap() {
        Some(Object::Template(t)) => assert_eq!(t.key, None),
        other => panic!("not a template: {other:?}"),
    }
    // Deleted, a template frees its key.
    let gone = Change::Delete {
        object: id_of(&habit),
        hlc: next(),
    };
    assert!(is_applied(&store.apply(&gone, &person(), now()).unwrap()));
    assert!(is_applied(&set(
        &mut store,
        id_of(&plain),
        "key",
        json!("h"),
        next()
    )));
    match store.get(id_of(&plain)).unwrap() {
        Some(Object::Template(t)) => assert_eq!(t.key.as_deref(), Some("h")),
        other => panic!("not a template: {other:?}"),
    }
}

/// ADR 0072: a scope's layout is set, read back and taken away; a part or
/// a note mode from a newer samu is refused.
#[test]
fn a_scope_layout_is_set_and_checked() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let scope = new_scope("Gewohnt", "only", &[]);
    let mut wall = 1_000;
    let mut next = || {
        wall += 1;
        stamp(wall, "laptop")
    };
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Scope,
        scope.clone(),
        next()
    )));
    let id = id_of(&scope);
    for bad in [
        json!({"inspector": {"shown": ["note", "unknown"]}}),
        json!({"inspector": {"shown": ["note"], "note": "unknown"}}),
        json!({"inspector": {"shown": "note"}}),
    ] {
        let outcome = set(&mut store, id, "layout", bad.clone(), next());
        assert!(is_rejected(&outcome), "{bad}: {outcome:?}");
    }
    let habit = json!({"inspector": {"shown": ["note", "repeat", "planned", "reminders"], "note": "rendered"}});
    assert!(is_applied(&set(&mut store, id, "layout", habit, next())));
    match store.get(id).unwrap() {
        Some(Object::Scope(s)) => {
            let inspector = s.layout.unwrap().inspector.unwrap();
            assert_eq!(inspector.shown.len(), 4);
            assert_eq!(
                inspector.note,
                Some(samu_store::core::api::NoteMode::Rendered)
            );
        }
        other => panic!("not a scope: {other:?}"),
    }
    assert!(is_applied(&set(
        &mut store,
        id,
        "layout",
        Value::Null,
        next()
    )));
    match store.get(id).unwrap() {
        Some(Object::Scope(s)) => assert_eq!(s.layout, None),
        other => panic!("not a scope: {other:?}"),
    }
}

/// ADR 0094: `own_scope` is set and taken away on a folder, a project and
/// a tag, and `scope_only` follows it; `scope_only` itself is no longer
/// set, and anything but a uuid is refused.
#[test]
fn own_scope_is_set_on_folders_projects_and_tags() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let project = json!({
        "id": Id::new().to_string(),
        "created": "2026-11-11T08:00:00Z",
        "modified": "2026-11-11T08:00:00Z",
        "title": "Gewohnheiten",
        "kind": "single_actions",
        "status": "active",
        "position": 0
    });
    let folder = json!({
        "id": Id::new().to_string(),
        "created": "2026-11-11T08:00:00Z",
        "modified": "2026-11-11T08:00:00Z",
        "title": "Habits",
        "status": "active",
        "position": 0
    });
    let tag = new_tag("habit");
    let mut wall = 1_000;
    let mut next = || {
        wall += 1;
        stamp(wall, "laptop")
    };
    for (kind, value) in [
        (ObjectKind::Project, &project),
        (ObjectKind::Folder, &folder),
        (ObjectKind::Tag, &tag),
    ] {
        assert!(is_applied(&create(&mut store, kind, value.clone(), next())));
    }
    let scope = new_scope("Habits", "only", &[]);
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Scope,
        scope.clone(),
        next()
    )));
    let own = |store: &AccountStore, id| match store.get(id).unwrap() {
        Some(Object::Project(p)) => (p.own_scope, p.scope_only),
        Some(Object::Folder(f)) => (f.own_scope, f.scope_only),
        Some(Object::Tag(t)) => (t.own_scope, t.scope_only),
        other => panic!("not a folder, project or tag: {other:?}"),
    };
    for owner in [id_of(&project), id_of(&folder), id_of(&tag)] {
        assert_eq!(own(&store, owner), (None, false));
        let outcome = set(&mut store, owner, "scope_only", json!(true), next());
        assert!(is_rejected(&outcome), "derived: {outcome:?}");
        for bad in [json!("yes"), json!(1), json!(true)] {
            let outcome = set(&mut store, owner, "own_scope", bad.clone(), next());
            assert!(is_rejected(&outcome), "{bad}: {outcome:?}");
        }
        assert!(is_applied(&set(
            &mut store,
            owner,
            "own_scope",
            json!(id_of(&scope)),
            next()
        )));
        assert_eq!(own(&store, owner), (Some(id_of(&scope)), true));
        assert!(is_applied(&set(
            &mut store,
            owner,
            "own_scope",
            Value::Null,
            next()
        )));
        assert_eq!(own(&store, owner), (None, false));
    }
}

/// ADR 0059 §2, 0060 §3: a folder's or project's icon is set and taken
/// away, an empty or too long one is refused; a scope's colour only from
/// the palette.
#[test]
fn an_icon_and_a_scope_colour_are_checked() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let project = json!({
        "id": Id::new().to_string(),
        "created": "2026-11-10T08:00:00Z",
        "modified": "2026-11-10T08:00:00Z",
        "title": "Haus",
        "kind": "parallel",
        "status": "active",
        "position": 0
    });
    let folder = json!({
        "id": Id::new().to_string(),
        "created": "2026-11-10T08:00:00Z",
        "modified": "2026-11-10T08:00:00Z",
        "title": "Privat",
        "status": "active",
        "position": 0
    });
    let scope = new_scope("wrk", "only", &[]);
    let mut wall = 1_000;
    let mut next = || {
        wall += 1;
        stamp(wall, "laptop")
    };
    for (kind, value) in [
        (ObjectKind::Project, &project),
        (ObjectKind::Folder, &folder),
        (ObjectKind::Scope, &scope),
    ] {
        assert!(is_applied(&create(&mut store, kind, value.clone(), next())));
    }
    for owner in [id_of(&project), id_of(&folder)] {
        // 33 bytes is one too many; a family of four with ZWJ (25) fits.
        for bad in [json!(""), json!("a".repeat(33)), json!(1)] {
            let outcome = set(&mut store, owner, "icon", bad.clone(), next());
            assert!(is_rejected(&outcome), "{bad}: {outcome:?}");
        }
        assert!(is_applied(&set(
            &mut store,
            owner,
            "icon",
            json!("👨‍👩‍👧‍👦"),
            next()
        )));
        assert!(is_applied(&set(
            &mut store,
            owner,
            "icon",
            json!("🏠"),
            next()
        )));
    }
    match store.get(id_of(&project)).unwrap() {
        Some(Object::Project(p)) => assert_eq!(p.icon.as_deref(), Some("🏠")),
        other => panic!("not a project: {other:?}"),
    }
    match store.get(id_of(&folder)).unwrap() {
        Some(Object::Folder(f)) => assert_eq!(f.icon.as_deref(), Some("🏠")),
        other => panic!("not a folder: {other:?}"),
    }
    assert!(is_applied(&set(
        &mut store,
        id_of(&project),
        "icon",
        Value::Null,
        next()
    )));
    match store.get(id_of(&project)).unwrap() {
        Some(Object::Project(p)) => assert_eq!(p.icon, None),
        other => panic!("not a project: {other:?}"),
    }

    let id = id_of(&scope);
    for bad in [json!("unknown"), json!("black"), json!("#ff0000")] {
        let outcome = set(&mut store, id, "color", bad.clone(), next());
        assert!(is_rejected(&outcome), "{bad}: {outcome:?}");
    }
    assert!(is_applied(&set(
        &mut store,
        id,
        "color",
        json!("teal"),
        next()
    )));
    match store.get(id).unwrap() {
        Some(Object::Scope(s)) => {
            assert_eq!(s.color, Some(samu_store::core::api::ScopeColor::Teal));
        }
        other => panic!("not a scope: {other:?}"),
    }
}

/// Every fixture goes in through `create` and comes back out of `get` as the
/// same JSON, except `modified`, which the store sets.
#[test]
fn every_fixture_survives_the_round_trip_through_the_database() {
    let mut store = AccountStore::open_in_memory().unwrap();
    // The full task carries a second tag the fixtures do not define.
    let mut second_tag = new_tag("B");
    second_tag["id"] = json!("01936f5e-8a00-7c3e-9b1a-3f2d4c5b6d02");

    let order: Vec<(ObjectKind, Value)> = vec![
        (ObjectKind::Folder, fixture("folder/minimal.json")),
        (ObjectKind::Folder, fixture("folder/full.json")),
        (ObjectKind::Folder, fixture("folder/icon.json")),
        (ObjectKind::Folder, fixture("folder/scope_only.json")),
        (ObjectKind::Tag, fixture("tag/group.json")),
        (ObjectKind::Tag, fixture("tag/full.json")),
        (ObjectKind::Tag, fixture("tag/someday.json")),
        (ObjectKind::Tag, fixture("tag/workflow_group.json")),
        (ObjectKind::Tag, fixture("tag/workflow_state.json")),
        (ObjectKind::Tag, fixture("tag/scope_only.json")),
        (ObjectKind::Tag, second_tag),
        (ObjectKind::Project, fixture("project/full.json")),
        (ObjectKind::Project, fixture("project/minimal.json")),
        (ObjectKind::Project, fixture("project/completed.json")),
        (ObjectKind::Project, fixture("project/properties.json")),
        (ObjectKind::Project, fixture("project/workflow.json")),
        (ObjectKind::Project, fixture("project/icon.json")),
        (ObjectKind::Project, fixture("project/scope_only.json")),
        (ObjectKind::Task, fixture("task/action_group.json")),
        (ObjectKind::Task, fixture("task/inbox.json")),
        (ObjectKind::Task, fixture("task/done.json")),
        (ObjectKind::Task, fixture("task/full.json")),
        (ObjectKind::Task, fixture("task/properties.json")),
        (ObjectKind::Task, fixture("task/priority.json")),
        (ObjectKind::Task, fixture("task/habit.json")),
        (ObjectKind::HabitEntry, fixture("habit_entry/done.json")),
        (ObjectKind::HabitEntry, fixture("habit_entry/pause.json")),
        (ObjectKind::Link, fixture("link/ref.json")),
        (ObjectKind::Link, fixture("link/blocked_by.json")),
        (ObjectKind::Clock, fixture("clock/done.json")),
        (ObjectKind::Clock, fixture("clock/running.json")),
        (ObjectKind::Reminder, fixture("reminder/at.json")),
        (ObjectKind::Reminder, fixture("reminder/before_due.json")),
        (ObjectKind::Scope, fixture("scope/only.json")),
        (ObjectKind::Scope, fixture("scope/except.json")),
        (ObjectKind::Scope, fixture("scope/empty.json")),
        (ObjectKind::Scope, fixture("scope/second_round.json")),
        (ObjectKind::Scope, fixture("scope/priority.json")),
        (ObjectKind::Scope, fixture("scope/color.json")),
        (ObjectKind::Scope, fixture("scope/layout.json")),
        (ObjectKind::Scope, fixture("scope/series.json")),
        (ObjectKind::Template, fixture("template/habit.json")),
        (ObjectKind::Template, fixture("template/minimal.json")),
        (ObjectKind::Perspective, fixture("perspective/minimal.json")),
        (ObjectKind::Perspective, fixture("perspective/full.json")),
        (ObjectKind::Task, fixture("task/by_agent.json")),
        (ObjectKind::Project, fixture("project/by_human.json")),
        (ObjectKind::Comment, fixture("comment/by_agent.json")),
        (ObjectKind::Comment, fixture("comment/before_authors.json")),
        (ObjectKind::Attachment, fixture("attachment/photo.json")),
        (ObjectKind::Attachment, fixture("attachment/recording.json")),
    ];
    // The files of the attachments, recorded as the blob route would.
    for (kind, value) in &order {
        if *kind == ObjectKind::Attachment {
            let (hash, size) = (value["blob"].as_str().unwrap(), value["size"].as_u64());
            let media_type = value["media_type"].as_str().unwrap();
            store
                .add_blob(hash, size.unwrap(), media_type, now())
                .unwrap();
        }
    }
    // The author is the session's: written through one of the fixture's
    // author where it has one, else through a person on the laptop.
    for (i, (kind, value)) in order.iter().enumerate() {
        let wall = 1_000 + i64::try_from(i).unwrap();
        let author = match value.get("created_by") {
            Some(by) => serde_json::from_value(by.clone()).unwrap(),
            None => person(),
        };
        let change = Change::Create {
            object: id_of(value),
            kind: *kind,
            value: value.clone(),
            hlc: stamp(wall, "pixel"),
        };
        let outcome = store.apply(&change, &author, now()).unwrap();
        assert!(is_applied(&outcome), "{}: {outcome:?}", value["id"]);
    }

    // Reading all at once (`live_after`, behind `snapshot` and `/changes`)
    // gives each object exactly as `get` does, the account's own settings
    // among them; a deleted one is gone, and a later change number keeps
    // only what changed after it.
    let all = store.live_after(0).unwrap();
    assert_eq!(all.len(), order.len() + 1, "the fixtures and the settings");
    for object in &all {
        assert_eq!(Some(object), store.get(object.id()).unwrap().as_ref());
    }
    let seq = store.seq().unwrap();
    // Something nothing points at: a folder or tag still in use is not
    // deleted plainly (ADR 0076 §2).
    let first = id_of(
        &order
            .iter()
            .find(|(kind, _)| *kind == ObjectKind::Template)
            .unwrap()
            .1,
    );
    let last = id_of(&order[order.len() - 1].1);
    let gone = Change::Delete {
        object: first,
        hlc: stamp(5_000, "pixel"),
    };
    assert!(is_applied(&store.apply(&gone, &person(), now()).unwrap()));
    assert!(store.live_after(0).unwrap().iter().all(|o| o.id() != first));
    assert!(is_applied(&set(
        &mut store,
        last,
        "name",
        json!("tafel2.jpg"),
        stamp(5_001, "pixel")
    )));
    let later: Vec<Id> = store
        .live_after(seq)
        .unwrap()
        .iter()
        .map(Object::id)
        .collect();
    assert_eq!(later, vec![last]);
    // Put back what the checks above changed, for the round trip below.
    let order: Vec<_> = order
        .into_iter()
        .filter(|(_, value)| id_of(value) != first)
        .collect();
    assert!(is_applied(&set(
        &mut store,
        last,
        "name",
        order[order.len() - 1].1["name"].clone(),
        stamp(5_002, "pixel")
    )));

    for (kind, value) in order {
        let back = store.get(id_of(&value)).unwrap().unwrap();
        let mut back = Value::Object(back.to_json().unwrap());
        assert_eq!(back["modified"], json!(now().to_string()));
        back["modified"] = value["modified"].clone();
        let authored = matches!(
            kind,
            ObjectKind::Task
                | ObjectKind::Project
                | ObjectKind::Comment
                | ObjectKind::Attachment
                | ObjectKind::HabitEntry
        );
        if authored && value.get("created_by").is_none() {
            assert_eq!(
                back["created_by"],
                json!({"device": "laptop", "kind": "human"}),
                "{}",
                value["id"]
            );
            back.as_object_mut().unwrap().remove("created_by");
        }
        assert_eq!(back, value);
    }
}

#[test]
fn a_later_change_wins_and_an_earlier_one_is_superseded() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let task = new_task("Milch");
    let id = id_of(&task);
    create(&mut store, ObjectKind::Task, task, stamp(1_000, "pixel"));

    assert!(is_applied(&set(
        &mut store,
        id,
        "title",
        json!("Hafermilch"),
        stamp(3_000, "laptop")
    )));
    let late = set(
        &mut store,
        id,
        "title",
        json!("Sojamilch"),
        stamp(2_000, "pixel"),
    );
    assert_eq!(
        late,
        ChangeOutcome::Superseded {
            by: stamp(3_000, "laptop")
        }
    );
    assert_eq!(task_of(&store, id).title, "Hafermilch");
}

#[test]
fn the_same_change_twice_is_answered_the_same() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let task = new_task("Milch");
    let id = id_of(&task);
    let created = create(
        &mut store,
        ObjectKind::Task,
        task.clone(),
        stamp(1_000, "pixel"),
    );
    assert_eq!(
        create(&mut store, ObjectKind::Task, task, stamp(1_000, "pixel")),
        created
    );

    let first = set(
        &mut store,
        id,
        "flagged",
        json!(true),
        stamp(2_000, "pixel"),
    );
    let seq = store.seq().unwrap();
    let again = set(
        &mut store,
        id,
        "flagged",
        json!(true),
        stamp(2_000, "pixel"),
    );
    assert_eq!(again, first);
    assert_eq!(
        store.seq().unwrap(),
        seq,
        "a repeat spends no change number"
    );
}

#[test]
fn two_devices_in_the_same_millisecond_are_decided_by_the_device() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let task = new_task("Milch");
    let id = id_of(&task);
    create(&mut store, ObjectKind::Task, task, stamp(1_000, "pixel"));
    // Arriving in either order gives the same result.
    set(
        &mut store,
        id,
        "title",
        json!("von pixel"),
        stamp(2_000, "pixel"),
    );
    set(
        &mut store,
        id,
        "title",
        json!("von laptop"),
        stamp(2_000, "laptop"),
    );
    assert_eq!(task_of(&store, id).title, "von pixel");
}

/// Each tag on a task has its own stamp; whatever order the changes arrive
/// in, the newest wins, and a removal is not undone by an older add.
#[test]
fn tags_converge_whatever_the_order_of_arrival() {
    let orders: [[usize; 3]; 4] = [[0, 1, 2], [2, 1, 0], [1, 0, 2], [2, 0, 1]];
    for order in orders {
        let mut store = AccountStore::open_in_memory().unwrap();
        let tag = new_tag("Telefon");
        let tag_id = id_of(&tag);
        create(&mut store, ObjectKind::Tag, tag, stamp(100, "pixel"));
        let task = new_task("anrufen");
        let id = id_of(&task);
        create(&mut store, ObjectKind::Task, task, stamp(100, "pixel"));

        let changes = [
            Change::Add {
                object: id,
                field: "tags".into(),
                element: tag_id,
                position: 0,
                note: None,
                hlc: stamp(1_000, "pixel"),
            },
            Change::Remove {
                object: id,
                field: "tags".into(),
                element: tag_id,
                note: None,
                hlc: stamp(2_000, "laptop"),
            },
            Change::Add {
                object: id,
                field: "tags".into(),
                element: tag_id,
                position: 3,
                note: None,
                hlc: stamp(3_000, "pixel"),
            },
        ];
        for i in order {
            store.apply(&changes[i], &person(), now()).unwrap();
        }
        let tags = task_of(&store, id).tags;
        assert_eq!(tags.len(), 1, "order {order:?}");
        assert_eq!(tags[0].position, 3, "order {order:?}");
    }

    // Without the last add, the removal holds even if it arrives first.
    let mut store = AccountStore::open_in_memory().unwrap();
    let tag = new_tag("Telefon");
    let tag_id = id_of(&tag);
    create(&mut store, ObjectKind::Tag, tag, stamp(100, "pixel"));
    let task = new_task("anrufen");
    let id = id_of(&task);
    create(&mut store, ObjectKind::Task, task, stamp(100, "pixel"));
    let remove = Change::Remove {
        object: id,
        field: "tags".into(),
        element: tag_id,
        note: None,
        hlc: stamp(2_000, "laptop"),
    };
    let add = Change::Add {
        object: id,
        field: "tags".into(),
        element: tag_id,
        position: 0,
        note: None,
        hlc: stamp(1_000, "pixel"),
    };
    store.apply(&remove, &person(), now()).unwrap();
    assert!(matches!(
        store.apply(&add, &person(), now()).unwrap(),
        ChangeOutcome::Superseded { .. }
    ));
    assert!(task_of(&store, id).tags.is_empty());
}

#[test]
fn what_the_store_does_not_take_is_rejected_and_leaves_no_trace() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let task = new_task("Milch");
    let id = id_of(&task);
    create(&mut store, ObjectKind::Task, task, stamp(1_000, "pixel"));
    let seq = store.seq().unwrap();

    let cases = [
        ("title", json!(42)),
        ("title", json!(null)),
        ("id", json!(Id::new().to_string())),
        ("created", json!("2020-01-01T00:00:00Z")),
        ("tags", json!([])),
        ("no_such_field", json!(1)),
        ("due", json!({"unknown": null})),
        ("due", json!("2026-11-20")),
        (
            "repeat",
            json!({"rule": "FREQ=DAILY", "mode": "from_the_future"}),
        ),
        ("project", json!(Id::new().to_string())),
        ("parent", json!(id.to_string())),
    ];
    for (i, (field, value)) in cases.into_iter().enumerate() {
        let wall = 2_000 + i64::try_from(i).unwrap();
        let outcome = set(&mut store, id, field, value.clone(), stamp(wall, "pixel"));
        assert!(is_rejected(&outcome), "{field} = {value}: {outcome:?}");
    }
    assert_eq!(
        store.seq().unwrap(),
        seq,
        "a refusal spends no change number"
    );
    assert_eq!(task_of(&store, id).title, "Milch");

    let link_kind = Change::Create {
        object: Id::new(),
        kind: ObjectKind::Unknown,
        value: json!({}),
        hlc: stamp(3_000, "pixel"),
    };
    assert!(is_rejected(
        &store.apply(&link_kind, &person(), now()).unwrap()
    ));
    assert!(is_rejected(
        &store.apply(&Change::Unknown, &person(), now()).unwrap()
    ));
}

#[test]
fn a_tombstone_takes_no_further_changes() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let task = new_task("Milch");
    let id = id_of(&task);
    create(
        &mut store,
        ObjectKind::Task,
        task.clone(),
        stamp(1_000, "pixel"),
    );
    let delete = Change::Delete {
        object: id,
        hlc: stamp(2_000, "pixel"),
    };
    let deleted = store.apply(&delete, &person(), now()).unwrap();
    assert!(is_applied(&deleted));
    assert_eq!(
        store.apply(&delete, &person(), now()).unwrap(),
        deleted,
        "deleting twice is one deletion"
    );
    assert!(store.get(id).unwrap().is_none());
    assert!(is_rejected(&set(
        &mut store,
        id,
        "title",
        json!("x"),
        stamp(3_000, "pixel")
    )));
    assert!(is_rejected(&create(
        &mut store,
        ObjectKind::Task,
        task,
        stamp(1_000, "pixel")
    )));
}

#[test]
fn every_change_gets_the_next_number_and_changed_after_finds_it() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let a = new_task("a");
    let b = new_task("b");
    let (a_id, b_id) = (id_of(&a), id_of(&b));
    create(&mut store, ObjectKind::Task, a, stamp(1_000, "pixel"));
    create(&mut store, ObjectKind::Task, b, stamp(1_001, "pixel"));
    let before = store.seq().unwrap();
    // 1 is the settings object made on opening (ADR 0045 §4).
    assert_eq!(before, 3);
    set(&mut store, a_id, "title", json!("A"), stamp(2_000, "pixel"));
    store
        .apply(
            &Change::Delete {
                object: b_id,
                hlc: stamp(2_001, "pixel"),
            },
            &person(),
            now(),
        )
        .unwrap();
    assert!(store.changed_after(0).unwrap().iter().all(|c| c.created));
    let changed = store.changed_after(before).unwrap();
    assert_eq!(changed.len(), 2);
    assert!(
        changed.iter().all(|c| !c.created),
        "changed later, not created"
    );
    assert_eq!(
        (changed[0].id, changed[0].seq, changed[0].deleted),
        (a_id, 4, false)
    );
    assert_eq!(
        (changed[1].id, changed[1].seq, changed[1].deleted),
        (b_id, 5, true)
    );
    assert!(store.changed_after(5).unwrap().is_empty());
}

/// The database itself, not the store's answers: stamps, change numbers and
/// the observed clock are really in the tables.
#[test]
fn the_tables_hold_what_the_answers_claim() {
    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = Dir(std::env::temp_dir().join(format!("samu-store-tables-{}", Id::new())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let path = dir.0.join("account.db");

    let id;
    {
        let mut store = AccountStore::open(&path).unwrap();
        let task = new_task("Milch");
        id = id_of(&task);
        create(&mut store, ObjectKind::Task, task, stamp(1_000, "pixel"));
        set(
            &mut store,
            id,
            "flagged",
            json!(true),
            stamp(5_000, "laptop"),
        );
        set(
            &mut store,
            id,
            "flagged",
            json!(false),
            stamp(4_000, "pixel"),
        );
    }
    let conn = rusqlite::Connection::open(&path).unwrap();
    let (wall, device, seq): (i64, String, i64) = conn
        .query_row(
            "SELECT s.wall, s.device, s.seq FROM stamp s JOIN object o ON o.id = s.object \
             WHERE o.uuid = ?1 AND s.field = 'flagged'",
            [id.uuid().as_bytes().to_vec()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    // 1 is the settings object made on opening (ADR 0045 §4).
    assert_eq!((wall, device.as_str(), seq), (5_000, "laptop", 3));
    let flagged: i64 = conn
        .query_row("SELECT flagged FROM task", [], |r| r.get(0))
        .unwrap();
    assert_eq!(flagged, 1, "the superseded change did not reach the row");
    let (account_seq, observed): (i64, i64) = conn
        .query_row("SELECT seq, observed_wall FROM account", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!((account_seq, observed), (3, 5_000));
}

/// The snapshot is read once and kept: the same one until something is
/// written, and what was written is in the next.
#[test]
fn a_snapshot_is_kept_until_a_write() {
    let titles = |store: &AccountStore| -> Vec<String> {
        let snapshot = store.snapshot().unwrap();
        snapshot.tasks.iter().map(|t| t.title.clone()).collect()
    };
    let mut store = AccountStore::open_in_memory().unwrap();
    let task = new_task("erst");
    let id = id_of(&task);
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Task,
        task,
        stamp(1_000, "pixel")
    )));
    let first = store.snapshot().unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &store.snapshot().unwrap()));
    assert_eq!(titles(&store), ["erst"]);

    // A field, a second object and a deletion each show at once.
    assert!(is_applied(&set(
        &mut store,
        id,
        "title",
        json!("anders"),
        stamp(1_001, "pixel")
    )));
    assert!(!std::sync::Arc::ptr_eq(&first, &store.snapshot().unwrap()));
    assert_eq!(titles(&store), ["anders"]);
    create(
        &mut store,
        ObjectKind::Task,
        new_task("dann"),
        stamp(1_002, "pixel"),
    );
    assert_eq!(titles(&store), ["anders", "dann"]);
    let gone = Change::Delete {
        object: id,
        hlc: stamp(1_003, "pixel"),
    };
    assert!(is_applied(&store.apply(&gone, &person(), now()).unwrap()));
    assert_eq!(titles(&store), ["dann"]);
    // The one an earlier reader holds stays as it was.
    assert_eq!(first.tasks[0].title, "erst");
}

/// From the outbox to availability: what the store holds is what
/// `evaluate` needs.
#[test]
fn a_snapshot_feeds_availability() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let mut project = fixture("project/minimal.json");
    project["kind"] = json!("sequential");
    project["status"] = json!("active");
    let project_id = id_of(&project);
    create(
        &mut store,
        ObjectKind::Project,
        project,
        stamp(1_000, "pixel"),
    );
    let mut first = new_task("erst");
    first["project"] = json!(project_id.to_string());
    let mut second = new_task("dann");
    second["project"] = json!(project_id.to_string());
    second["position"] = json!(1);
    let (first_id, second_id) = (id_of(&first), id_of(&second));
    create(&mut store, ObjectKind::Task, first, stamp(1_001, "pixel"));
    create(&mut store, ObjectKind::Task, second, stamp(1_002, "pixel"));

    let snapshot = store.snapshot().unwrap();
    let result = evaluate(&snapshot.snapshot(), now(), &store.settings().unwrap());
    assert!(result.tasks[&first_id].available);
    assert_eq!(
        result.tasks[&second_id].reasons,
        vec![Reason::WaitingInSequence { first: first_id }]
    );

    set(
        &mut store,
        first_id,
        "completed_at",
        json!("2026-11-07T08:30:00Z"),
        stamp(2_000, "pixel"),
    );
    let snapshot = store.snapshot().unwrap();
    let result = evaluate(&snapshot.snapshot(), now(), &store.settings().unwrap());
    assert!(result.tasks[&second_id].available);
}

/// ADR 0022 §1: the same delivery twice makes one task, and the delivery
/// row is in the database next to it. Asked of the tables, not the answers.
#[test]
fn a_delivery_makes_its_task_once() {
    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = Dir(std::env::temp_dir().join(format!("samu-store-delivery-{}", Id::new())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let path = dir.0.join("account.db");

    let change = |title: &str| {
        let task = new_task(title);
        Change::Create {
            object: id_of(&task),
            kind: ObjectKind::Task,
            value: task,
            hlc: stamp(1_000, "hook:ring"),
        }
    };
    let first = change("Milch");
    let first_id = match &first {
        Change::Create { object, .. } => *object,
        _ => unreachable!(),
    };
    {
        let mut store = AccountStore::open(&path).unwrap();
        assert_eq!(
            store
                .apply_delivered("pebble-index", "rec-1", &first, &person(), now())
                .unwrap(),
            Delivered::Created(first_id)
        );
        // Again, as a retry from the app would, even with a fresh task id.
        assert_eq!(
            store
                .apply_delivered("pebble-index", "rec-1", &change("Milch"), &person(), now())
                .unwrap(),
            Delivered::Found(first_id)
        );
        // Another delivery is another task: the probe that must find one.
        assert!(matches!(
            store
                .apply_delivered("pebble-index", "rec-2", &change("Brot"), &person(), now())
                .unwrap(),
            Delivered::Created(_)
        ));
        // A refused object leaves no delivery behind.
        let mut bad = new_task("kaputt");
        bad["title"] = json!(42);
        let bad = Change::Create {
            object: id_of(&bad),
            kind: ObjectKind::Task,
            value: bad,
            hlc: stamp(1_000, "hook:ring"),
        };
        assert!(matches!(
            store
                .apply_delivered("pebble-index", "rec-3", &bad, &person(), now())
                .unwrap(),
            Delivered::Rejected(_)
        ));
    }
    let conn = rusqlite::Connection::open(&path).unwrap();
    let tasks: i64 = conn
        .query_row("SELECT count(*) FROM task", [], |r| r.get(0))
        .unwrap();
    assert_eq!(tasks, 2);
    let deliveries: Vec<(String, Vec<u8>)> = conn
        .prepare("SELECT delivery_id, object FROM delivery ORDER BY delivery_id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(deliveries.len(), 2);
    assert_eq!(deliveries[0].0, "rec-1");
    assert_eq!(deliveries[0].1, first_id.uuid().as_bytes().to_vec());
    assert_eq!(deliveries[1].0, "rec-2");
}

/// ADR 0023 §2: each key has its own stamp; a later write wins per key, a
/// removal stays, and what is not a key or not a string is refused.
#[test]
fn properties_are_decided_key_by_key() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let mut task = new_task("Rauchmelder");
    task["properties"] = json!({"org_id": "abc"});
    let id = id_of(&task);
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Task,
        task,
        stamp(1_000, "pixel")
    )));
    assert_eq!(task_of(&store, id).properties["org_id"], "abc");

    // Two devices, two keys: both stay.
    let a = set(
        &mut store,
        id,
        "properties.source",
        json!("org-import"),
        stamp(3_000, "laptop"),
    );
    let b = set(
        &mut store,
        id,
        "properties.org_id",
        json!("def"),
        stamp(2_000, "pixel"),
    );
    assert!(is_applied(&a) && is_applied(&b), "{a:?} {b:?}");
    let properties = task_of(&store, id).properties;
    assert_eq!(properties["org_id"], "def");
    assert_eq!(properties["source"], "org-import");

    // An older write of the same key loses.
    let old = set(
        &mut store,
        id,
        "properties.source",
        json!("index-ring"),
        stamp(2_500, "pixel"),
    );
    assert!(matches!(old, ChangeOutcome::Superseded { .. }), "{old:?}");

    // A removal stays against an older write.
    assert!(is_applied(&set(
        &mut store,
        id,
        "properties.org_id",
        Value::Null,
        stamp(4_000, "laptop")
    )));
    let older = set(
        &mut store,
        id,
        "properties.org_id",
        json!("ghi"),
        stamp(3_500, "pixel"),
    );
    assert!(
        matches!(older, ChangeOutcome::Superseded { .. }),
        "{older:?}"
    );
    assert!(!task_of(&store, id).properties.contains_key("org_id"));

    // A key written for the first time with a stamp older than the object
    // loses against the creation.
    let before = set(
        &mut store,
        id,
        "properties.fresh",
        json!("x"),
        stamp(500, "pixel"),
    );
    assert!(
        matches!(before, ChangeOutcome::Superseded { .. }),
        "{before:?}"
    );

    for (field, value) in [
        ("properties.Org", json!("x")),
        ("properties.1st", json!("x")),
        ("properties.", json!("x")),
        ("properties.ok", json!(3)),
        ("properties", json!({"ok": "x"})),
    ] {
        let outcome = set(&mut store, id, field, value, stamp(9_000, "pixel"));
        assert!(is_rejected(&outcome), "{field}: {outcome:?}");
    }
    let tag = new_tag("A");
    let tag_id = id_of(&tag);
    create(&mut store, ObjectKind::Tag, tag, stamp(1_001, "pixel"));
    let on_tag = set(
        &mut store,
        tag_id,
        "properties.ok",
        json!("x"),
        stamp(9_000, "pixel"),
    );
    assert!(is_rejected(&on_tag), "{on_tag:?}");

    let mut bad = new_task("bad key");
    bad["properties"] = json!({"Bad Key": "x"});
    let outcome = create(
        &mut store,
        ObjectKind::Task,
        bad.clone(),
        stamp(9_001, "pixel"),
    );
    assert!(is_rejected(&outcome), "{outcome:?}");
    assert!(store.get(id_of(&bad)).unwrap().is_none());
}

/// ADR 0023 §5: a database from before properties gets `source=index-ring`
/// on what the ring delivered, with its creation stamp and a change number
/// of its own. Asked of the tables.
#[test]
fn the_migration_marks_what_the_ring_delivered() {
    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = Dir(std::env::temp_dir().join(format!("samu-store-ring-{}", Id::new())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let path = dir.0.join("account.db");

    let ring = new_task("vom Ring");
    let ring_id = id_of(&ring);
    let other = new_task("von Hand");
    let other_id = id_of(&other);
    {
        let mut store = AccountStore::open(&path).unwrap();
        let change = Change::Create {
            object: ring_id,
            kind: ObjectKind::Task,
            value: ring,
            hlc: stamp(1_000, "hook:ring"),
        };
        store
            .apply_delivered("pebble-index", "rec-1", &change, &person(), now())
            .unwrap();
        create(&mut store, ObjectKind::Task, other, stamp(1_001, "pixel"));
    }
    // Back to schema 2, as a database from before this migration.
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "DROP TABLE habit_entry; ALTER TABLE task DROP COLUMN habit; \
             DROP TABLE perspective; DROP TABLE template; ALTER TABLE scope DROP COLUMN layout; \
             ALTER TABLE folder DROP COLUMN own_scope; \
             ALTER TABLE project DROP COLUMN own_scope; ALTER TABLE tag DROP COLUMN own_scope; \
             ALTER TABLE folder DROP COLUMN scope_only; \
             ALTER TABLE project DROP COLUMN scope_only; ALTER TABLE tag DROP COLUMN scope_only; \
             DROP INDEX logbook_by_at; \
             DROP TABLE appointment_write; ALTER TABLE logbook DROP COLUMN field; \
             DROP TABLE calendar_source; DROP TABLE appointment; DROP TABLE calendar; \
             DELETE FROM stamp WHERE object IN (SELECT object FROM settings); \
             DELETE FROM object WHERE kind = 'settings'; DROP TABLE settings; \
             DROP TABLE attachment; DROP TABLE blob; DROP TABLE comment; \
             ALTER TABLE logbook DROP COLUMN author_kind; \
             DROP TABLE scope_member; DROP TABLE scope; \
             DROP TABLE reminder; ALTER TABLE account DROP COLUMN remind_time; \
             DROP TABLE property; DROP INDEX tag_by_parent; \
             ALTER TABLE tag DROP COLUMN key; ALTER TABLE tag DROP COLUMN ask_note; \
             ALTER TABLE tag DROP COLUMN default_workflow; \
             ALTER TABLE project DROP COLUMN workflow; ALTER TABLE logbook DROP COLUMN device; \
             ALTER TABLE task DROP COLUMN priority; \
             ALTER TABLE folder DROP COLUMN icon; ALTER TABLE project DROP COLUMN icon; \
             DELETE FROM schema_history WHERE version >= 3; PRAGMA user_version = 2;",
        )
        .unwrap();
    }
    let store = AccountStore::open(&path).unwrap();
    assert_eq!(task_of(&store, ring_id).properties["source"], "index-ring");
    assert!(task_of(&store, other_id).properties.is_empty());
    // Before going back: settings 1, ring 2, other 3. On opening again
    // the ring gets 4 and a new settings object 5 (ADR 0045 §4).
    let changed = store.changed_after(3).unwrap();
    assert_eq!(changed.len(), 2);
    assert_eq!((changed[0].id, changed[0].seq), (ring_id, 4));
    drop(store);

    let conn = rusqlite::Connection::open(&path).unwrap();
    let (wall, device, seq): (i64, String, i64) = conn
        .query_row(
            "SELECT wall, device, seq FROM property WHERE key = 'source'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!((wall, device.as_str(), seq), (1_000, "hook:ring", 4));
    let account_seq: i64 = conn
        .query_row("SELECT seq FROM account", [], |r| r.get(0))
        .unwrap();
    assert_eq!(account_seq, 5);
}

// ------------------------------------------------------------- workflows

/// A workflow group `Status` with the states `In Arbeit` and `Wartet`, and
/// a task: their ids.
fn workflow(store: &mut AccountStore) -> (Id, Id, Id, Id) {
    let mut group = new_tag("Status");
    group["exclusive"] = json!(true);
    let group_id = id_of(&group);
    create(store, ObjectKind::Tag, group, stamp(100, "pixel"));
    let mut ids = Vec::new();
    for (i, title) in ["In Arbeit", "Wartet"].into_iter().enumerate() {
        let mut state = new_tag(title);
        state["parent"] = json!(group_id.to_string());
        state["position"] = json!(i);
        ids.push(id_of(&state));
        create(store, ObjectKind::Tag, state, stamp(101, "pixel"));
    }
    let task = new_task("Dach");
    let task_id = id_of(&task);
    create(store, ObjectKind::Task, task, stamp(102, "pixel"));
    (group_id, ids[0], ids[1], task_id)
}

fn add(task: Id, tag: Id, hlc: Hlc, note: Option<&str>) -> Change {
    Change::Add {
        object: task,
        field: "tags".into(),
        element: tag,
        position: 0,
        note: note.map(str::to_string),
        hlc,
    }
}

fn remove(task: Id, tag: Id, hlc: Hlc) -> Change {
    Change::Remove {
        object: task,
        field: "tags".into(),
        element: tag,
        note: None,
        hlc,
    }
}

fn tags_of(store: &AccountStore, id: Id) -> Vec<Id> {
    task_of(store, id).tags.iter().map(|t| t.tag).collect()
}

/// ADR 0024 §2: a group is one register per object. Two devices switching
/// at once, and a later clearing, end the same whatever arrives first.
#[test]
fn a_workflow_state_converges_in_any_order() {
    let orders: [[usize; 3]; 6] = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    for with_remove in [false, true] {
        // Every order ends where it must; each store has its own ids, so
        // the expectation is per store.
        for order in orders {
            let mut store = AccountStore::open_in_memory().unwrap();
            let (_, working, waiting, task) = workflow(&mut store);
            let changes = [
                add(task, working, stamp(1_000, "pixel"), None),
                add(task, waiting, stamp(1_001, "laptop"), None),
                remove(task, waiting, stamp(1_002, "pixel")),
            ];
            for i in order {
                if i == 2 && !with_remove {
                    continue;
                }
                store.apply(&changes[i], &person(), now()).unwrap();
            }
            let expected = if with_remove { vec![] } else { vec![waiting] };
            assert_eq!(
                tags_of(&store, task),
                expected,
                "{order:?} remove={with_remove}"
            );
        }
    }
}

/// The table, not the answer: switching leaves one present row, and the
/// displaced state keeps the stamp that displaced it.
#[test]
fn switching_a_state_removes_the_sibling_in_the_table() {
    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = Dir(std::env::temp_dir().join(format!("samu-store-workflow-{}", Id::new())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let path = dir.0.join("account.db");
    {
        let mut store = AccountStore::open(&path).unwrap();
        let (_, working, waiting, task) = workflow(&mut store);
        store
            .apply(
                &add(task, working, stamp(1_000, "pixel"), None),
                &person(),
                now(),
            )
            .unwrap();
        let outcome = store
            .apply(
                &add(task, waiting, stamp(2_000, "laptop"), None),
                &person(),
                now(),
            )
            .unwrap();
        assert!(is_applied(&outcome), "{outcome:?}");
        // An older switch back loses against the newer one.
        let old = store
            .apply(
                &add(task, working, stamp(1_500, "pixel"), None),
                &person(),
                now(),
            )
            .unwrap();
        assert!(matches!(old, ChangeOutcome::Superseded { .. }), "{old:?}");
    }
    let conn = rusqlite::Connection::open(&path).unwrap();
    let rows: Vec<(i64, i64, String)> = conn
        .prepare("SELECT present, wall, device FROM tagging ORDER BY wall")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![(0, 2_000, "laptop".into()), (1, 2_000, "laptop".into())]
    );
}

/// A tag outside an exclusive group stays a set.
#[test]
fn tags_outside_a_group_are_still_a_set() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let (a, b) = (new_tag("a"), new_tag("b"));
    let (a_id, b_id) = (id_of(&a), id_of(&b));
    create(&mut store, ObjectKind::Tag, a, stamp(100, "pixel"));
    create(&mut store, ObjectKind::Tag, b, stamp(100, "pixel"));
    let task = new_task("x");
    let task_id = id_of(&task);
    create(&mut store, ObjectKind::Task, task, stamp(101, "pixel"));
    store
        .apply(
            &add(task_id, a_id, stamp(1_000, "pixel"), None),
            &person(),
            now(),
        )
        .unwrap();
    store
        .apply(
            &add(task_id, b_id, stamp(1_001, "pixel"), None),
            &person(),
            now(),
        )
        .unwrap();
    assert_eq!(tags_of(&store, task_id).len(), 2);
}

/// ADR 0024 §5: switching, completing and opening again are logged with
/// their note and device; a change sent twice or overtaken is not. Asked
/// of the table as well.
#[test]
fn the_logbook_records_states_and_completion() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let (_, working, waiting, task) = workflow(&mut store);
    let first = add(task, working, stamp(1_000, "pixel"), None);
    store.apply(&first, &person(), now()).unwrap();
    store.apply(&first, &person(), now()).unwrap();
    store
        .apply(
            &add(
                task,
                waiting,
                stamp(2_000, "laptop"),
                Some("Angebot angefragt"),
            ),
            &person(),
            now(),
        )
        .unwrap();
    // Overtaken: no entry.
    store
        .apply(
            &add(task, working, stamp(1_500, "pixel"), None),
            &person(),
            now(),
        )
        .unwrap();
    store
        .apply(
            &remove(task, waiting, stamp(3_000, "pixel")),
            &person(),
            now(),
        )
        .unwrap();
    let done = Change::Set {
        object: task,
        field: "completed_at".into(),
        value: json!("2026-11-07T10:00:00Z"),
        note: Some("erledigt".into()),
        hlc: stamp(4_000, "laptop"),
    };
    store.apply(&done, &person(), now()).unwrap();
    set(
        &mut store,
        task,
        "completed_at",
        Value::Null,
        stamp(5_000, "pixel"),
    );
    // Clearing what was never set is no reopening.
    set(
        &mut store,
        task,
        "dropped_at",
        Value::Null,
        stamp(6_000, "pixel"),
    );

    let log = store.logbook(task).unwrap().unwrap();
    let kinds: Vec<LogKind> = log.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        [
            LogKind::State,
            LogKind::State,
            LogKind::State,
            LogKind::Completed,
            LogKind::Reopened
        ]
    );
    assert_eq!((log[0].old, log[0].new), (None, Some(working)));
    assert_eq!((log[1].old, log[1].new), (Some(working), Some(waiting)));
    assert_eq!(log[1].note.as_deref(), Some("Angebot angefragt"));
    assert_eq!(log[1].device, "laptop");
    assert_eq!((log[2].old, log[2].new), (Some(waiting), None));
    assert_eq!(log[3].note.as_deref(), Some("erledigt"));
    assert!(store.logbook(Id::new()).unwrap().is_none());
}

/// ADR 0024 §3: a project's workflow is an exclusive group.
#[test]
fn a_project_workflow_must_be_an_exclusive_group() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let (group, working, _, _) = workflow(&mut store);
    let mut project = fixture("project/minimal.json");
    project["workflow"] = json!(working.to_string());
    let outcome = create(
        &mut store,
        ObjectKind::Project,
        project.clone(),
        stamp(1_000, "pixel"),
    );
    assert!(is_rejected(&outcome), "{outcome:?}");
    project["workflow"] = json!(group.to_string());
    let outcome = create(
        &mut store,
        ObjectKind::Project,
        project,
        stamp(1_001, "pixel"),
    );
    assert!(is_applied(&outcome), "{outcome:?}");
}

// ------------------------------------------------------------- reminders

fn new_reminder(owner: Id, anchor: &str) -> Value {
    json!({
        "id": Id::new().to_string(),
        "created": "2026-11-07T08:00:00Z",
        "modified": "2026-11-07T08:00:00Z",
        "owner": owner.to_string(),
        "anchor": anchor,
        "before": "PT1H"
    })
}

/// ADR 0093 §1: `daily` comes with its `time` and nothing else; a template
/// may carry it too.
#[test]
fn a_daily_reminder_has_its_time_and_nothing_else() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let task = new_task("Dehnen");
    let task_id = id_of(&task);
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Task,
        task,
        stamp(1, "pixel")
    )));
    let daily = |extra: Value| {
        let mut r = new_reminder(task_id, "daily");
        r.as_object_mut().unwrap().remove("before");
        for (k, v) in extra.as_object().unwrap() {
            r[k] = v.clone();
        }
        r
    };
    let good = daily(json!({"time": "20:00:00"}));
    let good_id = id_of(&good);
    let outcome = create(&mut store, ObjectKind::Reminder, good, stamp(2, "pixel"));
    assert!(is_applied(&outcome), "{outcome:?}");
    let Some(Object::Reminder(made)) = store.get(good_id).unwrap() else {
        panic!("no reminder");
    };
    assert_eq!(made.anchor, ReminderAnchor::Daily);
    assert_eq!(made.time, Some(jiff::civil::time(20, 0, 0, 0)));
    for (n, bad) in [
        daily(json!({})),
        daily(json!({"time": "20:00:00", "before": "PT1H"})),
        daily(json!({"time": "20:00:00", "at": "2026-11-11T08:00:00Z"})),
        {
            let mut r = new_reminder(task_id, "due");
            r["time"] = json!("20:00:00");
            r
        },
    ]
    .into_iter()
    .enumerate()
    {
        let outcome = create(
            &mut store,
            ObjectKind::Reminder,
            bad.clone(),
            stamp(10 + n as i64, "pixel"),
        );
        assert!(is_rejected(&outcome), "{bad}: {outcome:?}");
    }
    let template = json!({
        "id": Id::new().to_string(),
        "created": "2026-11-13T08:00:00Z",
        "modified": "2026-11-13T08:00:00Z",
        "title": "habit",
        "text": "#Gewohnheiten habit:1/d",
        "reminders": [{"anchor": "daily", "time": "20:00:00"}],
        "position": 0
    });
    let outcome = create(
        &mut store,
        ObjectKind::Template,
        template,
        stamp(20, "pixel"),
    );
    assert!(is_applied(&outcome), "{outcome:?}");
}

/// ADR 0030 §1: a reminder belongs to a task or project, its anchor is one
/// of the four, and `at` goes with `anchor = at` only.
#[test]
fn a_reminder_is_checked_like_any_object() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let task = new_task("Steuer");
    let task_id = id_of(&task);
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Task,
        task,
        stamp(1, "pixel")
    )));
    let tag = new_tag("Ort");
    let tag_id = id_of(&tag);
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Tag,
        tag,
        stamp(2, "pixel")
    )));

    let good = new_reminder(task_id, "due");
    let good_id = id_of(&good);
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Reminder,
        good,
        stamp(3, "pixel")
    )));
    let outcome = set(
        &mut store,
        good_id,
        "before",
        json!("PT30M"),
        stamp(4, "pixel"),
    );
    assert!(is_applied(&outcome), "{outcome:?}");

    let on_a_tag = new_reminder(tag_id, "due");
    let outcome = create(
        &mut store,
        ObjectKind::Reminder,
        on_a_tag,
        stamp(5, "pixel"),
    );
    assert!(is_rejected(&outcome), "{outcome:?}");

    let unknown = new_reminder(task_id, "sunrise");
    let outcome = create(&mut store, ObjectKind::Reminder, unknown, stamp(6, "pixel"));
    assert!(is_rejected(&outcome), "{outcome:?}");

    let at_without_time = new_reminder(task_id, "at");
    let outcome = create(
        &mut store,
        ObjectKind::Reminder,
        at_without_time,
        stamp(7, "pixel"),
    );
    assert!(is_rejected(&outcome), "{outcome:?}");

    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.reminders.len(), 1);
    assert_eq!(snapshot.reminders[0].id, good_id);
    assert_eq!(
        snapshot.reminders[0].before.map(|p| p.0.fieldwise()),
        Some(jiff::Span::new().minutes(30).fieldwise())
    );
}

fn member(scope: Id, element: Id, add: bool, hlc: Hlc) -> Change {
    if add {
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
}

fn new_scope(title: &str, mode: &str, members: &[Id]) -> Value {
    json!({
        "id": Id::new().to_string(),
        "created": "2026-11-08T08:00:00Z",
        "modified": "2026-11-08T08:00:00Z",
        "title": title,
        "mode": mode,
        "members": members.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "position": 0
    })
}

fn members_of(store: &AccountStore, id: Id) -> Vec<Id> {
    match store.get(id).unwrap() {
        Some(Object::Scope(scope)) => scope.members,
        other => panic!("not a scope: {other:?}"),
    }
}

/// ADR 0035 §1: members are a set like tags; a removal with a later stamp
/// holds against an older add, whatever arrives first.
#[test]
fn scope_members_converge_whatever_the_order_of_arrival() {
    for removal_first in [false, true] {
        let mut store = AccountStore::open_in_memory().unwrap();
        let tag = new_tag("IT");
        let tag_id = id_of(&tag);
        create(&mut store, ObjectKind::Tag, tag, stamp(1_000, "pixel"));
        let scope = new_scope("Privat ohne IT", "except", &[]);
        let scope_id = id_of(&scope);
        assert!(is_applied(&create(
            &mut store,
            ObjectKind::Scope,
            scope,
            stamp(1_001, "pixel")
        )));
        let add = member(scope_id, tag_id, true, stamp(2_000, "laptop"));
        let removal = member(scope_id, tag_id, false, stamp(3_000, "pixel"));
        let (first, second) = if removal_first {
            (&removal, &add)
        } else {
            (&add, &removal)
        };
        store.apply(first, &person(), now()).unwrap();
        store.apply(second, &person(), now()).unwrap();
        assert!(members_of(&store, scope_id).is_empty(), "{removal_first}");
        // A later add brings it back.
        let again = member(scope_id, tag_id, true, stamp(4_000, "laptop"));
        assert!(is_applied(&store.apply(&again, &person(), now()).unwrap()));
        assert_eq!(members_of(&store, scope_id), vec![tag_id]);
    }
}

/// ADR 0035 §1/§2: only folders, projects and tags become members; one that
/// is deleted later does not block a rename, and the mode takes no value
/// this server does not know.
#[test]
fn what_a_scope_does_not_take_is_rejected() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let task = new_task("Milch");
    let task_id = id_of(&task);
    create(&mut store, ObjectKind::Task, task, stamp(1_000, "pixel"));
    let tag = new_tag("IT");
    let tag_id = id_of(&tag);
    create(&mut store, ObjectKind::Tag, tag, stamp(1_001, "pixel"));

    let with_task = new_scope("falsch", "only", &[task_id]);
    let outcome = create(
        &mut store,
        ObjectKind::Scope,
        with_task,
        stamp(1_002, "pixel"),
    );
    assert!(is_rejected(&outcome), "{outcome:?}");
    let unknown = new_scope("neu", "from_the_future", &[]);
    let outcome = create(
        &mut store,
        ObjectKind::Scope,
        unknown,
        stamp(1_003, "pixel"),
    );
    assert!(is_rejected(&outcome), "{outcome:?}");

    let scope = new_scope("Privat", "only", &[tag_id]);
    let scope_id = id_of(&scope);
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Scope,
        scope,
        stamp(1_004, "pixel")
    )));
    let outcome = store
        .apply(
            &member(scope_id, task_id, true, stamp(1_005, "pixel")),
            &person(),
            now(),
        )
        .unwrap();
    assert!(is_rejected(&outcome), "{outcome:?}");
    let outcome = store
        .apply(
            &member(task_id, tag_id, true, stamp(1_006, "pixel")),
            &person(),
            now(),
        )
        .unwrap();
    assert!(is_rejected(&outcome), "members on a task: {outcome:?}");

    // A member of a scope is not deleted plainly (ADR 0076 §2); the scope
    // goes on as it was.
    let delete = Change::Delete {
        object: tag_id,
        hlc: stamp(1_007, "pixel"),
    };
    let outcome = store.apply(&delete, &person(), now()).unwrap();
    assert!(
        matches!(&outcome, ChangeOutcome::Rejected { reason } if reason.contains("1 scopes")),
        "{outcome:?}"
    );
    let outcome = set(
        &mut store,
        scope_id,
        "title",
        json!("Zuhause"),
        stamp(1_008, "pixel"),
    );
    assert!(is_applied(&outcome), "{outcome:?}");
    let outcome = set(
        &mut store,
        scope_id,
        "mode",
        json!("from_the_future"),
        stamp(1_009, "pixel"),
    );
    assert!(is_rejected(&outcome), "{outcome:?}");
    let outcome = set(
        &mut store,
        scope_id,
        "members",
        json!([]),
        stamp(1_010, "pixel"),
    );
    assert!(is_rejected(&outcome), "members is a set: {outcome:?}");
    let snapshot = store.snapshot().unwrap();
    assert_eq!(snapshot.scopes.len(), 1);
    assert_eq!(snapshot.scopes[0].title, "Zuhause");
}

/// Asked of the database: the members are rows with their own stamps.
#[test]
fn scope_members_are_rows_with_their_own_stamp() {
    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = Dir(std::env::temp_dir().join(format!("samu-store-scope-{}", Id::new())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let path = dir.0.join("account.db");
    let (a, b) = (new_tag("IT"), new_tag("Haus"));
    {
        let mut store = AccountStore::open(&path).unwrap();
        create(
            &mut store,
            ObjectKind::Tag,
            a.clone(),
            stamp(1_000, "pixel"),
        );
        create(
            &mut store,
            ObjectKind::Tag,
            b.clone(),
            stamp(1_001, "pixel"),
        );
        let scope = new_scope("Privat", "only", &[id_of(&a)]);
        let scope_id = id_of(&scope);
        create(&mut store, ObjectKind::Scope, scope, stamp(1_002, "pixel"));
        store
            .apply(
                &member(scope_id, id_of(&b), true, stamp(2_000, "laptop")),
                &person(),
                now(),
            )
            .unwrap();
        store
            .apply(
                &member(scope_id, id_of(&a), false, stamp(3_000, "pixel")),
                &person(),
                now(),
            )
            .unwrap();
    }
    let conn = rusqlite::Connection::open(&path).unwrap();
    let mut statement = conn
        .prepare("SELECT member, present, wall, device FROM scope_member ORDER BY wall")
        .unwrap();
    let rows: Vec<(Vec<u8>, i64, i64, String)> = statement
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let blob = |v: &Value| id_of(v).uuid().as_bytes().to_vec();
    assert_eq!(
        rows,
        vec![
            (blob(&b), 1, 2_000, "laptop".to_string()),
            (blob(&a), 0, 3_000, "pixel".to_string()),
        ]
    );
    let (title, mode): (String, String) = conn
        .query_row("SELECT title, mode FROM scope", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!((title.as_str(), mode.as_str()), ("Privat", "only"));
}

/// A batch is all or nothing (ADR 0036 §1): a refused change at the end
/// takes back the changes before it.
#[test]
fn a_batch_applies_whole_or_not_at_all() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let task = new_task("Milch");
    let id = id_of(&task);
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Task,
        task,
        stamp(1, "a")
    )));
    let rename = Change::Set {
        object: id,
        field: "title".into(),
        value: json!("Hafermilch"),
        note: None,
        hlc: stamp(2, "a"),
    };
    let nowhere = Change::Delete {
        object: Id::new(),
        hlc: stamp(3, "a"),
    };

    let refused = store
        .apply_all(&[rename.clone(), nowhere], &person(), now())
        .unwrap()
        .unwrap_err();
    assert_eq!(refused.0, 1);
    assert!(is_rejected(&refused.1));
    assert_eq!(
        task_of(&store, id).title,
        "Milch",
        "the rename is taken back"
    );

    let done = store
        .apply_all(
            &[
                rename,
                Change::Delete {
                    object: id,
                    hlc: stamp(4, "a"),
                },
            ],
            &person(),
            now(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(done.len(), 2);
    assert!(store.get(id).unwrap().is_none(), "the probe deletes it");
}

/// ADR 0037 §2: a folder or tag never goes below itself; elsewhere it may.
#[test]
fn a_folder_or_tag_never_goes_below_itself() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let folder = |title: &str, parent: Option<Id>| {
        let mut value = json!({"id": Id::new(), "created": "2026-11-08T08:00:00Z",
            "modified": "2026-11-08T08:00:00Z", "title": title, "status": "active",
            "position": 0});
        if let Some(parent) = parent {
            value["parent"] = json!(parent);
        }
        value
    };
    let top = folder("Haus", None);
    let top_id = id_of(&top);
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Folder,
        top,
        stamp(1, "a")
    )));
    let child = folder("Garten", Some(top_id));
    let child_id = id_of(&child);
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Folder,
        child,
        stamp(2, "a")
    )));
    let grand = folder("Beete", Some(child_id));
    let grand_id = id_of(&grand);
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Folder,
        grand,
        stamp(3, "a")
    )));
    let other = folder("GTD", None);
    let other_id = id_of(&other);
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Folder,
        other,
        stamp(4, "a")
    )));

    let under_grandchild = set(&mut store, top_id, "parent", json!(grand_id), stamp(5, "a"));
    assert!(is_rejected(&under_grandchild), "{under_grandchild:?}");
    let under_itself = set(&mut store, top_id, "parent", json!(top_id), stamp(6, "a"));
    assert!(is_rejected(&under_itself));
    let elsewhere = set(&mut store, top_id, "parent", json!(other_id), stamp(7, "a"));
    assert!(is_applied(&elsewhere), "the probe moves it: {elsewhere:?}");

    let a = new_tag("Orte");
    let a_id = id_of(&a);
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Tag,
        a,
        stamp(8, "a")
    )));
    let mut b = new_tag("Telefon");
    b["parent"] = json!(a_id);
    let b_id = id_of(&b);
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Tag,
        b,
        stamp(9, "a")
    )));
    assert!(is_rejected(&set(
        &mut store,
        a_id,
        "parent",
        json!(b_id),
        stamp(10, "a")
    )));
    assert!(is_applied(&set(
        &mut store,
        b_id,
        "title",
        json!("Anrufe"),
        stamp(11, "a")
    )));
}

/// ADR 0043: the fields of the second round are checked when they are
/// written; a later deletion of what they name blocks no rename.
#[test]
fn a_scope_takes_only_filters_captures_and_windows_it_can_read() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let tag = new_tag("Warten");
    let tag_id = id_of(&tag);
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Tag,
        tag,
        stamp(1_000, "pixel")
    )));
    let mut scope = fixture("scope/empty.json");
    for field in ["filter", "capture", "schedule"] {
        scope.as_object_mut().unwrap().remove(field);
    }
    let scope_id = id_of(&scope);
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Scope,
        scope,
        stamp(1_001, "pixel")
    )));

    let good =
        json!([{"kind": "due", "value": "7t"}, {"kind": "tag", "value": tag_id, "not": true}]);
    assert!(is_applied(&set(
        &mut store,
        scope_id,
        "filter",
        good.clone(),
        stamp(1_002, "pixel")
    )));
    let refused = [
        ("filter", json!([{"kind": "from_the_future", "value": "x"}])),
        ("filter", json!([{"kind": "due", "value": "bald"}])),
        ("filter", json!([{"kind": "estimate", "value": "<15x"}])),
        ("filter", json!([{"kind": "priority", "value": "d"}])),
        ("filter", json!([{"kind": "tag", "value": "not-an-id"}])),
        ("filter", json!([{"kind": "folder", "value": tag_id}])),
        ("capture", json!({"project": tag_id})),
        (
            "schedule",
            json!([{"days": [], "from": "08:00:00", "to": "17:00:00"}]),
        ),
        (
            "schedule",
            json!([{"days": ["xx"], "from": "08:00:00", "to": "17:00:00"}]),
        ),
    ];
    for (i, (field, value)) in refused.into_iter().enumerate() {
        let wall = 1_010 + i64::try_from(i).unwrap();
        let outcome = set(
            &mut store,
            scope_id,
            field,
            value.clone(),
            stamp(wall, "pixel"),
        );
        assert!(is_rejected(&outcome), "{field} {value}: {outcome:?}");
    }
    let night = json!([{"days": ["sa"], "from": "22:00:00", "to": "02:00:00"}]);
    assert!(is_applied(&set(
        &mut store,
        scope_id,
        "schedule",
        night.clone(),
        stamp(1_020, "pixel")
    )));
    assert!(is_applied(&set(
        &mut store,
        scope_id,
        "capture",
        json!({"tags": [tag_id]}),
        stamp(1_021, "pixel")
    )));

    // The tag goes; the scope can still be renamed, and keeps its filter.
    let gone = Change::Delete {
        object: tag_id,
        hlc: stamp(1_030, "pixel"),
    };
    assert!(is_applied(&store.apply(&gone, &person(), now()).unwrap()));
    assert!(is_applied(&set(
        &mut store,
        scope_id,
        "title",
        json!("Neu"),
        stamp(1_031, "pixel")
    )));
    let Some(Object::Scope(back)) = store.get(scope_id).unwrap() else {
        panic!("the scope is gone");
    };
    assert_eq!(back.title, "Neu");
    assert_eq!(serde_json::to_value(&back.filter).unwrap(), good);
    assert_eq!(serde_json::to_value(&back.schedule).unwrap(), night);
    assert_eq!(back.capture.unwrap().tags, vec![tag_id]);
}

/// ADR 0054 §3: the templates of the settings are written per key like
/// properties, live in the table `property` under the settings object, and
/// only the settings take them.
#[test]
fn property_links_live_at_the_settings_key_by_key() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let settings = store.snapshot().unwrap().settings.clone().unwrap().id;
    let jira = "https://jira.example.org/browse/{value}";
    // The settings carry the stamp of their making, at the real time.
    let base = Timestamp::now().as_millisecond() + 60_000;
    let at = |ms: i64, device: &str| stamp(base + ms, device);
    assert!(is_applied(&set(
        &mut store,
        settings,
        "property_links.jira",
        json!(jira),
        at(2_000, "laptop")
    )));
    assert!(is_applied(&set(
        &mut store,
        settings,
        "property_links.ticket",
        json!("https://t.example.org/{value}"),
        at(2_100, "pixel")
    )));
    let old = set(
        &mut store,
        settings,
        "property_links.jira",
        json!("https://old.example.org/{value}"),
        at(1_500, "pixel"),
    );
    assert!(matches!(old, ChangeOutcome::Superseded { .. }), "{old:?}");
    assert!(is_applied(&set(
        &mut store,
        settings,
        "property_links.ticket",
        Value::Null,
        at(3_000, "laptop")
    )));
    let links = store
        .snapshot()
        .unwrap()
        .settings
        .clone()
        .unwrap()
        .property_links;
    assert_eq!(links.len(), 1, "{links:?}");
    assert_eq!(links["jira"], jira);

    // A task takes no property links, the settings no properties.
    let task = new_task("Rinne");
    let id = id_of(&task);
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Task,
        task,
        at(4_000, "pixel")
    )));
    assert!(is_rejected(&set(
        &mut store,
        id,
        "property_links.jira",
        json!(jira),
        at(5_000, "pixel")
    )));
    assert!(is_rejected(&set(
        &mut store,
        settings,
        "properties.jira",
        json!("x"),
        at(5_000, "pixel")
    )));
    assert!(task_of(&store, id).properties.is_empty());
}

/// A slash in a tag title would make a path like `Ort/Büro` mean two tags
/// (ADR 0027 §3): the server refuses it on create and on a rename, from
/// any surface.
#[test]
fn a_tag_title_with_a_slash_is_refused() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let outcome = create(
        &mut store,
        ObjectKind::Tag,
        new_tag("Ort/Büro"),
        stamp(100, "pixel"),
    );
    assert!(
        matches!(outcome, ChangeOutcome::Rejected { .. }),
        "{outcome:?}"
    );
    assert!(store.snapshot().unwrap().tags.is_empty());

    let tag = new_tag("Büro");
    let id = id_of(&tag);
    create(&mut store, ObjectKind::Tag, tag, stamp(200, "pixel"));
    let outcome = set(
        &mut store,
        id,
        "title",
        json!("Ort/Büro"),
        stamp(300, "pixel"),
    );
    assert!(
        matches!(&outcome, ChangeOutcome::Rejected { reason } if reason.starts_with("title:")),
        "{outcome:?}"
    );
    let outcome = set(
        &mut store,
        id,
        "title",
        json!("Büro 2"),
        stamp(400, "pixel"),
    );
    assert!(
        matches!(outcome, ChangeOutcome::Applied { .. }),
        "{outcome:?}"
    );
    assert_eq!(store.snapshot().unwrap().tags[0].title, "Büro 2");
}

/// ADR 0080 §4: the logbook over a span of time, of every object, with
/// the object it is on; the span's end not included.
#[test]
fn the_logbook_reads_by_time_across_objects() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let a = new_task("Altpapier");
    let b = new_task("Rasen");
    let (a_id, b_id) = (id_of(&a), id_of(&b));
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Task,
        a,
        stamp(1_000, "laptop")
    )));
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Task,
        b,
        stamp(1_001, "laptop")
    )));
    let at = |text: &str| -> Timestamp { text.parse().unwrap() };
    let complete = |store: &mut AccountStore, id: Id, wall: i64, when: &str| {
        let change = Change::Set {
            object: id,
            field: "completed_at".into(),
            value: json!("2026-11-07T08:30:00Z"),
            note: None,
            hlc: stamp(wall, "laptop"),
        };
        store.apply(&change, &person(), at(when)).unwrap()
    };
    assert!(is_applied(&complete(
        &mut store,
        a_id,
        2_000,
        "2026-11-07T09:00:00Z"
    )));
    // A fraction of a second sorts as text before the whole second.
    assert!(is_applied(&complete(
        &mut store,
        b_id,
        2_001,
        "2026-11-08T00:00:00.5Z"
    )));

    let day = store
        .logbook_between(at("2026-11-07T00:00:00Z"), at("2026-11-08T00:00:00Z"))
        .unwrap();
    assert_eq!(day.len(), 1);
    assert_eq!((day[0].object, day[0].kind), (a_id, LogKind::Completed));
    assert_eq!(
        day[0].author_kind,
        Some(samu_store::core::api::AuthorKind::Human)
    );
    let next = store
        .logbook_between(at("2026-11-08T00:00:00Z"), at("2026-11-09T00:00:00Z"))
        .unwrap();
    assert_eq!(next.iter().map(|e| e.object).collect::<Vec<_>>(), [b_id]);
    assert!(
        store
            .logbook_between(at("2026-11-09T00:00:00Z"), at("2026-11-10T00:00:00Z"))
            .unwrap()
            .is_empty()
    );
}

/// ADR 0080 §1: the field stamps of a span, the last per field, without the
/// server's own or a creation's.
#[test]
fn field_stamps_read_by_time_without_the_server() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let task = new_task("Steuer");
    let id = id_of(&task);
    let at = |ms: i64| Timestamp::from_millisecond(ms).unwrap();
    assert!(is_applied(&create(
        &mut store,
        ObjectKind::Task,
        task,
        stamp(1_000, "laptop")
    )));
    assert!(is_applied(&set(
        &mut store,
        id,
        "title",
        json!("Steuer 2026"),
        stamp(2_000, "pixel")
    )));
    assert!(is_applied(&set(
        &mut store,
        id,
        "flagged",
        json!(true),
        stamp(3_000, "server")
    )));
    assert!(is_applied(&set(
        &mut store,
        id,
        "note",
        json!("Belege"),
        stamp(9_000, "claude")
    )));

    let stamps = store.stamps_between(at(0), at(5_000)).unwrap();
    assert_eq!(stamps.len(), 1, "{stamps:?}");
    assert_eq!(
        (
            stamps[0].object,
            stamps[0].field.as_str(),
            stamps[0].device.as_str()
        ),
        (id, "title", "pixel")
    );
    assert_eq!(stamps[0].at, at(2_000));
    // Changed again later: the earlier span no longer sees it.
    assert!(is_applied(&set(
        &mut store,
        id,
        "title",
        json!("Steuer 2027"),
        stamp(10_000, "pixel")
    )));
    assert!(store.stamps_between(at(0), at(5_000)).unwrap().is_empty());
    let later = store.stamps_between(at(5_000), at(20_000)).unwrap();
    assert_eq!(
        later.iter().map(|s| s.field.as_str()).collect::<Vec<_>>(),
        ["note", "title"]
    );
}

/// ADR 0094 §6: a database from before `own_scope` gives each `scope_only`
/// object the `only` scope with the smallest position that unlocked it
/// (the object itself, something below it), none when no scope did; and a
/// change number of its own, so clients learn of it. Asked of the store.
#[test]
fn the_migration_gives_what_was_scope_only_its_unlocking_scope() {
    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = Dir(std::env::temp_dir().join(format!("samu-store-own-{}", Id::new())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let path = dir.0.join("account.db");
    let place = |title: &str, extra: Value| {
        let mut value = json!({
            "id": Id::new().to_string(),
            "created": "2026-11-11T08:00:00Z",
            "modified": "2026-11-11T08:00:00Z",
            "title": title,
            "status": "active",
            "position": 0
        });
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        value
    };
    // Privat ⊃ Habits (scope_only) ∋ Sport; AOD at the top (scope_only),
    // Geheim at the top (scope_only, no scope unlocks it); tag habit
    // (scope_only) ⊃ lauf.
    let privat = place("Privat", json!({}));
    let habits = place("Habits", json!({"parent": id_of(&privat)}));
    let sport = place(
        "Sport",
        json!({"folder": id_of(&habits), "kind": "parallel"}),
    );
    let aod = place("AOD", json!({"kind": "parallel"}));
    let geheim = place("Geheim", json!({"kind": "parallel"}));
    let habit = new_tag("habit");
    let mut lauf = new_tag("lauf");
    lauf["parent"] = json!(id_of(&habit));
    // Scopes: 1 only Sport (below Habits), 0 only AOD, 2 only Privat (above
    // Habits: no unlock), 3 except AOD (no unlock), 4 only lauf.
    let mut by_sport = new_scope("Sport", "only", &[id_of(&sport)]);
    by_sport["position"] = json!(1);
    let by_aod = new_scope("AOD", "only", &[id_of(&aod)]);
    let mut by_privat = new_scope("Privat", "only", &[id_of(&privat)]);
    by_privat["position"] = json!(0);
    let mut except_aod = new_scope("ohne AOD", "except", &[id_of(&aod)]);
    except_aod["position"] = json!(-1);
    let by_lauf = new_scope("Lauf", "only", &[id_of(&lauf)]);
    {
        let mut store = AccountStore::open(&path).unwrap();
        let mut wall = 1_000;
        for (kind, value) in [
            (ObjectKind::Folder, &privat),
            (ObjectKind::Folder, &habits),
            (ObjectKind::Project, &sport),
            (ObjectKind::Project, &aod),
            (ObjectKind::Project, &geheim),
            (ObjectKind::Tag, &habit),
            (ObjectKind::Tag, &lauf),
            (ObjectKind::Scope, &by_sport),
            (ObjectKind::Scope, &by_aod),
            (ObjectKind::Scope, &by_privat),
            (ObjectKind::Scope, &except_aod),
            (ObjectKind::Scope, &by_lauf),
        ] {
            wall += 1;
            assert!(is_applied(&create(
                &mut store,
                kind,
                value.clone(),
                stamp(wall, "laptop")
            )));
        }
    }
    // Back to schema 23, with scope_only set as before.
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        let mark = |table: &str, value: &Value| {
            let uuid = id_of(value).to_string().replace('-', "");
            conn.execute_batch(&format!(
                "UPDATE {table} SET scope_only = 1 WHERE object = \
                 (SELECT id FROM object WHERE hex(uuid) = upper('{uuid}'));"
            ))
            .unwrap();
        };
        conn.execute_batch(
            "ALTER TABLE folder DROP COLUMN own_scope; \
             ALTER TABLE project DROP COLUMN own_scope; ALTER TABLE tag DROP COLUMN own_scope; \
             ALTER TABLE appointment DROP COLUMN alarms; \
             ALTER TABLE perspective DROP COLUMN table_view; \
             ALTER TABLE perspective DROP COLUMN blocks; ALTER TABLE perspective DROP COLUMN dedupe; \
             DELETE FROM schema_history WHERE version >= 24; PRAGMA user_version = 23;",
        )
        .unwrap();
        mark("folder", &habits);
        mark("project", &aod);
        mark("project", &geheim);
        mark("tag", &habit);
    }
    let store = AccountStore::open(&path).unwrap();
    assert_eq!(
        store.migration().applied,
        vec![
            "0024_own_scope",
            "0025_appointment_alarms",
            "0026_perspective_table",
            "0027_perspective_blocks"
        ]
    );
    assert_eq!(
        store.migration().notes.len(),
        1,
        "{:?}",
        store.migration().notes
    );
    assert!(store.migration().notes[0].contains("project „Geheim“"));
    let own = |value: &Value| match store.get(id_of(value)).unwrap() {
        Some(Object::Project(p)) => (p.own_scope, p.scope_only),
        Some(Object::Folder(f)) => (f.own_scope, f.scope_only),
        Some(Object::Tag(t)) => (t.own_scope, t.scope_only),
        other => panic!("not a folder, project or tag: {other:?}"),
    };
    assert_eq!(own(&habits), (Some(id_of(&by_sport)), true), "below it");
    assert_eq!(own(&aod), (Some(id_of(&by_aod)), true), "itself, only");
    assert_eq!(own(&geheim), (None, false), "no scope unlocked it");
    assert_eq!(own(&habit), (Some(id_of(&by_lauf)), true), "a tag below");
    assert_eq!(own(&privat), (None, false), "never scope_only");
    // The probe that must find something: those three are new for clients.
    let seq = store.seq().unwrap();
    let changed: Vec<Id> = store
        .changed_after(seq - 4)
        .unwrap()
        .iter()
        .map(|c| c.id)
        .collect();
    for value in [&habits, &aod, &geheim, &habit] {
        assert!(changed.contains(&id_of(value)), "{changed:?}");
    }
}

/// ADR 0101: after the migration with `alarms`, every calendar is read
/// whole on its next fetch: the version it was read at is forgotten.
#[test]
fn the_alarms_migration_makes_every_calendar_fetch_whole() {
    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = Dir(std::env::temp_dir().join(format!("samu-store-alarms-{}", Id::new())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let path = dir.0.join("account.db");
    let calendar = json!({
        "id": Id::new().to_string(),
        "created": "2026-11-11T08:00:00Z",
        "modified": "2026-11-11T08:00:00Z",
        "title": "Familie",
        "kind": "caldav",
        "enabled": true,
        "refresh": 15,
        "position": 0
    });
    {
        let mut store = AccountStore::open(&path).unwrap();
        assert!(is_applied(&create(
            &mut store,
            ObjectKind::Calendar,
            calendar.clone(),
            stamp(1_000, "laptop")
        )));
    }
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(&format!(
            "INSERT INTO calendar_source (calendar, sealed, etag, last_modified, next_fetch) \
             SELECT id, zeroblob(64), '\"ctag-7\"', 'Mon, 09 Nov 2026 08:00:00 GMT', \
             '2026-11-14T10:00:00Z' FROM object WHERE hex(uuid) = upper('{}'); \
             ALTER TABLE appointment DROP COLUMN alarms; \
             ALTER TABLE perspective DROP COLUMN table_view; \
             ALTER TABLE perspective DROP COLUMN blocks; ALTER TABLE perspective DROP COLUMN dedupe; \
             DELETE FROM schema_history WHERE version >= 25; PRAGMA user_version = 24;",
            id_of(&calendar).to_string().replace('-', "")
        ))
        .unwrap();
    }
    let store = AccountStore::open(&path).unwrap();
    assert_eq!(
        store.migration().applied,
        vec![
            "0025_appointment_alarms",
            "0026_perspective_table",
            "0027_perspective_blocks"
        ]
    );
    drop(store);
    let conn = rusqlite::Connection::open(&path).unwrap();
    let (rows, forgotten): (i64, i64) = conn
        .query_row(
            "SELECT count(*), count(*) FILTER (WHERE etag IS NULL AND last_modified IS NULL \
             AND next_fetch IS NULL) FROM calendar_source",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((rows, forgotten), (1, 1), "a probe that must find the row");
}
