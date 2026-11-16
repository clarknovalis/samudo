// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The replica on a computer (ADR 0066, ADR 0067): a real server as the
//! upstream, the replica before it, both in this process.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;
use samu_store::ServerStore;
use samu_store::core::api::{ROUTES, SessionInfo};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{Answer, PASSWORD, TempDir};
use crate::app::{AppState, account_path, router};
use crate::replica::{Loopback, Replica, Where, locality};

/// An upstream server on `address` (a free port without one) over `dir`;
/// aborting the handle stops it taking connections.
async fn upstream(dir: &std::path::Path, address: Option<SocketAddr>) -> (SocketAddr, Upstream) {
    let state = Arc::new(AppState::new(dir).unwrap());
    let listener = tokio::net::TcpListener::bind(address.unwrap_or(([127, 0, 0, 1], 0).into()))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, router(state)).await.unwrap();
    });
    (address, Upstream(task))
}

struct Upstream(tokio::task::JoinHandle<()>);

impl Drop for Upstream {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// A replica of `upstream` on a free port.
async fn replica(upstream: SocketAddr, dir: &std::path::Path) -> SocketAddr {
    replica_of_profile(upstream, dir, None).await
}

/// A replica of `upstream` on a free port, run as a profile or not.
async fn replica_of_profile(
    upstream: SocketAddr,
    dir: &std::path::Path,
    profile: Option<&str>,
) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let loopback = Loopback {
        port: address.port(),
        name: profile.map(|p| format!("{p}.samu.localhost")),
    };
    let state = Replica::start(dir, &format!("http://{upstream}"), "rechner", loopback).unwrap();
    tokio::spawn(async move {
        axum::serve(listener, crate::replica::router(state))
            .await
            .unwrap();
    });
    address
}

/// One request to the replica with the `Host` it accepts, or `host`.
async fn call_as(
    address: SocketAddr,
    host: &str,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&Value>,
) -> Answer {
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let body = body.map(Value::to_string).unwrap_or_default();
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n{auth}\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).await.unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((&raw, ""));
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    Answer {
        status,
        headers: head.to_string(),
        body: body.to_string(),
    }
}

async fn call(
    address: SocketAddr,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&Value>,
) -> Answer {
    let host = format!("127.0.0.1:{}", address.port());
    call_as(address, &host, method, path, token, body).await
}

async fn sign_in(address: SocketAddr) -> Answer {
    call(
        address,
        "POST",
        "/login",
        None,
        Some(&json!({"account": "privat", "password": PASSWORD, "device": "browser"})),
    )
    .await
}

/// Waits up to `seconds` until `check` holds.
async fn eventually<F, Fut>(seconds: u64, mut check: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    while tokio::time::Instant::now() < deadline {
        if check().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    false
}

/// The titles of the live tasks in the upstream account's database, asked
/// directly, not taken from what the replica reports.
fn titles_upstream(dir: &std::path::Path) -> Vec<String> {
    let conn = rusqlite::Connection::open(account_path(dir, "privat")).unwrap();
    let mut statement = conn
        .prepare(
            "SELECT t.title FROM task t JOIN object o ON o.id = t.object \
             WHERE o.deleted_at IS NULL",
        )
        .unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// A TCP proxy before the upstream that can cut the line and mend it: the
/// network failing, the server running on.
struct Line {
    address: SocketAddr,
    target: SocketAddr,
    tasks: Arc<std::sync::Mutex<Vec<tokio::task::JoinHandle<()>>>>,
}

impl Line {
    async fn open(target: SocketAddr) -> Self {
        let mut line = Self {
            address: ([127, 0, 0, 1], 0).into(),
            target,
            tasks: Arc::default(),
        };
        line.address = line.listen(None).await;
        line
    }

    async fn listen(&self, address: Option<SocketAddr>) -> SocketAddr {
        let listener = tokio::net::TcpListener::bind(address.unwrap_or(self.address))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let (target, tasks) = (self.target, self.tasks.clone());
        let accept = tokio::spawn(async move {
            while let Ok((mut inbound, _)) = listener.accept().await {
                let pipe = tokio::spawn(async move {
                    if let Ok(mut outbound) = tokio::net::TcpStream::connect(target).await {
                        let _ = tokio::io::copy_bidirectional(&mut inbound, &mut outbound).await;
                    }
                });
                tasks.lock().unwrap().push(pipe);
            }
        });
        self.tasks.lock().unwrap().push(accept);
        address
    }

    /// Every connection dropped, no new one taken.
    fn cut(&self) {
        for task in self.tasks.lock().unwrap().drain(..) {
            task.abort();
        }
    }

    async fn mend(&self) {
        self.listen(Some(self.address)).await;
    }
}

fn create_task(title: &str) -> Value {
    let id = samu_store::core::api::Id::new();
    json!([{
        "op": "create", "object": id.to_string(), "kind": "task",
        "value": {
            "id": id.to_string(), "created": "2026-11-10T18:00:00Z",
            "modified": "2026-11-10T18:00:00Z", "title": title,
            "flagged": false, "sequential": false,
            "complete_with_last_child": false, "position": 0
        },
        "hlc": {"wall": 0, "counter": 0, "device": "x"}
    }])
}

fn upstream_dir() -> TempDir {
    let dir = TempDir::new();
    ServerStore::open(&dir.0.join("server.db"))
        .unwrap()
        .add_account("privat", PASSWORD, Timestamp::now())
        .unwrap();
    dir
}

/// ADR 0066 §5: every route has its place, and a new one without fails.
#[test]
fn every_route_has_a_place_on_the_replica() {
    for route in ROUTES {
        assert!(
            locality(route).is_some(),
            "{} {} needs an entry in replica::locality",
            route.method,
            route.path
        );
    }
    let logbook = ROUTES
        .iter()
        .find(|r| r.path == "/objects/{id}/logbook")
        .unwrap();
    assert_eq!(locality(logbook), Some(Where::Online));
    let sessions = ROUTES.iter().find(|r| r.path == "/sessions").unwrap();
    assert_eq!(locality(sessions), Some(Where::Relay));
    for route in ROUTES.iter().filter(|r| r.path.starts_with("/calendars")) {
        assert_eq!(locality(route), Some(Where::Relay), "{}", route.path);
    }
}

/// ADR 0066 §5: the replica answers every listed route, neither 404 nor
/// 405, and what is online only with 503 `offline`.
#[tokio::test]
async fn the_replica_answers_every_listed_route() {
    let up_dir = upstream_dir();
    let (up, _upstream) = upstream(&up_dir.0, None).await;
    let dir = TempDir::new();
    let address = replica(up, &dir.0).await;
    for route in ROUTES {
        let login = sign_in(address).await;
        assert_eq!(login.status, 200, "{}", login.body);
        let token = serde_json::from_str::<SessionInfo>(&login.body)
            .unwrap()
            .token
            .unwrap();
        let path = route
            .path
            .replace("{id}", "01936f5e-8a00-7c3e-9b1a-000000000001")
            .replace("{sha256}", &"a".repeat(64))
            .replace("{name}", "ring");
        if route.path == "/events" {
            continue;
        }
        let answer = call(address, route.method, &path, Some(&token), Some(&json!({}))).await;
        assert!(
            answer.status != 405 && !(answer.status == 404 && answer.body.is_empty()),
            "{} {}: {} {}",
            route.method,
            route.path,
            answer.status,
            answer.body
        );
        if locality(route) == Some(Where::Online) {
            assert_eq!(answer.status, 503, "{} {}", route.method, route.path);
            assert_eq!(answer.json()["error"], "offline");
        }
    }
}

/// ADR 0096: the sessions go through to the server while it is there,
/// the replica's own cannot be ended, and away they are `offline`.
#[tokio::test]
async fn sessions_go_through_to_the_server() {
    let up_dir = upstream_dir();
    let (up, _upstream) = upstream(&up_dir.0, None).await;
    let line = Line::open(up).await;
    let dir = TempDir::new();
    let address = replica(line.address, &dir.0).await;
    let login = sign_in(address).await;
    assert_eq!(login.status, 200, "{}", login.body);
    let token = login.json()["token"].as_str().unwrap().to_string();
    let other = super::sign_in(up).await;

    // Without a local session: nothing about the account.
    let anonymous = call(address, "GET", "/sessions", None, None).await;
    assert_eq!(anonymous.status, 401, "{}", anonymous.body);

    let listed = call(address, "GET", "/sessions", Some(&token), None).await;
    assert_eq!(listed.status, 200, "{}", listed.body);
    let list = listed.json();
    let rows = list.as_array().unwrap();
    assert_eq!(rows.len(), 2, "{}", listed.body);
    let own = rows.iter().find(|r| r["current"] == true).unwrap();
    assert_eq!(own["device"], "rechner");
    let laptop = rows.iter().find(|r| r["device"] == "laptop").unwrap();

    // Its own session the replica keeps, without asking the server.
    let own_id = own["id"].as_str().unwrap();
    let refused = call(
        address,
        "POST",
        &format!("/sessions/{own_id}/end"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(refused.status, 403, "{}", refused.body);
    assert_eq!(refused.json()["error"], "forbidden");
    let bad = call(address, "POST", "/sessions/xyz/end", Some(&token), None).await;
    assert_eq!(bad.status, 400, "{}", bad.body);

    // Another one it ends at the server, and the server says so.
    let laptop_id = laptop["id"].as_str().unwrap();
    let ended = call(
        address,
        "POST",
        &format!("/sessions/{laptop_id}/end"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(ended.status, 204, "{}", ended.body);
    let gone = super::call(up, "GET", "/session", Some(&other), None).await;
    assert_eq!(gone.status, 401, "{}", gone.body);
    let unknown = call(
        address,
        "POST",
        &format!("/sessions/{laptop_id}/end"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(unknown.status, 404, "{}", unknown.body);
    let after = call(address, "GET", "/sessions", Some(&token), None).await;
    assert_eq!(after.json().as_array().unwrap().len(), 1, "{}", after.body);

    // Away: offline, and the local session holds.
    line.cut();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let away = call(address, "GET", "/sessions", Some(&token), None).await;
    assert_eq!(away.status, 503, "{}", away.body);
    assert_eq!(away.json()["error"], "offline");
    let still = call(address, "GET", "/session", Some(&token), None).await;
    assert_eq!(still.status, 200, "{}", still.body);

    // The server ends the replica's session: offline with a hint, not 401,
    // and the local session holds (ADR 0096 §3).
    line.mend().await;
    let laptop = super::sign_in(up).await;
    let cut_off = super::call(
        up,
        "POST",
        &format!("/sessions/{own_id}/end"),
        Some(&laptop),
        None,
    )
    .await;
    assert_eq!(cut_off.status, 204, "{}", cut_off.body);
    let stale = call(address, "GET", "/sessions", Some(&token), None).await;
    assert_eq!(stale.status, 503, "{}", stale.body);
    assert!(stale.body.contains("sign in again"), "{}", stale.body);
    let still = call(address, "GET", "/session", Some(&token), None).await;
    assert_eq!(still.status, 200, "{}", still.body);
}

/// ADR 0100 §9: the calendars' sources go through to the server with
/// their body, and the server's answer comes back as it is; away they are
/// `offline`. Before, they were always 503 (finding `01a1d70a`).
#[tokio::test]
async fn calendar_routes_go_through_with_their_body() {
    let up_dir = upstream_dir();
    let (up, _upstream) = upstream(&up_dir.0, None).await;
    let line = Line::open(up).await;
    let dir = TempDir::new();
    let address = replica(line.address, &dir.0).await;
    let login = sign_in(address).await;
    assert_eq!(login.status, 200, "{}", login.body);
    let token = login.json()["token"].as_str().unwrap().to_string();

    let anonymous = call(address, "POST", "/calendars/discover", None, None).await;
    assert_eq!(anonymous.status, 401, "{}", anonymous.body);

    // Only the server says this, and only about the address in the body.
    let body = json!({"url": "not a url", "username": "u", "password": "p"});
    let found = call(
        address,
        "POST",
        "/calendars/discover",
        Some(&token),
        Some(&body),
    )
    .await;
    assert_eq!(found.status, 400, "{}", found.body);
    assert!(found.body.contains("from that host"), "{}", found.body);
    assert!(
        found
            .headers
            .to_ascii_lowercase()
            .contains("content-type: application/json"),
        "{}",
        found.headers
    );

    let unknown = "/calendars/01936f5e-8a00-7c3e-9b1a-000000000001/refresh";
    let refresh = call(address, "POST", unknown, Some(&token), None).await;
    assert_ne!(refresh.status, 503, "{}", refresh.body);
    assert_eq!(refresh.json()["error"], "not_found", "{}", refresh.body);
    let import = "/calendars/01936f5e-8a00-7c3e-9b1a-000000000001/import";
    let imported = call(
        address,
        "POST",
        import,
        Some(&token),
        Some(&json!("BEGIN:VCALENDAR")),
    )
    .await;
    // The upstream here has no key: its own answer, passed through.
    assert!(
        imported.body.contains("SAMU_SECRET_KEY"),
        "{}",
        imported.body
    );

    line.cut();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let away = call(
        address,
        "POST",
        "/calendars/discover",
        Some(&token),
        Some(&body),
    )
    .await;
    assert_eq!(away.status, 503, "{}", away.body);
    assert_eq!(away.json()["error"], "offline");
}

/// ADR 0066 §8: only 127.0.0.1 or localhost with its port.
#[tokio::test]
async fn a_foreign_host_is_turned_away() {
    let up_dir = upstream_dir();
    let (up, _upstream) = upstream(&up_dir.0, None).await;
    let dir = TempDir::new();
    let address = replica(up, &dir.0).await;
    let port = address.port();
    for host in [
        "evil.example",
        "127.0.0.1",
        "127.0.0.1:1",
        "localhost.evil:80",
    ] {
        let answer = call_as(address, host, "GET", "/health", None, None).await;
        assert_eq!(answer.status, 421, "{host}");
    }
    for host in [format!("127.0.0.1:{port}"), format!("localhost:{port}")] {
        let answer = call_as(address, &host, "GET", "/health", None, None).await;
        assert_eq!(answer.status, 200, "{host}");
    }
    // Without a profile it has no name (ADR 0087 §3).
    let answer = call_as(
        address,
        "privat.samu.localhost",
        "GET",
        "/health",
        None,
        None,
    )
    .await;
    assert_eq!(answer.status, 421);
}

/// ADR 0087 §3: a profile answers to its name too, with or without a
/// port, and to no other profile's.
#[tokio::test]
async fn a_profile_answers_to_its_name() {
    let up_dir = upstream_dir();
    let (up, _upstream) = upstream(&up_dir.0, None).await;
    let dir = TempDir::new();
    let address = replica_of_profile(up, &dir.0, Some("privat")).await;
    let port = address.port();
    for host in [
        "privat.samu.localhost".to_string(),
        "PRIVAT.samu.localhost".to_string(),
        format!("privat.samu.localhost:{port}"),
        "privat.samu.localhost:80".to_string(),
        format!("127.0.0.1:{port}"),
    ] {
        let answer = call_as(address, &host, "GET", "/health", None, None).await;
        assert_eq!(answer.status, 200, "{host}");
    }
    for host in [
        "dienst.samu.localhost",
        "privat.samu.localhost.evil",
        "privat.samu.localhost:",
        "privat.samu.localhost:x",
        "samu.localhost",
        "xprivat.samu.localhost",
    ] {
        let answer = call_as(address, host, "GET", "/health", None, None).await;
        assert_eq!(answer.status, 421, "{host}");
    }
}

/// ADR 0067: an agent signs in at the server, a wrong password is refused
/// as there.
#[tokio::test]
async fn signing_in_goes_through_and_agents_stay_out() {
    let up_dir = upstream_dir();
    let (up, _upstream) = upstream(&up_dir.0, None).await;
    let dir = TempDir::new();
    let address = replica(up, &dir.0).await;
    let wrong = call(
        address,
        "POST",
        "/login",
        None,
        Some(&json!({"account": "privat", "password": "nope", "device": "b"})),
    )
    .await;
    assert_eq!(wrong.status, 401, "{}", wrong.body);
    let agent = call(
        address,
        "POST",
        "/login",
        None,
        Some(&json!({"account": "privat", "password": PASSWORD,
                     "device": "claude", "kind": "agent"})),
    )
    .await;
    assert_eq!(agent.status, 403, "{}", agent.body);
    // ADR 0095 §6: the replica mirrors a whole account.
    let restricted = call(
        address,
        "POST",
        "/login",
        None,
        Some(
            &json!({"account": "privat", "password": PASSWORD, "device": "b",
                     "only": {"objects": [samu_store::core::api::Id::new()], "access": "ro"}}),
        ),
    )
    .await;
    assert_eq!(restricted.status, 400, "{}", restricted.body);
    let login = sign_in(address).await;
    assert_eq!(login.status, 200, "{}", login.body);
    // The cookie is not `Secure`: plain HTTP on the loopback.
    assert!(login.headers.contains("samu_session="), "{}", login.headers);
    assert!(!login.headers.contains("Secure"), "{}", login.headers);
    let token = login.json()["token"].as_str().unwrap().to_string();
    // The upstream knows the replica's device, not the browser's.
    let mut server = ServerStore::open(&up_dir.0.join("server.db")).unwrap();
    assert!(server.session(&token, Timestamp::now()).unwrap().is_none());
    let session = call(address, "GET", "/session", Some(&token), None).await;
    assert_eq!(session.json()["device"], "browser");
    assert_eq!(
        session.json()["replica"]["upstream"],
        format!("http://{up}")
    );
    // Signed out, the token is gone.
    let out = call(address, "POST", "/logout", Some(&token), None).await;
    assert_eq!(out.status, 204);
    let after = call(address, "GET", "/session", Some(&token), None).await;
    assert_eq!(after.status, 401);
}

/// ADR 0066 §2, §3, ADR 0067 §4: written while the server is away, kept
/// in the outbox and shown at once; on the server once it is back. The
/// local session holds meanwhile.
#[tokio::test]
async fn offline_writes_reach_the_server_when_it_is_back() {
    let up_dir = upstream_dir();
    let (up, _upstream) = upstream(&up_dir.0, None).await;
    let line = Line::open(up).await;
    let dir = TempDir::new();
    let address = replica(line.address, &dir.0).await;
    let login = sign_in(address).await;
    assert_eq!(login.status, 200, "{}", login.body);
    let token = login.json()["token"].as_str().unwrap().to_string();

    // Online: a capture reaches the server's database.
    let captured = call(
        address,
        "POST",
        "/capture",
        Some(&token),
        Some(&json!({"text": "online erfasst"})),
    )
    .await;
    assert_eq!(captured.status, 200, "{}", captured.body);
    assert!(
        eventually(10, || async {
            titles_upstream(&up_dir.0).contains(&"online erfasst".to_string())
        })
        .await
    );

    // A change made on the server comes to the replica and its /events.
    let other = super::sign_in(up).await;
    let events = tokio::spawn({
        let token = token.clone();
        async move { stream_replica(address, &token, "event: changed", 15).await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let made = super::call(
        up,
        "POST",
        "/changes?stamp=server",
        Some(&other),
        Some(&create_task("vom Server")),
    )
    .await;
    assert_eq!(made.json()[0]["outcome"], "applied", "{}", made.body);
    let seen = events.await.unwrap();
    assert!(seen.contains("event: changed"), "{seen}");
    assert!(
        eventually(10, || async {
            let all = call(address, "GET", "/available?all=true", Some(&token), None).await;
            all.body.contains("vom Server")
        })
        .await
    );

    // The line to the server breaks.
    line.cut();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let written = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(&create_task("offline geschrieben")),
    )
    .await;
    assert_eq!(written.json()[0]["outcome"], "applied", "{}", written.body);
    let shown = call(address, "GET", "/available", Some(&token), None).await;
    assert!(shown.body.contains("offline geschrieben"), "{}", shown.body);
    assert!(
        eventually(15, || async {
            let session = call(address, "GET", "/session", Some(&token), None).await;
            session.json()["replica"]["online"] == false && session.json()["replica"]["outbox"] == 1
        })
        .await
    );
    // Only the server can, and it is away: 503 offline.
    let logbook = call(
        address,
        "GET",
        "/objects/01936f5e-8a00-7c3e-9b1a-000000000001/logbook",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(logbook.status, 503);
    assert_eq!(logbook.json()["error"], "offline");
    let again = sign_in(address).await;
    assert_eq!(again.status, 503, "{}", again.body);
    assert_eq!(again.json()["error"], "offline");
    assert!(!titles_upstream(&up_dir.0).contains(&"offline geschrieben".to_string()));
    // An edit by a line works offline too: the rule is in samu-core
    // (ADR 0078 §3).
    let online = captured.json()["task"]["id"].as_str().unwrap().to_string();
    let edited = call(
        address,
        "POST",
        "/capture",
        Some(&token),
        Some(&json!({"text": "!B", "task": online})),
    )
    .await;
    assert_eq!(edited.status, 200, "{}", edited.body);
    assert_eq!(edited.json()["task"]["priority"], "B");
    assert_eq!(edited.json()["created"], false);

    // The line is back: the outbox goes up.
    line.mend().await;
    assert!(
        eventually(30, || async {
            titles_upstream(&up_dir.0).contains(&"offline geschrieben".to_string())
        })
        .await,
        "the outbox did not reach the server"
    );
    assert!(
        eventually(10, || async {
            let conn = rusqlite::Connection::open(account_path(&up_dir.0, "privat")).unwrap();
            conn.query_row(
                "SELECT t.priority FROM task t JOIN object o ON o.id = t.object \
                 WHERE t.title = 'online erfasst'",
                [],
                |row| row.get::<_, Option<String>>(0),
            )
            .unwrap()
                == Some("B".into())
        })
        .await,
        "the offline edit did not reach the server"
    );
    assert!(
        eventually(10, || async {
            let session = call(address, "GET", "/session", Some(&token), None).await;
            session.json()["replica"]["online"] == true && session.json()["replica"]["outbox"] == 0
        })
        .await
    );
}

/// Reads the replica's `/events` until `until` shows up.
async fn stream_replica(address: SocketAddr, token: &str, until: &str, seconds: u64) -> String {
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let request = format!(
        "GET /events HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {token}\r\n\r\n",
        address.port()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut seen = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut buffer = [0u8; 4096];
    while !seen.contains(until) {
        match tokio::time::timeout_at(deadline, stream.read(&mut buffer)).await {
            Ok(Ok(n)) if n > 0 => seen.push_str(&String::from_utf8_lossy(&buffer[..n])),
            _ => break,
        }
    }
    seen
}

/// ADR 0080 §4: the replica reports done and time from its mirror, with the
/// clocks a `z:` booked locally; `worked` asked for needs the server.
#[tokio::test]
async fn the_replica_reports_done_and_time_but_not_worked() {
    let up_dir = upstream_dir();
    let (up, _upstream) = upstream(&up_dir.0, None).await;
    let dir = TempDir::new();
    let address = replica(up, &dir.0).await;
    let login = sign_in(address).await;
    assert_eq!(login.status, 200, "{}", login.body);
    let token = login.json()["token"].as_str().unwrap().to_string();
    let captured = call(
        address,
        "POST",
        "/capture",
        Some(&token),
        Some(&json!({"text": "Ablage z:45m x:"})),
    )
    .await;
    assert_eq!(captured.status, 200, "{}", captured.body);
    // The replica syncs beside the request, and its first sync may show an
    // empty mirror for a moment: the checks are on the answer that settled.
    let seen = Arc::new(std::sync::Mutex::new(Value::Null));
    let settled = eventually(10, || {
        let seen = Arc::clone(&seen);
        let token = token.clone();
        async move {
            let path = format!("/report?{}", super::around_now());
            let report = call(address, "GET", &path, Some(&token), None).await.json();
            let ok = report["done"]["rows"][0]["title"] == "Ablage"
                && report["time"]["seconds"] == 45 * 60;
            *seen.lock().unwrap() = report;
            ok
        }
    })
    .await;
    let report = seen.lock().unwrap().clone();
    assert!(settled, "{report}");
    assert!(report.get("worked").is_none());
    let worked = call(address, "GET", "/report?parts=worked", Some(&token), None).await;
    assert_eq!(worked.status, 503, "{}", worked.body);
    assert_eq!(worked.json()["error"], "offline");
}

/// ADR 0101 §5: `/alarms` is the replica's own, from its mirror and
/// samu-core; it answers also with the server away.
#[tokio::test]
async fn alarms_come_from_the_mirror_also_without_the_server() {
    let up_dir = upstream_dir();
    let (up, upstream) = upstream(&up_dir.0, None).await;
    let dir = TempDir::new();
    let address = replica(up, &dir.0).await;
    let login = sign_in(address).await;
    let token = serde_json::from_str::<SessionInfo>(&login.body)
        .unwrap()
        .token
        .unwrap();
    let route = ROUTES.iter().find(|r| r.path == "/alarms").unwrap();
    assert_eq!(locality(route), Some(Where::Local));
    drop(upstream);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let answer = call(address, "GET", "/alarms?tasks=false", Some(&token), None).await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    assert_eq!(answer.json(), json!([]));
    let long = call(
        address,
        "GET",
        "/alarms?from=2026-11-01T00:00:00Z&to=2026-11-09T00:00:00Z",
        Some(&token),
        None,
    )
    .await;
    assert_eq!(long.status, 400, "{}", long.body);
}
