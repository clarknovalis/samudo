// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `samu-server replica`: the replica on a computer (ADR 0066). It answers
//! the same API and serves the same web UI on 127.0.0.1, from a mirror of
//! the server's objects with an outbox laid over it (`samu-replica`), and
//! computes what the routes answer with the same `samu_core::views` as the
//! server. What only the server can do answers 503 `offline`. Signing in
//! goes through to the server (ADR 0067); the sessions and the calendars'
//! sources are passed on to it (ADR 0096, ADR 0100 §9).

use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Path as UrlPath, Query, Request, State};
use axum::http::header::{HOST, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use jiff::{SignedDuration, Timestamp};
use samu_client::Client;
use samu_replica::Cache;
use samu_store::core::api::{
    AnyObject, AuthorKind, BareStatus, Blob, CaptureRequest, CaptureResponse, Change, ChangeNotice,
    ChangeOutcome, ChangesPage, DeleteRequest, DeleteResponse, ErrorCode, Hlc, Id, Login, Period,
    RenderRequest, RenderResponse, ReplicaState, ResetNotice, Route, SessionInfo, ToProjectRequest,
    ToProjectResponse,
};
use samu_store::core::availability::Settings;
use samu_store::core::clock::{next_stamp, restamp};
use samu_store::core::overlay::{self, Objects};
use samu_store::core::views;
use samu_store::{BlobDir, new_token, token_digest};
use serde::Deserialize;

use crate::app::{COOKIE_NAME, Failure, lock, token_of};

/// Where a route is answered on the replica (ADR 0066 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Where {
    /// From the mirror with the outbox over it.
    Local,
    /// Only the server can: 503 `offline`, also while it is reachable.
    Online,
    /// Passed on to the server with the replica's own session while it is
    /// reachable, else 503 `offline` (ADR 0096).
    Relay,
}

/// Every route of `samu_api::ROUTES` and where the replica answers it. A
/// new route without an entry fails a test (ADR 0066 §5).
pub(crate) fn locality(route: &Route) -> Option<Where> {
    use Where::{Local, Online, Relay};
    Some(match (route.method, route.path) {
        ("GET", "/health")
        | ("POST", "/login")
        | ("POST", "/logout")
        | ("GET", "/session")
        | ("POST", "/changes")
        | ("GET", "/changes")
        | ("GET", "/objects/{id}")
        | ("POST", "/objects/{id}/delete")
        | ("POST", "/objects/{id}/to-project")
        | ("GET", "/available")
        | ("GET", "/blocks")
        | ("GET", "/available/projects")
        | ("GET", "/available/folders")
        | ("GET", "/scopes/scheduled")
        | ("GET", "/forecast")
        | ("GET", "/alarms")
        | ("GET", "/clocks/report")
        | ("GET", "/report")
        | ("GET", "/habits")
        | ("GET", "/habits/{id}")
        | ("POST", "/capture")
        | ("POST", "/render")
        | ("GET", "/events")
        | ("PUT", "/blobs/{sha256}")
        | ("GET", "/blobs/{sha256}")
        | ("GET", "/appointments")
        | ("POST", "/appointments/{id}/edit") => Local,
        ("GET", "/objects/{id}/logbook")
        | ("GET", "/logbook")
        | ("GET", "/export")
        | ("GET", "/hooks")
        | ("POST", "/hooks")
        | ("POST", "/hooks/{name}")
        | ("POST", "/pair/code")
        | ("POST", "/pair") => Online,
        // Only the server makes or deletes accounts (ADR 0099 §2, §5).
        ("GET" | "POST", "/register") | ("POST", "/account/delete") => Online,
        // The demo is written by the server, into its store (ADR 0099 §7).
        ("POST", "/demo") => Online,
        ("GET", "/sessions") | ("POST", "/sessions/{id}/end") => Relay,
        // The calendars' sources and fetching live on the server; the
        // replica passes them on (ADR 0100 §9, changes ADR 0066 §5).
        ("PUT", "/calendars/{id}/source")
        | ("POST", "/calendars/{id}/refresh")
        | ("POST", "/calendars/discover")
        | ("POST", "/calendars/{id}/discover")
        | ("POST", "/calendars/{id}/import") => Relay,
        _ => return None,
    })
}

/// How long a call to the server may take before it counts as offline.
const TIMEOUT: Duration = Duration::from_secs(15);
/// How often the replica syncs while nothing kicks it.
const PERIOD: Duration = Duration::from_secs(60);
/// The first wait after a failed try; it doubles up to [`PERIOD`].
const FIRST_RETRY: Duration = Duration::from_secs(5);
/// A local session lasts until it signs out; this is what it says.
const LOCAL_SESSION_DAYS: i64 = 3650;

/// The replica's state.
pub(crate) struct Replica {
    upstream: String,
    /// The name it signs in to the server with (ADR 0067 §1).
    device: String,
    /// What it answers to, for the `Host` check (ADR 0066 §8).
    loopback: Loopback,
    cache: Arc<Mutex<Cache>>,
    blobs: BlobDir,
    /// The view, kept until the local change number moves.
    view: Mutex<Option<(u64, Arc<Objects>)>>,
    online: Arc<AtomicBool>,
    kick: Mutex<Sender<()>>,
}

impl Replica {
    /// Opens the replica's database under `data` and starts the threads
    /// that sync with the server.
    pub(crate) fn start(
        data: &Path,
        upstream: &str,
        device: &str,
        loopback: Loopback,
    ) -> anyhow::Result<Arc<Self>> {
        std::fs::create_dir_all(data)?;
        let cache = Arc::new(Mutex::new(Cache::open(&data.join("replica.db"))?));
        let (kick, kicked) = std::sync::mpsc::channel();
        let replica = Arc::new(Self {
            upstream: upstream.trim_end_matches('/').to_string(),
            device: device.to_string(),
            loopback,
            cache,
            blobs: BlobDir::new(data.join("blobs")),
            view: Mutex::new(None),
            online: Arc::new(AtomicBool::new(false)),
            kick: Mutex::new(kick),
        });
        let syncer = replica.clone();
        std::thread::Builder::new()
            .name("samu-sync".into())
            .spawn(move || syncer.sync_loop(&kicked))?;
        let listener = replica.clone();
        std::thread::Builder::new()
            .name("samu-listen".into())
            .spawn(move || listener.listen_loop())?;
        Ok(replica)
    }

    fn cache(&self) -> Result<MutexGuard<'_, Cache>, Failure> {
        lock(&self.cache)
    }

    /// Asks the sync thread to sync now.
    fn kick(&self) {
        if let Ok(kick) = self.kick.lock() {
            let _ = kick.send(());
        }
    }

    /// The client towards the server with the replica's session, if it has
    /// one.
    fn client(&self) -> Option<Client> {
        let token = self.cache.lock().ok()?.meta("token").ok()??;
        Client::new(&self.upstream, Some(token))
            .ok()
            .map(|c| c.with_timeout(TIMEOUT))
    }

    /// Syncs on every kick, else once a [`PERIOD`]; after a failure sooner,
    /// backing off.
    fn sync_loop(&self, kicked: &Receiver<()>) {
        let mut wait = Duration::ZERO;
        loop {
            match kicked.recv_timeout(wait) {
                Ok(()) | Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            // Several kicks at once are one sync.
            while kicked.try_recv().is_ok() {}
            let Some(client) = self.client() else {
                wait = PERIOD;
                continue;
            };
            let read = |hash: &str| {
                self.blobs
                    .path(hash)
                    .and_then(|path| std::fs::read(path).ok())
            };
            // After a good sync the first retry is soon again.
            let failing = |wait: Duration| {
                if self.online.load(Ordering::Relaxed) && wait == PERIOD {
                    FIRST_RETRY
                } else {
                    next_wait(wait)
                }
            };
            match samu_replica::sync(&self.cache, &client, &read) {
                Ok(_) => {
                    self.online.store(true, Ordering::Relaxed);
                    wait = PERIOD;
                }
                Err(samu_replica::Error::Offline(reason)) => {
                    wait = failing(wait);
                    if self.online.swap(false, Ordering::Relaxed) {
                        crate::calendar::say(&format!("offline: {reason}"));
                    }
                }
                Err(other) => {
                    // Reached, but refused: the error is in `last_error`.
                    wait = failing(wait);
                    self.online.store(true, Ordering::Relaxed);
                    crate::calendar::say(&format!("sync: {other}"));
                }
            }
        }
    }

    /// Listens to the server's `/events` and kicks a sync on each one
    /// (ADR 0066 §3); reconnects after a pause when the stream ends.
    fn listen_loop(&self) {
        let mut wait = FIRST_RETRY;
        loop {
            let Some(client) = self.client() else {
                // Not signed in yet: look again soon, the sign-in is near.
                std::thread::sleep(Duration::from_secs(1));
                continue;
            };
            {
                let started = std::time::Instant::now();
                let _ = client.events::<samu_client::Error>("/events", |name, _| {
                    if name == "changed" || name == "reset" {
                        self.kick();
                    }
                    Ok(true)
                });
                // A stream that stood a while was no failure.
                if started.elapsed() > PERIOD {
                    wait = FIRST_RETRY;
                }
                // The stream broke: what came meanwhile is missing.
                self.kick();
            }
            std::thread::sleep(wait);
            wait = next_wait(wait);
        }
    }

    /// The mirror with the outbox over it, read again only when the local
    /// change number moved.
    fn view(&self) -> Result<Arc<Objects>, Failure> {
        let cache = self.cache()?;
        let seq = cache.local_seq().map_err(store)?;
        let mut kept = lock(&self.view)?;
        if let Some((at, view)) = kept.as_ref()
            && *at == seq
        {
            return Ok(view.clone());
        }
        let (objects, _) = samu_replica::view(&cache).map_err(replica_error)?;
        let view = Arc::new(objects);
        *kept = Some((seq, view.clone()));
        Ok(view)
    }

    fn read(&self) -> Result<(Arc<Objects>, Settings), Failure> {
        let view = self.view()?;
        let (settings, _) = samu_replica::settings(&*self.cache()?).map_err(replica_error)?;
        Ok((view, settings))
    }

    /// Lays `changes` over the view and queues them, all or none; each
    /// with `stamp` made anew unless the client stamped it.
    fn queue(&self, changes: Vec<Change>, by_replica: bool) -> Result<u64, String> {
        let view = self.view().map_err(|e| e.message().to_string())?;
        let mut cache = self.cache.lock().map_err(|_| "poisoned".to_string())?;
        let device = cache
            .meta("device")
            .map_err(|e| e.to_string())?
            .unwrap_or_else(|| self.device.clone());
        let mut own: Option<Hlc> = cache
            .meta("hlc")
            .map_err(|e| e.to_string())?
            .and_then(|s| serde_json::from_str(&s).ok());
        let mut objects = (*view).clone();
        let mut queued = Vec::with_capacity(changes.len());
        for change in changes {
            let change = if by_replica {
                let stamp = next_stamp(&device, own.as_ref(), None, Timestamp::now());
                own = Some(stamp.clone());
                restamp(change, stamp)
            } else {
                change
            };
            overlay::apply(&mut objects, &change).map_err(|e| e.to_string())?;
            queued.push(change);
        }
        cache.push(&queued).map_err(|e| e.to_string())?;
        if let Some(own) = own {
            let text = serde_json::to_string(&own).map_err(|e| e.to_string())?;
            cache
                .set_meta("hlc", Some(&text))
                .map_err(|e| e.to_string())?;
        }
        let seq = cache.local_seq().map_err(|e| e.to_string())?;
        drop(cache);
        self.kick();
        Ok(seq)
    }

    /// How the replica stands, for `/session` (ADR 0066 §6).
    fn state(&self) -> Result<ReplicaState, Failure> {
        let cache = self.cache()?;
        Ok(ReplicaState {
            upstream: self.upstream.clone(),
            online: self.online.load(Ordering::Relaxed),
            outbox: cache.outbox_len().map_err(store)?,
            last_sync: cache
                .meta("last_sync")
                .map_err(store)?
                .and_then(|t| t.parse().ok()),
            last_error: cache.meta("last_error").map_err(store)?,
        })
    }
}

fn next_wait(wait: Duration) -> Duration {
    if wait.is_zero() {
        FIRST_RETRY
    } else {
        (wait * 2).min(PERIOD)
    }
}

fn store(error: impl std::fmt::Display) -> Failure {
    Failure::internal(error.to_string())
}

fn replica_error(error: samu_replica::Error) -> Failure {
    Failure::internal(error.to_string())
}

/// What only the server can do (ADR 0066 §5).
pub(crate) fn offline(what: &str) -> Failure {
    Failure::status(
        StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::Offline,
        format!("only with a connection to the server: {what}"),
    )
}

/// A local session (ADR 0067 §3).
#[derive(Debug, Clone)]
struct LocalSession {
    digest: String,
    device: String,
}

/// What is kept of a local session, under `session:<digest>` in meta.
#[derive(Debug, serde::Serialize, Deserialize)]
struct Kept {
    account: String,
    device: String,
}

/// What a service on the loopback answers to (ADR 0066 §8, ADR 0087 §3):
/// its port, and the name of its profile in the browser if it runs one
/// (`Profile::host`).
#[derive(Debug, Clone)]
pub(crate) struct Loopback {
    pub(crate) port: u16,
    pub(crate) name: Option<String>,
}

impl Loopback {
    /// Whether a `Host` header names this service: `127.0.0.1` or
    /// `localhost` with its port, or `<profile>.samu.localhost` with any
    /// port or none. A name under `.localhost` cannot point elsewhere, so
    /// it guards against DNS rebinding as well as `127.0.0.1` does.
    pub(crate) fn allows(&self, host: &str) -> bool {
        let port = self.port.to_string();
        let by_port = ["127.0.0.1", "localhost"]
            .iter()
            .any(|name| host.strip_prefix(name).and_then(|r| r.strip_prefix(':')) == Some(&port));
        by_port
            || self.name.as_deref().is_some_and(|wanted| {
                let name = host.rsplit_once(':').map_or(host, |(name, port)| {
                    if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) {
                        name
                    } else {
                        host
                    }
                });
                name.eq_ignore_ascii_case(wanted)
            })
    }

    fn names(&self) -> String {
        let port = self.port;
        match &self.name {
            Some(name) => format!("127.0.0.1:{port}, localhost:{port} and {name}"),
            None => format!("127.0.0.1:{port} and localhost:{port}"),
        }
    }
}

/// Lets a request through only to a name of this service, against DNS
/// rebinding from a page in the same browser (ADR 0066 §8). The local
/// server of a profile does the same (ADR 0086 §3); both answer to the
/// profile's name too (ADR 0087 §3).
pub(crate) async fn loopback_host(
    State(loopback): State<Loopback>,
    request: Request,
    next: Next,
) -> Result<Response, Failure> {
    let host = request
        .headers()
        .get(HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !loopback.allows(host) {
        return Err(Failure::status(
            StatusCode::MISDIRECTED_REQUEST,
            ErrorCode::Forbidden,
            format!("this service answers only {}", loopback.names()),
        ));
    }
    Ok(next.run(request).await)
}

/// Lets a request through only with a local session of the account the
/// replica mirrors.
async fn require_local_session(
    State(replica): State<Arc<Replica>>,
    mut request: Request,
    next: Next,
) -> Result<Response, Failure> {
    let token =
        token_of(request.headers()).ok_or_else(|| Failure::unauthorized("not signed in"))?;
    let session = replica.local_session(&token)?;
    request.extensions_mut().insert(session);
    Ok(next.run(request).await)
}

impl Replica {
    /// The local session of a token, if it belongs to the account the
    /// replica mirrors.
    fn local_session(&self, token: &str) -> Result<LocalSession, Failure> {
        let digest = token_digest(token);
        let cache = self.cache()?;
        let kept: Option<Kept> = cache
            .meta(&format!("session:{digest}"))
            .map_err(store)?
            .and_then(|text| serde_json::from_str::<Kept>(&text).ok());
        let account = cache.meta("account").map_err(store)?;
        let kept = kept
            .filter(|k| Some(&k.account) == account.as_ref())
            .ok_or_else(|| Failure::unauthorized("the session is unknown or has ended"))?;
        Ok(LocalSession {
            digest,
            device: kept.device,
        })
    }
}

/// The cookie of a local session. Not `Secure`: the replica speaks plain
/// HTTP on the loopback, and only there.
fn cookie(token: &str, max_age: i64) -> Option<HeaderValue> {
    HeaderValue::from_str(&format!(
        "{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}"
    ))
    .ok()
}

/// `POST /login`: through to the server with the replica's device
/// (ADR 0067).
async fn login(
    State(replica): State<Arc<Replica>>,
    Json(login): Json<Login>,
) -> Result<Response, Failure> {
    if login.kind == Some(AuthorKind::Agent) {
        return Err(Failure::status(
            StatusCode::FORBIDDEN,
            ErrorCode::Forbidden,
            "an agent signs in at the server itself, not at the replica (ADR 0067 §5)",
        ));
    }
    if login.only.is_some() {
        return Err(Failure::status(
            StatusCode::BAD_REQUEST,
            ErrorCode::BadRequest,
            "the replica mirrors a whole account; a restricted session signs in at the server (ADR 0095 §6)",
        ));
    }
    let local_device = match login.device.trim() {
        "" => replica.device.clone(),
        name => name.to_string(),
    };
    let worker = replica.clone();
    let session = tokio::task::spawn_blocking(move || worker.sign_in_upstream(&login))
        .await
        .map_err(|e| Failure::internal(e.to_string()))??;
    let token = new_token()?;
    let kept = Kept {
        account: session.account.clone(),
        device: local_device.clone(),
    };
    {
        let cache = replica.cache()?;
        let text = serde_json::to_string(&kept).map_err(store)?;
        cache
            .set_meta(&format!("session:{}", token_digest(&token)), Some(&text))
            .map_err(store)?;
    }
    replica.kick();
    let mut info = session_info(&replica, &session.account, &local_device)?;
    info.token = Some(token.clone());
    let mut response = Json(info).into_response();
    if let Some(value) = cookie(&token, LOCAL_SESSION_DAYS * 24 * 3600) {
        response.headers_mut().insert(SET_COOKIE, value);
    }
    Ok(response)
}

impl Replica {
    /// Signs in at the server and keeps its session in place of the old
    /// one; another account than before forgets the mirror (ADR 0067 §2).
    fn sign_in_upstream(&self, login: &Login) -> Result<SessionInfo, Failure> {
        let client = Client::new(&self.upstream, None)
            .map_err(|e| Failure::internal(e.to_string()))?
            .with_timeout(TIMEOUT);
        let body = serde_json::json!({
            "account": login.account,
            "password": login.password,
            "device": self.device,
        });
        let answer = client
            .call("POST", "/login", Some(&body))
            .map_err(|e| upstream_failure(&e, "signing in"))?;
        self.online.store(true, Ordering::Relaxed);
        let session: SessionInfo = serde_json::from_value(
            answer
                .json()
                .map_err(|e| Failure::internal(e.to_string()))?,
        )
        .map_err(|e| Failure::internal(e.to_string()))?;
        let token = session
            .token
            .clone()
            .ok_or_else(|| Failure::internal("the server sent no token"))?;
        let mut cache = self.cache()?;
        let same = cache.meta("server").map_err(store)?.as_deref() == Some(self.upstream.as_str())
            && cache.meta("account").map_err(store)?.as_deref() == Some(session.account.as_str());
        let old = cache.meta("token").map_err(store)?;
        if !same {
            cache.clear(true).map_err(store)?;
            *lock(&self.view)? = None;
        }
        cache
            .set_meta("server", Some(&self.upstream))
            .map_err(store)?;
        cache
            .set_meta("account", Some(&session.account))
            .map_err(store)?;
        cache
            .set_meta("device", Some(&session.device))
            .map_err(store)?;
        cache.set_meta("token", Some(&token)).map_err(store)?;
        samu_replica::remember_session(&cache, &session).map_err(replica_error)?;
        drop(cache);
        if let Some(old) = old.filter(|old| *old != token)
            && let Ok(client) = Client::new(&self.upstream, Some(old))
        {
            let _ = client.with_timeout(TIMEOUT).call("POST", "/logout", None);
        }
        Ok(session)
    }
}

/// Passes a request on to the server with the replica's own session and
/// gives back its answer as it is (ADR 0096 §1). Away, or with a session
/// the server no longer knows, it is `offline` (§3).
fn relay(
    replica: &Replica,
    method: &str,
    path: &str,
    content_type: &str,
    body: &[u8],
) -> Result<Response, Failure> {
    let what = format!("{method} {path}");
    let Some(client) = replica.client() else {
        return Err(offline(&what));
    };
    let answer = match client.exchange(method, path, content_type, body) {
        Ok(answer) => answer,
        Err(error) => return Err(upstream_failure(&error, &what)),
    };
    replica.online.store(true, Ordering::Relaxed);
    if answer.status == StatusCode::UNAUTHORIZED.as_u16() {
        return Err(offline(&format!(
            "{what}; the server no longer knows this computer, sign in again"
        )));
    }
    let status = StatusCode::from_u16(answer.status).unwrap_or(StatusCode::BAD_GATEWAY);
    let content_type = answer
        .content_type
        .as_deref()
        .and_then(|value| HeaderValue::from_str(value).ok());
    let mut response = (status, answer.body).into_response();
    if let Some(value) = content_type {
        response
            .headers_mut()
            .insert(axum::http::header::CONTENT_TYPE, value);
    } else {
        response
            .headers_mut()
            .remove(axum::http::header::CONTENT_TYPE);
    }
    Ok(response)
}

/// The largest body the replica passes on: an iCalendar file to import
/// (ADR 0100 §4) with room for its request around it.
const RELAY_MAX_BYTES: usize = 11 * 1024 * 1024;

/// A route the replica passes on as it came, body and `Content-Type`
/// included (ADR 0100 §9); the router has matched it already.
async fn relay_as_is(
    State(replica): State<Arc<Replica>>,
    method: axum::http::Method,
    uri: axum::extract::OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, Failure> {
    let path = uri
        .0
        .path_and_query()
        .map_or_else(|| uri.0.path().to_string(), ToString::to_string);
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/json")
        .to_string();
    tokio::task::spawn_blocking(move || {
        relay(&replica, method.as_str(), &path, &content_type, &body)
    })
    .await
    .map_err(|e| Failure::internal(e.to_string()))?
}

/// `GET /sessions` through the server (ADR 0096 §2).
async fn sessions(State(replica): State<Arc<Replica>>) -> Result<Response, Failure> {
    tokio::task::spawn_blocking(move || {
        relay(&replica, "GET", "/sessions", "application/json", &[])
    })
    .await
    .map_err(|e| Failure::internal(e.to_string()))?
}

/// `POST /sessions/{id}/end` through the server; never the replica's own
/// session (ADR 0096 §6).
async fn end_session(
    State(replica): State<Arc<Replica>>,
    UrlPath(id): UrlPath<String>,
) -> Result<Response, Failure> {
    // Checked here as on the server, so nothing else goes into the path.
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Failure::bad_request(format!(
            "{id:?} is not a session id (hex)"
        )));
    }
    let own = replica.cache()?.meta("token").map_err(store)?;
    if own.is_some_and(|token| token_digest(&token).starts_with(&id.to_ascii_lowercase())) {
        return Err(crate::grant::forbidden(
            "this is the replica's own session at the server; sign out here instead (ADR 0096 §6)",
        ));
    }
    let path = format!("/sessions/{id}/end");
    tokio::task::spawn_blocking(move || relay(&replica, "POST", &path, "application/json", &[]))
        .await
        .map_err(|e| Failure::internal(e.to_string()))?
}

/// A failure from the server, as the replica answers it.
fn upstream_failure(error: &samu_client::Error, what: &str) -> Failure {
    use samu_client::Error as E;
    match error {
        E::NoServer(_) | E::Io(_) => offline(what),
        E::Unauthorized(message) => Failure::unauthorized(message.clone()),
        E::NotFound(message) => Failure::not_found(message.clone()),
        other => Failure::bad_request(other.to_string()),
    }
}

fn session_info(replica: &Replica, account: &str, device: &str) -> Result<SessionInfo, Failure> {
    let (settings, known) = samu_replica::settings(&*replica.cache()?).map_err(replica_error)?;
    let expires = Timestamp::now()
        .checked_add(SignedDuration::from_hours(LOCAL_SESSION_DAYS * 24))
        .unwrap_or(Timestamp::MAX);
    Ok(SessionInfo {
        account: account.to_string(),
        device: device.to_string(),
        kind: Some(AuthorKind::Human),
        expires,
        token: None,
        zone: known
            .then(|| settings.zone.iana_name().map(str::to_string))
            .flatten(),
        due_soon: Some(Period(settings.due_soon)),
        remind_time: Some(settings.remind_time),
        replica: Some(replica.state()?),
        grant: None,
        demo_until: None,
    })
}

async fn logout(
    State(replica): State<Arc<Replica>>,
    Extension(session): Extension<LocalSession>,
) -> Result<Response, Failure> {
    replica
        .cache()?
        .set_meta(&format!("session:{}", session.digest), None)
        .map_err(store)?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    if let Some(value) = cookie("", 0) {
        response.headers_mut().insert(SET_COOKIE, value);
    }
    Ok(response)
}

async fn session(
    State(replica): State<Arc<Replica>>,
    Extension(session): Extension<LocalSession>,
) -> Result<Json<SessionInfo>, Failure> {
    let account = replica
        .cache()?
        .meta("account")
        .map_err(store)?
        .unwrap_or_default();
    Ok(Json(session_info(&replica, &account, &session.device)?))
}

#[derive(Debug, Deserialize)]
struct ChangesQuery {
    stamp: Option<String>,
}

/// `POST /changes`: into the outbox, each laid over the view first
/// (ADR 0066 §2). What does not lay over is refused here and now.
async fn post_changes(
    State(replica): State<Arc<Replica>>,
    Query(query): Query<ChangesQuery>,
    Json(changes): Json<Vec<Change>>,
) -> Result<Json<Vec<ChangeOutcome>>, Failure> {
    let by_replica = match query.stamp.as_deref() {
        None => false,
        Some("server") => true,
        Some(other) => return Err(Failure::bad_request(format!("stamp={other} is not known"))),
    };
    let mut outcomes = Vec::with_capacity(changes.len());
    for change in changes {
        outcomes.push(match replica.queue(vec![change], by_replica) {
            Ok(seq) => ChangeOutcome::Applied { seq },
            Err(reason) => ChangeOutcome::Rejected { reason },
        });
    }
    Ok(Json(outcomes))
}

#[derive(Debug, Deserialize)]
struct AfterQuery {
    #[serde(default)]
    after: u64,
}

/// `GET /changes?after=N`: what changed in the view after the replica's own
/// change number N (ADR 0066 §3).
async fn get_changes(
    State(replica): State<Arc<Replica>>,
    Query(query): Query<AfterQuery>,
) -> Result<Json<ChangesPage>, Failure> {
    let view = replica.view()?;
    let cache = replica.cache()?;
    let seq = cache.local_seq().map_err(store)?;
    let mut page = ChangesPage {
        seq,
        ..ChangesPage::default()
    };
    if query.after == 0 {
        page.objects = view.all();
        return Ok(Json(page));
    }
    for id in cache.local_after(query.after).map_err(store)? {
        match view.get(id) {
            Some(object) => page.objects.push(object),
            None => page.gone.push(id),
        }
    }
    Ok(Json(page))
}

fn parse_id(id: &str) -> Result<Id, Failure> {
    id.parse()
        .map_err(|_| Failure::bad_request(format!("{id:?} is not an id")))
}

async fn get_object(
    State(replica): State<Arc<Replica>>,
    UrlPath(id): UrlPath<String>,
) -> Result<Json<AnyObject>, Failure> {
    let id = parse_id(&id)?;
    replica
        .view()?
        .get(id)
        .map(Json)
        .ok_or_else(|| Failure::not_found(format!("no object {id}")))
}

/// `POST /objects/{id}/delete`: planned in samu-core like on the server,
/// into the outbox.
async fn delete_object(
    State(replica): State<Arc<Replica>>,
    UrlPath(id): UrlPath<String>,
    Json(request): Json<DeleteRequest>,
) -> Result<Json<DeleteResponse>, Failure> {
    let id = parse_id(&id)?;
    let view = replica.view()?;
    if view.get(id).is_none() {
        return Err(Failure::not_found(format!("no object {id}")));
    }
    let plan = samu_store::core::delete::plan(&view.snapshot(), &view.scopes, id, request.tasks)
        .map_err(|e| Failure::bad_request(e.to_string()))?;
    let mut answer = DeleteResponse {
        deleted: plan.deleted,
        moved: plan.moved,
        untagged: plan.untagged,
        seq: None,
    };
    if request.preview {
        return Ok(Json(answer));
    }
    let seq = replica
        .queue(plan.changes, true)
        .map_err(|reason| Failure::internal(format!("delete not queued: {reason}")))?;
    answer.seq = Some(seq);
    Ok(Json(answer))
}

/// `POST /objects/{id}/to-project` from the mirror, queued as one batch
/// (ADR 0126).
async fn to_project(
    State(replica): State<Arc<Replica>>,
    UrlPath(id): UrlPath<String>,
    Json(request): Json<ToProjectRequest>,
) -> Result<Json<ToProjectResponse>, Failure> {
    let id = parse_id(&id)?;
    let view = replica.view()?;
    if view.get(id).is_none() {
        return Err(Failure::not_found(format!("no object {id}")));
    }
    let plan = samu_store::core::convert::plan(
        &view.snapshot(),
        id,
        Id::new(),
        &request,
        Timestamp::now(),
    )
    .map_err(|e| Failure::bad_request(e.to_string()))?;
    let mut answer = ToProjectResponse {
        project: plan.project,
        moved: plan.moved,
        seq: None,
    };
    if request.preview {
        return Ok(Json(answer));
    }
    let seq = replica
        .queue(plan.changes, true)
        .map_err(|reason| Failure::internal(format!("project not queued: {reason}")))?;
    answer.seq = Some(seq);
    Ok(Json(answer))
}

#[derive(Debug, Deserialize)]
struct AvailableQuery {
    #[serde(default)]
    available: bool,
    #[serde(default)]
    today: bool,
    project: Option<Id>,
    #[serde(default)]
    all: bool,
    scope: Option<Id>,
    perspective: Option<Id>,
    /// Also what another own scope hides, marked (ADR 0124 §2).
    #[serde(default)]
    elsewhere: bool,
    #[serde(default)]
    bare: bool,
}

async fn available(
    State(replica): State<Arc<Replica>>,
    Query(query): Query<AvailableQuery>,
) -> Result<Response, Failure> {
    let (objects, settings) = replica.read()?;
    let filter = views::AvailableFilter {
        available: query.available,
        today: query.today,
        project: query.project,
        all: query.all,
        scope: query.scope,
        perspective: query.perspective,
        elsewhere: query.elsewhere,
    };
    let statuses = views::available(&objects, &settings, Timestamp::now(), &filter)?;
    if query.bare {
        let bare: Vec<BareStatus> = statuses.into_iter().map(BareStatus::from).collect();
        return Ok(Json(bare).into_response());
    }
    Ok(Json(statuses).into_response())
}

/// `GET /blocks` from the mirror (ADR 0116 §4).
async fn blocks(
    State(replica): State<Arc<Replica>>,
    Query(query): Query<crate::app::BlocksQuery>,
) -> Result<Response, Failure> {
    let (objects, settings) = replica.read()?;
    let sections = views::blocks(
        &objects,
        &settings,
        Timestamp::now(),
        query.scope,
        query.perspective,
    )?;
    Ok(Json(sections).into_response())
}

/// `GET /habits` from the mirror (ADR 0091 §4).
async fn habits(
    State(replica): State<Arc<Replica>>,
    Query(query): Query<crate::app::HabitsQuery>,
) -> Result<Response, Failure> {
    let (objects, settings) = replica.read()?;
    let habits = views::habits(
        &objects,
        &settings,
        Timestamp::now(),
        query.scope,
        query.from,
    )?;
    Ok(Json(habits).into_response())
}

/// `GET /habits/{id}` from the mirror (ADR 0091 §4).
async fn habit(
    State(replica): State<Arc<Replica>>,
    UrlPath(id): UrlPath<String>,
    Query(query): Query<crate::app::HabitsQuery>,
) -> Result<Response, Failure> {
    let id = parse_id(&id)?;
    let (objects, settings) = replica.read()?;
    let habit = views::habit(&objects, &settings, Timestamp::now(), id, query.from)?;
    Ok(Json(habit).into_response())
}

#[derive(Debug, Deserialize)]
struct ProjectsQuery {
    #[serde(default)]
    stuck: bool,
    /// Only projects due for review (ADR 0088 §4).
    #[serde(default)]
    review: bool,
    #[serde(default)]
    all: bool,
    scope: Option<Id>,
    /// Also what another own scope hides, marked (ADR 0124 §2).
    #[serde(default)]
    elsewhere: bool,
}

async fn available_projects(
    State(replica): State<Arc<Replica>>,
    Query(query): Query<ProjectsQuery>,
) -> Result<Response, Failure> {
    let (objects, settings) = replica.read()?;
    let filter = views::ProjectsFilter {
        stuck: query.stuck,
        review: query.review,
        all: query.all,
        scope: query.scope,
        elsewhere: query.elsewhere,
    };
    Ok(Json(views::projects(
        &objects,
        &settings,
        Timestamp::now(),
        &filter,
    )?)
    .into_response())
}

#[derive(Debug, Deserialize)]
struct ScopeQuery {
    scope: Option<Id>,
}

async fn available_folders(
    State(replica): State<Arc<Replica>>,
    Query(query): Query<ScopeQuery>,
) -> Result<Response, Failure> {
    let (objects, settings) = replica.read()?;
    Ok(Json(views::folders(
        &objects,
        &settings,
        Timestamp::now(),
        query.scope,
    )?)
    .into_response())
}

async fn scheduled_scope(State(replica): State<Arc<Replica>>) -> Result<Response, Failure> {
    let (objects, settings) = replica.read()?;
    Ok(Json(views::scheduled(&objects, &settings, Timestamp::now())).into_response())
}

async fn forecast(
    State(replica): State<Arc<Replica>>,
    Query(query): Query<ScopeQuery>,
) -> Result<Response, Failure> {
    let (objects, settings) = replica.read()?;
    // Appointments come with the mirror (ADR 0066, Für die Planung).
    Ok(Json(views::forecast(
        &objects,
        &objects.appointments,
        &settings,
        Timestamp::now(),
        query.scope,
    )?)
    .into_response())
}

/// `GET /alarms` from the mirror and samu-core (ADR 0101 §5): Local.
async fn alerts(
    State(replica): State<Arc<Replica>>,
    Query(query): Query<crate::app::AlertsQuery>,
) -> Result<Response, Failure> {
    let (objects, settings) = replica.read()?;
    Ok(Json(views::alerts(
        &objects,
        &objects.appointments,
        &settings,
        Timestamp::now(),
        query.view(),
    )?)
    .into_response())
}

#[derive(Debug, Deserialize)]
struct ClockReportQuery {
    from: Option<jiff::civil::Date>,
    to: Option<jiff::civil::Date>,
    #[serde(default)]
    by: samu_store::core::api::ClockReportBy,
    scope: Option<Id>,
}

async fn clock_report(
    State(replica): State<Arc<Replica>>,
    Query(query): Query<ClockReportQuery>,
) -> Result<Response, Failure> {
    let (objects, settings) = replica.read()?;
    let query = views::ClockReportQuery {
        from: query.from,
        to: query.to,
        by: query.by,
        scope: query.scope,
    };
    Ok(Json(views::clock_report(
        &objects,
        &settings,
        Timestamp::now(),
        &query,
    )?)
    .into_response())
}

#[derive(Debug, Deserialize)]
struct ReportQuery {
    from: Option<jiff::civil::Date>,
    to: Option<jiff::civil::Date>,
    #[serde(default)]
    by: samu_store::core::api::ClockReportBy,
    scope: Option<Id>,
    #[serde(default)]
    q: String,
    parts: Option<String>,
    #[serde(default)]
    dropped: bool,
}

/// `GET /report` from the mirror (ADR 0080 §4): done and time; the mirror
/// has no logbook, so `worked` asked for by name is 503 `offline`, and
/// without `parts` it is left out. Who completed what is not known here:
/// every completion counts as a person's.
async fn report(
    State(replica): State<Arc<Replica>>,
    Query(query): Query<ReportQuery>,
) -> Result<Response, Failure> {
    use samu_store::core::api::ReportPart;
    let mut parts = views::report_parts(query.parts.as_deref())?;
    if parts.contains(&ReportPart::Worked) {
        return Err(offline("what was worked on needs the logbook"));
    }
    if parts.is_empty() {
        parts = vec![ReportPart::Done, ReportPart::Time];
    }
    let (objects, settings) = replica.read()?;
    let query = views::ReportQuery {
        from: query.from,
        to: query.to,
        by: query.by,
        scope: query.scope,
        q: query.q,
        parts,
        dropped: query.dropped,
    };
    let agents = std::collections::HashSet::new();
    let account = replica
        .cache()?
        .meta("account")
        .map_err(store)?
        .unwrap_or_default();
    let events = samu_store::core::report::Events {
        log: &[],
        comments: &objects.comments,
        stamps: &[],
        agent_devices: &agents,
        account: &account,
    };
    Ok(Json(views::report(
        &objects,
        &settings,
        Timestamp::now(),
        &query,
        &events,
    )?)
    .into_response())
}

async fn render(
    State(replica): State<Arc<Replica>>,
    Json(request): Json<RenderRequest>,
) -> Result<Json<RenderResponse>, Failure> {
    let mut documents: Vec<_> = request
        .texts
        .iter()
        .map(|text| samu_store::core::render::render_text(&text.text, text.format))
        .collect();
    if documents.iter().any(|d| views::has_task_link(&d.blocks)) {
        let view = replica.view()?;
        views::resolve_task_links(&mut documents, &view);
    }
    Ok(Json(RenderResponse { documents }))
}

/// `POST /capture`: as on the server, into the outbox, stamped by the
/// replica.
async fn capture(
    State(replica): State<Arc<Replica>>,
    Json(request): Json<CaptureRequest>,
) -> Result<Json<CaptureResponse>, Failure> {
    let (objects, settings) = replica.read()?;
    let placeholder = Hlc {
        wall: 0,
        counter: 0,
        device: replica.device.clone(),
    };
    let views::Capture {
        mut answer,
        changes,
    } = views::capture(
        &objects,
        &settings,
        &request,
        Timestamp::now(),
        &placeholder,
        &mut Id::new,
    )?;
    if request.preview {
        return Ok(Json(answer));
    }
    let id = answer.task.id;
    replica
        .queue(changes, true)
        .map_err(|reason| Failure::internal(format!("capture not queued: {reason}")))?;
    if let Some(AnyObject::Task(task)) = replica.view()?.get(id) {
        answer.task = task;
    }
    answer.created = request.task.is_none();
    Ok(Json(answer))
}

/// `POST /appointments/{id}/edit`: the same changes the server would make,
/// computed over the view and queued (ADR 0068 §5, ADR 0066).
async fn edit_appointment(
    State(replica): State<Arc<Replica>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    Json(edit): Json<samu_store::core::api::AppointmentEdit>,
) -> Result<Json<samu_store::core::api::AppointmentEditResponse>, Failure> {
    let id: Id = id
        .parse()
        .map_err(|_| Failure::bad_request(format!("{id:?} is not an id")))?;
    let (objects, settings) = replica.read()?;
    let placeholder = Hlc {
        wall: 0,
        counter: 0,
        device: replica.device.clone(),
    };
    let changes: Vec<Change> = samu_store::core::calendar::edit::edit_appointment(
        &objects.appointments,
        &objects.calendars,
        id,
        &edit,
        &settings.zone,
        Timestamp::now(),
        Id::new,
    )
    .map_err(|e| crate::app::edit_failure(&e))?
    .into_iter()
    .map(|draft| draft.stamped(placeholder.clone()))
    .collect();
    let seq = replica
        .queue(changes.clone(), true)
        .map_err(|reason| Failure::bad_request(format!("not queued: {reason}")))?;
    Ok(Json(samu_store::core::api::AppointmentEditResponse {
        results: changes
            .into_iter()
            .map(|change| samu_store::core::api::EditResult {
                change,
                outcome: ChangeOutcome::Applied { seq },
            })
            .collect(),
    }))
}

#[derive(Debug, Deserialize)]
struct AppointmentsQuery {
    from: Option<jiff::civil::Date>,
    to: Option<jiff::civil::Date>,
    scope: Option<Id>,
}

/// `GET /appointments`: the instances of the mirrored appointments
/// (ADR 0047 §10), like the server computes them.
async fn appointments(
    State(replica): State<Arc<Replica>>,
    Query(query): Query<AppointmentsQuery>,
) -> Result<Response, Failure> {
    let (objects, settings) = replica.read()?;
    Ok(Json(views::appointments(
        &objects,
        &objects.appointments,
        &settings,
        Timestamp::now(),
        (query.from, query.to),
        query.scope,
    )?)
    .into_response())
}

/// `PUT /blobs/{sha256}`: kept here and queued to go up before the outbox
/// (ADR 0039 §6), like the phone does.
async fn put_blob(
    State(replica): State<Arc<Replica>>,
    UrlPath(hash): UrlPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Blob>, Failure> {
    crate::blobs::checked_hash(&hash)?;
    let media_type = crate::blobs::media_type(&headers)?;
    let dir = replica.blobs.clone();
    let expected = hash.clone();
    let written = tokio::task::spawn_blocking(move || dir.put(&expected, body.as_ref()))
        .await
        .map_err(|e| Failure::internal(format!("writing the blob: {e}")))??
        .map_err(|refused| crate::blobs::refused_upload(&refused))?;
    replica
        .cache()?
        .push_blob(&written.hash, &media_type, written.size)
        .map_err(store)?;
    replica.kick();
    Ok(Json(Blob {
        hash: written.hash,
        size: written.size,
        media_type,
    }))
}

/// `GET /blobs/{sha256}`: from the replica's files, else fetched from the
/// server once and kept; offline and not here, 503.
async fn get_blob(
    State(replica): State<Arc<Replica>>,
    UrlPath(hash): UrlPath<String>,
) -> Result<Response, Failure> {
    crate::blobs::checked_hash(&hash)?;
    let media_type = {
        let view = replica.view()?;
        let waiting = replica.cache()?.blob_outbox().map_err(store)?;
        view.attachments
            .iter()
            .find(|a| a.blob == hash)
            .map(|a| a.media_type.clone())
            .or_else(|| {
                waiting
                    .into_iter()
                    .find(|(h, _, _)| *h == hash)
                    .map(|(_, t, _)| t)
            })
            .unwrap_or_else(|| "application/octet-stream".into())
    };
    let path = replica
        .blobs
        .path(&hash)
        .ok_or_else(|| Failure::bad_request("not a hash"))?;
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(_) => {
            let worker = replica.clone();
            let wanted = hash.clone();
            tokio::task::spawn_blocking(move || worker.fetch_blob(&wanted))
                .await
                .map_err(|e| Failure::internal(e.to_string()))??
        }
    };
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let mut response = bytes.into_response();
    crate::blobs::file_headers(response.headers_mut(), &media_type, size);
    Ok(response)
}

impl Replica {
    /// A file from the server, kept under its hash.
    fn fetch_blob(&self, hash: &str) -> Result<Vec<u8>, Failure> {
        let client = self.client().ok_or_else(|| offline("a file"))?;
        let answer = client
            .call_bytes(
                "GET",
                &format!("/blobs/{hash}"),
                "application/json",
                &[],
                &[],
            )
            .map_err(|e| upstream_failure(&e, "a file not yet on this computer"))?;
        if let Ok(Err(refused)) = self.blobs.put(hash, answer.body.as_slice()) {
            return Err(Failure::internal(format!(
                "a file from the server: {refused}"
            )));
        }
        Ok(answer.body)
    }
}

/// More objects than this in one notice become a reset, as on the server.
const NOTICE_LIMIT: usize = 500;

/// A notice of what changed in the view after `after`, or of the
/// replica's state alone (ADR 0066 §6): the web UI reads `/session` again
/// on every notice.
fn notice(replica: &Replica, after: u64) -> Option<Event> {
    let cache = replica.cache().ok()?;
    let seq = cache.local_seq().ok()?;
    let ids = cache.local_after(after).ok()?;
    drop(cache);
    if ids.len() > NOTICE_LIMIT {
        return Some(reset_event(seq));
    }
    let view = replica.view().ok()?;
    let mut notice = ChangeNotice {
        seq,
        ..ChangeNotice::default()
    };
    for id in ids {
        if view.get(id).is_some() {
            notice.changed.push(id);
        } else {
            notice.gone.push(id);
        }
    }
    let data = serde_json::to_string(&notice).ok()?;
    Some(
        Event::default()
            .event("changed")
            .id(seq.to_string())
            .data(data),
    )
}

fn reset_event(seq: u64) -> Event {
    let data = serde_json::to_string(&ResetNotice { seq }).unwrap_or_default();
    Event::default()
        .event("reset")
        .id(seq.to_string())
        .data(data)
}

/// What a notice of the state alone compares: online, the outbox, the
/// last error.
fn fingerprint(replica: &Replica) -> Option<(bool, u32, Option<String>)> {
    let state = replica.state().ok()?;
    Some((state.online, state.outbox, state.last_error))
}

/// `GET /events`: like the server's, over the replica's own change number,
/// and a notice when only its state moved.
async fn events(
    State(replica): State<Arc<Replica>>,
    headers: HeaderMap,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, Failure> {
    let current = replica.cache()?.local_seq().map_err(store)?;
    let last: Option<u64> = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    let first = Some(last.filter(|last| *last < current).map_or_else(
        || Event::default().comment("connected"),
        |_| reset_event(current),
    ));
    let seen_state = fingerprint(&replica);
    let stream = futures_util::stream::unfold(
        (replica, current, seen_state, first),
        |(replica, mut seen, mut state, mut pending)| async move {
            loop {
                if let Some(event) = pending.take() {
                    return Some((Ok(event), (replica, seen, state, None)));
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
                let now = replica.cache().ok()?.local_seq().ok()?;
                let now_state = fingerprint(&replica);
                if now > seen || now_state != state {
                    pending = notice(&replica, seen);
                    seen = now;
                    state = now_state;
                }
            }
        },
    );
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// The replica's API: every route of `samu_api::ROUTES`, local or 503
/// `offline` as [`locality`] says (ADR 0066 §5), behind the `Host` check
/// (§8).
pub(crate) fn router(replica: Arc<Replica>) -> Router {
    let signed_in = Router::new()
        .route("/logout", post(logout))
        .route("/session", get(session))
        .route("/changes", post(post_changes).get(get_changes))
        .route("/objects/{id}", get(get_object))
        .route("/objects/{id}/delete", post(delete_object))
        .route("/objects/{id}/to-project", post(to_project))
        .route("/available", get(available))
        .route("/blocks", get(blocks))
        .route("/habits", get(habits))
        .route("/habits/{id}", get(habit))
        .route("/available/projects", get(available_projects))
        .route("/available/folders", get(available_folders))
        .route("/scopes/scheduled", get(scheduled_scope))
        .route("/forecast", get(forecast))
        .route("/alarms", get(alerts))
        .route("/clocks/report", get(clock_report))
        .route("/report", get(report))
        .route("/capture", post(capture))
        .route("/render", post(render))
        .route("/appointments", get(appointments))
        .route("/appointments/{id}/edit", post(edit_appointment))
        .route("/events", get(events))
        .route("/sessions", get(sessions))
        .route("/sessions/{id}/end", post(end_session))
        .route(
            "/calendars/{id}/source",
            axum::routing::put(relay_as_is)
                .layer(axum::extract::DefaultBodyLimit::max(RELAY_MAX_BYTES)),
        )
        .route(
            "/calendars/{id}/refresh",
            post(relay_as_is).layer(axum::extract::DefaultBodyLimit::max(RELAY_MAX_BYTES)),
        )
        .route(
            "/calendars/discover",
            post(relay_as_is).layer(axum::extract::DefaultBodyLimit::max(RELAY_MAX_BYTES)),
        )
        .route(
            "/calendars/{id}/discover",
            post(relay_as_is).layer(axum::extract::DefaultBodyLimit::max(RELAY_MAX_BYTES)),
        )
        .route(
            "/calendars/{id}/import",
            post(relay_as_is).layer(axum::extract::DefaultBodyLimit::max(RELAY_MAX_BYTES)),
        )
        .route(
            "/blobs/{sha256}",
            get(get_blob)
                .put(put_blob)
                .layer(axum::extract::DefaultBodyLimit::max(
                    usize::try_from(samu_store::MAX_BLOB_BYTES).unwrap_or(usize::MAX),
                )),
        )
        .route_layer(middleware::from_fn_with_state(
            replica.clone(),
            require_local_session,
        ));
    let mut router = Router::new()
        .route("/", get(crate::web::index))
        .route("/o/{id}", get(crate::web::object_page))
        .route("/{name}", get(crate::web::root_file))
        .route("/lib/{name}", get(crate::web::lib_file))
        .route("/health", get(crate::app::health))
        .route("/login", post(login));
    // What only the server can do answers so without a session too: it
    // says nothing about the account.
    let mut online: Vec<(&str, axum::routing::MethodRouter<Arc<Replica>>)> = Vec::new();
    for route in samu_store::core::api::ROUTES {
        if locality(route) != Some(Where::Online) {
            continue;
        }
        let Ok(method) = axum::http::Method::from_bytes(route.method.as_bytes()) else {
            continue;
        };
        let Ok(filter) = axum::routing::MethodFilter::try_from(method) else {
            continue;
        };
        let what = format!("{} {}", route.method, route.path);
        let handler = axum::routing::on(filter, move || async move { offline(&what) });
        match online.iter_mut().find(|(path, _)| *path == route.path) {
            Some((_, methods)) => *methods = methods.clone().merge(handler),
            None => online.push((route.path, handler)),
        }
    }
    for (path, methods) in online {
        router = router.route(path, methods);
    }
    router
        .merge(signed_in)
        .layer(middleware::from_fn_with_state(
            replica.loopback.clone(),
            loopback_host,
        ))
        .with_state(replica)
        .layer(tower_http::compression::CompressionLayer::new().compress_when(crate::app::Text))
}

/// The name a computer signs in with: its host name, else `computer`.
pub(crate) fn default_device() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "computer".into())
}

/// `$XDG_DATA_HOME/samu`, else `~/.local/share/samu`.
pub(crate) fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("samu")
}

/// Where the replica keeps its data without `--data`:
/// `$XDG_DATA_HOME/samu/replica`, else `~/.local/share/samu/replica`.
pub(crate) fn default_data() -> PathBuf {
    data_home().join("replica")
}
