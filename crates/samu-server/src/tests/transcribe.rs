// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Transcripts (ADR 0140) against a transcript service that runs in the
//! test: what it is sent, and what the server makes of its answers.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use jiff::Timestamp;
use samu_store::core::api::{Author, AuthorKind, Change, Hlc, ObjectKind};
use serde_json::{Value, json};

use super::{
    TempDir, call, call_bytes, create, new_task, sha256, sign_in, start, start_state, start_with,
};
use crate::app::AppState;
use crate::transcribe::Service;

const RECORDING: &[u8] = b"not really audio, but the server does not listen";

/// One request as the service got it.
#[derive(Debug, Clone)]
struct Heard {
    authorization: Option<String>,
    content_type: String,
    body: Vec<u8>,
}

/// What the service answers next, and what it was sent.
#[derive(Clone)]
struct Fake {
    answer: Arc<Mutex<(u16, String)>>,
    heard: Arc<Mutex<Vec<Heard>>>,
}

impl Fake {
    fn answers(&self, status: u16, body: &str) {
        *self.answer.lock().unwrap() = (status, body.to_string());
    }

    fn heard(&self) -> Vec<Heard> {
        self.heard.lock().unwrap().clone()
    }
}

/// A transcript service on a free port.
async fn service() -> (SocketAddr, Fake) {
    let fake = Fake {
        answer: Arc::new(Mutex::new((
            200,
            json!({"text": " Buy oat milk and call the plumber.\nBefore Friday. "}).to_string(),
        ))),
        heard: Arc::new(Mutex::new(Vec::new())),
    };
    let served = fake.clone();
    let app = Router::new().route(
        "/v1/audio/transcriptions",
        post(move |headers: HeaderMap, body: Bytes| {
            let fake = served.clone();
            async move {
                let header = |name: &str| {
                    headers
                        .get(name)
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string)
                };
                fake.heard.lock().unwrap().push(Heard {
                    authorization: header("authorization"),
                    content_type: header("content-type").unwrap_or_default(),
                    body: body.to_vec(),
                });
                let (status, body) = fake.answer.lock().unwrap().clone();
                (
                    StatusCode::from_u16(status).unwrap(),
                    [("content-type", "application/json")],
                    body,
                )
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (address, fake)
}

fn with_service(state: AppState, at: SocketAddr, token: Option<&str>) -> AppState {
    state.with_transcriber(Some(
        Service::new(
            &format!("http://{at}/v1/audio/transcriptions"),
            Some("large-v3-turbo"),
            token,
            Some("en"),
        )
        .unwrap(),
    ))
}

/// The recording on the server; its hash.
async fn upload(address: SocketAddr, token: &str) -> String {
    let hash = sha256(RECORDING);
    let (status, _, body) = call_bytes(
        address,
        "PUT",
        &format!("/blobs/{hash}"),
        &format!("Authorization: Bearer {token}\r\nContent-Type: audio/mp4\r\n"),
        RECORDING,
    )
    .await;
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    hash
}

fn recording(owner: &str, hash: &str, asks: bool) -> (String, Value) {
    let id = samu_store::core::api::Id::new().to_string();
    let mut value = json!({
        "id": id,
        "created": "2026-11-21T09:00:00Z",
        "modified": "2026-11-21T09:00:00Z",
        "owner": owner,
        "blob": hash,
        "name": "voice-2026-11-21.m4a",
        "media_type": "audio/mp4",
        "size": RECORDING.len()
    });
    if asks {
        value["transcript_state"] = json!("pending");
    }
    let change = json!({"op": "create", "object": id, "kind": "attachment", "value": value,
                        "hlc": {"wall": 0, "counter": 0, "device": "ignored"}});
    (id, change)
}

async fn write(address: SocketAddr, token: &str, changes: Value) {
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(token),
        Some(&changes),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    for outcome in answer.json().as_array().unwrap() {
        assert_eq!(outcome["outcome"], "applied", "{}", answer.body);
    }
}

async fn object(address: SocketAddr, token: &str, id: &str) -> Value {
    let answer = call(address, "GET", &format!("/objects/{id}"), Some(token), None).await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    answer.json()["object"].clone()
}

/// The attachment once `done` holds for it: the work runs in the
/// background.
async fn attachment_when(
    address: SocketAddr,
    token: &str,
    id: &str,
    done: impl Fn(&Value) -> bool,
) -> Value {
    for _ in 0..200 {
        let attachment = object(address, token, id).await;
        if done(&attachment) {
            return attachment;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!(
        "the attachment never got there: {}",
        object(address, token, id).await
    );
}

/// A voice note with its recording: the task and the attachment.
async fn voice_note(address: SocketAddr, token: &str) -> (String, String) {
    let hash = upload(address, token).await;
    let (task, mut value) = new_task("Voice note 21.11. 10:00");
    value["properties"] = json!({"source": "voice"});
    let (attachment, change) = recording(&task, &hash, true);
    write(address, token, json!([create(&task, &value), change])).await;
    (task, attachment)
}

/// §1, §3, §4: a voice note asks with its attachment; the service gets
/// the file, the model, the language and the token, and its text becomes
/// transcript, title and note.
#[tokio::test]
async fn a_voice_note_gets_transcript_title_and_note() {
    let (at, fake) = service().await;
    let (address, _dir) = start_with(true, |s| with_service(s, at, Some(" s3cret\n"))).await;
    let token = sign_in(address).await;
    let session = call(address, "GET", "/session", Some(&token), None).await;
    assert_eq!(session.json()["transcribe"], true, "{}", session.body);

    let (task, attachment) = voice_note(address, &token).await;
    let after = attachment_when(address, &token, &attachment, |a| {
        a["transcript"].is_string()
    })
    .await;
    assert_eq!(
        after["transcript"],
        "Buy oat milk and call the plumber.\nBefore Friday."
    );
    assert!(after.get("transcript_state").is_none(), "{after}");
    let task = object(address, &token, &task).await;
    assert_eq!(task["title"], "Buy oat milk and call the plumber.");
    assert_eq!(
        task["note"],
        "Buy oat milk and call the plumber.\nBefore Friday."
    );

    let heard = fake.heard();
    assert_eq!(heard.len(), 1);
    assert_eq!(heard[0].authorization.as_deref(), Some("Bearer s3cret"));
    let boundary = heard[0]
        .content_type
        .strip_prefix("multipart/form-data; boundary=")
        .unwrap();
    let body = String::from_utf8_lossy(&heard[0].body);
    assert!(body.starts_with(&format!("--{boundary}\r\n")), "{body}");
    for part in [
        "name=\"model\"\r\n\r\nlarge-v3-turbo\r\n",
        "name=\"language\"\r\n\r\nen\r\n",
        "name=\"file\"; filename=\"voice-2026-11-21.m4a\"\r\nContent-Type: audio/mp4\r\n\r\n\
         not really audio, but the server does not listen\r\n",
    ] {
        assert!(body.contains(part), "{part:?} is not in {body}");
    }
    assert!(!body.contains("response_format"), "{body}");
}

/// §1: without a service nothing is sent anywhere; a request that came
/// with an attachment is dropped, and the route says why.
#[tokio::test]
async fn without_a_service_nothing_waits() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let session = call(address, "GET", "/session", Some(&token), None).await;
    assert!(
        session.json().get("transcribe").is_none(),
        "{}",
        session.body
    );

    let (task, attachment) = voice_note(address, &token).await;
    let after = attachment_when(address, &token, &attachment, |a| {
        a.get("transcript_state").is_none()
    })
    .await;
    assert!(after.get("transcript").is_none(), "{after}");
    assert_eq!(
        object(address, &token, &task).await["title"],
        "Voice note 21.11. 10:00"
    );

    let answer = call(
        address,
        "POST",
        &format!("/attachments/{attachment}/transcribe"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(answer.status, 503, "{}", answer.body);
    assert!(
        answer.body.contains("SAMU_TRANSCRIBE_URL"),
        "{}",
        answer.body
    );
}

/// §3: the route asks for an attachment that is there already, answers
/// at once, and a failure keeps its reason until it is asked again.
#[tokio::test]
async fn the_route_asks_and_a_failure_can_be_asked_again() {
    let (at, fake) = service().await;
    let (address, _dir) = start_with(true, |s| with_service(s, at, None)).await;
    let token = sign_in(address).await;
    let hash = upload(address, &token).await;
    let (task, value) = new_task("Offer");
    let (attachment, change) = recording(&task, &hash, false);
    write(address, &token, json!([create(&task, &value), change])).await;
    assert!(fake.heard().is_empty());
    let path = format!("/attachments/{attachment}/transcribe");

    fake.answers(503, "model is loading\n");
    let answer = call(address, "POST", &path, Some(&token), None).await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    assert_eq!(answer.json()["transcript_state"], "pending");
    let failed = attachment_when(address, &token, &attachment, |a| {
        a["transcript_state"] == "failed"
    })
    .await;
    assert_eq!(
        failed["transcript_error"],
        "the transcript service answered 503: model is loading"
    );
    assert!(fake.heard()[0].authorization.is_none());

    fake.answers(200, r#"{"text": "Send the offer."}"#);
    let answer = call(address, "POST", &path, Some(&token), None).await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    assert!(answer.json().get("transcript_error").is_none());
    let after = attachment_when(address, &token, &attachment, |a| {
        a["transcript"].is_string()
    })
    .await;
    assert_eq!(after["transcript"], "Send the offer.");
    assert!(after.get("transcript_error").is_none(), "{after}");
    assert_eq!(fake.heard().len(), 2);
    // Not a voice note: title and note are the task's own.
    let task = object(address, &token, &task).await;
    assert_eq!(task["title"], "Offer");
    assert!(task.get("note").is_none(), "{task}");
}

/// §3: only a live audio attachment can be asked for.
#[tokio::test]
async fn the_route_refuses_what_is_not_an_audio_attachment() {
    let (at, fake) = service().await;
    let (address, _dir) = start_with(true, |s| with_service(s, at, None)).await;
    let token = sign_in(address).await;
    let (task, value) = new_task("Offer");
    let text = b"plain";
    let hash = sha256(text);
    let (status, _, _) = call_bytes(
        address,
        "PUT",
        &format!("/blobs/{hash}"),
        &format!("Authorization: Bearer {token}\r\nContent-Type: text/plain\r\n"),
        text,
    )
    .await;
    assert_eq!(status, 200);
    let file = samu_store::core::api::Id::new().to_string();
    let attach = json!({"op": "create", "object": file, "kind": "attachment",
        "value": {"id": file, "created": "2026-11-21T09:00:00Z",
                  "modified": "2026-11-21T09:00:00Z", "owner": task, "blob": hash,
                  "name": "note.txt", "media_type": "text/plain", "size": text.len()},
        "hlc": {"wall": 0, "counter": 0, "device": "ignored"}});
    write(address, &token, json!([create(&task, &value), attach])).await;

    let ask = |id: String| {
        let token = token.clone();
        async move {
            call(
                address,
                "POST",
                &format!("/attachments/{id}/transcribe"),
                Some(&token),
                None,
            )
            .await
        }
    };
    let answer = ask(file).await;
    assert_eq!(answer.status, 400, "{}", answer.body);
    assert!(answer.body.contains("is not audio"), "{}", answer.body);
    assert_eq!(ask(task).await.status, 404);
    assert_eq!(
        ask(samu_store::core::api::Id::new().to_string())
            .await
            .status,
        404
    );
    assert_eq!(ask("nonsense".into()).await.status, 400);
    let answer = call(address, "POST", "/attachments/x/transcribe", None, None).await;
    assert_eq!(answer.status, 401);
    assert!(fake.heard().is_empty());
}

/// §3: what waited when the server stopped is picked up when it starts.
#[tokio::test]
async fn what_waited_is_picked_up_at_the_start() {
    let (at, fake) = service().await;
    let (address, dir, state) = start_state(true, |s| with_service(s, at, None)).await;
    let token = sign_in(address).await;
    let hash = upload(address, &token).await;
    let (task, mut value) = new_task("Voice note 21.11. 10:00");
    value["properties"] = json!({"source": "voice"});
    let (attachment, change) = recording(&task, &hash, true);
    // Written past the route, as if the server had stopped before it heard.
    {
        let store = state.account("privat").unwrap();
        let mut store = store.lock().unwrap();
        let by = Author {
            device: "pixel".into(),
            kind: AuthorKind::Human,
        };
        for (n, (id, kind, value)) in [
            (&task, ObjectKind::Task, &value),
            (&attachment, ObjectKind::Attachment, &change["value"]),
        ]
        .into_iter()
        .enumerate()
        {
            let change = Change::Create {
                object: id.parse().unwrap(),
                kind,
                value: value.clone(),
                hlc: Hlc::new(1_000 + i64::try_from(n).unwrap(), 0, "pixel"),
            };
            store.apply(&change, &by, Timestamp::now()).unwrap();
        }
    }
    assert!(fake.heard().is_empty());
    assert_eq!(
        object(address, &token, &attachment).await["transcript_state"],
        "pending"
    );

    crate::transcribe::resume(state.clone()).await;
    assert_eq!(fake.heard().len(), 1);
    let after = object(address, &token, &attachment).await;
    assert_eq!(
        after["transcript"],
        "Buy oat milk and call the plumber.\nBefore Friday."
    );
    assert_eq!(
        object(address, &token, &task).await["title"],
        "Buy oat milk and call the plumber."
    );
    drop::<TempDir>(dir);
}

/// A service that is not there is a failure with a reason, not a hang.
#[tokio::test]
async fn a_service_that_is_away_fails_the_transcript() {
    // A port nothing listens on: bound once, then closed.
    let gone = {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap()
    };
    let (address, _dir) = start_with(true, |s| with_service(s, gone, None)).await;
    let token = sign_in(address).await;
    let (_, attachment) = voice_note(address, &token).await;
    let failed = attachment_when(address, &token, &attachment, |a| {
        a["transcript_state"] == "failed"
    })
    .await;
    let reason = failed["transcript_error"].as_str().unwrap();
    assert!(
        reason.starts_with("the transcript service did not answer"),
        "{reason}"
    );
    assert!(!reason.contains("127.0.0.1"), "{reason}");
}
