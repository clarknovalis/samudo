// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! ADR 0123: deleting and moving go into the logbook for every kind; a
//! delete with what comes with it is one entry; the account's logbook is
//! read newest first, filtered. Synthetic data only.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

use jiff::Timestamp;
use samu_store::core::api::{
    Author, AuthorKind, Change, ChangeOutcome, Hlc, Id, LogKind, ObjectKind,
};
use samu_store::{AccountStore, LogFilter};
use serde_json::{Value, json};

fn at(minute: i64) -> Timestamp {
    Timestamp::from_second(1_795_000_000 + minute * 60).unwrap()
}

struct W {
    store: AccountStore,
    wall: i64,
}

fn author(device: &str) -> Author {
    Author {
        device: device.into(),
        kind: AuthorKind::Human,
    }
}

impl W {
    fn stamp(&mut self, device: &str) -> Hlc {
        self.wall += 1;
        Hlc::new(4_000_000_000_000 + self.wall, 0, device)
    }

    fn apply(&mut self, change: &Change, device: &str, minute: i64) {
        let outcome = self
            .store
            .apply(change, &author(device), at(minute))
            .unwrap();
        assert!(
            matches!(outcome, ChangeOutcome::Applied { .. }),
            "{outcome:?}"
        );
    }

    fn create(&mut self, kind: ObjectKind, fields: Value) -> Id {
        let id = Id::new();
        let mut value = json!({"id": id.to_string(), "created": "2026-11-10T08:00:00Z",
                               "modified": "2026-11-10T08:00:00Z"});
        for (k, v) in fields.as_object().unwrap() {
            value[k] = v.clone();
        }
        let hlc = self.stamp("laptop");
        self.apply(
            &Change::Create {
                object: id,
                kind,
                value,
                hlc,
            },
            "laptop",
            0,
        );
        id
    }

    fn set(&mut self, object: Id, field: &str, value: Value, device: &str, minute: i64) {
        let hlc = self.stamp(device);
        let change = Change::Set {
            object,
            field: field.into(),
            value,
            note: None,
            hlc,
        };
        self.apply(&change, device, minute);
    }

    fn project(&mut self, title: &str) -> Id {
        self.create(
            ObjectKind::Project,
            json!({"title": title, "kind": "parallel", "status": "active", "flagged": false,
                   "position": 0}),
        )
    }

    fn task(&mut self, title: &str, project: Id) -> Id {
        self.create(
            ObjectKind::Task,
            json!({"title": title, "project": project.to_string(), "flagged": false,
                   "sequential": false, "complete_with_last_child": false, "position": 0}),
        )
    }

    fn all(
        &self,
        kinds: Vec<LogKind>,
        device: Option<&str>,
    ) -> Vec<samu_store::core::api::AccountLogEntry> {
        self.store
            .logbook_all(&LogFilter {
                since: at(-60),
                until: at(60),
                kinds,
                device: device.map(str::to_string),
                limit: 100,
            })
            .unwrap()
    }
}

#[test]
fn deleting_and_moving_are_logged_and_read_for_the_account() {
    let mut w = W {
        store: AccountStore::open_in_memory().unwrap(),
        wall: 0,
    };
    let steuer = w.project("Steuer");
    let exit = w.project("Exit");
    let belege = w.task("Belege", steuer);
    let anruf = w.task("Anruf", steuer);

    // A move, and the same value again, which is none.
    w.set(belege, "project", json!(exit.to_string()), "browser", 1);
    w.set(belege, "project", json!(exit.to_string()), "browser", 2);
    // A plain delete of a task.
    let hlc = w.stamp("phone");
    w.apply(
        &Change::Delete {
            object: belege,
            hlc,
        },
        "phone",
        3,
    );

    // A project with a task, as POST /objects/{id}/delete sends it: the
    // task's move and the project's tombstone, one entry for both.
    let (to_inbox, tombstone) = (w.stamp("browser"), w.stamp("browser"));
    let changes = [
        Change::Set {
            object: anruf,
            field: "project".into(),
            value: Value::Null,
            note: None,
            hlc: to_inbox,
        },
        Change::Delete {
            object: steuer,
            hlc: tombstone,
        },
    ];
    let counts = json!({"deleted": 1, "moved": 1, "untagged": 0});
    let outcome = w
        .store
        .apply_delete(&changes, &author("browser"), at(4), steuer, &counts)
        .unwrap();
    assert!(outcome.is_ok(), "{outcome:?}");

    let all = w.all(Vec::new(), None);
    let seen: Vec<(LogKind, Option<&str>)> = all
        .iter()
        .map(|e| (e.entry.kind, e.title.as_deref()))
        .collect();
    // Newest first; the task moved by the delete has no entry of its own.
    assert_eq!(
        seen,
        [
            (LogKind::Deleted, Some("Steuer")),
            (LogKind::Deleted, Some("Belege")),
            (LogKind::Moved, Some("Belege")),
        ]
    );
    assert_eq!(all[0].object_kind, ObjectKind::Project);
    assert!(all[0].deleted);
    assert_eq!(all[0].entry.new_value, Some(counts));
    assert_eq!(all[2].entry.field.as_deref(), Some("project"));
    assert_eq!(all[2].entry.old_value, Some(json!(steuer.to_string())));
    assert_eq!(all[2].entry.new_value, Some(json!(exit.to_string())));
    assert_eq!(all[1].entry.device, "phone");

    // The filters: kind, device, span.
    assert_eq!(w.all(vec![LogKind::Moved], None).len(), 1);
    assert_eq!(w.all(Vec::new(), Some("phone")).len(), 1);
    let later = w
        .store
        .logbook_all(&LogFilter {
            since: at(4),
            until: at(60),
            kinds: Vec::new(),
            device: None,
            limit: 100,
        })
        .unwrap();
    assert_eq!(later.len(), 1);

    // The object's own logbook reads the counts back too.
    let own = w.store.logbook(steuer).unwrap().unwrap();
    assert_eq!(own.len(), 1);
    assert_eq!(own[0].kind, LogKind::Deleted);
}
