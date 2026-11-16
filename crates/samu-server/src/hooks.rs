// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Named hooks (ADR 0010, ADR 0022): managing them with a session, and the
//! delivery route a sender reaches without one.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{FromRequest, Multipart, Path as UrlPath, Request, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use jiff::Timestamp;
use samu_store::core::api::{
    Author, AuthorKind, Change, ErrorCode, Hlc, HookChange, HookChangeResponse, HookDelivery,
    HookFormat, HookView, Id, ObjectKind,
};
use samu_store::core::clock::next_stamp;
use samu_store::core::hook::{
    INDEX_BODY_LIMIT, INDEX_RING_SYSTEM, IndexHeaders, index_field, index_header, ring_task,
    verify_index_request,
};
use samu_store::{Delivered, HookRow};

use crate::app::{AppState, Failure, Session, lock};

/// How long a delivery may take to arrive.
const RECEIVING_TIME: std::time::Duration = std::time::Duration::from_secs(120);

fn view(row: HookRow) -> HookView {
    HookView {
        name: row.name,
        format: row.format,
        created: row.created,
    }
}

/// `GET /hooks`: the hooks of the signed-in account.
pub(crate) async fn list(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
) -> Result<Json<Vec<HookView>>, Failure> {
    let hooks = state.server()?.hooks(&session.account)?;
    Ok(Json(hooks.into_iter().map(view).collect()))
}

/// `POST /hooks`: make, remove or renew a hook of the signed-in account.
pub(crate) async fn change(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    Json(change): Json<HookChange>,
) -> Result<Json<HookChangeResponse>, Failure> {
    let mut server = state.server()?;
    let response = match change {
        HookChange::Add { name, format } => {
            let (row, secret) = server
                .add_hook(&session.account, &name, format, Timestamp::now())
                .map_err(|e| match e {
                    samu_store::Error::Unsupported { what } => Failure::bad_request(what),
                    other => Failure::from(other),
                })?;
            HookChangeResponse {
                hook: Some(view(row)),
                secret: Some(secret),
                removed: false,
            }
        }
        HookChange::Remove { name } => {
            if !server.remove_hook(&session.account, &name)? {
                return Err(Failure::not_found(format!("no hook {name:?}")));
            }
            HookChangeResponse {
                hook: None,
                secret: None,
                removed: true,
            }
        }
        HookChange::Secret { name } => {
            let (row, secret) = server
                .renew_hook_secret(&session.account, &name)?
                .ok_or_else(|| Failure::not_found(format!("no hook {name:?}")))?;
            HookChangeResponse {
                hook: Some(view(row)),
                secret: Some(secret),
                removed: false,
            }
        }
    };
    Ok(Json(response))
}

/// Refused for whatever reason the sender should not retry: an unknown
/// hook looks like a wrong signature (ADR 0022 §4).
fn refused(message: impl Into<String>) -> Failure {
    Failure::unauthorized(message)
}

/// `POST /hooks/{name}`: a delivery, signed with the hook's secret. No
/// session; the signature is the sign-in.
pub(crate) async fn deliver(
    State(state): State<Arc<AppState>>,
    UrlPath(name): UrlPath<String>,
    request: Request,
) -> Result<(StatusCode, Json<HookDelivery>), Failure> {
    let hook = state.server()?.hook(&name)?;
    let Some((hook, secret)) = hook else {
        return Err(refused(
            "the signature does not match; is the hook's secret entered in the Pebble app?",
        ));
    };

    let (parts, body) = request.into_parts();
    let bytes = tokio::time::timeout(RECEIVING_TIME, axum::body::to_bytes(body, INDEX_BODY_LIMIT))
        .await
        .map_err(|_| {
            Failure::status(
                StatusCode::REQUEST_TIMEOUT,
                ErrorCode::BadRequest,
                "the delivery did not arrive in time",
            )
        })?
        .map_err(|_| {
            Failure::status(
                StatusCode::PAYLOAD_TOO_LARGE,
                ErrorCode::BadRequest,
                "the delivery is larger than 64 MiB",
            )
        })?;

    match hook.format {
        HookFormat::IndexV1 => deliver_index(&state, &hook, &secret, parts, bytes).await,
        HookFormat::Unknown => Err(Failure::bad_request(
            "a hook format this server cannot read",
        )),
    }
}

async fn deliver_index(
    state: &AppState,
    hook: &HookRow,
    secret: &str,
    parts: axum::http::request::Parts,
    bytes: axum::body::Bytes,
) -> Result<(StatusCode, Json<HookDelivery>), Failure> {
    let header = |name: &str| parts.headers.get(name).and_then(|v| v.to_str().ok());
    let headers = IndexHeaders {
        version: header(index_header::VERSION),
        timestamp: header(index_header::TIMESTAMP),
        delivery: header(index_header::DELIVERY),
        trigger: header(index_header::TRIGGER),
        test: header(index_header::TEST),
        signature: header(index_header::SIGNATURE),
    };
    let now = Timestamp::now();
    let delivery =
        verify_index_request(secret, &headers, &bytes, now.as_second()).map_err(|refusal| {
            let status = StatusCode::from_u16(refusal.status()).unwrap_or(StatusCode::BAD_REQUEST);
            let code = if status == StatusCode::UNAUTHORIZED {
                ErrorCode::Unauthorized
            } else {
                ErrorCode::BadRequest
            };
            Failure::status(status, code, refusal.to_string())
        })?;

    if delivery.test {
        return Ok((
            StatusCode::OK,
            Json(HookDelivery {
                task: None,
                created: false,
                test: true,
            }),
        ));
    }

    let request = Request::from_parts(parts, Body::from(bytes));
    let multipart = Multipart::from_request(request, &())
        .await
        .map_err(|e| Failure::bad_request(format!("not a form: {}", e.body_text())))?;
    let form = read_index_form(multipart).await?;

    let store = state.account(&hook.account)?;
    let mut store = lock(&store)?;
    // The time of the recording, not of the delivery (ADR 0010 §3); a form
    // without one is still filed, so nothing is lost silently.
    let recorded = form.recorded_at.unwrap_or(now);
    let text = form.transcription.unwrap_or_default();
    let zone = store
        .settings()
        .map(|s| s.zone)
        .unwrap_or(jiff::tz::TimeZone::UTC);
    let task = ring_task(Id::new(), &text, recorded, now, &zone, None);
    let (id, task) = (
        task.id,
        serde_json::to_value(&task).map_err(|e| Failure::internal(e.to_string()))?,
    );
    let device = format!("hook:{}", hook.name);
    let seen = store.observed()?;
    let stamp: Hlc = next_stamp(&device, None, seen.as_ref(), now);
    let change = Change::Create {
        object: id,
        kind: ObjectKind::Task,
        value: task,
        hlc: stamp,
    };
    // What the ring brings, a person said (ADR 0039 §1).
    let author = Author {
        device,
        kind: AuthorKind::Human,
    };
    match store.apply_delivered(INDEX_RING_SYSTEM, &delivery.delivery, &change, &author, now)? {
        Delivered::Created(task) => Ok((
            StatusCode::CREATED,
            Json(HookDelivery {
                task: Some(task),
                created: true,
                test: false,
            }),
        )),
        Delivered::Found(task) => Ok((
            StatusCode::OK,
            Json(HookDelivery {
                task: Some(task),
                created: false,
                test: false,
            }),
        )),
        // Our own task refused by our own store: a fault here, but answered
        // as a 4xx would be retried never; a 500 lets the app try again.
        Delivered::Rejected(reason) => Err(Failure::internal(format!(
            "the task from the ring was refused: {reason}"
        ))),
    }
}

/// What an `index-v1` form brought. The audio is read and dropped
/// (ADR 0010 §4).
#[derive(Default)]
struct IndexForm {
    transcription: Option<String>,
    recorded_at: Option<Timestamp>,
}

/// Reads the fields this hook uses; others are passed over, because the
/// app adds fields without a new version.
async fn read_index_form(mut multipart: Multipart) -> Result<IndexForm, Failure> {
    let mut form = IndexForm::default();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| Failure::bad_request(format!("the form: {}", e.body_text())))?
    {
        let name = field.name().unwrap_or_default().to_string();
        let read = |e: axum::extract::multipart::MultipartError| {
            Failure::bad_request(format!("the field {name:?}: {}", e.body_text()))
        };
        match name.as_str() {
            index_field::TRANSCRIPTION => {
                form.transcription = Some(field.text().await.map_err(read)?);
            }
            index_field::RECORDED_AT => {
                let text = field.text().await.map_err(read)?;
                let millis: i64 = text.trim().parse().map_err(|_| {
                    Failure::bad_request(format!(
                        "{}: {text:?} is not Unix milliseconds",
                        index_field::RECORDED_AT
                    ))
                })?;
                form.recorded_at = Some(Timestamp::from_millisecond(millis).map_err(|e| {
                    Failure::bad_request(format!("{}: {e}", index_field::RECORDED_AT))
                })?);
            }
            // The audio and anything else: read to the end, then dropped.
            _ => {
                let _ = field.bytes().await.map_err(read)?;
            }
        }
    }
    Ok(form)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use std::net::SocketAddr;

    use samu_store::core::hook::index_signature;
    use serde_json::{Value, json};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use crate::tests::{Answer, TempDir, call, sign_in, start};

    const BOUNDARY: &str = "samu-test-boundary";

    fn form(text: Option<&str>, recorded_at: i64) -> Vec<u8> {
        let mut body = String::new();
        if let Some(text) = text {
            body.push_str(&format!(
                "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"transcription\"\r\n\r\n{text}\r\n"
            ));
        }
        body.push_str(&format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"recordedAt\"\r\n\r\n{recorded_at}\r\n\
             --{BOUNDARY}\r\nContent-Disposition: form-data; name=\"audio\"; filename=\"r.m4a\"\r\n\
             Content-Type: audio/mp4\r\n\r\nnot really audio\r\n\
             --{BOUNDARY}\r\nContent-Disposition: form-data; name=\"client\"\r\n\r\nring\r\n\
             --{BOUNDARY}--\r\n"
        ));
        body.into_bytes()
    }

    /// A delivery as the Pebble app sends it, signed with `secret`.
    async fn deliver(
        address: SocketAddr,
        hook: &str,
        secret: &str,
        delivery: &str,
        test: bool,
        body: &[u8],
    ) -> Answer {
        let now = jiff::Timestamp::now().as_second();
        let trigger = if test {
            "test-event"
        } else {
            "double-click-hold"
        };
        let signature = index_signature(secret, now, delivery, trigger, test, body);
        let mut head = format!(
            "POST /hooks/{hook} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\
             Content-Type: multipart/form-data; boundary={BOUNDARY}\r\n\
             Content-Length: {}\r\n\
             X-Index-Webhook-Version: 1\r\nX-Index-Timestamp: {now}\r\n\
             X-Index-Delivery: {delivery}\r\nX-Index-Trigger: {trigger}\r\n\
             X-Index-Signature: {signature}\r\n",
            body.len()
        );
        if test {
            head.push_str("X-Index-Test: true\r\n");
        }
        head.push_str("\r\n");
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        stream.write_all(head.as_bytes()).await.unwrap();
        stream.write_all(body).await.unwrap();
        let mut raw = String::new();
        stream.read_to_string(&mut raw).await.unwrap();
        let (head, body) = raw.split_once("\r\n\r\n").unwrap();
        Answer {
            status: head.split(' ').nth(1).unwrap().parse().unwrap(),
            headers: head.to_string(),
            body: body.to_string(),
        }
    }

    async fn add_ring(address: SocketAddr, token: &str) -> String {
        let answer = call(
            address,
            "POST",
            "/hooks",
            Some(token),
            Some(&json!({"action": "add", "name": "ring", "format": "index-v1"})),
        )
        .await;
        assert_eq!(answer.status, 200, "{}", answer.body);
        answer.json()["secret"].as_str().unwrap().to_string()
    }

    /// Title, note, created and project of a task.
    type Row = (String, Option<String>, String, Option<Vec<u8>>);

    /// The tasks in the account database, asked of the table itself.
    fn tasks(dir: &TempDir) -> Vec<Row> {
        let conn = rusqlite::Connection::open(dir.0.join("accounts/privat.db")).unwrap();
        conn.prepare(
            "SELECT t.title, t.note, o.created, t.project FROM task t \
             JOIN object o ON o.id = t.object ORDER BY o.created",
        )
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect()
    }

    fn task_count(dir: &TempDir) -> i64 {
        let path = dir.0.join("accounts/privat.db");
        if !path.exists() {
            return 0;
        }
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.query_row("SELECT count(*) FROM task", [], |r| r.get(0))
            .unwrap()
    }

    #[tokio::test]
    async fn a_signed_delivery_puts_one_task_into_the_inbox() {
        let (address, dir) = start(true).await;
        let token = sign_in(address).await;
        let secret = add_ring(address, &token).await;

        let body = form(
            Some("Milch kaufen\nund Brot nicht vergessen"),
            1_762_506_000_000,
        );
        let first = deliver(address, "ring", &secret, "rec-1", false, &body).await;
        assert_eq!(first.status, 201, "{}", first.body);
        assert_eq!(first.json()["created"], json!(true));
        let id = first.json()["task"].clone();

        // The app retries: the same delivery finds the same task.
        let again = deliver(address, "ring", &secret, "rec-1", false, &body).await;
        assert_eq!(again.status, 200, "{}", again.body);
        assert_eq!(again.json()["created"], json!(false));
        assert_eq!(again.json()["task"], id);

        let tasks = tasks(&dir);
        assert_eq!(tasks.len(), 1, "{tasks:?}");
        let (title, note, created, project) = &tasks[0];
        assert_eq!(title, "Milch kaufen");
        assert_eq!(
            note.as_deref(),
            Some("Milch kaufen\nund Brot nicht vergessen")
        );
        assert_eq!(created, "2025-11-07T09:00:00Z");
        assert_eq!(*project, None, "into the inbox");

        // The task is stamped with the hook as its device.
        let object: Value = call(
            address,
            "GET",
            &format!("/objects/{}", id.as_str().unwrap()),
            Some(&token),
            None,
        )
        .await
        .json();
        assert_eq!(object["object"]["title"], json!("Milch kaufen"));
        assert_eq!(
            object["object"]["properties"],
            json!({"source": "index-ring"})
        );
        let conn = rusqlite::Connection::open(dir.0.join("accounts/privat.db")).unwrap();
        let device: String = conn
            .query_row(
                "SELECT s.device FROM stamp s JOIN object o ON o.id = s.object \
                 WHERE s.field = '@create' AND o.kind = 'task'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(device, "hook:ring");
        let source: String = conn
            .query_row("SELECT value FROM property WHERE key = 'source'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(source, "index-ring");
    }

    #[tokio::test]
    async fn a_wrong_or_missing_signature_makes_nothing() {
        let (address, dir) = start(true).await;
        let token = sign_in(address).await;
        let secret = add_ring(address, &token).await;
        let body = form(Some("Milch"), 1_762_506_000_000);

        let wrong = deliver(address, "ring", "not the secret", "rec-1", false, &body).await;
        assert_eq!(wrong.status, 401, "{}", wrong.body);
        let unknown = deliver(address, "nope", &secret, "rec-1", false, &body).await;
        assert_eq!(unknown.status, 401, "{}", unknown.body);
        let unsigned = call(address, "POST", "/hooks/ring", None, Some(&json!({}))).await;
        assert_eq!(unsigned.status, 401, "{}", unsigned.body);
        assert_eq!(task_count(&dir), 0);

        // The probe that must find something: the right secret does.
        let right = deliver(address, "ring", &secret, "rec-1", false, &body).await;
        assert_eq!(right.status, 201, "{}", right.body);
        assert_eq!(task_count(&dir), 1);
    }

    #[tokio::test]
    async fn a_test_event_makes_nothing() {
        let (address, dir) = start(true).await;
        let token = sign_in(address).await;
        let secret = add_ring(address, &token).await;
        let answer = deliver(
            address,
            "ring",
            &secret,
            "test-1",
            true,
            &form(Some("Test"), 0),
        )
        .await;
        assert_eq!(answer.status, 200, "{}", answer.body);
        assert_eq!(answer.json()["test"], json!(true));
        assert_eq!(task_count(&dir), 0);
    }

    #[tokio::test]
    async fn a_recording_without_text_is_kept_with_a_title_of_its_own() {
        let (address, dir) = start(true).await;
        let token = sign_in(address).await;
        let secret = add_ring(address, &token).await;
        // 07.11.2025 09:00 UTC is 10:00 in Berlin.
        let answer = deliver(
            address,
            "ring",
            &secret,
            "rec-2",
            false,
            &form(None, 1_762_506_000_000),
        )
        .await;
        assert_eq!(answer.status, 201, "{}", answer.body);
        let tasks = tasks(&dir);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].0, "Aufnahme vom Ring, 07.11.2025 10:00");
        assert_eq!(tasks[0].1, None);
    }

    #[tokio::test]
    async fn hooks_belong_to_the_signed_in_account() {
        let (address, _dir) = start(true).await;
        let token = sign_in(address).await;
        let secret = add_ring(address, &token).await;

        let listed = call(address, "GET", "/hooks", Some(&token), None).await;
        assert_eq!(listed.status, 200);
        let listed = listed.json();
        assert_eq!(listed.as_array().unwrap().len(), 1);
        assert_eq!(listed[0]["name"], json!("ring"));
        assert!(listed[0].get("secret").is_none(), "no secret in a view");

        let twice = call(
            address,
            "POST",
            "/hooks",
            Some(&token),
            Some(&json!({"action": "add", "name": "ring", "format": "index-v1"})),
        )
        .await;
        assert_eq!(twice.status, 400, "{}", twice.body);

        let renewed = call(
            address,
            "POST",
            "/hooks",
            Some(&token),
            Some(&json!({"action": "secret", "name": "ring"})),
        )
        .await;
        assert_eq!(renewed.status, 200);
        let new_secret = renewed.json()["secret"].as_str().unwrap().to_string();
        assert_ne!(new_secret, secret);
        let body = form(Some("Milch"), 1_762_506_000_000);
        let old = deliver(address, "ring", &secret, "rec-1", false, &body).await;
        assert_eq!(old.status, 401, "the old secret stops at once");

        let removed = call(
            address,
            "POST",
            "/hooks",
            Some(&token),
            Some(&json!({"action": "remove", "name": "ring"})),
        )
        .await;
        assert_eq!(removed.json()["removed"], json!(true));
        let gone = deliver(address, "ring", &new_secret, "rec-1", false, &body).await;
        assert_eq!(gone.status, 401);

        let none = call(address, "GET", "/hooks", None, None).await;
        assert_eq!(none.status, 401, "managing hooks needs a session");
    }
}
