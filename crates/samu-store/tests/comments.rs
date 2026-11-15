//! Comments, attachments and the author (ADR 0039) against a real account
//! database. Synthetic data only.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

use jiff::Timestamp;
use samu_store::core::api::{
    Author, AuthorKind, Change, ChangeOutcome, Hlc, Id, LogKind, ObjectKind,
};
use samu_store::{AccountStore, Object};
use serde_json::{Value, json};

fn now() -> Timestamp {
    "2026-11-08T18:00:00Z".parse().unwrap()
}

fn person() -> Author {
    Author {
        device: "laptop".into(),
        kind: AuthorKind::Human,
    }
}

fn agent() -> Author {
    Author {
        device: "claude".into(),
        kind: AuthorKind::Agent,
    }
}

/// Stamps rise with every call, so no change loses on its stamp alone.
fn stamp(wall: i64) -> Hlc {
    Hlc::new(wall, 0, "pixel")
}

fn create(store: &mut AccountStore, kind: ObjectKind, value: &Value, by: &Author) -> ChangeOutcome {
    let change = Change::Create {
        object: value["id"].as_str().unwrap().parse().unwrap(),
        kind,
        value: value.clone(),
        hlc: stamp(1_000),
    };
    store.apply(&change, by, now()).unwrap()
}

fn set(
    store: &mut AccountStore,
    id: Id,
    field: &str,
    value: Value,
    wall: i64,
    by: &Author,
) -> ChangeOutcome {
    let change = Change::Set {
        object: id,
        field: field.into(),
        value,
        note: None,
        hlc: stamp(wall),
    };
    store.apply(&change, by, now()).unwrap()
}

fn delete(store: &mut AccountStore, id: Id, wall: i64, by: &Author) -> ChangeOutcome {
    let change = Change::Delete {
        object: id,
        hlc: stamp(wall),
    };
    store.apply(&change, by, now()).unwrap()
}

fn applied(outcome: &ChangeOutcome) -> bool {
    matches!(outcome, ChangeOutcome::Applied { .. })
}

fn rejected(outcome: &ChangeOutcome) -> bool {
    matches!(outcome, ChangeOutcome::Rejected { .. })
}

fn task(title: &str) -> (Id, Value) {
    let id = Id::new();
    let value = json!({
        "id": id.to_string(),
        "created": "2026-11-08T08:00:00Z",
        "modified": "2026-11-08T08:00:00Z",
        "title": title,
        "flagged": false,
        "sequential": false,
        "complete_with_last_child": false,
        "position": 0
    });
    (id, value)
}

fn comment(owner: Id, body: &str) -> (Id, Value) {
    let id = Id::new();
    let value = json!({
        "id": id.to_string(),
        "created": "2026-11-08T18:00:00Z",
        "modified": "2026-11-08T18:00:00Z",
        "owner": owner.to_string(),
        "body": body
    });
    (id, value)
}

const HASH: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

fn attachment(owner: Id, size: u64) -> (Id, Value) {
    let id = Id::new();
    let value = json!({
        "id": id.to_string(),
        "created": "2026-11-08T18:00:00Z",
        "modified": "2026-11-08T18:00:00Z",
        "owner": owner.to_string(),
        "blob": HASH,
        "name": "tafel.jpg",
        "media_type": "image/jpeg",
        "size": size
    });
    (id, value)
}

/// A store with one task, made by a person.
fn with_task() -> (AccountStore, Id) {
    let mut store = AccountStore::open_in_memory().unwrap();
    let (id, value) = task("Angebot");
    assert!(applied(&create(
        &mut store,
        ObjectKind::Task,
        &value,
        &person()
    )));
    (store, id)
}

fn body_of(store: &AccountStore, id: Id) -> String {
    match store.get(id).unwrap() {
        Some(Object::Comment(c)) => c.body,
        other => panic!("not a comment: {other:?}"),
    }
}

#[test]
fn the_author_is_the_session_not_what_the_client_says() {
    let (mut store, _) = with_task();
    let (id, mut value) = task("Befund");
    value["created_by"] = json!({"device": "laptop", "kind": "human"});
    assert!(applied(&create(
        &mut store,
        ObjectKind::Task,
        &value,
        &agent()
    )));
    match store.get(id).unwrap() {
        Some(Object::Task(t)) => assert_eq!(t.created_by, Some(agent())),
        other => panic!("not a task: {other:?}"),
    }
}

#[test]
fn an_author_kind_the_server_does_not_know_is_refused() {
    let mut store = AccountStore::open_in_memory().unwrap();
    let (_, value) = task("Fremd");
    let stranger = Author {
        device: "x".into(),
        kind: AuthorKind::Unknown,
    };
    assert!(rejected(&create(
        &mut store,
        ObjectKind::Task,
        &value,
        &stranger
    )));
}

#[test]
fn a_comment_is_changed_only_by_the_kind_that_wrote_it() {
    let (mut store, owner) = with_task();
    let (mine, value) = comment(owner, "von Alice");
    assert!(applied(&create(
        &mut store,
        ObjectKind::Comment,
        &value,
        &person()
    )));
    let (theirs, value) = comment(owner, "vom Agent");
    assert!(applied(&create(
        &mut store,
        ObjectKind::Comment,
        &value,
        &agent()
    )));

    // The agent touches neither text nor existence of the person's comment.
    assert!(rejected(&set(
        &mut store,
        mine,
        "body",
        json!("umgeschrieben"),
        2_000,
        &agent()
    )));
    assert!(rejected(&delete(&mut store, mine, 2_001, &agent())));
    assert_eq!(body_of(&store, mine), "von Alice");
    // Nor the person the agent's.
    assert!(rejected(&set(
        &mut store,
        theirs,
        "body",
        json!("x"),
        2_002,
        &person()
    )));
    assert!(rejected(&delete(&mut store, theirs, 2_003, &person())));

    // The probe that must succeed: each kind changes its own, from another
    // device of the same kind too.
    let phone = Author {
        device: "pixel".into(),
        kind: AuthorKind::Human,
    };
    assert!(applied(&set(
        &mut store,
        mine,
        "body",
        json!("von Alice, ergänzt"),
        2_004,
        &phone
    )));
    assert_eq!(body_of(&store, mine), "von Alice, ergänzt");
    assert!(applied(&set(
        &mut store,
        theirs,
        "body",
        json!("vom Agent, ergänzt"),
        2_005,
        &agent()
    )));
    assert!(applied(&delete(&mut store, theirs, 2_006, &agent())));
    assert!(store.get(theirs).unwrap().is_none());
}

#[test]
fn a_comment_keeps_its_owner() {
    let (mut store, owner) = with_task();
    let (other, value) = task("Anderes");
    assert!(applied(&create(
        &mut store,
        ObjectKind::Task,
        &value,
        &person()
    )));
    let (id, value) = comment(owner, "hier");
    assert!(applied(&create(
        &mut store,
        ObjectKind::Comment,
        &value,
        &person()
    )));
    assert!(rejected(&set(
        &mut store,
        id,
        "owner",
        json!(other.to_string()),
        2_000,
        &person()
    )));
}

#[test]
fn a_comment_belongs_to_a_task_or_project() {
    let (mut store, owner) = with_task();
    let (on_comment, value) = comment(owner, "erster");
    assert!(applied(&create(
        &mut store,
        ObjectKind::Comment,
        &value,
        &person()
    )));
    // Not to another comment, and not to nothing.
    let (_, value) = comment(on_comment, "Antwort");
    assert!(rejected(&create(
        &mut store,
        ObjectKind::Comment,
        &value,
        &person()
    )));
    let (_, value) = comment(Id::new(), "ins Leere");
    assert!(rejected(&create(
        &mut store,
        ObjectKind::Comment,
        &value,
        &person()
    )));
}

#[test]
fn an_attachment_needs_its_file_first() {
    let (mut store, owner) = with_task();
    let (id, value) = attachment(owner, 5);
    assert!(rejected(&create(
        &mut store,
        ObjectKind::Attachment,
        &value,
        &person()
    )));
    assert!(store.get(id).unwrap().is_none());

    assert_eq!(store.blob_bytes().unwrap(), 0);
    assert!(store.add_blob(HASH, 5, "image/jpeg", now()).unwrap());
    // Recorded once; the second time changes nothing.
    assert!(!store.add_blob(HASH, 5, "text/plain", now()).unwrap());
    assert_eq!(store.blob_bytes().unwrap(), 5);
    assert_eq!(store.blob(HASH).unwrap(), Some((5, "image/jpeg".into())));

    // A size that is not the file's is refused, the right one is taken.
    let (_, wrong) = attachment(owner, 6);
    assert!(rejected(&create(
        &mut store,
        ObjectKind::Attachment,
        &wrong,
        &person()
    )));
    assert!(applied(&create(
        &mut store,
        ObjectKind::Attachment,
        &value,
        &person()
    )));
    match store.get(id).unwrap() {
        Some(Object::Attachment(a)) => {
            assert_eq!((a.blob.as_str(), a.size), (HASH, 5));
            assert_eq!(a.created_by, Some(person()));
        }
        other => panic!("not an attachment: {other:?}"),
    }
}

#[test]
fn an_attachment_on_a_comment_and_only_its_name_changes() {
    let (mut store, owner) = with_task();
    store.add_blob(HASH, 5, "image/jpeg", now()).unwrap();
    let (on, value) = comment(owner, "mit Bild");
    assert!(applied(&create(
        &mut store,
        ObjectKind::Comment,
        &value,
        &agent()
    )));
    let (id, value) = attachment(on, 5);
    assert!(applied(&create(
        &mut store,
        ObjectKind::Attachment,
        &value,
        &agent()
    )));

    assert!(applied(&set(
        &mut store,
        id,
        "name",
        json!("tafel-2.jpg"),
        2_000,
        &agent()
    )));
    for (field, value) in [
        ("blob", json!("a".repeat(64))),
        ("size", json!(9)),
        ("owner", json!(owner.to_string())),
        ("media_type", json!("text/plain")),
    ] {
        assert!(
            rejected(&set(&mut store, id, field, value, 2_001, &agent())),
            "{field}"
        );
    }
    // A person deletes neither the agent's picture nor its comment.
    assert!(rejected(&delete(&mut store, id, 2_002, &person())));
    assert!(rejected(&delete(&mut store, on, 2_003, &person())));
}

#[test]
fn the_logbook_says_whether_a_person_or_an_agent_acted() {
    let (mut store, id) = with_task();
    assert!(applied(&set(
        &mut store,
        id,
        "completed_at",
        json!("2026-11-08T18:00:00Z"),
        2_000,
        &agent()
    )));
    assert!(applied(&set(
        &mut store,
        id,
        "completed_at",
        Value::Null,
        2_001,
        &person()
    )));
    let log = store.logbook(id).unwrap().unwrap();
    let seen: Vec<(LogKind, Option<AuthorKind>)> =
        log.iter().map(|e| (e.kind, e.author_kind)).collect();
    assert_eq!(
        seen,
        vec![
            (LogKind::Completed, Some(AuthorKind::Agent)),
            (LogKind::Reopened, Some(AuthorKind::Human)),
        ]
    );
}
