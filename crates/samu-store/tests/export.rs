// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Export and restore of an account (ADR 0074 §7): an account with every
//! kind of object, a logbook, removed tags, properties and members, and a
//! file is exported, restored into an empty account and exported again;
//! the two exports are the same but for the time in the manifest. Asked of
//! the database too. Synthetic data only.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

use std::path::PathBuf;

use jiff::Timestamp;
use samu_store::core::api::{
    Author, AuthorKind, CalendarSource, Change, ChangeOutcome, EXPORT_KINDS, Export, Hlc, Id,
    ObjectKind, export_file, kind_of,
};
use samu_store::{AccountStore, RestoreRefused, SecretKey};
use serde_json::{Value, json};

fn now() -> Timestamp {
    "2026-11-11T12:00:00Z".parse().unwrap()
}

fn later() -> Timestamp {
    "2026-11-11T13:00:00Z".parse().unwrap()
}

/// After every stamp the server made from the real clock on opening.
const BASE: i64 = 4_000_000_000_000;

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn dir() -> Dir {
    let dir = Dir(std::env::temp_dir().join(format!("samu-store-export-{}", Id::new())));
    std::fs::create_dir_all(&dir.0).unwrap();
    dir
}

/// Applies changes with rising stamps, alternately as a person and an
/// agent, and insists each one is taken.
struct Writer {
    wall: i64,
}

impl Writer {
    fn apply(&mut self, store: &mut AccountStore, change: impl FnOnce(Hlc) -> Change) {
        self.wall += 1;
        let (device, kind) = if self.wall % 2 == 0 {
            ("laptop", AuthorKind::Human)
        } else {
            ("claude", AuthorKind::Agent)
        };
        let author = Author {
            device: device.into(),
            kind,
        };
        let change = change(Hlc::new(BASE + self.wall, 0, device));
        // Arriving when the clock says `BASE`: against `now()` the stamp
        // would be from the future and capped.
        let arrival = Timestamp::from_millisecond(BASE).unwrap();
        let outcome = store.apply(&change, &author, arrival).unwrap();
        assert!(
            matches!(outcome, ChangeOutcome::Applied { .. }),
            "{change:?}: {outcome:?}"
        );
    }

    fn create(&mut self, store: &mut AccountStore, kind: ObjectKind, value: Value) -> Id {
        let id: Id = value["id"].as_str().unwrap().parse().unwrap();
        self.apply(store, |hlc| Change::Create {
            object: id,
            kind,
            value,
            hlc,
        });
        id
    }

    fn set(&mut self, store: &mut AccountStore, object: Id, field: &str, value: Value) {
        self.apply(store, |hlc| Change::Set {
            object,
            field: field.into(),
            value,
            note: Some("synthetisch".into()),
            hlc,
        });
    }
}

fn base(id: Id) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    map.insert("id".into(), json!(id.to_string()));
    map.insert("created".into(), json!("2026-11-10T08:00:00Z"));
    map.insert("modified".into(), json!("2026-11-10T08:00:00Z"));
    map
}

fn with(id: Id, fields: Value) -> Value {
    let mut map = base(id);
    for (k, v) in fields.as_object().unwrap() {
        map.insert(k.clone(), v.clone());
    }
    Value::Object(map)
}

const HASH: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

/// An account with every kind of object and what hangs on them.
fn fill(store: &mut AccountStore) {
    let mut w = Writer { wall: 0 };
    let folder = w.create(
        store,
        ObjectKind::Folder,
        with(
            Id::new(),
            json!({"title": "Haus", "status": "active", "position": 0}),
        ),
    );
    let tag = w.create(
        store,
        ObjectKind::Tag,
        with(
            Id::new(),
            json!({"title": "Ort", "status": "active", "position": 0}),
        ),
    );
    let gone_tag = w.create(
        store,
        ObjectKind::Tag,
        with(
            Id::new(),
            json!({"title": "Alt", "status": "active", "position": 1}),
        ),
    );
    let project = w.create(
        store,
        ObjectKind::Project,
        with(
            Id::new(),
            json!({"title": "Dachrinne", "kind": "parallel", "status": "active",
                   "flagged": false, "folder": folder.to_string(), "position": 0,
                   "properties": {"jira": "SAMU-1"}}),
        ),
    );
    let task = w.create(
        store,
        ObjectKind::Task,
        with(
            Id::new(),
            json!({"title": "Angebot einholen", "note": "zwei *Firmen*",
                   "project": project.to_string(), "flagged": true, "sequential": false,
                   "complete_with_last_child": false, "position": 0,
                   "due": {"date": "2026-11-20"},
                   "tags": [{"tag": tag.to_string(), "position": 0},
                            {"tag": gone_tag.to_string(), "position": 1}],
                   "properties": {"status": "wartet", "alt": "weg"}}),
        ),
    );
    w.apply(store, |hlc| Change::Remove {
        object: task,
        field: "tags".into(),
        element: gone_tag,
        note: None,
        hlc,
    });
    w.set(store, task, "properties.alt", Value::Null);
    w.set(store, task, "title", json!("Angebote einholen"));
    w.set(store, task, "completed_at", json!("2026-11-11T09:00:00Z"));
    w.create(
        store,
        ObjectKind::Link,
        with(
            Id::new(),
            json!({"owner": task.to_string(), "kind": "ref",
                   "uri": "catastr:01936f5e-8a00-7c3e-9b1a-3f2d4c5b6f00", "label": "Angebot"}),
        ),
    );
    w.create(
        store,
        ObjectKind::Clock,
        with(
            Id::new(),
            json!({"owner": task.to_string(), "start": "2026-11-10T10:00:00Z",
                   "end": "2026-11-10T10:25:00Z"}),
        ),
    );
    w.create(
        store,
        ObjectKind::Reminder,
        with(
            Id::new(),
            json!({"owner": task.to_string(), "anchor": "due", "before": "PT2H"}),
        ),
    );
    let scope = w.create(
        store,
        ObjectKind::Scope,
        with(
            Id::new(),
            json!({"title": "Privat", "mode": "only", "position": 0,
                   "members": [folder.to_string(), tag.to_string()]}),
        ),
    );
    w.apply(store, |hlc| Change::Remove {
        object: scope,
        field: "members".into(),
        element: tag,
        note: None,
        hlc,
    });
    let comment = w.create(
        store,
        ObjectKind::Comment,
        with(
            Id::new(),
            json!({"owner": task.to_string(), "body": "Stufe 1 gebaut."}),
        ),
    );
    store.add_blob(HASH, 5, "text/plain", now()).unwrap();
    w.create(
        store,
        ObjectKind::Attachment,
        with(
            Id::new(),
            json!({"owner": comment.to_string(), "blob": HASH, "name": "hallo.txt",
                   "media_type": "text/plain", "size": 5}),
        ),
    );
    w.create(
        store,
        ObjectKind::Template,
        with(
            Id::new(),
            json!({"title": "habit", "text": "@Ort every+:1d", "position": 0,
                   "reminders": [{"anchor": "planned"}]}),
        ),
    );
    let habit = w.create(
        store,
        ObjectKind::Task,
        with(
            Id::new(),
            json!({"title": "Wasser", "flagged": false, "sequential": false,
                   "complete_with_last_child": false, "position": 1,
                   "habit": {"kind": "build", "times": 8, "per": "day", "unit": "Glas"}}),
        ),
    );
    w.create(
        store,
        ObjectKind::HabitEntry,
        with(
            Id::new(),
            json!({"habit": habit.to_string(), "day": "2026-11-10", "status": "done",
                   "amount": 3}),
        ),
    );
    w.create(
        store,
        ObjectKind::Perspective,
        with(
            Id::new(),
            json!({"title": "Kurz", "chips": [{"kind": "estimate", "value": "<15m"}],
                   "filter": "remaining", "arrange": {"group": "none", "sort": "title"},
                   "view": "list", "position": 0}),
        ),
    );
    let calendar = w.create(
        store,
        ObjectKind::Calendar,
        with(
            Id::new(),
            json!({"title": "Kalender", "kind": "caldav", "enabled": true, "refresh": 15,
                   "position": 0}),
        ),
    );
    let source = CalendarSource {
        url: format!("https://dav.example.org/cal/{calendar}/"),
        username: Some("alice".into()),
        password: Some("app-password".into()),
        credentials_from: None,
    };
    store
        .set_calendar_source(calendar, &source, &SecretKey::new([7; 32]), now())
        .unwrap()
        .unwrap();
    store.set_writable(calendar, true, now()).unwrap();
    w.create(
        store,
        ObjectKind::Appointment,
        with(
            Id::new(),
            json!({"calendar": calendar.to_string(), "uid": "a1@example.org",
                   "title": "Zahnarzt", "start": {"date": "2026-11-20"},
                   "end": {"date": "2026-11-21"}, "status": "confirmed", "busy": true}),
        ),
    );
    // The settings the server made, changed by a client.
    let settings = store
        .export("alice", now())
        .unwrap()
        .objects
        .iter()
        .find(|o| kind_of(&o.object) == ObjectKind::Settings)
        .and_then(|o| samu_store::core::api::id_of(&o.object))
        .unwrap();
    w.set(store, settings, "forecast_days", json!(14));
}

fn files(export: &Export) -> Vec<(String, String)> {
    export.to_files().unwrap()
}

/// The two exports, the manifests' time aside.
fn same_but_the_time(a: &Export, b: &Export) {
    let mut b = b.clone();
    b.manifest.exported_at = a.manifest.exported_at;
    assert_eq!(files(a), files(&b));
}

#[test]
fn an_export_restores_into_an_empty_account_as_it_was() {
    let d = dir();
    let mut source = AccountStore::open(&d.0.join("a.db")).unwrap();
    fill(&mut source);
    let first = source.export("alice", now()).unwrap();

    // Every kind is in it (ADR 0074 §7): a kind left out of the seed, or
    // out of the export, fails here.
    for kind in EXPORT_KINDS {
        assert!(
            first.objects.iter().any(|o| kind_of(&o.object) == *kind),
            "no {kind:?} in the export"
        );
    }
    // Removed elements travel with their stamps.
    let task = first
        .objects
        .iter()
        .find(|o| kind_of(&o.object) == ObjectKind::Task)
        .unwrap();
    assert!(task.sync.tags.iter().any(|t| !t.present));
    assert!(task.sync.properties.iter().any(|p| p.value.is_none()));
    assert!(task.sync.stamps.contains_key("title"));
    assert!(
        first
            .objects
            .iter()
            .any(|o| o.sync.members.iter().any(|m| !m.present))
    );
    assert!(first.logbook.len() >= 2, "{:?}", first.logbook);
    assert_eq!(first.files.len(), 1);
    // No secret of a calendar's source anywhere in it (ADR 0074 §4).
    let text: String = files(&first).into_iter().map(|(_, t)| t).collect();
    assert!(!text.contains("app-password"));
    // The host is there on purpose (`Calendar.host`), the address is not.
    assert!(!text.contains("dav.example.org/cal/"));

    let mut target = AccountStore::open(&d.0.join("b.db")).unwrap();
    assert!(target.is_empty().unwrap());
    target.restore(&first).unwrap().unwrap();
    let second = target.export("alice", later()).unwrap();
    same_but_the_time(&first, &second);

    // Asked of the database, not of the export.
    drop(target);
    let conn = rusqlite::Connection::open(d.0.join("b.db")).unwrap();
    let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
    assert_eq!(
        count("SELECT count(*) FROM object"),
        first.objects.len() as i64
    );
    assert_eq!(count("SELECT seq FROM account"), first.manifest.seq as i64);
    assert_eq!(count("SELECT count(*) FROM tagging WHERE present = 0"), 1);
    assert_eq!(
        count("SELECT count(*) FROM property WHERE value IS NULL"),
        1
    );
    assert_eq!(count("SELECT count(*) FROM settings"), 1);
    assert_eq!(count("SELECT forecast_days FROM settings"), 14);
    assert_eq!(count("SELECT count(*) FROM calendar_source"), 0);
    assert_eq!(
        count("SELECT count(*) FROM logbook"),
        first.logbook.len() as i64
    );
    assert_eq!(count("SELECT count(*) FROM blob"), 1);
}

#[test]
fn the_same_state_exports_to_the_same_bytes() {
    let d = dir();
    let mut store = AccountStore::open(&d.0.join("a.db")).unwrap();
    fill(&mut store);
    let a = store.export("alice", now()).unwrap();
    let b = store.export("alice", now()).unwrap();
    assert_eq!(files(&a), files(&b));
    // And written out and read back, too.
    let read = Export::from_files(|name| {
        files(&a)
            .into_iter()
            .find(|(n, _)| n == name)
            .map(|(_, t)| t)
    })
    .unwrap();
    assert_eq!(files(&read), files(&a));
    for kind in EXPORT_KINDS {
        assert!(
            files(&a)
                .iter()
                .any(|(n, _)| Some(n.as_str()) == export_file(*kind))
        );
    }
}

#[test]
fn a_restore_into_an_account_with_anything_in_it_writes_nothing() {
    let d = dir();
    let mut source = AccountStore::open(&d.0.join("a.db")).unwrap();
    fill(&mut source);
    let export = source.export("alice", now()).unwrap();

    let mut target = AccountStore::open(&d.0.join("b.db")).unwrap();
    let mut w = Writer { wall: 100 };
    w.create(
        &mut target,
        ObjectKind::Tag,
        with(
            Id::new(),
            json!({"title": "Da", "status": "active", "position": 0}),
        ),
    );
    assert!(!target.is_empty().unwrap());
    let before = target.seq().unwrap();
    assert_eq!(
        target.restore(&export).unwrap(),
        Err(RestoreRefused::NotEmpty)
    );
    assert_eq!(target.seq().unwrap(), before);
    drop(target);
    let conn = rusqlite::Connection::open(d.0.join("b.db")).unwrap();
    let objects: i64 = conn
        .query_row("SELECT count(*) FROM object", [], |r| r.get(0))
        .unwrap();
    assert_eq!(objects, 2, "the tag and the settings, nothing more");
}

#[test]
fn a_restore_that_does_not_fit_leaves_the_account_empty() {
    let d = dir();
    let mut source = AccountStore::open(&d.0.join("a.db")).unwrap();
    fill(&mut source);
    let mut export = source.export("alice", now()).unwrap();
    // The file of the attachment is missing.
    export.files.clear();

    let mut target = AccountStore::open(&d.0.join("b.db")).unwrap();
    let refused = target.restore(&export).unwrap().unwrap_err();
    assert!(
        matches!(refused, RestoreRefused::Invalid(ref why) if why.contains(HASH)),
        "{refused}"
    );
    assert!(target.is_empty().unwrap());
    assert_eq!(target.seq().unwrap(), 1);
}

#[test]
fn a_running_server_does_not_keep_an_export_from_reading() {
    let d = dir();
    let path = d.0.join("a.db");
    let mut writer = AccountStore::open(&path).unwrap();
    fill(&mut writer);
    // The writer holds the lock; a second writer is refused, a reader not.
    assert!(AccountStore::open(&path).is_err());
    let reader = AccountStore::open_read_only(&path).unwrap();
    let read = reader.export("alice", now()).unwrap();
    let written = writer.export("alice", now()).unwrap();
    assert_eq!(files(&read), files(&written));
}
