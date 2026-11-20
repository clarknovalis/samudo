// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The API in process, over real TCP (ADR 0019).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::http::HeaderMap;
use jiff::Timestamp;
use samu_store::ServerStore;
use samu_store::core::api::{ApiError, ErrorCode, ROUTES, SessionInfo};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::app::{AppState, router};

pub(crate) struct TempDir(pub(crate) PathBuf);

impl TempDir {
    pub(crate) fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "samu-server-{}-{}",
            std::process::id(),
            Timestamp::now().as_nanosecond()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

pub(crate) const PASSWORD: &str = "correct horse";

/// A server on a free port with one account `privat`, or none.
pub(crate) async fn start(with_account: bool) -> (SocketAddr, TempDir) {
    start_with_key(with_account, None).await
}

/// The same, with the key that seals calendar sources (ADR 0047 §3).
pub(crate) async fn start_with_key(
    with_account: bool,
    key: Option<samu_store::SecretKey>,
) -> (SocketAddr, TempDir) {
    start_with(with_account, |state| state.with_secret_key(key)).await
}

/// The same, with a state made to measure.
pub(crate) async fn start_with(
    with_account: bool,
    made: impl FnOnce(AppState) -> AppState,
) -> (SocketAddr, TempDir) {
    let (address, dir, _) = start_state(with_account, made).await;
    (address, dir)
}

/// The same, and the state to look into.
pub(crate) async fn start_state(
    with_account: bool,
    made: impl FnOnce(AppState) -> AppState,
) -> (SocketAddr, TempDir, Arc<AppState>) {
    let dir = TempDir::new();
    if with_account {
        let mut server = ServerStore::open(&dir.0.join("server.db")).unwrap();
        server
            .add_account("privat", PASSWORD, Timestamp::now())
            .unwrap();
    }
    let state = Arc::new(made(AppState::new(&dir.0).unwrap()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let served = state.clone();
    tokio::spawn(async move {
        axum::serve(listener, router(served)).await.unwrap();
    });
    (address, dir, state)
}

pub(crate) struct Answer {
    pub(crate) status: u16,
    pub(crate) headers: String,
    pub(crate) body: String,
}

impl Answer {
    pub(crate) fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

/// One request with `Connection: close`, read to the end.
pub(crate) async fn call(
    address: SocketAddr,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&Value>,
) -> Answer {
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let body = body.map(Value::to_string).unwrap_or_default();
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{auth}\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).await.unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    Answer {
        status,
        headers: head.to_string(),
        body: body.to_string(),
    }
}

/// Opens a request that does not end (`/events`) and reads until `until`
/// shows up, or gives up after five seconds.
pub(crate) async fn stream_until(
    address: SocketAddr,
    path: &str,
    token: &str,
    extra: &str,
    until: &str,
) -> String {
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {token}\r\n{extra}\r\n"
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut seen = String::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut buffer = [0u8; 4096];
    while !seen.contains(until) {
        let read = tokio::time::timeout_at(deadline, stream.read(&mut buffer)).await;
        match read {
            Ok(Ok(n)) if n > 0 => seen.push_str(&String::from_utf8_lossy(&buffer[..n])),
            _ => break,
        }
    }
    seen
}

pub(crate) async fn sign_in(address: SocketAddr) -> String {
    let answer = call(
        address,
        "POST",
        "/login",
        None,
        Some(&json!({"account": "privat", "password": PASSWORD, "device": "laptop"})),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    let session: SessionInfo = serde_json::from_str(&answer.body).unwrap();
    session.token.unwrap()
}

/// ADR 0039 §1, §2: an agent signs in as one, the session says so, and
/// what it writes carries it as the author, whatever the client claims.
#[tokio::test]
async fn an_agent_session_writes_as_an_agent() {
    let (address, _dir) = start(true).await;
    let login = call(
        address,
        "POST",
        "/login",
        None,
        Some(&json!({"account": "privat", "password": PASSWORD,
                     "device": "claude", "kind": "agent"})),
    )
    .await;
    assert_eq!(login.status, 200, "{}", login.body);
    let token = serde_json::from_str::<SessionInfo>(&login.body)
        .unwrap()
        .token
        .unwrap();
    let session = call(address, "GET", "/session", Some(&token), None).await;
    assert_eq!(session.json()["kind"], "agent");
    // A person's session says so too.
    let person = sign_in(address).await;
    let session = call(address, "GET", "/session", Some(&person), None).await;
    assert_eq!(session.json()["kind"], "human");

    let id = samu_store::core::api::Id::new();
    let create = json!([{
        "op": "create", "object": id.to_string(), "kind": "task",
        "value": {
            "id": id.to_string(), "created": "2026-11-08T18:00:00Z",
            "modified": "2026-11-08T18:00:00Z", "title": "Befund",
            "created_by": {"device": "laptop", "kind": "human"},
            "flagged": false, "sequential": false,
            "complete_with_last_child": false, "position": 0
        },
        "hlc": {"wall": 1_762_624_800_000_i64, "counter": 0, "device": "claude"}
    }]);
    let applied = call(address, "POST", "/changes", Some(&token), Some(&create)).await;
    assert_eq!(applied.json()[0]["outcome"], "applied", "{}", applied.body);
    let task = call(
        address,
        "GET",
        &format!("/objects/{id}"),
        Some(&person),
        None,
    )
    .await;
    assert_eq!(
        task.json()["object"]["created_by"],
        json!({"device": "claude", "kind": "agent"}),
        "{}",
        task.body
    );
}

/// ADR 0039 §2: a kind the server does not know signs nobody in.
#[tokio::test]
async fn an_unknown_session_kind_is_refused() {
    let (address, _dir) = start(true).await;
    let login = call(
        address,
        "POST",
        "/login",
        None,
        Some(&json!({"account": "privat", "password": PASSWORD,
                     "device": "x", "kind": "robot"})),
    )
    .await;
    assert_eq!(login.status, 400, "{}", login.body);
}

#[tokio::test]
async fn health_is_open() {
    let (address, _dir) = start(false).await;
    let answer = call(address, "GET", "/health", None, None).await;
    assert_eq!(answer.status, 200);
    assert_eq!(answer.json()["status"], "ok");
    assert_eq!(answer.json()["version"], crate::app::VERSION);
    assert_eq!(answer.json()["api"], samu_store::core::api::VERSION);
}

#[tokio::test]
async fn signing_in_gives_a_session_a_cookie_and_a_token() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;

    let answer = call(address, "GET", "/session", Some(&token), None).await;
    assert_eq!(answer.status, 200);
    let session: SessionInfo = serde_json::from_str(&answer.body).unwrap();
    assert_eq!(
        (session.account.as_str(), session.device.as_str()),
        ("privat", "laptop")
    );
    assert!(
        session.token.is_none(),
        "the token is only in the answer to /login"
    );
    // The account's zone and due-soon span, for clients that compute
    // availability themselves (ADR 0028 §6); a new account has the defaults.
    assert!(
        session.zone.as_deref().is_some_and(|z| !z.is_empty()),
        "{session:?}"
    );
    assert_eq!(
        session.due_soon.map(|p| p.0.fieldwise()),
        Some(jiff::ToSpan::days(2).fieldwise())
    );
    // When a reminder on a date goes off (ADR 0030 §3).
    assert_eq!(session.remind_time, Some(jiff::civil::time(9, 0, 0, 0)));

    let login = call(
        address,
        "POST",
        "/login",
        None,
        Some(&json!({"account": "privat", "password": PASSWORD, "device": "phone"})),
    )
    .await;
    assert!(login.headers.contains("samu_session="), "{}", login.headers);
    assert!(login.headers.contains("HttpOnly"));
}

#[tokio::test]
async fn a_wrong_name_and_a_wrong_password_look_the_same() {
    let (address, _dir) = start(true).await;
    let wrong_password = call(
        address,
        "POST",
        "/login",
        None,
        Some(&json!({"account": "privat", "password": "nope nope nope", "device": "x"})),
    )
    .await;
    let wrong_name = call(
        address,
        "POST",
        "/login",
        None,
        Some(&json!({"account": "nobody", "password": PASSWORD, "device": "x"})),
    )
    .await;
    assert_eq!(wrong_password.status, 401);
    assert_eq!(wrong_password.status, wrong_name.status);
    assert_eq!(wrong_password.body, wrong_name.body);
}

/// A sign-in with this password, answered.
async fn try_password(address: SocketAddr, account: &str, password: &str) -> Answer {
    call(
        address,
        "POST",
        "/login",
        None,
        Some(&json!({"account": account, "password": password, "device": "x"})),
    )
    .await
}

#[tokio::test]
async fn five_wrong_passwords_hold_the_name_with_429() {
    let (address, _dir) = start(true).await;
    for _ in 0..crate::brake::PER_ACCOUNT {
        assert_eq!(try_password(address, "privat", "nope").await.status, 401);
    }
    // Now also the right password waits, under any spelling of the name,
    // and the answer still says unauthorized (ADR 0097 §1).
    let held = try_password(address, "Privat", PASSWORD).await;
    assert_eq!(held.status, 429, "{}", held.body);
    assert!(
        held.headers.to_lowercase().contains("retry-after: "),
        "{}",
        held.headers
    );
    let error: ApiError = serde_json::from_str(&held.body).unwrap();
    assert_eq!(error.error, ErrorCode::Unauthorized);
    assert!(error.message.contains("try again in"), "{}", error.message);
    // Another name is not held.
    assert_eq!(try_password(address, "nobody", "nope").await.status, 401);
}

/// ADR 0098 §1: an account with a public password is not held, so no
/// one locks a demo for everyone.
#[tokio::test]
async fn an_unbraked_account_is_never_held() {
    let (address, _dir) = start_with(true, |state| state.with_unbraked(&["Privat".into()])).await;
    for _ in 0..=crate::brake::PER_ACCOUNT {
        assert_eq!(try_password(address, "privat", "nope").await.status, 401);
    }
    assert_eq!(try_password(address, "privat", PASSWORD).await.status, 200);
}

#[tokio::test]
async fn a_good_sign_in_forgets_the_failures() {
    let (address, _dir) = start(true).await;
    for _ in 1..crate::brake::PER_ACCOUNT {
        assert_eq!(try_password(address, "privat", "nope").await.status, 401);
    }
    assert_eq!(try_password(address, "privat", PASSWORD).await.status, 200);
    for _ in 1..crate::brake::PER_ACCOUNT {
        assert_eq!(try_password(address, "privat", "nope").await.status, 401);
    }
    assert_eq!(try_password(address, "privat", PASSWORD).await.status, 200);
}

#[tokio::test]
async fn wrong_pairing_codes_hold_pairing_with_429() {
    let (address, _dir) = start(true).await;
    let wrong = json!({"code": "WRONG-CODE"});
    let pair = || call(address, "POST", "/pair", None, Some(&wrong));
    for _ in 0..crate::brake::PAIRING {
        assert_eq!(pair().await.status, 401);
    }
    let held = pair().await;
    assert_eq!(held.status, 429, "{}", held.body);
    // Signing in with a password is another brake.
    assert_eq!(try_password(address, "privat", PASSWORD).await.status, 200);
}

#[tokio::test]
async fn without_a_session_the_routes_are_closed() {
    let (address, _dir) = start(true).await;
    for (method, path) in [("GET", "/session"), ("POST", "/logout")] {
        let answer = call(address, method, path, None, None).await;
        assert_eq!(answer.status, 401, "{method} {path}");
        let error: ApiError = serde_json::from_str(&answer.body).unwrap();
        assert_eq!(error.error, ErrorCode::Unauthorized);
        let forged = call(address, method, path, Some("0000"), None).await;
        assert_eq!(forged.status, 401, "{method} {path} with a made-up token");
    }
}

#[tokio::test]
async fn without_an_account_nothing_but_health_is_open() {
    let (address, _dir) = start(false).await;
    let login = call(
        address,
        "POST",
        "/login",
        None,
        Some(&json!({"account": "privat", "password": PASSWORD, "device": "x"})),
    )
    .await;
    assert_eq!(login.status, 401);
    assert!(
        login.body.contains("samu-server account add"),
        "{}",
        login.body
    );
}

#[tokio::test]
async fn logout_ends_the_session() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    assert_eq!(
        call(address, "POST", "/logout", Some(&token), None)
            .await
            .status,
        204
    );
    assert_eq!(
        call(address, "GET", "/session", Some(&token), None)
            .await
            .status,
        401
    );
}

/// Every route in `samu_api::ROUTES` is served (ADR 0019 §7): neither 404
/// nor 405, signed in.
#[tokio::test]
async fn every_listed_route_is_served() {
    let (address, _dir) = start(true).await;
    for route in ROUTES {
        // A fresh session each time: /logout ends it.
        let token = sign_in(address).await;
        if route.path == "/events" {
            // A stream that does not end: its head is enough.
            let head = stream_until(address, route.path, &token, "", "\r\n\r\n").await;
            assert!(head.starts_with("HTTP/1.1 200"), "{head:?}");
            continue;
        }
        let answer = call(
            address,
            route.method,
            route.path,
            Some(&token),
            Some(&json!({})),
        )
        .await;
        assert!(
            answer.status != 404 && answer.status != 405,
            "{} {} answered {}",
            route.method,
            route.path,
            answer.status
        );
    }
}

pub(crate) fn new_task(title: &str) -> (String, Value) {
    let id = samu_store::core::api::Id::new().to_string();
    let task = json!({
        "id": id,
        "created": "2026-11-07T08:00:00Z",
        "modified": "2026-11-07T08:00:00Z",
        "title": title,
        "flagged": false,
        "sequential": false,
        "complete_with_last_child": false,
        "position": 0
    });
    (id, task)
}

pub(crate) fn create(id: &str, task: &Value) -> Value {
    json!({"op": "create", "object": id, "kind": "task", "value": task,
           "hlc": {"wall": 0, "counter": 0, "device": "ignored"}})
}

/// Online writes are stamped by the server with the session's device
/// (ADR 0019 §4) — asked of the database, not of the answer.
#[tokio::test]
async fn the_server_stamps_online_writes_with_the_sessions_device() {
    let (address, dir) = start(true).await;
    let token = sign_in(address).await;
    let (id, task) = new_task("Milch");
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&json!([create(&id, &task)])),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    assert_eq!(answer.json()[0]["outcome"], "applied");

    let set = json!([{"op": "set", "object": id, "field": "title", "value": "Hafermilch",
                      "hlc": {"wall": 0, "counter": 0, "device": "ignored"}}]);
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&set),
    )
    .await;
    assert_eq!(answer.json()[0]["outcome"], "applied", "{}", answer.body);

    let conn = rusqlite::Connection::open(dir.0.join("accounts/privat.db")).unwrap();
    let (wall, device): (i64, String) = conn
        .query_row(
            "SELECT wall, device FROM stamp WHERE field = 'title'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(device, "laptop");
    assert!(
        wall > 1_700_000_000_000,
        "a real clock, not the 0 that was sent: {wall}"
    );
}

/// Without `stamp=server` the client's stamp counts, and an older one is
/// superseded (ADR 0016).
#[tokio::test]
async fn an_outbox_keeps_its_stamps() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let (id, task) = new_task("Milch");
    let mut first = create(&id, &task);
    first["hlc"] = json!({"wall": 1000, "counter": 0, "device": "pixel"});
    let set = |wall: i64, title: &str| {
        json!({"op": "set", "object": id, "field": "title", "value": title,
               "hlc": {"wall": wall, "counter": 0, "device": "pixel"}})
    };
    let answer = call(
        address,
        "POST",
        "/changes",
        Some(&token),
        Some(&json!([first, set(3000, "neu"), set(2000, "alt")])),
    )
    .await;
    let outcomes = answer.json();
    assert_eq!(outcomes[0]["outcome"], "applied");
    assert_eq!(outcomes[1]["outcome"], "applied");
    assert_eq!(outcomes[2]["outcome"], "superseded", "{}", answer.body);

    let object = call(
        address,
        "GET",
        &format!("/objects/{id}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(object.json()["kind"], "task");
    assert_eq!(object.json()["object"]["title"], "neu");
}

/// Who made a change is the session's word: a stamp that names another
/// device, or the server itself, is taken with the session's device, in
/// the stamp and in the logbook. Its time and counter stay.
#[tokio::test]
async fn a_client_stamp_carries_the_device_of_its_session() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let (id, task) = new_task("Rechnung");
    let now = Timestamp::now().as_millisecond();
    let stamped = |mut change: Value, wall: i64, device: &str| {
        change["hlc"] = json!({"wall": wall, "counter": 4, "device": device});
        change
    };
    let done = json!({"op": "set", "object": id, "field": "completed_at",
                      "value": "2026-11-07T10:00:00Z", "note": "bezahlt"});
    let title = |title: &str| json!({"op": "set", "object": id, "field": "title", "value": title});
    let answer = call(
        address,
        "POST",
        "/changes",
        Some(&token),
        Some(&json!([
            stamped(create(&id, &task), now - 3000, "server"),
            stamped(done, now - 2000, "pixel"),
            stamped(title("neu"), now - 1000, "server"),
            stamped(title("alt"), now - 1500, "pixel"),
        ])),
    )
    .await;
    let outcomes = answer.json();
    for applied in 0..3 {
        assert_eq!(outcomes[applied]["outcome"], "applied", "{}", answer.body);
    }
    // The stamp that holds the title is the one named `server`.
    assert_eq!(outcomes[3]["outcome"], "superseded", "{}", answer.body);
    assert_eq!(
        outcomes[3]["by"],
        json!({"wall": now - 1000, "counter": 4, "device": "laptop"})
    );

    let answer = call(
        address,
        "GET",
        &format!("/objects/{id}/logbook"),
        Some(&token),
        None,
    )
    .await;
    let entries: Vec<samu_store::core::api::LogEntry> = serde_json::from_str(&answer.body).unwrap();
    assert_eq!(entries.len(), 1, "{}", answer.body);
    assert_eq!(entries[0].device, "laptop");
}

/// `server` is the device of the server's own stamps: no session gets it,
/// by sign-in or by pairing, however it is written.
#[tokio::test]
async fn no_session_is_named_server() {
    let (address, _dir) = start(true).await;
    for device in ["server", " Server ", "SERVER"] {
        let answer = call(
            address,
            "POST",
            "/login",
            None,
            Some(&json!({"account": "privat", "password": PASSWORD, "device": device})),
        )
        .await;
        assert_eq!(answer.status, 400, "{device:?}: {}", answer.body);
        assert!(answer.body.contains("the server's own"), "{}", answer.body);
    }
    let token = sign_in(address).await;
    let code = json!({"device": "Server", "password": PASSWORD});
    let answer = call(address, "POST", "/pair/code", Some(&token), Some(&code)).await;
    assert_eq!(answer.status, 400, "{}", answer.body);
    // A name that only contains it is an ordinary one.
    let answer = call(
        address,
        "POST",
        "/login",
        None,
        Some(&json!({"account": "privat", "password": PASSWORD, "device": "homeserver"})),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);
}

#[tokio::test]
async fn changes_after_a_number_lists_objects_and_tombstones() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let (a, task_a) = new_task("a");
    let (b, task_b) = new_task("b");
    call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&json!([create(&a, &task_a), create(&b, &task_b)])),
    )
    .await;
    let page = call(address, "GET", "/changes?after=0", Some(&token), None)
        .await
        .json();
    // The two tasks after the account's settings object (ADR 0045 §4).
    assert_eq!(page["seq"], 3);
    assert_eq!(page["objects"].as_array().unwrap().len(), 3);

    let delete =
        json!([{"op": "delete", "object": b, "hlc": {"wall": 0, "counter": 0, "device": "x"}}]);
    call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&delete),
    )
    .await;
    let page = call(address, "GET", "/changes?after=3", Some(&token), None)
        .await
        .json();
    assert_eq!(page["seq"], 4);
    assert_eq!(page["objects"], json!([]));
    assert_eq!(page["gone"], json!([b]));
}

/// ADR 0024 §5, §7: completing with a note is in the logbook, with the
/// session's device; an object that is not there is 404.
#[tokio::test]
async fn the_logbook_of_an_object() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let (id, task) = new_task("Rechnung");
    let done = json!({"op": "set", "object": id, "field": "completed_at",
                      "value": "2026-11-07T10:00:00Z", "note": "bezahlt",
                      "hlc": {"wall": 0, "counter": 0, "device": "ignored"}});
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&json!([create(&id, &task), done])),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    let answer = call(
        address,
        "GET",
        &format!("/objects/{id}/logbook"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    let entries: Vec<samu_store::core::api::LogEntry> = serde_json::from_str(&answer.body).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, samu_store::core::api::LogKind::Completed);
    assert_eq!(entries[0].note.as_deref(), Some("bezahlt"));
    assert_eq!(entries[0].device, "laptop");

    let missing = samu_store::core::api::Id::new();
    let answer = call(
        address,
        "GET",
        &format!("/objects/{missing}/logbook"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(answer.status, 404);
}

#[tokio::test]
async fn an_unknown_or_malformed_object_is_answered_properly() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let missing = samu_store::core::api::Id::new();
    let answer = call(
        address,
        "GET",
        &format!("/objects/{missing}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(answer.status, 404);
    let error: ApiError = serde_json::from_str(&answer.body).unwrap();
    assert_eq!(error.error, ErrorCode::NotFound);
    assert_eq!(
        call(address, "GET", "/objects/nonsense", Some(&token), None)
            .await
            .status,
        400
    );
    assert_eq!(
        call(
            address,
            "POST",
            "/changes?stamp=moon",
            Some(&token),
            Some(&json!([]))
        )
        .await
        .status,
        400
    );
}

/// An open repeating task says what completing it now would make
/// (ADR 0007 §3), and when its rule ends with it.
#[tokio::test]
async fn a_repeating_task_carries_its_next_dates() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let (weekly, mut weekly_task) = new_task("wöchentlich");
    weekly_task["due"] = json!({"date": "2099-01-05"});
    weekly_task["repeat"] = json!({"rule": "FREQ=WEEKLY", "mode": "fixed"});
    let (last, mut last_task) = new_task("letztes Mal");
    last_task["due"] = json!({"date": "2099-01-05"});
    last_task["repeat"] = json!({"rule": "FREQ=WEEKLY;COUNT=1", "mode": "fixed"});
    let (plain, plain_task) = new_task("einmal");
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&json!([
            create(&weekly, &weekly_task),
            create(&last, &last_task),
            create(&plain, &plain_task)
        ])),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);

    let statuses = call(address, "GET", "/available", Some(&token), None)
        .await
        .json();
    let of = |tid: &str| {
        statuses
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["task"]["id"] == tid)
            .unwrap()
            .clone()
    };
    assert_eq!(
        of(&weekly)["next_repeat"],
        json!({"due": {"date": "2099-01-12"}})
    );
    assert_eq!(of(&last)["next_repeat"], json!({"ended": true}));
    assert!(of(&plain).get("next_repeat").is_none());
}

#[tokio::test]
async fn available_tasks_carry_their_reasons() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let (free, free_task) = new_task("frei");
    let (later, mut later_task) = new_task("später");
    later_task["defer"] = json!({"date": "2099-01-01"});
    let (done, mut done_task) = new_task("erledigt");
    done_task["completed_at"] = json!("2026-11-01T00:00:00Z");
    call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&json!([
            create(&free, &free_task),
            create(&later, &later_task),
            create(&done, &done_task)
        ])),
    )
    .await;

    let open = call(address, "GET", "/available", Some(&token), None)
        .await
        .json();
    assert_eq!(
        open.as_array().unwrap().len(),
        2,
        "completed tasks only with all=true"
    );
    let later_status = open
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["task"]["id"] == later.as_str())
        .unwrap();
    assert_eq!(later_status["available"], false);
    assert_eq!(later_status["reasons"][0]["reason"], "deferred");

    let only = call(
        address,
        "GET",
        "/available?available=true",
        Some(&token),
        None,
    )
    .await
    .json();
    assert_eq!(only.as_array().unwrap().len(), 1);
    assert_eq!(only[0]["task"]["id"], free.as_str());

    let all = call(address, "GET", "/available?all=true", Some(&token), None)
        .await
        .json();
    assert_eq!(all.as_array().unwrap().len(), 3);

    // ADR 0065: with `bare`, the same answers in the same order, each with
    // the id in place of the task.
    for query in ["", "?available=true", "?all=true"] {
        let full = call(
            address,
            "GET",
            &format!("/available{query}"),
            Some(&token),
            None,
        )
        .await
        .json();
        let joiner = if query.is_empty() { "?" } else { "&" };
        let bare = call(
            address,
            "GET",
            &format!("/available{query}{joiner}bare=true"),
            Some(&token),
            None,
        )
        .await
        .json();
        let stripped: Vec<Value> = full
            .as_array()
            .unwrap()
            .iter()
            .map(|status| {
                let mut status = status.clone();
                let task = status.as_object_mut().unwrap().remove("task").unwrap();
                status["id"] = task["id"].clone();
                status
            })
            .collect();
        assert!(!stripped.is_empty());
        assert_eq!(bare, Value::Array(stripped), "{query}");
    }
}

/// ADR 0065 §6: JSON and the web's files go out gzipped to a client that
/// asks for it, and plain to one that does not; `/events` never, because
/// its lines must leave one by one.
#[tokio::test]
async fn text_is_gzipped_for_a_client_that_takes_it() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let head = |answer: String| answer.split("\r\n\r\n").next().unwrap().to_lowercase();
    let gzip = "Accept-Encoding: gzip\r\n";

    let page = head(stream_until(address, "/app.js", &token, gzip, "\r\n\r\n").await);
    assert!(page.contains("content-encoding: gzip"), "{page}");
    let plain = head(stream_until(address, "/app.js", &token, "", "\r\n\r\n").await);
    assert!(!plain.contains("content-encoding"), "{plain}");

    let (id, mut task) = new_task("lang");
    task["note"] = json!("viel Text ".repeat(200));
    call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&json!([create(&id, &task)])),
    )
    .await;
    let json = head(stream_until(address, "/changes?after=0", &token, gzip, "\r\n\r\n").await);
    assert!(json.contains("content-encoding: gzip"), "{json}");
    // Too short to be worth it.
    let short = head(stream_until(address, "/health", &token, gzip, "\r\n\r\n").await);
    assert!(!short.contains("content-encoding"), "{short}");

    let events = stream_until(address, "/events", &token, gzip, "connected").await;
    assert!(events.contains("connected"), "{events}");
    assert!(!head(events).contains("content-encoding"));
}

/// ADR 0025 §3: `/available` carries the state in the default workflow,
/// the next one, and the summary of an action group, computed by the
/// server.
#[tokio::test]
async fn available_carries_state_and_group_summary() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let id = || samu_store::core::api::Id::new().to_string();
    let (group_tag, waiting, someday) = (id(), id(), id());
    let tag = |tid: &str, title: &str, extra: Value| {
        let mut value = json!({"id": tid, "created": "2026-11-07T08:00:00Z",
            "modified": "2026-11-07T08:00:00Z", "title": title, "status": "active",
            "exclusive": false, "position": 0});
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        json!({"op": "create", "object": tid, "kind": "tag", "value": value,
               "hlc": {"wall": 0, "counter": 0, "device": "ignored"}})
    };
    let (parent_id, mut parent) = new_task("Dachrinne");
    parent["sequential"] = json!(true);
    let (first_id, mut first) = new_task("Angebot");
    first["parent"] = json!(parent_id);
    first["tags"] = json!([{"tag": waiting, "position": 0}]);
    let (second_id, mut second) = new_task("Montage");
    second["parent"] = json!(parent_id);
    second["position"] = json!(1);
    let changes = json!([
        tag(
            &group_tag,
            "Status",
            json!({"exclusive": true, "default_workflow": true})
        ),
        tag(
            &waiting,
            "Wartet",
            json!({"parent": group_tag, "status": "on_hold"})
        ),
        tag(
            &someday,
            "Irgendwann",
            json!({"parent": group_tag, "position": 1})
        ),
        create(&parent_id, &parent),
        create(&first_id, &first),
        create(&second_id, &second),
    ]);
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&changes),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    assert!(
        answer
            .json()
            .as_array()
            .unwrap()
            .iter()
            .all(|o| o["outcome"] == "applied"),
        "{}",
        answer.body
    );

    let statuses = call(address, "GET", "/available", Some(&token), None)
        .await
        .json();
    let of = |tid: &str| {
        statuses
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["task"]["id"] == tid)
            .unwrap()
            .clone()
    };
    let angebot = of(&first_id);
    assert_eq!(angebot["workflow"], group_tag.as_str());
    assert_eq!(angebot["state"], waiting.as_str());
    assert_eq!(angebot["next_state"], someday.as_str());
    assert_eq!(angebot["available"], false);
    let rinne = of(&parent_id);
    assert!(rinne.get("state").is_none());
    assert_eq!(rinne["next_state"], waiting.as_str());
    assert_eq!(
        rinne["group"],
        json!({"open": 2, "done": 0, "available": 0, "next": first_id})
    );
    assert!(of(&second_id).get("group").is_none());
}

/// `POST /capture` (ADR 0027): a preview writes nothing; a capture makes
/// the task with project, date and tags, new tags first; an unknown project
/// sends it to the inbox with a warning; nothing is made under a workflow.
#[tokio::test]
async fn capture_parses_resolves_and_creates() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let id = || samu_store::core::api::Id::new().to_string();
    let (project, status, waiting, phone) = (id(), id(), id(), id());
    let hlc = json!({"wall": 0, "counter": 0, "device": "ignored"});
    let tag = |tid: &str, title: &str, extra: Value| {
        let mut value = json!({"id": tid, "created": "2026-11-07T08:00:00Z",
            "modified": "2026-11-07T08:00:00Z", "title": title, "status": "active",
            "exclusive": false, "position": 0});
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        json!({"op": "create", "object": tid, "kind": "tag", "value": value, "hlc": hlc})
    };
    let setup = json!([
        {"op": "create", "object": project, "kind": "project",
         "value": {"id": project, "created": "2026-11-07T08:00:00Z",
                   "modified": "2026-11-07T08:00:00Z", "title": "Neues Bad",
                   "kind": "parallel", "status": "active", "flagged": false, "position": 0},
         "hlc": hlc},
        tag(&status, "Status", json!({"exclusive": true})),
        tag(&waiting, "Wartet", json!({"parent": status})),
        tag(&phone, "telefon", json!({})),
    ]);
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&setup),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    let objects = || async {
        call(address, "GET", "/changes?after=0", Some(&token), None)
            .await
            .json()["objects"]
            .as_array()
            .unwrap()
            .clone()
    };
    let before = objects().await.len();

    let text = "Fliesenleger anrufen #neu @tel @einkauf d:2026-11-20 ~15m";
    let preview = call(
        address,
        "POST",
        "/capture",
        Some(&token),
        Some(&json!({"text": text, "preview": true})),
    )
    .await;
    assert_eq!(preview.status, 200, "{}", preview.body);
    let preview = preview.json();
    assert_eq!(preview["created"], false);
    assert_eq!(preview["task"]["title"], "Fliesenleger anrufen");
    assert_eq!(preview["new_tags"], json!(["einkauf"]));
    assert_eq!(objects().await.len(), before, "a preview writes nothing");

    let made = call(
        address,
        "POST",
        "/capture",
        Some(&token),
        Some(&json!({"text": text})),
    )
    .await;
    assert_eq!(made.status, 200, "{}", made.body);
    let made = made.json();
    assert_eq!(made["created"], true);
    let task_id = made["task"]["id"].as_str().unwrap().to_string();
    // Asked of the store, not of the answer.
    let stored = call(
        address,
        "GET",
        &format!("/objects/{task_id}"),
        Some(&token),
        None,
    )
    .await
    .json();
    let stored = &stored["object"];
    assert_eq!(stored["title"], "Fliesenleger anrufen");
    assert_eq!(stored["project"], project.as_str());
    assert_eq!(stored["due"], json!({"date": "2026-11-20"}));
    assert_eq!(stored["estimate"], "PT15M");
    let all = objects().await;
    let einkauf = all
        .iter()
        .find(|o| o["kind"] == "tag" && o["object"]["title"] == "einkauf")
        .expect("the new tag is stored");
    let on_task: Vec<&str> = stored["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["tag"].as_str().unwrap())
        .collect();
    assert_eq!(
        on_task,
        [phone.as_str(), einkauf["object"]["id"].as_str().unwrap()]
    );

    let missed = call(
        address,
        "POST",
        "/capture",
        Some(&token),
        Some(&json!({"text": "Anruf #gibtsnicht @Status/Neu", "project": project})),
    )
    .await
    .json();
    assert_eq!(missed["task"]["title"], "Anruf #gibtsnicht @Status/Neu");
    assert!(missed["task"].get("project").is_none(), "{missed}");
    assert_eq!(
        missed["warnings"],
        json!([{"warning": "unknown_project", "name": "gibtsnicht"},
               {"warning": "unknown_tag", "name": "Status/Neu"}])
    );
    assert!(
        !objects()
            .await
            .iter()
            .any(|o| o["object"]["title"] == "Neu")
    );

    let fallback = call(
        address,
        "POST",
        "/capture",
        Some(&token),
        Some(&json!({"text": "Silikon", "project": project})),
    )
    .await
    .json();
    assert_eq!(fallback["task"]["project"], project.as_str());
    assert_eq!(
        call(
            address,
            "POST",
            "/capture",
            None,
            Some(&json!({"text": "x"}))
        )
        .await
        .status,
        401
    );
}

/// `POST /capture` with `task` (ADR 0078 §3): the line edits that task —
/// a tag of a group replaces its sibling, the note grows; a preview writes
/// nothing, free text is refused, `task` with `parent` too.
#[tokio::test]
async fn capture_with_task_edits_it() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let id = || samu_store::core::api::Id::new().to_string();
    let (task, status, waiting, done) = (id(), id(), id(), id());
    let hlc = json!({"wall": 0, "counter": 0, "device": "ignored"});
    let tag = |tid: &str, title: &str, extra: Value| {
        let mut value = json!({"id": tid, "created": "2026-11-07T08:00:00Z",
            "modified": "2026-11-07T08:00:00Z", "title": title, "status": "active",
            "exclusive": false, "position": 0});
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        json!({"op": "create", "object": tid, "kind": "tag", "value": value, "hlc": hlc})
    };
    let setup = json!([
        tag(&status, "Status", json!({"exclusive": true})),
        tag(&waiting, "Wartet", json!({"parent": status})),
        tag(&done, "Fertig", json!({"parent": status})),
        {"op": "create", "object": task, "kind": "task",
         "value": {"id": task, "created": "2026-11-07T08:00:00Z",
                   "modified": "2026-11-07T08:00:00Z", "title": "Fliesen",
                   "note": "erste Zeile", "flagged": false, "sequential": false,
                   "complete_with_last_child": false, "position": 0},
         "hlc": hlc},
        {"op": "add", "object": task, "field": "tags", "element": waiting, "hlc": hlc},
    ]);
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&setup),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    let stored = || async {
        call(
            address,
            "GET",
            &format!("/objects/{task}"),
            Some(&token),
            None,
        )
        .await
        .json()["object"]
            .clone()
    };
    let before = stored().await;

    let line = "@Status/Fertig !A d:2026-11-20 // zweite";
    let preview = call(
        address,
        "POST",
        "/capture",
        Some(&token),
        Some(&json!({"text": line, "task": task, "preview": true})),
    )
    .await;
    assert_eq!(preview.status, 200, "{}", preview.body);
    let preview = preview.json();
    assert_eq!(preview["created"], false);
    assert_eq!(preview["task"]["priority"], "A");
    assert_eq!(stored().await, before, "a preview writes nothing");

    let edited = call(
        address,
        "POST",
        "/capture",
        Some(&token),
        Some(&json!({"text": line, "task": task})),
    )
    .await;
    assert_eq!(edited.status, 200, "{}", edited.body);
    assert_eq!(edited.json()["created"], false);
    // Asked of the store, not of the answer.
    let after = stored().await;
    assert_eq!(after["title"], "Fliesen");
    assert_eq!(after["priority"], "A");
    assert_eq!(after["due"], json!({"date": "2026-11-20"}));
    assert_eq!(after["note"], "erste Zeile\n\nzweite");
    let on_task: Vec<&str> = after["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["tag"].as_str().unwrap())
        .collect();
    assert_eq!(on_task, [done.as_str()]);

    for (body, status) in [
        (json!({"text": "morgen anrufen", "task": task}), 422),
        (json!({"text": "!B", "task": task, "parent": task}), 400),
        (json!({"text": "!B", "task": id()}), 422),
    ] {
        let refused = call(address, "POST", "/capture", Some(&token), Some(&body)).await;
        assert_eq!(refused.status, status, "{body}: {}", refused.body);
    }
    assert_eq!(
        stored().await["priority"],
        "A",
        "nothing refused was written"
    );
}

/// `POST /capture/lines` (ADR 0129): a preview writes nothing; the lines
/// are written with an indented one under its line, a new tag made once;
/// a failing line writes none.
#[tokio::test]
async fn capture_lines_makes_a_task_per_line() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let objects = || async {
        call(address, "GET", "/changes?after=0", Some(&token), None)
            .await
            .json()["objects"]
            .as_array()
            .unwrap()
            .clone()
    };
    let before = objects().await.len();
    let text = "Einkauf @laden\n  - Milch @laden\n  - [ ] Brot\nAnrufen !A";
    let preview = call(
        address,
        "POST",
        "/capture/lines",
        Some(&token),
        Some(&json!({"text": text, "preview": true})),
    )
    .await;
    assert_eq!(preview.status, 200, "{}", preview.body);
    assert_eq!(preview.json()["tasks"].as_array().unwrap().len(), 4);
    assert_eq!(objects().await.len(), before, "a preview writes nothing");

    let made = call(
        address,
        "POST",
        "/capture/lines",
        Some(&token),
        Some(&json!({"text": text})),
    )
    .await;
    assert_eq!(made.status, 200, "{}", made.body);
    let made = made.json();
    let ids: Vec<String> = made["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["task"]["id"].as_str().unwrap().to_string())
        .collect();
    // Asked of the store, not of the answer.
    let mut stored = Vec::new();
    for id in &ids {
        let answer = call(
            address,
            "GET",
            &format!("/objects/{id}"),
            Some(&token),
            None,
        )
        .await;
        stored.push(answer.json()["object"].clone());
    }
    let [einkauf, milch, brot, anrufen] = &stored[..] else {
        panic!("four tasks: {stored:?}");
    };
    assert_eq!(einkauf["title"], "Einkauf");
    assert_eq!(milch["parent"], ids[0].as_str());
    assert_eq!(brot["title"], "Brot");
    assert_eq!(brot["parent"], ids[0].as_str());
    assert!(anrufen.get("parent").is_none(), "{anrufen}");
    assert_eq!(anrufen["priority"], "A");
    assert_eq!(einkauf["tags"][0]["tag"], milch["tags"][0]["tag"]);
    let all = objects().await;
    let laden = all
        .iter()
        .filter(|o| o["kind"] == "tag" && o["object"]["title"] == "laden")
        .count();
    assert_eq!(laden, 1, "one new tag for both lines");
    assert_eq!(all.len(), before + 5);

    let refused = call(
        address,
        "POST",
        "/capture/lines",
        Some(&token),
        Some(&json!({"text": "geht\nauch", "parent": samu_store::core::api::Id::new()})),
    )
    .await;
    assert_eq!(refused.status, 422, "{}", refused.body);
    assert!(refused.body.contains("line 1"), "{}", refused.body);
    assert_eq!(
        objects().await.len(),
        before + 5,
        "nothing refused was written"
    );
}

/// `POST /capture` with `parent` (ADR 0062 §3): the new task is the last
/// child, in the parent's project; a closed or unknown parent is refused.
#[tokio::test]
async fn capture_under_a_task_makes_its_last_child() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let project = samu_store::core::api::Id::new().to_string();
    let (group, mut group_task) = new_task("Bad fliesen");
    group_task["project"] = json!(project);
    let (child, mut child_task) = new_task("Fliesen aussuchen");
    child_task["project"] = json!(project);
    child_task["parent"] = json!(group);
    child_task["position"] = json!(3);
    let (closed, mut closed_task) = new_task("Erledigt");
    closed_task["completed_at"] = json!("2026-11-07T09:00:00Z");
    let setup = json!([
        {"op": "create", "object": project, "kind": "project",
         "value": {"id": project, "created": "2026-11-07T08:00:00Z",
                   "modified": "2026-11-07T08:00:00Z", "title": "Bad",
                   "kind": "parallel", "status": "active", "flagged": false,
                   "position": 0},
         "hlc": {"wall": 0, "counter": 0, "device": "ignored"}},
        create(&group, &group_task),
        create(&child, &child_task),
        create(&closed, &closed_task),
    ]);
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&setup),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);

    let made = call(
        address,
        "POST",
        "/capture",
        Some(&token),
        Some(&json!({"text": "Fliesen bestellen", "parent": group})),
    )
    .await;
    assert_eq!(made.status, 200, "{}", made.body);
    let id = made.json()["task"]["id"].as_str().unwrap().to_string();
    // Asked of the store, not of the answer.
    let stored = call(
        address,
        "GET",
        &format!("/objects/{id}"),
        Some(&token),
        None,
    )
    .await
    .json();
    let stored = &stored["object"];
    assert_eq!(stored["parent"], group.as_str());
    assert_eq!(stored["project"], project.as_str());
    assert_eq!(stored["position"], 4);

    for parent in [closed, samu_store::core::api::Id::new().to_string()] {
        let refused = call(
            address,
            "POST",
            "/capture",
            Some(&token),
            Some(&json!({"text": "x", "parent": parent})),
        )
        .await;
        assert_eq!(refused.status, 422, "{}", refused.body);
    }
}

/// `/available/projects?review=true` (ADR 0088): only the projects due for
/// review, oldest first; marking one reviewed is two fields over
/// `/changes`, and it leaves the list.
#[tokio::test]
async fn available_projects_names_the_ones_due_for_review() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let id = || samu_store::core::api::Id::new().to_string();
    let project = |pid: &str, title: &str, created: &str, extra: Value| {
        let mut value = json!({"id": pid, "created": created,
                         "modified": created, "title": title,
                         "kind": "parallel", "status": "active", "flagged": false,
                         "position": 0});
        for (key, field) in extra.as_object().unwrap() {
            value[key] = field.clone();
        }
        json!({"op": "create", "object": pid, "kind": "project", "value": value,
               "hlc": {"wall": 0, "counter": 0, "device": "ignored"}})
    };
    let (plain, newer, older, later) = (id(), id(), id(), id());
    let changes = json!([
        project(&plain, "Ohne", "2020-01-01T08:00:00Z", json!({})),
        project(
            &newer,
            "Neuer",
            "2020-02-01T08:00:00Z",
            json!({"review_interval": "P1W"})
        ),
        project(
            &older,
            "Älter",
            "2020-01-01T08:00:00Z",
            json!({"review_interval": "P1M"})
        ),
        project(
            &later,
            "Später",
            "2020-01-01T08:00:00Z",
            json!({"review_interval": "P1W", "next_review": "2099-01-01"})
        ),
    ]);
    let post = |changes: Value| {
        let token = token.clone();
        async move {
            let answer = call(
                address,
                "POST",
                "/changes?stamp=server",
                Some(&token),
                Some(&changes),
            )
            .await;
            assert_eq!(answer.status, 200, "{}", answer.body);
        }
    };
    post(changes).await;

    let due = || async {
        call(
            address,
            "GET",
            "/available/projects?review=true",
            Some(&token),
            None,
        )
        .await
        .json()
    };
    let list = due().await;
    let titles: Vec<_> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["project"]["title"].clone())
        .collect();
    assert_eq!(titles, vec![json!("Älter"), json!("Neuer")], "{list}");
    assert_eq!(list[0]["review"], "2020-02-01");
    assert_eq!(list[0]["review_due"], true);

    let all = call(address, "GET", "/available/projects", Some(&token), None)
        .await
        .json();
    let later_row = all
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["project"]["id"] == later.as_str())
        .unwrap()
        .clone();
    assert_eq!(later_row["review"], "2099-01-01");
    assert!(later_row.get("review_due").is_none(), "{later_row}");
    let plain_row = all
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["project"]["id"] == plain.as_str())
        .unwrap()
        .clone();
    assert!(plain_row.get("review").is_none(), "{plain_row}");

    let hlc = json!({"wall": 0, "counter": 0, "device": "ignored"});
    post(json!([
        {"op": "set", "object": older, "field": "last_reviewed", "value": Timestamp::now().to_string(), "hlc": hlc},
        {"op": "set", "object": older, "field": "next_review", "value": null, "hlc": hlc},
    ]))
    .await;
    let list = due().await;
    let titles: Vec<_> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["project"]["title"].clone())
        .collect();
    assert_eq!(titles, vec![json!("Neuer")], "{list}");
}

/// `/available/projects`: a project without a next action is stuck, one
/// with an available action is not, and `?stuck=true` keeps only the first.
#[tokio::test]
async fn available_projects_names_the_stuck_ones() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let id = || samu_store::core::api::Id::new().to_string();
    let project = |pid: &str, title: &str, position: u32| {
        json!({"op": "create", "object": pid, "kind": "project",
               "value": {"id": pid, "created": "2026-11-07T08:00:00Z",
                         "modified": "2026-11-07T08:00:00Z", "title": title,
                         "kind": "parallel", "status": "active", "flagged": false,
                         "position": position},
               "hlc": {"wall": 0, "counter": 0, "device": "ignored"}})
    };
    let (empty, busy) = (id(), id());
    let (task_id, mut task) = new_task("Rasen mähen");
    task["project"] = json!(busy);
    let changes = json!([
        project(&empty, "Garten", 0),
        project(&busy, "Haus", 1),
        create(&task_id, &task),
    ]);
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&changes),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);

    let all = call(address, "GET", "/available/projects", Some(&token), None)
        .await
        .json();
    let all = all.as_array().unwrap();
    assert_eq!(all.len(), 2, "{all:?}");
    assert_eq!(all[0]["project"]["id"], empty.as_str());
    assert_eq!(all[0]["stuck"], true);
    assert_eq!(all[1]["stuck"], false);
    assert_eq!(all[1]["available"], true);

    let stuck = call(
        address,
        "GET",
        "/available/projects?stuck=true",
        Some(&token),
        None,
    )
    .await
    .json();
    let stuck = stuck.as_array().unwrap();
    assert_eq!(stuck.len(), 1);
    assert_eq!(stuck[0]["project"]["title"], "Garten");
    assert_eq!(
        call(address, "GET", "/available/projects", None, None)
            .await
            .status,
        401
    );
}

/// ADR 0035: `scope=` keeps what the scope shows, and what is due outside
/// marked `outside`; the inbox as its tick says (ADR 0043 §1); the same task shows again without
/// the scope, and an unknown scope is 404.
#[tokio::test]
async fn a_scope_narrows_available_and_counts_what_is_due_outside() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let id = || samu_store::core::api::Id::new().to_string();
    let hlc = json!({"wall": 0, "counter": 0, "device": "ignored"});
    let (it, haus, garten, scope) = (id(), id(), id(), id());
    let project = |pid: &str, title: &str, tags: Value| {
        json!({"op": "create", "object": pid, "kind": "project",
               "value": {"id": pid, "created": "2026-11-07T08:00:00Z",
                         "modified": "2026-11-07T08:00:00Z", "title": title,
                         "kind": "parallel", "status": "active", "flagged": false,
                         "tags": tags, "position": 0},
               "hlc": hlc})
    };
    let in_project = |title: &str, pid: &str, due: Option<&str>| {
        let (tid, mut task) = new_task(title);
        task["project"] = json!(pid);
        if let Some(date) = due {
            task["due"] = json!({"date": date});
        }
        (tid.clone(), create(&tid, &task))
    };
    let (patch, patch_create) = in_project("Server patchen", &haus, Some("2000-01-01"));
    let (cable, cable_create) = in_project("Kabel ziehen", &haus, None);
    let (lawn, lawn_create) = in_project("Rasen mähen", &garten, None);
    let (inbox, inbox_task) = new_task("Milch");
    let changes = json!([
        {"op": "create", "object": it, "kind": "tag",
         "value": {"id": it, "created": "2026-11-07T08:00:00Z",
                   "modified": "2026-11-07T08:00:00Z", "title": "IT",
                   "status": "active", "exclusive": false, "position": 0},
         "hlc": hlc},
        project(&haus, "Haus", json!([{"tag": it, "position": 0}])),
        project(&garten, "Garten", json!([])),
        patch_create,
        cable_create,
        lawn_create,
        create(&inbox, &inbox_task),
        {"op": "create", "object": scope, "kind": "scope",
         "value": {"id": scope, "created": "2026-11-08T08:00:00Z",
                   "modified": "2026-11-08T08:00:00Z", "title": "Privat ohne IT",
                   "mode": "except", "members": [it], "position": 0},
         "hlc": hlc},
    ]);
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&changes),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    assert!(
        answer
            .json()
            .as_array()
            .unwrap()
            .iter()
            .all(|o| o["outcome"] == "applied"),
        "{}",
        answer.body
    );

    let ids = |statuses: &Value, outside: bool| -> Vec<String> {
        let mut ids: Vec<String> = statuses
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s.get("outside").and_then(Value::as_bool).unwrap_or(false) == outside)
            .map(|s| s["task"]["id"].as_str().unwrap().to_owned())
            .collect();
        ids.sort();
        ids
    };
    let sorted = |mut v: Vec<String>| {
        v.sort();
        v
    };

    // The probe that must find something: without the scope, all four.
    let all = call(address, "GET", "/available", Some(&token), None)
        .await
        .json();
    assert_eq!(
        ids(&all, false),
        sorted(vec![
            patch.clone(),
            cable.clone(),
            lawn.clone(),
            inbox.clone()
        ])
    );

    let scoped = call(
        address,
        "GET",
        &format!("/available?scope={scope}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(scoped.status, 200, "{}", scoped.body);
    let scoped = scoped.json();
    assert_eq!(
        ids(&scoped, false),
        sorted(vec![lawn.clone(), inbox.clone()])
    );
    assert_eq!(ids(&scoped, true), vec![patch.clone()]);
    assert!(!ids(&scoped, false).contains(&cable));

    let projects = call(
        address,
        "GET",
        &format!("/available/projects?scope={scope}"),
        Some(&token),
        None,
    )
    .await
    .json();
    let titles: Vec<&str> = projects
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["project"]["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, vec!["Garten"]);

    // ADR 0043 §1: ticked in `except`, the inbox goes.
    let tick = json!([{"op": "set", "object": scope, "field": "inbox", "value": true, "hlc": hlc}]);
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&tick),
    )
    .await;
    assert_eq!(answer.json()[0]["outcome"], "applied", "{}", answer.body);
    let ticked = call(
        address,
        "GET",
        &format!("/available?scope={scope}"),
        Some(&token),
        None,
    )
    .await
    .json();
    assert_eq!(ids(&ticked, false), vec![lawn.clone()]);

    // ADR 0043 §3: members AND filter. Untick the inbox, filter on it:
    // the lawn goes, the milk stays, the patch is still due outside.
    let filter = json!([
        {"op": "set", "object": scope, "field": "inbox", "value": false, "hlc": hlc},
        {"op": "set", "object": scope, "field": "filter",
         "value": [{"kind": "project", "value": "inbox"}], "hlc": hlc},
    ]);
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&filter),
    )
    .await;
    assert!(
        answer
            .json()
            .as_array()
            .unwrap()
            .iter()
            .all(|o| o["outcome"] == "applied"),
        "{}",
        answer.body
    );
    let filtered = call(
        address,
        "GET",
        &format!("/available?scope={scope}"),
        Some(&token),
        None,
    )
    .await
    .json();
    assert_eq!(ids(&filtered, false), vec![inbox.clone()]);
    assert_eq!(ids(&filtered, true), vec![patch.clone()]);
    let bad = json!([{"op": "set", "object": scope, "field": "filter",
                      "value": [{"kind": "due", "value": "bald"}], "hlc": hlc}]);
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&bad),
    )
    .await;
    assert_eq!(answer.json()[0]["outcome"], "rejected", "{}", answer.body);

    // ADR 0043 §7: a window round the clock picks the scope.
    let always = json!([{"op": "set", "object": scope, "field": "schedule",
                         "value": [{"days": ["mo", "tu", "we", "th", "fr", "sa", "su"],
                                    "from": "00:00:00", "to": "23:59:59"}], "hlc": hlc}]);
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&always),
    )
    .await;
    assert_eq!(answer.json()[0]["outcome"], "applied", "{}", answer.body);
    let scheduled = call(address, "GET", "/scopes/scheduled", Some(&token), None).await;
    assert_eq!(scheduled.status, 200, "{}", scheduled.body);
    // Between 23:59:59 and midnight there is no window.
    let picked = scheduled.json();
    assert!(
        picked["scope"] == json!(scope) || picked["scope"].is_null(),
        "{picked}"
    );
    assert!(picked["until"].is_string(), "{picked}");

    // ADR 0043 §7: 00:00 to 00:00 ends the next day, a whole day; every
    // day of the week, so the window holds now.
    let always = json!([{"op": "set", "object": scope, "field": "schedule",
                         "value": [{"days": ["mo", "tu", "we", "th", "fr", "sa", "su"],
                                    "from": "00:00:00", "to": "00:00:00"}], "hlc": hlc}]);
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&always),
    )
    .await;
    assert_eq!(answer.json()[0]["outcome"], "applied", "{}", answer.body);
    let scheduled = call(address, "GET", "/scopes/scheduled", Some(&token), None).await;
    assert_eq!(scheduled.status, 200, "{}", scheduled.body);
    let picked = scheduled.json();
    assert_eq!(picked["scope"], json!(scope), "{picked}");
    assert!(
        picked["until"].is_string() && picked["since"].is_string(),
        "{picked}"
    );

    for path in ["/available", "/available/projects"] {
        let unknown = call(
            address,
            "GET",
            &format!("{path}?scope={}", id()),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(unknown.status, 404, "{path}: {}", unknown.body);
    }
}

/// A change shows up on `/events` as a notice with ids only (catastr
/// ADR 0095 §3).
#[tokio::test]
async fn a_change_is_announced_on_events() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let (id, task) = new_task("Milch");
    let listener = {
        let token = token.clone();
        tokio::spawn(
            async move { stream_until(address, "/events", &token, "", "event: changed").await },
        )
    };
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&json!([create(&id, &task)])),
    )
    .await;
    let seen = listener.await.unwrap();
    assert!(seen.contains("event: changed"), "{seen}");
    assert!(seen.contains(&format!("\"added\":[\"{id}\"]")), "{seen}");
    assert!(!seen.contains("Milch"), "ids only, never content");
}

#[tokio::test]
async fn reconnecting_behind_gets_a_reset() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let (id, task) = new_task("Milch");
    call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&json!([create(&id, &task)])),
    )
    .await;
    let seen = stream_until(
        address,
        "/events",
        &token,
        "Last-Event-ID: 0\r\n",
        "event: reset",
    )
    .await;
    assert!(seen.contains("event: reset"), "{seen}");
}

/// Without a first event the answer's head waits for the first change, and
/// a client sees nothing (found with curl on 07.11.2026). The server sends
/// a comment at once; the head has to be there within two seconds.
#[tokio::test]
async fn the_head_of_events_comes_at_once() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let started = tokio::time::Instant::now();
    let seen = stream_until(address, "/events", &token, "", ": connected").await;
    assert!(seen.starts_with("HTTP/1.1 200"), "{seen:?}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(2),
        "the head took {:?}",
        started.elapsed()
    );
}

/// ADR 0025 §2: the page and its modules come without a session, with
/// their type and `no-cache`; an unknown name is 404, and the API behind
/// it stays closed.
#[tokio::test]
async fn the_web_ui_is_served_without_a_session() {
    let (address, _dir) = start(true).await;
    for (path, kind) in [
        ("/", "text/html"),
        ("/app.js", "text/javascript"),
        ("/style.css", "text/css"),
        ("/favicon.svg", "image/svg+xml"),
        ("/lib/api.js", "text/javascript"),
        ("/lib/detail.js", "text/javascript"),
    ] {
        let answer = call(address, "GET", path, None, None).await;
        assert_eq!(answer.status, 200, "{path}");
        let head = answer.headers.to_ascii_lowercase();
        assert!(
            head.contains(&format!("content-type: {kind}")),
            "{path}: {head}"
        );
        assert!(head.contains("cache-control: no-cache"), "{path}: {head}");
        assert!(!answer.body.is_empty(), "{path}");
    }
    let page = call(address, "GET", "/", None, None).await;
    assert!(
        page.body
            .contains("<script type=\"module\" src=\"/app.js\">")
    );
    // One object's address is the same page (ADR 0075 §1), and its files
    // load from there too: absolute paths.
    let object = call(
        address,
        "GET",
        "/o/01936f5e-8a00-7c3e-9b1a-3f2d4c5b6c01",
        None,
        None,
    )
    .await;
    assert_eq!(object.status, 200);
    assert_eq!(object.body, page.body);
    for reference in ["href=\"/style.css\"", "href=\"/favicon.svg\""] {
        assert!(page.body.contains(reference), "{reference}");
    }
    // So is every view's (ADR 0075 §7).
    for view in crate::web::VIEW_PATHS {
        let page_of_view = call(address, "GET", &format!("/v/{view}"), None, None).await;
        assert_eq!(page_of_view.status, 200, "/v/{view}");
        assert_eq!(page_of_view.body, page.body, "/v/{view}");
    }
    for path in [
        "/v/nope",
        "/v/today/x",
        "/lib/nope.js",
        "/nope.js",
        "/lib/../app.js",
        "/o/nope",
        "/o/01936f5e-8a00-7c3e-9b1a-3f2d4c5b6c01/x",
    ] {
        assert_eq!(
            call(address, "GET", path, None, None).await.status,
            404,
            "{path}"
        );
    }
    assert_eq!(
        call(address, "GET", "/available", None, None).await.status,
        401
    );
    assert_eq!(
        call(address, "GET", "/health", None, None).await.status,
        200
    );
}

/// ADR 0031: a pairing code needs the session and the password, and is
/// traded once for a session like the one `/login` gives.
#[tokio::test]
async fn a_phone_pairs_once_with_a_code_from_a_signed_in_browser() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let body = json!({"device": "pixel", "password": PASSWORD});

    let without = call(address, "POST", "/pair/code", None, Some(&body)).await;
    assert_eq!(without.status, 401, "no session, no code");
    let wrong = json!({"device": "pixel", "password": "wrong password"});
    let wrong = call(address, "POST", "/pair/code", Some(&token), Some(&wrong)).await;
    assert_eq!(wrong.status, 401, "a session alone is not enough");
    let empty = json!({"device": " ", "password": PASSWORD});
    let empty = call(address, "POST", "/pair/code", Some(&token), Some(&empty)).await;
    assert_eq!(empty.status, 400, "{}", empty.body);

    let made = call(address, "POST", "/pair/code", Some(&token), Some(&body)).await;
    assert_eq!(made.status, 200, "{}", made.body);
    let code: samu_store::core::api::PairCode = serde_json::from_str(&made.body).unwrap();
    let uri = samu_store::core::pairing::PairingUri::parse(&code.uri).unwrap();
    assert_eq!(
        (
            uri.server.as_str(),
            uri.account.as_str(),
            uri.device.as_str()
        ),
        ("http://localhost", "privat", "pixel")
    );
    assert_eq!(uri.code, code.code);

    let paired = call(
        address,
        "POST",
        "/pair",
        None,
        Some(&json!({"code": code.code})),
    )
    .await;
    assert_eq!(paired.status, 200, "{}", paired.body);
    let session: SessionInfo = serde_json::from_str(&paired.body).unwrap();
    assert_eq!(
        (session.account.as_str(), session.device.as_str()),
        ("privat", "pixel")
    );
    assert!(session.zone.is_some() && session.remind_time.is_some());
    let phone = session.token.unwrap();
    let asked = call(address, "GET", "/session", Some(&phone), None).await;
    assert_eq!(asked.status, 200, "the new session works");

    let again = call(
        address,
        "POST",
        "/pair",
        None,
        Some(&json!({"code": code.code})),
    )
    .await;
    assert_eq!(again.status, 401, "once");
}

#[test]
fn the_address_in_the_code_is_the_one_the_request_came_to() {
    let mut headers = HeaderMap::new();
    headers.insert("host", "127.0.0.1:8491".parse().unwrap());
    assert_eq!(
        crate::app::origin(&headers, false).unwrap(),
        "http://127.0.0.1:8491"
    );
    headers.insert("host", "samu.example.org".parse().unwrap());
    headers.insert("x-forwarded-proto", "https".parse().unwrap());
    assert_eq!(
        crate::app::origin(&headers, true).unwrap(),
        "https://samu.example.org"
    );
    headers.insert("x-forwarded-host", "tasks.example.org".parse().unwrap());
    assert_eq!(
        crate::app::origin(&headers, true).unwrap(),
        "https://tasks.example.org"
    );
    headers.insert("host", "evil/path".parse().unwrap());
    headers.remove("x-forwarded-host");
    assert!(crate::app::origin(&headers, true).is_err());
}

/// Without `--trust-proxy` the forwarded headers are anybody's word: the
/// address is the `Host` header over plain HTTP, whatever they say.
#[test]
fn forwarded_headers_count_only_behind_a_trusted_proxy() {
    let mut headers = HeaderMap::new();
    headers.insert("host", "samu.example.org".parse().unwrap());
    headers.insert("x-forwarded-host", "evil.example.net".parse().unwrap());
    headers.insert("x-forwarded-proto", "https".parse().unwrap());
    assert_eq!(
        crate::app::origin(&headers, false).unwrap(),
        "http://samu.example.org"
    );
    assert_eq!(
        crate::app::origin(&headers, true).unwrap(),
        "https://evil.example.net"
    );
}

/// `--trust-proxy` is off unless asked for.
#[test]
fn trust_proxy_is_off_by_default() {
    use clap::Parser;
    let trust = |args: &[&str]| match crate::Args::try_parse_from(args).unwrap().command {
        crate::Command::Serve { trust_proxy, .. } => trust_proxy,
        other => panic!("not serve: {other:?}"),
    };
    assert!(!trust(&["samu-server", "serve"]));
    assert!(trust(&["samu-server", "serve", "--trust-proxy"]));
}

/// The same through `POST /pair/code`: the address in the code a phone
/// signs in with follows the forwarded headers only with `--trust-proxy`.
#[tokio::test]
async fn a_pairing_code_names_a_forwarded_host_only_behind_a_trusted_proxy() {
    async fn server_in_code(trust: bool) -> String {
        let (address, _dir) = start_with(true, |state| state.with_trust_proxy(trust)).await;
        let token = sign_in(address).await;
        let body = json!({"device": "pixel", "password": PASSWORD}).to_string();
        let request = format!(
            "POST /pair/code HTTP/1.1\r\nHost: samu.example.org\r\nConnection: close\r\n\
             X-Forwarded-Host: evil.example.net\r\nX-Forwarded-Proto: https\r\n\
             Authorization: Bearer {token}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\n\r\n{body}",
            body.len()
        );
        let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut raw = String::new();
        stream.read_to_string(&mut raw).await.unwrap();
        let (head, body) = raw.split_once("\r\n\r\n").unwrap();
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        let code: samu_store::core::api::PairCode = serde_json::from_str(body).unwrap();
        samu_store::core::pairing::PairingUri::parse(&code.uri)
            .unwrap()
            .server
    }
    assert_eq!(server_in_code(false).await, "http://samu.example.org");
    assert_eq!(server_in_code(true).await, "https://evil.example.net");
}

/// The hex form SQLite gives a uuid blob.
fn hex(id: &str) -> String {
    id.replace('-', "").to_uppercase()
}

/// ADR 0036: deleting a project moves or takes its tasks in one go, a
/// preview writes nothing, a second delete changes nothing, an orphan is in
/// the inbox — asked of the database where it writes.
#[tokio::test]
async fn deleting_a_project_moves_its_tasks_and_orphans_count_as_inbox() {
    let (address, dir) = start(true).await;
    let token = sign_in(address).await;
    let id = || samu_store::core::api::Id::new().to_string();
    let project = |pid: &str, title: &str| {
        json!({"op": "create", "object": pid, "kind": "project",
               "value": {"id": pid, "created": "2026-11-07T08:00:00Z",
                         "modified": "2026-11-07T08:00:00Z", "title": title,
                         "kind": "parallel", "status": "active", "flagged": false,
                         "position": 0},
               "hlc": {"wall": 0, "counter": 0, "device": "ignored"}})
    };
    let (old, new, gone) = (id(), id(), id());
    let (group_id, mut group) = new_task("Umzug");
    group["project"] = json!(old);
    let (child_id, mut child) = new_task("Kartons");
    child["project"] = json!(old);
    child["parent"] = json!(group_id);
    let (there_id, mut there) = new_task("Schon da");
    there["project"] = json!(new);
    there["position"] = json!(3);
    let (orphan_id, mut orphan) = new_task("Verwaist");
    orphan["project"] = json!(gone);
    let changes = json!([
        project(&old, "Alt"),
        project(&new, "Neu"),
        project(&gone, "Weg"),
        create(&group_id, &group),
        create(&child_id, &child),
        create(&there_id, &there),
        create(&orphan_id, &orphan),
    ]);
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&changes),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    // A plain tombstone on a project with a task is refused (ADR 0076 §2) ...
    let tombstone = json!([{"op": "delete", "object": gone,
                            "hlc": {"wall": 0, "counter": 0, "device": "x"}}]);
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&tombstone),
    )
    .await;
    assert_eq!(answer.json()[0]["outcome"], "rejected", "{}", answer.body);
    // ... but an orphan from before is still there to be shown: made in the
    // database itself, as the account had one until 11.11.2026 (01a1caff).
    let gone_uuid: samu_store::core::api::Id = gone.parse().unwrap();
    rusqlite::Connection::open(crate::app::account_path(&dir.0, "privat"))
        .unwrap()
        .execute(
            "UPDATE object SET deleted_at = '2026-11-08T12:14:00Z' WHERE uuid = ?1",
            [gone_uuid.uuid().as_bytes().to_vec()],
        )
        .unwrap();

    let statuses = call(address, "GET", "/available", Some(&token), None)
        .await
        .json();
    let inbox_of = |statuses: &Value, task: &str| {
        statuses
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["task"]["id"] == task)
            .map(|s| s["inbox"] == true)
    };
    assert_eq!(inbox_of(&statuses, &orphan_id), Some(true), "{statuses}");
    assert_eq!(inbox_of(&statuses, &child_id), Some(false), "the probe");

    let path = format!("/objects/{old}/delete");
    let to_new = json!({"tasks": {"project": new}, "preview": true});
    let preview = call(address, "POST", &path, Some(&token), Some(&to_new)).await;
    assert_eq!(preview.status, 200, "{}", preview.body);
    assert_eq!(
        preview.json(),
        json!({"deleted": 1, "moved": 2, "untagged": 0})
    );
    let conn = rusqlite::Connection::open(dir.0.join("accounts/privat.db")).unwrap();
    let in_project = |project: &str| -> i64 {
        conn.query_row(
            "SELECT count(*) FROM task t JOIN object o ON o.id = t.object \
             WHERE o.deleted_at IS NULL AND hex(t.project) = ?1",
            [hex(project)],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(in_project(&old), 2, "a preview writes nothing");

    let written = call(
        address,
        "POST",
        &path,
        Some(&token),
        Some(&json!({"tasks": {"project": new}})),
    )
    .await;
    assert_eq!(written.status, 200, "{}", written.body);
    assert!(written.json()["seq"].as_u64().is_some(), "{}", written.body);
    assert_eq!((in_project(&old), in_project(&new)), (0, 3));
    let (deleted, position, parent): (Option<String>, i64, Option<String>) = conn
        .query_row(
            "SELECT (SELECT deleted_at FROM object WHERE hex(uuid) = ?1), \
                    (SELECT t.position FROM task t JOIN object o ON o.id = t.object \
                      WHERE hex(o.uuid) = ?2), \
                    (SELECT hex(t.parent) FROM task t JOIN object o ON o.id = t.object \
                      WHERE hex(o.uuid) = ?3)",
            [hex(&old), hex(&group_id), hex(&child_id)],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert!(deleted.is_some(), "the project is a tombstone");
    assert_eq!(position, 4, "after the last task of the target");
    assert_eq!(parent, Some(hex(&group_id)), "the group stays whole");

    let again = call(address, "POST", &path, Some(&token), Some(&json!({}))).await;
    assert_eq!(again.status, 200, "{}", again.body);
    assert_eq!(
        again.json(),
        json!({"deleted": 0, "moved": 0, "untagged": 0})
    );

    let along = call(
        address,
        "POST",
        &format!("/objects/{new}/delete"),
        Some(&token),
        Some(&json!({"tasks": "delete"})),
    )
    .await;
    assert_eq!(along.json()["deleted"], 4, "{}", along.body);
    assert_eq!(in_project(&new), 0);

    let task_with_fate = call(
        address,
        "POST",
        &format!("/objects/{orphan_id}/delete"),
        Some(&token),
        Some(&json!({"tasks": "inbox"})),
    )
    .await;
    assert_eq!(task_with_fate.status, 400, "{}", task_with_fate.body);
    let nowhere = call(
        address,
        "POST",
        &format!("/objects/{}/delete", id()),
        Some(&token),
        Some(&json!({})),
    )
    .await;
    assert_eq!(nowhere.status, 404);
}

/// ADR 0037 §4: an empty folder is shown, without a scope and through one
/// that does not hide it; a scope that names only another folder hides it.
#[tokio::test]
async fn available_folders_keeps_an_empty_folder_unless_the_scope_hides_it() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let id = || samu_store::core::api::Id::new().to_string();
    let at = "2026-11-08T08:00:00Z";
    let (gtd, house, garden, only, except) = (id(), id(), id(), id(), id());
    let folder = |fid: &str, title: &str| {
        json!({"op": "create", "object": fid, "kind": "folder",
               "value": {"id": fid, "created": at, "modified": at, "title": title,
                         "status": "active", "position": 0},
               "hlc": {"wall": 0, "counter": 0, "device": "x"}})
    };
    let scope = |sid: &str, mode: &str, member: &str| {
        json!({"op": "create", "object": sid, "kind": "scope",
               "value": {"id": sid, "created": at, "modified": at, "title": mode,
                         "mode": mode, "members": [member], "position": 0},
               "hlc": {"wall": 0, "counter": 0, "device": "x"}})
    };
    let changes = json!([
        folder(&gtd, "GTD"),
        folder(&house, "Haus"),
        {"op": "create", "object": garden, "kind": "project",
         "value": {"id": garden, "created": at, "modified": at, "title": "Garten",
                   "folder": house, "kind": "parallel", "status": "active",
                   "flagged": false, "position": 0},
         "hlc": {"wall": 0, "counter": 0, "device": "x"}},
        scope(&only, "only", &house),
        scope(&except, "except", &house),
    ]);
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&changes),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    assert_eq!(
        folder_titles(address, &token, "/available/folders").await,
        ["GTD", "Haus"]
    );
    assert_eq!(
        folder_titles(
            address,
            &token,
            &format!("/available/folders?scope={except}")
        )
        .await,
        ["GTD"]
    );
    assert_eq!(
        folder_titles(address, &token, &format!("/available/folders?scope={only}")).await,
        ["Haus"],
        "the probe: a scope hides the empty one"
    );
}

async fn folder_titles(address: SocketAddr, token: &str, path: &str) -> Vec<String> {
    let answer = call(address, "GET", path, Some(token), None).await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    let mut titles: Vec<String> = answer
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["title"].as_str().unwrap().to_string())
        .collect();
    titles.sort();
    titles
}

/// One request with a body of raw bytes and headers of its own, answered
/// as bytes.
pub(crate) async fn call_bytes(
    address: SocketAddr,
    method: &str,
    path: &str,
    headers: &str,
    body: &[u8],
) -> (u16, String, Vec<u8>) {
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{headers}\
         Content-Length: {}\r\n\r\n",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    // A refused upload may be answered before the whole body is read.
    let _ = stream.write_all(body).await;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.unwrap();
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8(raw[..end].to_vec()).unwrap();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    let mut body = raw[end + 4..].to_vec();
    if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        body = dechunk(&body);
    }
    (status, head, body)
}

fn dechunk(mut raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let end = raw.windows(2).position(|w| w == b"\r\n").unwrap();
        let size = usize::from_str_radix(std::str::from_utf8(&raw[..end]).unwrap(), 16).unwrap();
        if size == 0 {
            return out;
        }
        out.extend_from_slice(&raw[end + 2..end + 2 + size]);
        raw = &raw[end + 4 + size..];
    }
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// ADR 0097 §2: a file over the limit per file, or over what the account
/// has left, keeps nothing; the same file again counts nothing.
#[tokio::test]
async fn files_stay_within_the_limits_per_file_and_per_account() {
    let (address, _dir) = start_with(true, |state| {
        state.with_blob_limits(crate::app::BlobLimits {
            per_file: 100,
            per_account: Some(250),
        })
    })
    .await;
    let token = sign_in(address).await;
    let auth = format!("Authorization: Bearer {token}\r\n");
    let put = |file: Vec<u8>| {
        let auth = auth.clone();
        async move {
            let path = format!("/blobs/{}", sha256(&file));
            let (status, _, body) = call_bytes(address, "PUT", &path, &auth, &file).await;
            (status, String::from_utf8_lossy(&body).to_string())
        }
    };
    let (status, body) = put(vec![1; 101]).await;
    assert_eq!(status, 413, "{body}");
    assert!(body.contains("at most 100 bytes"), "{body}");
    assert_eq!(put(vec![1; 100]).await.0, 200);
    assert_eq!(put(vec![2; 100]).await.0, 200);
    // 200 of 250 used: 51 bytes no longer fit, 50 do.
    let (status, body) = put(vec![3; 51]).await;
    assert_eq!(status, 413, "{body}");
    assert!(body.contains("the account has 200 of 250 bytes"), "{body}");
    // A file the account has already counts nothing.
    assert_eq!(put(vec![1; 100]).await.0, 200);
    assert_eq!(put(vec![3; 50]).await.0, 200);
    assert_eq!(put(vec![4; 1]).await.0, 413);
}

/// ADR 0039 §4: a file goes up under its hash, once, and comes back with
/// its type; a wrong hash keeps nothing, and the attachment needs the blob.
#[tokio::test]
async fn a_blob_goes_up_once_and_comes_back() {
    let (address, dir) = start(true).await;
    let token = sign_in(address).await;
    let auth = format!("Authorization: Bearer {token}\r\n");
    // Larger than axum's default body limit of 2 MB.
    let file: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
    let hash = sha256(&file);
    let path = format!("/blobs/{hash}");

    let (status, _, body) = call_bytes(
        address,
        "PUT",
        &path,
        &format!("{auth}Content-Type: image/PNG\r\n"),
        &file,
    )
    .await;
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    let blob: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        blob,
        json!({"hash": hash, "size": 3_000_000, "media_type": "image/png"})
    );
    // Again, with another type: the same answer, the first type stays.
    let (status, _, again) = call_bytes(
        address,
        "PUT",
        &path,
        &format!("{auth}Content-Type: text/plain\r\n"),
        &file,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(serde_json::from_slice::<Value>(&again).unwrap(), blob);

    // The database and the directory hold it once.
    let conn = rusqlite::Connection::open(dir.0.join("accounts/privat.db")).unwrap();
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM blob", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 1);
    let stored = dir
        .0
        .join(format!("accounts/privat.blobs/{}/{hash}", &hash[..2]));
    assert_eq!(std::fs::read(&stored).unwrap(), file);

    // Back with its type, and with the browser's cookie too.
    for header in [auth.clone(), format!("Cookie: samu_session={token}\r\n")] {
        let (status, head, body) = call_bytes(address, "GET", &path, &header, &[]).await;
        assert_eq!(status, 200, "{head}");
        assert!(head.contains("content-type: image/png"), "{head}");
        assert!(head.contains("x-content-type-options: nosniff"), "{head}");
        assert!(!head.contains("content-disposition"), "{head}");
        assert!(body == file, "the file came back changed");
    }

    // A wrong hash keeps nothing.
    let other = sha256(b"something else");
    let (status, _, body) = call_bytes(
        address,
        "PUT",
        &format!("/blobs/{other}"),
        &auth,
        b"not that",
    )
    .await;
    assert_eq!(status, 400, "{}", String::from_utf8_lossy(&body));
    assert!(
        !dir.0
            .join(format!("accounts/privat.blobs/{}/{other}", &other[..2]))
            .exists()
    );
    let leftovers = std::fs::read_dir(dir.0.join("accounts/privat.blobs/tmp"))
        .unwrap()
        .count();
    assert_eq!(leftovers, 0, "no half-written file stays");
    // Neither it nor a name that is no hash can be fetched.
    let (status, _, _) = call_bytes(address, "GET", &format!("/blobs/{other}"), &auth, &[]).await;
    assert_eq!(status, 404);
    let (status, _, _) = call_bytes(address, "GET", "/blobs/..%2Fprivat.db", &auth, &[]).await;
    assert_eq!(status, 400);
    // Not without a session.
    let (status, _, _) = call_bytes(address, "GET", &path, "", &[]).await;
    assert_eq!(status, 401);

    // An attachment on a task, over the blob.
    let (task_id, task) = new_task("Tafel");
    let attachment = samu_store::core::api::Id::new().to_string();
    let changes = json!([create(&task_id, &task), {
        "op": "create", "object": attachment, "kind": "attachment",
        "value": {
            "id": attachment, "created": "2026-11-08T18:00:00Z",
            "modified": "2026-11-08T18:00:00Z", "owner": task_id,
            "blob": hash, "name": "tafel.png", "media_type": "image/png",
            "size": 3_000_000
        },
        "hlc": {"wall": 0, "counter": 0, "device": "x"}
    }]);
    let applied = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&changes),
    )
    .await;
    assert_eq!(applied.json()[1]["outcome"], "applied", "{}", applied.body);
}

/// ADR 0039 §4: a file that is not a picture, sound or film is a download,
/// and never runs in samu's origin.
#[tokio::test]
async fn a_page_as_a_blob_is_a_download_in_a_sandbox() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let auth = format!("Authorization: Bearer {token}\r\n");
    let page = b"<script>fetch('/changes')</script>";
    let path = format!("/blobs/{}", sha256(page));
    let (status, _, _) = call_bytes(
        address,
        "PUT",
        &path,
        &format!("{auth}Content-Type: text/html\r\n"),
        page,
    )
    .await;
    assert_eq!(status, 200);
    let (status, head, _) = call_bytes(address, "GET", &path, &auth, &[]).await;
    assert_eq!(status, 200);
    assert!(head.contains("content-disposition: attachment"), "{head}");
    assert!(head.contains("content-security-policy:"), "{head}");
    assert!(head.contains("sandbox"), "{head}");
}

/// What every file of the web UI must go out with.
pub(crate) fn assert_ui_headers(path: &str, answer: &Answer) {
    assert_eq!(answer.status, 200, "{path}");
    let head = answer.headers.to_lowercase();
    let policy = head
        .lines()
        .find_map(|line| line.strip_prefix("content-security-policy: "))
        .unwrap_or_else(|| panic!("{path} has no policy: {head}"));
    for part in [
        "default-src 'none'",
        "script-src 'self'",
        "style-src 'self'",
        "frame-ancestors 'none'",
    ] {
        assert!(
            policy.split("; ").any(|p| p == part),
            "{path}: {part} is missing in {policy}"
        );
    }
    for open in ["unsafe-inline", "unsafe-eval", "data:", "blob:", "*"] {
        assert!(!policy.contains(open), "{path}: {open} in {policy}");
    }
    for header in [
        "x-content-type-options: nosniff",
        "referrer-policy: no-referrer",
        "x-frame-options: deny",
    ] {
        assert!(head.contains(header), "{path}: no {header} in {head}");
    }
}

/// The page and its files carry a strict policy, also without a session:
/// the sign-in is part of the page.
#[tokio::test]
async fn the_web_ui_is_served_with_its_security_headers() {
    let (address, _dir) = start(true).await;
    let object = format!("/o/{}", samu_store::core::api::Id::new());
    for path in [
        "/",
        object.as_str(),
        "/v/today",
        "/app.js",
        "/style.css",
        "/favicon.svg",
        "/lib/dom.js",
    ] {
        assert_ui_headers(path, &call(address, "GET", path, None, None).await);
    }
    // A demo's page is put together on the way out: the same headers.
    let (demo, _dir) = start_with(true, |state| state.with_demo_reset(Some(60))).await;
    let page = call(demo, "GET", "/", None, None).await;
    assert!(page.body.contains("samu-demo"), "not the demo's page");
    assert_ui_headers("/ (demo)", &page);
}

/// ADR 0046 §2, §5: a second running clock ends the first; the report
/// splits at midnight in the account's zone (Berlin) and refuses a span
/// backwards.
#[tokio::test]
async fn the_clock_report_splits_at_midnight_and_one_clock_runs() {
    let (address, dir) = start(true).await;
    let token = sign_in(address).await;
    let (task, value) = new_task("Steuer");
    let clock = |id: &str, start: &str, end: Option<&str>| {
        let mut value = json!({"id": id, "created": start, "modified": start,
                               "owner": task, "start": start});
        if let Some(end) = end {
            value["end"] = json!(end);
        }
        json!({"op": "create", "object": id, "kind": "clock", "value": value,
               "hlc": {"wall": 0, "counter": 0, "device": "ignored"}})
    };
    let first = samu_store::core::api::Id::new().to_string();
    let second = samu_store::core::api::Id::new().to_string();
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&json!([
            create(&task, &value),
            // 23:00 to 01:00 in Berlin.
            clock(&first, "2026-11-09T22:00:00Z", None),
            clock(&second, "2026-11-10T00:00:00Z", None),
        ])),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    let conn = rusqlite::Connection::open(dir.0.join("accounts/privat.db")).unwrap();
    let running: i64 = conn
        .query_row("SELECT count(*) FROM clock WHERE end_at IS NULL", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(running, 1);
    let stop = json!([{"op": "set", "object": second, "field": "end",
                       "value": "2026-11-10T00:30:00Z",
                       "hlc": {"wall": 0, "counter": 0, "device": "ignored"}}]);
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&stop),
    )
    .await;
    assert_eq!(answer.json()[0]["outcome"], "applied", "{}", answer.body);

    let report = call(
        address,
        "GET",
        "/clocks/report?from=2026-11-09&to=2026-11-10&by=day",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(report.status, 200, "{}", report.body);
    let report = report.json();
    assert_eq!(report["seconds"], 9000);
    assert_eq!(report["rows"][0]["date"], "2026-11-09");
    assert_eq!(report["rows"][0]["seconds"], 3600);
    assert_eq!(report["rows"][1]["date"], "2026-11-10");
    assert_eq!(report["rows"][1]["seconds"], 5400);

    let by_task = call(
        address,
        "GET",
        "/clocks/report?from=2026-11-10",
        Some(&token),
        None,
    )
    .await
    .json();
    assert_eq!(by_task["by"], "task");
    assert_eq!(by_task["rows"][0]["title"], "Steuer");
    assert_eq!(by_task["rows"][0]["seconds"], 5400);

    let backwards = call(
        address,
        "GET",
        "/clocks/report?from=2026-11-10&to=2026-11-09",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(backwards.status, 400);
}

mod accounts;
mod calendar;
mod demo;
mod export;
mod grant;
mod register;
mod replica;
mod writer;

/// `POST /render` (ADR 0058 §4): one tree per text, the format read off the
/// text or asked for, and `task` links found by id and by `org_id`.
#[tokio::test]
async fn render_gives_trees_and_finds_linked_tasks() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let (id, mut task) = new_task("Mit Org-ID");
    task["properties"] = json!({"org_id": "abc-123"});
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&json!([create(&id, &task)])),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    // A project is found too (ADR 0077).
    let project = samu_store::core::api::Id::new().to_string();
    let made = json!([{"op": "create", "object": project, "kind": "project",
        "value": {"id": project, "created": "2026-11-07T08:00:00Z",
                  "modified": "2026-11-07T08:00:00Z", "title": "Dach",
                  "kind": "parallel", "status": "active", "flagged": false, "position": 0},
        "hlc": {"wall": 0, "counter": 0, "device": "ignored"}}]);
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&made),
    )
    .await;
    assert_eq!(answer.json()[0]["outcome"], "applied", "{}", answer.body);
    let gone = samu_store::core::api::Id::new();
    let request = json!({"texts": [
        {"text": format!("- [ ] [a](samu:{id}) [b](samu:{gone})")},
        {"text": "#+title: x\n[[id:abc-123][c]] [[id:nope][d]] [[elisp:(x)][e]]"},
        {"text": "[[id:abc-123]]", "format": "markdown"},
        {"text": format!("[[samu:{project}][Dach]]")},
    ]});
    let answer = call(address, "POST", "/render", Some(&token), Some(&request)).await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    let documents = answer.json()["documents"].clone();
    assert_eq!(documents.as_array().unwrap().len(), 4);
    assert_eq!(documents[0]["format"], "markdown");
    assert_eq!(documents[1]["format"], "org");
    assert_eq!(documents[2]["format"], "markdown");
    let item = &documents[0]["blocks"][0]["items"][0];
    assert_eq!(item["checked"], false);
    assert_eq!(item["box"], json!({"start": 2, "end": 5}));
    let links = &item["blocks"][0]["content"];
    assert_eq!(links[0]["task"], json!(id));
    assert_eq!(links[0]["class"], "task");
    assert!(links[2].get("task").is_none(), "{links}");
    let org = &documents[1]["blocks"][1]["content"];
    assert_eq!(org[0]["task"], json!(id));
    assert!(org[2].get("task").is_none(), "{org}");
    assert_eq!(org[4]["class"], "inert");
    assert!(org[4].get("task").is_none());
    // A task link names its object as well; a project link only that.
    assert_eq!(links[0]["object"], json!({"id": id, "kind": "task"}));
    let to_project = &documents[3]["blocks"][0]["content"][0];
    assert_eq!(
        to_project["object"],
        json!({"id": project, "kind": "project"}),
        "{to_project}"
    );
    assert!(to_project.get("task").is_none(), "{to_project}");
}

/// ADR 0030: the reminders the web inspector writes, in its shapes (one
/// days before due, one at an instant), go in and out over `/changes`;
/// a delete takes one away; `remind_time` set as the settings pane sends
/// it reaches the session. Asked of the database and the session.
#[tokio::test]
async fn the_web_sets_and_deletes_reminders_and_the_remind_time() {
    let (address, dir) = start(true).await;
    let token = sign_in(address).await;
    let (task, mut value) = new_task("Steuer");
    value["due"] = json!({"date": "2026-11-20"});
    let stamp = json!({"wall": 0, "counter": 0, "device": "ignored"});
    let reminder = |extra: Value| {
        let id = samu_store::core::api::Id::new().to_string();
        let mut value = json!({"id": id, "created": "2026-11-10T08:00:00Z",
                               "modified": "2026-11-10T08:00:00Z", "owner": task});
        for (k, v) in extra.as_object().unwrap() {
            value[k] = v.clone();
        }
        (
            id.clone(),
            json!({"op": "create", "object": id, "kind": "reminder", "value": value,
                   "hlc": stamp}),
        )
    };
    let (before, made_before) = reminder(json!({"anchor": "due", "before": "P1DT2H"}));
    let (at, made_at) = reminder(json!({"anchor": "at", "at": "2026-11-19T07:30:00Z"}));
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&json!([create(&task, &value), made_before, made_at])),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    assert!(
        answer
            .json()
            .as_array()
            .unwrap()
            .iter()
            .all(|o| o["outcome"] == "applied"),
        "{}",
        answer.body
    );
    let conn = rusqlite::Connection::open(dir.0.join("accounts/privat.db")).unwrap();
    let rows = |conn: &rusqlite::Connection| -> Vec<(String, Option<String>, Option<String>)> {
        let mut statement = conn
            .prepare("SELECT anchor, at, before FROM reminder ORDER BY anchor")
            .unwrap();
        statement
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(
        rows(&conn),
        vec![
            (
                "at".to_owned(),
                Some("2026-11-19T07:30:00Z".to_owned()),
                None
            ),
            ("due".to_owned(), None, Some("P1DT2H".to_owned())),
        ]
    );

    let page = call(address, "GET", "/changes?after=0", Some(&token), None)
        .await
        .json();
    let reminders: Vec<&Value> = page["objects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| o["kind"] == "reminder")
        .collect();
    assert_eq!(reminders.len(), 2);
    let settings = page["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["kind"] == "settings")
        .unwrap()["object"]["id"]
        .clone();

    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&json!([
            {"op": "delete", "object": at, "hlc": stamp},
            {"op": "set", "object": settings, "field": "remind_time", "value": "07:30:00",
             "hlc": stamp},
        ])),
    )
    .await;
    assert!(
        answer
            .json()
            .as_array()
            .unwrap()
            .iter()
            .all(|o| o["outcome"] == "applied"),
        "{}",
        answer.body
    );
    // A delete leaves the row behind its tombstone; what a reader gets is
    // the page.
    let page = call(address, "GET", "/changes?after=0", Some(&token), None)
        .await
        .json();
    let left: Vec<&Value> = page["objects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| o["kind"] == "reminder")
        .map(|o| &o["object"]["id"])
        .collect();
    assert_eq!(left, vec![&json!(before)]);
    let session = call(address, "GET", "/session", Some(&token), None).await;
    assert_eq!(
        session.json()["remind_time"],
        "07:30:00",
        "{}",
        session.body
    );
}

/// `from=…&to=…` for `/report` that holds clocks of the last hours whatever
/// the time of day and zone (01a1d205): today's report alone cuts them at
/// midnight. Two days back to tomorrow.
pub(crate) fn around_now() -> String {
    let today = jiff::Zoned::now().date();
    let day = |n: i64| {
        today
            .checked_add(jiff::ToSpan::days(n))
            .expect("a day near today is a date")
    };
    format!("from={}&to={}", day(-2), day(1))
}

/// ADR 0081 §2 and ADR 0080: a task captured with `z:` and `x:` has its
/// clock and is completed, asked of the store; the report shows it as done
/// and worked on, and an agent's comment only with `autor:`.
#[tokio::test]
async fn capture_books_time_and_the_report_shows_whose_work() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let made = call(
        address,
        "POST",
        "/capture",
        Some(&token),
        Some(&json!({"text": "Review Angebot z:1h z:2h x:"})),
    )
    .await;
    assert_eq!(made.status, 200, "{}", made.body);
    let made = made.json();
    assert_eq!(made["task"]["title"], "Review Angebot");
    let task = made["task"]["id"].as_str().unwrap().to_string();
    let clocks = made["clocks"].as_array().unwrap().clone();
    assert_eq!(clocks.len(), 2);
    // Asked of the store, not of the answer.
    for clock in &clocks {
        let id = clock["id"].as_str().unwrap();
        let stored = call(
            address,
            "GET",
            &format!("/objects/{id}"),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(stored.status, 200, "{}", stored.body);
        assert_eq!(stored.json()["object"]["owner"], task.as_str());
        assert!(stored.json()["object"]["end"].is_string());
    }
    let stored = call(
        address,
        "GET",
        &format!("/objects/{task}"),
        Some(&token),
        None,
    )
    .await;
    assert!(stored.json()["object"]["completed_at"].is_string());
    let log = call(
        address,
        "GET",
        &format!("/objects/{task}/logbook"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(log.json()[0]["kind"], "completed");
    assert_eq!(log.json()[0]["author_kind"], "human");

    // An agent comments on it.
    let agent = call(
        address,
        "POST",
        "/login",
        None,
        Some(&json!({"account": "privat", "password": PASSWORD,
                     "device": "claude", "kind": "agent"})),
    )
    .await;
    let agent = agent.json()["token"].as_str().unwrap().to_string();
    let comment = samu_store::core::api::Id::new().to_string();
    let hlc = json!({"wall": 0, "counter": 0, "device": "ignored"});
    let wrote = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&agent),
        Some(
            &json!([{"op": "create", "object": comment, "kind": "comment",
            "value": {"id": comment, "created": Timestamp::now().to_string(),
                      "modified": Timestamp::now().to_string(), "owner": task,
                      "body": "geprüft"},
            "hlc": hlc}]),
        ),
    )
    .await;
    assert_eq!(wrote.status, 200, "{}", wrote.body);

    let report = |q: &str| {
        let token = token.clone();
        let q = q.to_string();
        async move {
            let answer = call(
                address,
                "GET",
                // From two days back: the clocks reach up to three hours
                // into the past, across midnight right after it (01a1d205).
                &format!("/report?{}&q={q}", around_now()),
                Some(&token),
                None,
            )
            .await;
            assert_eq!(answer.status, 200, "{}", answer.body);
            answer.json()
        }
    };
    let mine = report("").await;
    assert_eq!(mine["done"]["rows"][0]["id"], task.as_str());
    assert_eq!(mine["time"]["seconds"], 3 * 3600);
    let reasons: Vec<&str> = mine["worked"]["rows"][0]["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["reason"].as_str().unwrap())
        .collect();
    assert_eq!(reasons, ["clocked", "completed"]);
    let agents = report("autor:agent").await;
    assert!(agents["done"]["rows"].as_array().unwrap().is_empty());
    // Time has no author and counts for anyone (ADR 0080 §2).
    assert_eq!(
        agents["worked"]["rows"][0]["reasons"],
        json!([{"reason": "clocked", "count": 2}, {"reason": "commented", "count": 1}])
    );
    for bad in [
        "/report?q=autor:jemand",
        "/report?parts=alles",
        "/report?by=woche",
    ] {
        let refused = call(address, "GET", bad, Some(&token), None).await;
        assert_eq!(refused.status, 400, "{bad}: {}", refused.body);
    }
    let only = call(address, "GET", "/report?parts=done", Some(&token), None).await;
    let only = only.json();
    assert!(only.get("worked").is_none() && only.get("time").is_none());
}

/// ADR 0081 §2: `z:` on an existing task adds clocks and keeps the ones it
/// has; a clock in the future is not booked and says so.
#[tokio::test]
async fn capture_with_task_books_more_time() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let made = call(
        address,
        "POST",
        "/capture",
        Some(&token),
        Some(&json!({"text": "Steuer z:30m"})),
    )
    .await
    .json();
    let task = made["task"]["id"].as_str().unwrap().to_string();
    let edited = call(
        address,
        "POST",
        "/capture",
        Some(&token),
        Some(&json!({"text": "z:gestern 2h z:23:58-23:59", "task": task})),
    )
    .await;
    assert_eq!(edited.status, 200, "{}", edited.body);
    let edited = edited.json();
    // Whatever the hour, 23:58 today is in the future but at 23:59.
    let future = edited["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|w| w["warning"] == "clock_in_future");
    assert_eq!(
        edited["clocks"].as_array().unwrap().len(),
        if future { 1 } else { 2 }
    );
    let week = call(
        address,
        "GET",
        "/report?parts=time&from=2000-01-01&to=2100-01-01",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(week.status, 400, "ten years at most: {}", week.body);
    let from = (Timestamp::now() - jiff::SignedDuration::from_hours(72))
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date();
    let total = call(
        address,
        "GET",
        &format!(
            "/report?parts=time&from={from}&to={}",
            from.checked_add(jiff::Span::new().days(5)).unwrap()
        ),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(total.status, 200, "{}", total.body);
    let seconds = total.json()["time"]["seconds"].as_i64().unwrap();
    let want = 30 * 60 + 2 * 3600 + if future { 0 } else { 60 };
    assert_eq!(seconds, want);
}

/// ADR 0126: a task becomes a project — the project takes title, note and
/// children, the task keeps its id and comment and goes in last; a preview
/// writes nothing, a closed task is refused. Asked of the database.
#[tokio::test]
async fn a_task_becomes_a_project_and_keeps_its_id_and_comments() {
    let (address, dir) = start(true).await;
    let token = sign_in(address).await;
    let (group_id, mut group) = new_task("Umzug");
    group["note"] = json!("Wohnung in der Südstadt");
    group["properties"] = json!({"jira": "ABC-1"});
    let (first_id, mut first) = new_task("Kartons");
    first["parent"] = json!(group_id);
    first["position"] = json!(5);
    let (second_id, mut second) = new_task("Möbelwagen");
    second["parent"] = json!(group_id);
    second["position"] = json!(2);
    let (deep_id, mut deep) = new_task("Klebeband");
    deep["parent"] = json!(first_id);
    let (closed_id, mut closed) = new_task("Erledigt");
    closed["completed_at"] = json!("2026-11-17T08:00:00Z");
    let comment = samu_store::core::api::Id::new().to_string();
    let changes = json!([
        create(&group_id, &group),
        create(&first_id, &first),
        create(&second_id, &second),
        create(&deep_id, &deep),
        create(&closed_id, &closed),
        {"op": "create", "object": comment, "kind": "comment",
         "value": {"id": comment, "created": "2026-11-18T08:00:00Z",
                   "modified": "2026-11-18T08:00:00Z", "owner": group_id,
                   "body": "Angebot liegt vor"},
         "hlc": {"wall": 0, "counter": 0, "device": "ignored"}},
    ]);
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&changes),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);

    let conn = rusqlite::Connection::open(dir.0.join("accounts/privat.db")).unwrap();
    let projects = || -> i64 {
        conn.query_row("SELECT count(*) FROM project", [], |r| r.get(0))
            .unwrap()
    };
    let path = format!("/objects/{group_id}/to-project");
    let preview = call(
        address,
        "POST",
        &path,
        Some(&token),
        Some(&json!({"prefix": "Notizen: ", "preview": true})),
    )
    .await;
    assert_eq!(preview.status, 200, "{}", preview.body);
    assert_eq!(preview.json()["moved"], 4, "{}", preview.body);
    assert_eq!(projects(), 0, "a preview writes nothing");

    let written = call(
        address,
        "POST",
        &path,
        Some(&token),
        Some(&json!({"prefix": "Notizen: "})),
    )
    .await;
    assert_eq!(written.status, 200, "{}", written.body);
    assert!(written.json()["seq"].as_u64().is_some(), "{}", written.body);
    let project = written.json()["project"].as_str().unwrap().to_string();
    let (title, note, kind): (String, Option<String>, String) = conn
        .query_row(
            "SELECT p.title, p.note, p.kind FROM project p JOIN object o ON o.id = p.object \
             WHERE hex(o.uuid) = ?1",
            [hex(&project)],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        (title.as_str(), note.as_deref(), kind.as_str()),
        ("Umzug", Some("Wohnung in der Südstadt"), "parallel")
    );
    let task = |id: &str| -> (String, Option<String>, Option<String>, i64, Option<String>) {
        conn.query_row(
            "SELECT t.title, hex(t.project), nullif(hex(t.parent), ''), t.position, t.note \
             FROM task t JOIN object o ON o.id = t.object WHERE hex(o.uuid) = ?1",
            [hex(id)],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap()
    };
    let p = Some(hex(&project));
    assert_eq!(
        task(&second_id),
        ("Möbelwagen".into(), p.clone(), None, 0, None)
    );
    assert_eq!(
        task(&first_id),
        ("Kartons".into(), p.clone(), None, 1, None)
    );
    assert_eq!(
        task(&deep_id),
        ("Klebeband".into(), p.clone(), Some(hex(&first_id)), 0, None),
        "the grandchild keeps its parent"
    );
    assert_eq!(
        task(&group_id),
        ("Notizen: Umzug".into(), p, None, 2, None),
        "the task stays, last, its note gone to the project"
    );
    let owner: String = conn
        .query_row(
            "SELECT hex(c.owner) FROM comment c JOIN object o ON o.id = c.object \
             WHERE hex(o.uuid) = ?1",
            [hex(&comment)],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(owner, hex(&group_id), "the comment stays on the task");

    let refused = call(
        address,
        "POST",
        &format!("/objects/{closed_id}/to-project"),
        Some(&token),
        Some(&json!({})),
    )
    .await;
    assert_eq!(refused.status, 400, "{}", refused.body);
    assert_eq!(projects(), 1, "a refusal writes nothing");
}

/// ADR 0128: closing a group with open children asks first (nothing
/// written), then `close` completes them along and `lift` moves them one
/// level up; the logbook counts each. Asked of the database.
#[tokio::test]
async fn closing_a_group_asks_then_closes_or_lifts_its_open_children() {
    let (address, dir) = start(true).await;
    let token = sign_in(address).await;
    let (group_id, group) = new_task("Umzug");
    let (first_id, mut first) = new_task("Kartons");
    first["parent"] = json!(group_id);
    first["position"] = json!(0);
    let (deep_id, mut deep) = new_task("Klebeband");
    deep["parent"] = json!(first_id);
    let (other_id, mut other) = new_task("Bad");
    other["position"] = json!(3);
    let (rest_id, mut rest) = new_task("Rest");
    rest["parent"] = json!(other_id);
    let changes = json!([
        create(&group_id, &group),
        create(&first_id, &first),
        create(&deep_id, &deep),
        create(&other_id, &other),
        create(&rest_id, &rest),
    ]);
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&changes),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);

    let conn = rusqlite::Connection::open(dir.0.join("accounts/privat.db")).unwrap();
    let task = |id: &str| -> (bool, Option<String>, i64) {
        conn.query_row(
            "SELECT t.completed_at IS NOT NULL, nullif(hex(t.parent), ''), t.position \
             FROM task t JOIN object o ON o.id = t.object WHERE hex(o.uuid) = ?1",
            [hex(id)],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap()
    };

    let path = format!("/objects/{group_id}/close");
    let asked = call(
        address,
        "POST",
        &path,
        Some(&token),
        Some(&json!({"field": "completed"})),
    )
    .await;
    assert_eq!(asked.status, 200, "{}", asked.body);
    assert_eq!(asked.json()["open"], 2, "{}", asked.body);
    assert!(asked.json()["seq"].is_null(), "{}", asked.body);
    assert!(!task(&group_id).0, "asking writes nothing");

    let closed = call(
        address,
        "POST",
        &path,
        Some(&token),
        Some(&json!({"field": "completed", "children": "close", "note": "fertig"})),
    )
    .await;
    assert_eq!(closed.status, 200, "{}", closed.body);
    assert_eq!(closed.json()["closed"], 3, "{}", closed.body);
    assert!(closed.json()["seq"].as_u64().is_some(), "{}", closed.body);
    for id in [&group_id, &first_id, &deep_id] {
        assert!(task(id).0, "{id} completed");
    }
    let logged: i64 = conn
        .query_row(
            "SELECT count(*) FROM logbook WHERE kind = 'completed'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(logged, 3);
    let again = call(
        address,
        "POST",
        &path,
        Some(&token),
        Some(&json!({"field": "completed", "children": "close"})),
    )
    .await;
    assert_eq!(again.json()["closed"], 0, "a closed task stays as it is");

    let lifted = call(
        address,
        "POST",
        &format!("/objects/{other_id}/close"),
        Some(&token),
        Some(&json!({"field": "dropped", "children": "lift"})),
    )
    .await;
    assert_eq!(lifted.status, 200, "{}", lifted.body);
    assert_eq!(lifted.json()["lifted"], 1, "{}", lifted.body);
    assert_eq!(task(&rest_id), (false, None, 4), "right after the task");
    let dropped: bool = conn
        .query_row(
            "SELECT t.dropped_at IS NOT NULL FROM task t JOIN object o ON o.id = t.object \
             WHERE hex(o.uuid) = ?1",
            [hex(&other_id)],
            |r| r.get(0),
        )
        .unwrap();
    assert!(dropped);
}
