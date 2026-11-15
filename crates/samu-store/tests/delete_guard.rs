//! ADR 0076 §2: a folder, project or tag is not deleted while a live
//! object still points at it; once that is gone, it is. Asked of the
//! database. Synthetic data only.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

use jiff::Timestamp;
use samu_store::AccountStore;
use samu_store::core::api::{Author, AuthorKind, Change, ChangeOutcome, Hlc, Id, ObjectKind};
use serde_json::{Value, json};

fn now() -> Timestamp {
    "2026-11-11T12:00:00Z".parse().unwrap()
}

struct W {
    store: AccountStore,
    wall: i64,
}

impl W {
    fn apply(&mut self, change: impl FnOnce(Hlc) -> Change) -> ChangeOutcome {
        self.wall += 1;
        let author = Author {
            device: "laptop".into(),
            kind: AuthorKind::Human,
        };
        let change = change(Hlc::new(4_000_000_000_000 + self.wall, 0, "laptop"));
        self.store.apply(&change, &author, now()).unwrap()
    }

    fn create(&mut self, kind: ObjectKind, fields: Value) -> Id {
        let id = Id::new();
        let mut value = json!({"id": id.to_string(), "created": "2026-11-10T08:00:00Z",
                               "modified": "2026-11-10T08:00:00Z"});
        for (k, v) in fields.as_object().unwrap() {
            value[k] = v.clone();
        }
        let outcome = self.apply(|hlc| Change::Create {
            object: id,
            kind,
            value,
            hlc,
        });
        assert!(
            matches!(outcome, ChangeOutcome::Applied { .. }),
            "{outcome:?}"
        );
        id
    }

    fn delete(&mut self, object: Id) -> ChangeOutcome {
        self.apply(|hlc| Change::Delete { object, hlc })
    }

    fn set(&mut self, object: Id, field: &str, value: Value) {
        let outcome = self.apply(|hlc| Change::Set {
            object,
            field: field.into(),
            value,
            note: None,
            hlc,
        });
        assert!(
            matches!(outcome, ChangeOutcome::Applied { .. }),
            "{outcome:?}"
        );
    }

    fn remove(&mut self, object: Id, field: &str, element: Id) {
        let outcome = self.apply(|hlc| Change::Remove {
            object,
            field: field.into(),
            element,
            note: None,
            hlc,
        });
        assert!(
            matches!(outcome, ChangeOutcome::Applied { .. }),
            "{outcome:?}"
        );
    }
}

fn refused(outcome: &ChangeOutcome, what: &str) {
    match outcome {
        ChangeOutcome::Rejected { reason } => {
            assert!(reason.contains(what), "{reason}");
            assert!(reason.contains("POST /objects/{id}/delete"), "{reason}");
        }
        other => panic!("not refused: {other:?}"),
    }
}

fn applied(outcome: &ChangeOutcome) {
    assert!(
        matches!(outcome, ChangeOutcome::Applied { .. }),
        "{outcome:?}"
    );
}

#[test]
fn a_place_still_pointed_at_is_not_deleted() {
    let mut w = W {
        store: AccountStore::open_in_memory().unwrap(),
        wall: 0,
    };
    let folder = w.create(
        ObjectKind::Folder,
        json!({"title": "Haus", "status": "active", "position": 0}),
    );
    let inner = w.create(
        ObjectKind::Folder,
        json!({"title": "Keller", "status": "active", "position": 0, "parent": folder.to_string()}),
    );
    let project = w.create(
        ObjectKind::Project,
        json!({"title": "Dach", "kind": "parallel", "status": "active", "flagged": false,
               "position": 0, "folder": folder.to_string()}),
    );
    let tag = w.create(
        ObjectKind::Tag,
        json!({"title": "Ort", "status": "active", "position": 0}),
    );
    let task = w.create(
        ObjectKind::Task,
        json!({"title": "Rinne", "project": project.to_string(), "flagged": false,
               "sequential": false, "complete_with_last_child": false, "position": 0,
               "completed_at": "2026-11-11T09:00:00Z",
               "tags": [{"tag": tag.to_string(), "position": 0}]}),
    );
    let scope = w.create(
        ObjectKind::Scope,
        json!({"title": "Privat", "mode": "only", "position": 0, "members": [project.to_string()]}),
    );

    // Even a completed task holds its project (01a1caff).
    refused(&w.delete(project), "1 tasks");
    w.set(task, "project", Value::Null);
    refused(&w.delete(project), "1 scopes");
    w.remove(scope, "members", project);
    applied(&w.delete(project));

    refused(&w.delete(folder), "1 folders");
    w.set(inner, "parent", Value::Null);
    applied(&w.delete(folder));

    refused(&w.delete(tag), "1 tasks and projects tagged with it");
    w.remove(task, "tags", tag);
    applied(&w.delete(tag));

    // A task is deleted as before.
    applied(&w.delete(task));
}
