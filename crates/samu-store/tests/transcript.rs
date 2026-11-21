// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Transcripts of audio attachments (ADR 0140) against a real account
//! database. Synthetic data only.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

use jiff::Timestamp;
use samu_store::core::api::{
    Attachment, Author, AuthorKind, Change, ChangeOutcome, Hlc, Id, ObjectKind, Task,
    TranscriptState,
};
use samu_store::{AccountStore, Asked, Object, Transcribed};
use serde_json::{Value, json};

const HASH: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";

fn now() -> Timestamp {
    "2026-11-21T09:00:00Z".parse().unwrap()
}

fn person() -> Author {
    Author {
        device: "pixel".into(),
        kind: AuthorKind::Human,
    }
}

fn create(store: &mut AccountStore, kind: ObjectKind, value: &Value) -> ChangeOutcome {
    let change = Change::Create {
        object: value["id"].as_str().unwrap().parse().unwrap(),
        kind,
        value: value.clone(),
        hlc: Hlc::new(1_000, 0, "pixel"),
    };
    store.apply(&change, &person(), now()).unwrap()
}

fn applied(outcome: &ChangeOutcome) -> bool {
    matches!(outcome, ChangeOutcome::Applied { .. })
}

/// A store with the file and one task; `voice` marks it as a voice note.
fn with_task(title: &str, voice: bool) -> (AccountStore, Id) {
    let mut store = AccountStore::open_in_memory().unwrap();
    assert!(store.add_blob(HASH, 5, "audio/mp4", now()).unwrap());
    let id = Id::new();
    let mut value = json!({
        "id": id.to_string(),
        "created": "2026-11-21T08:59:00Z",
        "modified": "2026-11-21T08:59:00Z",
        "title": title,
        "flagged": false,
        "sequential": false,
        "complete_with_last_child": false,
        "position": 0
    });
    if voice {
        value["properties"] = json!({"source": "voice"});
    }
    assert!(applied(&create(&mut store, ObjectKind::Task, &value)));
    (store, id)
}

fn recording(owner: Id, media_type: &str) -> (Id, Value) {
    let id = Id::new();
    let value = json!({
        "id": id.to_string(),
        "created": "2026-11-21T08:59:30Z",
        "modified": "2026-11-21T08:59:30Z",
        "owner": owner.to_string(),
        "blob": HASH,
        "name": "voice.m4a",
        "media_type": media_type,
        "size": 5
    });
    (id, value)
}

fn attachment(store: &AccountStore, id: Id) -> Attachment {
    match store.get(id).unwrap() {
        Some(Object::Attachment(a)) => a,
        other => panic!("not an attachment: {other:?}"),
    }
}

fn task(store: &AccountStore, id: Id) -> Task {
    match store.get(id).unwrap() {
        Some(Object::Task(t)) => t,
        other => panic!("not a task: {other:?}"),
    }
}

/// A voice note with its recording, asked for at creation.
fn voice_note() -> (AccountStore, Id, Id) {
    let (mut store, owner) = with_task("Voice note 21.11. 10:00", true);
    let (id, mut value) = recording(owner, "audio/mp4");
    value["transcript_state"] = json!("pending");
    assert!(applied(&create(&mut store, ObjectKind::Attachment, &value)));
    (store, owner, id)
}

#[test]
fn a_new_attachment_may_ask_and_say_nothing_else() {
    let (mut store, owner) = with_task("Offer", false);
    let (id, mut value) = recording(owner, "audio/mp4");
    value["transcript_state"] = json!("pending");
    assert!(applied(&create(&mut store, ObjectKind::Attachment, &value)));
    assert_eq!(
        attachment(&store, id).transcript_state,
        Some(TranscriptState::Pending)
    );
    assert_eq!(store.pending_transcripts().unwrap(), vec![id]);

    for (field, given) in [
        ("transcript", json!("made up")),
        ("transcript_error", json!("made up")),
        ("transcript_state", json!("failed")),
        ("transcript_state", json!("from_the_future")),
    ] {
        let (other, mut value) = recording(owner, "audio/mp4");
        value[field] = given;
        let outcome = create(&mut store, ObjectKind::Attachment, &value);
        assert!(!applied(&outcome), "{field}: {outcome:?}");
        assert!(store.get(other).unwrap().is_none());
    }
    // Not of a picture.
    let (_, mut picture) = recording(owner, "image/jpeg");
    picture["transcript_state"] = json!("pending");
    assert!(!applied(&create(
        &mut store,
        ObjectKind::Attachment,
        &picture
    )));
}

#[test]
fn a_client_does_not_set_the_transcript() {
    let (mut store, _, id) = voice_note();
    for (field, value) in [
        ("transcript", json!("made up")),
        ("transcript_state", Value::Null),
        ("transcript_error", json!("x")),
    ] {
        let change = Change::Set {
            object: id,
            field: field.into(),
            value,
            note: None,
            hlc: Hlc::new(2_000, 0, "pixel"),
        };
        let outcome = store.apply(&change, &person(), now()).unwrap();
        assert!(!applied(&outcome), "{field}: {outcome:?}");
    }
    assert_eq!(attachment(&store, id).transcript, None);
}

#[test]
fn the_transcript_fills_an_untouched_voice_note() {
    let (mut store, owner, id) = voice_note();
    let before = store.seq().unwrap();
    let text = "Buy oat milk and call the plumber.\nBefore Friday.";
    assert!(
        store
            .store_transcript(id, &Transcribed::Text(format!("  {text}\n")), now())
            .unwrap()
    );
    let a = attachment(&store, id);
    assert_eq!(a.transcript.as_deref(), Some(text));
    assert_eq!((a.transcript_state, a.transcript_error), (None, None));
    assert!(store.pending_transcripts().unwrap().is_empty());
    let t = task(&store, owner);
    assert_eq!(t.title, "Buy oat milk and call the plumber.");
    assert_eq!(t.note.as_deref(), Some(text));
    // Both reach the clients through the changes.
    let changed = store.live_after(before).unwrap();
    assert_eq!(changed.len(), 2);
}

#[test]
fn a_changed_title_and_a_written_note_stay() {
    let (mut store, owner, id) = voice_note();
    for (field, value) in [("title", "Plumber"), ("note", "Mine.")] {
        let change = Change::Set {
            object: owner,
            field: field.into(),
            value: json!(value),
            note: None,
            hlc: Hlc::new(2_000, 0, "pixel"),
        };
        assert!(applied(&store.apply(&change, &person(), now()).unwrap()));
    }
    store
        .store_transcript(id, &Transcribed::Text("Call the plumber.".into()), now())
        .unwrap();
    let t = task(&store, owner);
    assert_eq!(
        (t.title.as_str(), t.note.as_deref()),
        ("Plumber", Some("Mine."))
    );
    assert_eq!(
        attachment(&store, id).transcript.as_deref(),
        Some("Call the plumber.")
    );
}

#[test]
fn a_task_that_is_no_voice_note_keeps_title_and_note() {
    let (mut store, owner) = with_task("Offer", false);
    let (id, value) = recording(owner, "audio/mp4");
    assert!(applied(&create(&mut store, ObjectKind::Attachment, &value)));
    assert!(matches!(
        store.ask_transcript(id, now()).unwrap(),
        Asked::Pending(a) if a.transcript_state == Some(TranscriptState::Pending)
    ));
    store
        .store_transcript(id, &Transcribed::Text("Something said.".into()), now())
        .unwrap();
    let t = task(&store, owner);
    assert_eq!((t.title.as_str(), t.note), ("Offer", None));
}

#[test]
fn nothing_said_is_an_empty_transcript() {
    let (mut store, owner, id) = voice_note();
    store
        .store_transcript(id, &Transcribed::Text(" \n".into()), now())
        .unwrap();
    let a = attachment(&store, id);
    assert_eq!(
        (a.transcript.as_deref(), a.transcript_state),
        (Some(""), None)
    );
    assert_eq!(task(&store, owner).title, "Voice note 21.11. 10:00");
}

#[test]
fn a_failure_keeps_its_reason_and_asking_again_waits_again() {
    let (mut store, owner, id) = voice_note();
    store
        .store_transcript(
            id,
            &Transcribed::Failed("the service answered 503".into()),
            now(),
        )
        .unwrap();
    let a = attachment(&store, id);
    assert_eq!(a.transcript_state, Some(TranscriptState::Failed));
    assert_eq!(
        a.transcript_error.as_deref(),
        Some("the service answered 503")
    );
    assert!(store.pending_transcripts().unwrap().is_empty());
    assert_eq!(task(&store, owner).title, "Voice note 21.11. 10:00");

    assert!(matches!(
        store.ask_transcript(id, now()).unwrap(),
        Asked::Pending(_)
    ));
    let a = attachment(&store, id);
    assert_eq!(
        (a.transcript_state, a.transcript_error),
        (Some(TranscriptState::Pending), None)
    );
    assert_eq!(store.pending_transcripts().unwrap(), vec![id]);
}

#[test]
fn asking_needs_a_live_audio_attachment() {
    let (mut store, owner) = with_task("Offer", false);
    assert_eq!(
        store.ask_transcript(Id::new(), now()).unwrap(),
        Asked::Missing
    );
    assert_eq!(store.ask_transcript(owner, now()).unwrap(), Asked::Missing);
    let (picture, value) = recording(owner, "image/jpeg");
    assert!(applied(&create(&mut store, ObjectKind::Attachment, &value)));
    assert_eq!(
        store.ask_transcript(picture, now()).unwrap(),
        Asked::NotAudio
    );

    let (id, value) = recording(owner, "audio/webm");
    assert!(applied(&create(&mut store, ObjectKind::Attachment, &value)));
    assert!(matches!(
        store.ask_transcript(id, now()).unwrap(),
        Asked::Pending(_)
    ));
    let delete = Change::Delete {
        object: id,
        hlc: Hlc::new(3_000, 0, "pixel"),
    };
    assert!(applied(&store.apply(&delete, &person(), now()).unwrap()));
    assert!(store.pending_transcripts().unwrap().is_empty());
    assert!(
        !store
            .store_transcript(id, &Transcribed::Text("late".into()), now())
            .unwrap()
    );
}

#[test]
fn without_a_service_the_request_is_dropped() {
    let (mut store, owner, id) = voice_note();
    store.drop_transcript_request(id, now()).unwrap();
    let a = attachment(&store, id);
    assert_eq!((a.transcript, a.transcript_state), (None, None));
    assert!(store.pending_transcripts().unwrap().is_empty());
    assert_eq!(task(&store, owner).title, "Voice note 21.11. 10:00");
}
