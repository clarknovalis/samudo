// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The HTTP API (ADR 0019).

use std::collections::HashMap;
use std::convert::Infallible;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use axum::extract::{Path as UrlPath, Query, Request, State};
use axum::http::header::{AUTHORIZATION, COOKIE, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Extension, Json, Router};
use jiff::Timestamp;
use samu_store::core::api::{
    AnyObject, ApiError, Author, AuthorKind, BareStatus, CaptureLinesRequest, CaptureLinesResponse,
    CaptureRequest, CaptureResponse, Change, ChangeNotice, ChangeOutcome, ChangesPage,
    CloseRequest, CloseResponse, DeleteRequest, DeleteResponse, ErrorCode, Export, Folder, Hlc, Id,
    LogEntry, LogKind, Login, PairCode, PairCodeRequest, PairRequest, Period, ProjectAvailability,
    RenderRequest, RenderResponse, ResetNotice, SessionInfo, ToProjectRequest, ToProjectResponse,
};
use samu_store::core::availability::Settings;
use samu_store::core::clock::{next_stamp, restamp};
use samu_store::core::pairing::PairingUri;
use samu_store::core::views::{self, ViewError};
use samu_store::{AccountStore, OwnedSnapshot, SecretKey, ServerStore};
use serde::{Deserialize, Serialize};

/// The cookie a browser keeps its session in.
pub(crate) const COOKIE_NAME: &str = "samu_session";

/// Everything a request may need.
pub(crate) struct AppState {
    data: PathBuf,
    server: Mutex<ServerStore>,
    /// Account databases, opened on first use and closed after a rest:
    /// the server is their one writer (ADR 0018 §1).
    accounts: Mutex<Accounts>,
    /// Seals calendar sources (ADR 0047 §3); none without `SAMU_SECRET_KEY`.
    secret_key: Option<SecretKey>,
    fetcher: crate::calendar::Fetcher,
    /// One writer of appointments at a time (ADR 0068 §6).
    writing: tokio::sync::Mutex<()>,
    /// Failed sign-ins by account name (ADR 0097 §1).
    sign_ins: crate::brake::Brake,
    /// Failed pairing codes, all under one key (ADR 0097 §1).
    pairings: crate::brake::Brake,
    /// Password checks at the same time (ADR 0097 §1).
    checks: tokio::sync::Semaphore,
    /// Account names the brake per name leaves alone: their password is
    /// public (ADR 0098 §1).
    unbraked: Vec<String>,
    blob_limits: BlobLimits,
    /// Minutes between two resets of a demo (ADR 0098 §2); none elsewhere.
    demo_reset: Option<u32>,
    /// Days after its last use a registered demo account goes (ADR 0099 §4).
    demo_days: u32,
    /// Who may make an account over HTTP (ADR 0099 §1).
    registration: crate::register::Registration,
    /// When `last_active` of an account was last moved on (ADR 0099 §4).
    touched: Mutex<HashMap<String, std::time::Instant>>,
    /// Whether `X-Forwarded-Host` and `X-Forwarded-Proto` are believed:
    /// only behind a reverse proxy that sets them itself.
    trust_proxy: bool,
}

/// Days a registered demo account lives after its last use, unless
/// `SAMU_DEMO_ACCOUNT_DAYS` says otherwise (ADR 0099 §4).
pub(crate) const DEMO_DAYS: u32 = 7;

/// How often `last_active` is moved on while an account is used.
const TOUCH_EVERY: std::time::Duration = std::time::Duration::from_secs(3600);

/// How long an account's database stays open without a request
/// (13.11.2026, to `0b785034`).
pub(crate) const IDLE: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// How long a deletion waits for the requests still running in the account.
const DELETE_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// The open account databases and what is known of the closed ones.
#[derive(Default)]
struct Accounts {
    open: HashMap<String, OpenAccount>,
    /// For an account closed since the start: whether the calendar loops
    /// have anything to do in it ([`AccountStore::has_calendar_work`]).
    /// Only the server makes calendars or writes, and only while the
    /// account is open, so this holds until it is opened again.
    calendar_work: HashMap<String, bool>,
    /// Accounts being deleted: not opened again.
    closing: std::collections::HashSet<String>,
}

struct OpenAccount {
    store: Arc<Mutex<AccountStore>>,
    used: std::time::Instant,
}

/// How large files may be (ADR 0097 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BlobLimits {
    /// Per file, at most [`samu_store::MAX_BLOB_BYTES`].
    pub(crate) per_file: u64,
    /// All files of an account together; `None` is no limit.
    pub(crate) per_account: Option<u64>,
}

impl Default for BlobLimits {
    fn default() -> Self {
        Self {
            per_file: samu_store::MAX_BLOB_BYTES,
            per_account: None,
        }
    }
}

impl AppState {
    pub(crate) fn new(data: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(data.join("accounts"))?;
        Ok(Self {
            data: data.to_path_buf(),
            server: Mutex::new(ServerStore::open(&data.join("server.db"))?),
            accounts: Mutex::new(Accounts::default()),
            secret_key: None,
            fetcher: crate::calendar::Fetcher::new()?,
            writing: tokio::sync::Mutex::new(()),
            sign_ins: crate::brake::Brake::new(crate::brake::PER_ACCOUNT, crate::brake::WINDOW),
            pairings: crate::brake::Brake::new(crate::brake::PAIRING, crate::brake::WINDOW),
            checks: tokio::sync::Semaphore::new(crate::brake::CHECKS_AT_ONCE),
            unbraked: Vec::new(),
            blob_limits: BlobLimits::default(),
            demo_reset: None,
            demo_days: DEMO_DAYS,
            registration: crate::register::Registration::default(),
            touched: Mutex::new(HashMap::new()),
            trust_proxy: false,
        })
    }

    /// With the key that seals calendar sources.
    #[must_use]
    pub(crate) fn with_secret_key(mut self, key: Option<SecretKey>) -> Self {
        self.secret_key = key;
        self
    }

    /// With limits for files; the one per file never above
    /// [`samu_store::MAX_BLOB_BYTES`].
    #[must_use]
    pub(crate) fn with_blob_limits(mut self, limits: BlobLimits) -> Self {
        self.blob_limits = BlobLimits {
            per_file: limits.per_file.min(samu_store::MAX_BLOB_BYTES),
            per_account: limits.per_account,
        };
        self
    }

    /// Fetching calendars only from these hosts, when given (ADR 0097 §3).
    pub(crate) fn with_calendar_hosts(
        mut self,
        hosts: Option<Vec<String>>,
    ) -> anyhow::Result<Self> {
        self.fetcher = crate::calendar::Fetcher::on_hosts(hosts)?;
        Ok(self)
    }

    /// Without the brake per name for these accounts (ADR 0098 §1).
    #[must_use]
    pub(crate) fn with_unbraked(mut self, names: &[String]) -> Self {
        self.unbraked = names.iter().map(|n| crate::brake::Brake::key(n)).collect();
        self
    }

    /// A demo, reset every so many minutes (ADR 0098 §2).
    #[must_use]
    pub(crate) const fn with_demo_reset(mut self, minutes: Option<u32>) -> Self {
        self.demo_reset = minutes;
        self
    }

    /// Registered demo accounts go so many days after their last use.
    #[must_use]
    pub(crate) fn with_demo_days(mut self, days: Option<u32>) -> Self {
        self.demo_days = days.unwrap_or(DEMO_DAYS).max(1);
        self
    }

    /// Who may make an account over HTTP (ADR 0099 §1).
    #[must_use]
    pub(crate) fn with_registration(mut self, registration: crate::register::Registration) -> Self {
        self.registration = registration;
        self
    }

    /// Behind a reverse proxy that sets the `X-Forwarded-*` headers itself.
    #[must_use]
    pub(crate) const fn with_trust_proxy(mut self, trust: bool) -> Self {
        self.trust_proxy = trust;
        self
    }

    pub(crate) const fn registration(&self) -> &crate::register::Registration {
        &self.registration
    }

    /// Password checks at the same time (ADR 0097 §1).
    pub(crate) const fn checks(&self) -> &tokio::sync::Semaphore {
        &self.checks
    }

    /// Failed sign-ins by account name (ADR 0097 §1).
    pub(crate) const fn sign_ins(&self) -> &crate::brake::Brake {
        &self.sign_ins
    }

    /// Moves `last_active` of an account on, at most once in
    /// [`TOUCH_EVERY`] (ADR 0099 §4).
    fn touch(&self, account: &str, server: &mut ServerStore) -> Result<(), Failure> {
        let now = std::time::Instant::now();
        let mut touched = self
            .touched
            .lock()
            .map_err(|_| Failure::internal("the touch map is poisoned"))?;
        if touched
            .get(account)
            .is_some_and(|at| now.saturating_duration_since(*at) < TOUCH_EVERY)
        {
            return Ok(());
        }
        server.touch(account, Timestamp::now())?;
        touched.insert(account.to_string(), now);
        Ok(())
    }

    /// Deletes every registered demo account unused for
    /// [`Self::demo_days`] before `now` (ADR 0099 §4). The names deleted;
    /// a failure is said and tried again next time.
    pub(crate) async fn expire_demo_accounts(&self, now: Timestamp) -> Vec<String> {
        let span = jiff::SignedDuration::from_hours(24 * i64::from(self.demo_days));
        let expired: Vec<String> = match self.server().and_then(|s| Ok(s.accounts()?)) {
            Ok(accounts) => accounts
                .into_iter()
                .filter(|a| {
                    a.registered == samu_store::Registered::Demo
                        && a.last_active.checked_add(span).is_ok_and(|end| end <= now)
                })
                .map(|a| a.name)
                .collect(),
            Err(e) => {
                crate::calendar::say(&format!("demo accounts: {}", e.message()));
                return Vec::new();
            }
        };
        let mut deleted = Vec::new();
        for name in expired {
            match self.delete_account(&name).await {
                Ok(()) => {
                    crate::calendar::say(&format!(
                        "demo account {name} deleted: unused for {} days",
                        self.demo_days
                    ));
                    deleted.push(name);
                }
                Err(e) => crate::calendar::say(&format!("demo account {name}: {}", e.message())),
            }
        }
        deleted
    }

    pub(crate) const fn demo_reset(&self) -> Option<u32> {
        self.demo_reset
    }

    pub(crate) const fn blob_limits(&self) -> BlobLimits {
        self.blob_limits
    }

    pub(crate) fn secret_key(&self) -> Option<&SecretKey> {
        self.secret_key.as_ref()
    }

    pub(crate) fn fetcher(&self) -> &crate::calendar::Fetcher {
        &self.fetcher
    }

    pub(crate) fn writing(&self) -> &tokio::sync::Mutex<()> {
        &self.writing
    }

    /// The data directory: `server.db` and `accounts/`.
    pub(crate) fn data(&self) -> &Path {
        &self.data
    }

    pub(crate) fn server(&self) -> Result<MutexGuard<'_, ServerStore>, Failure> {
        self.server
            .lock()
            .map_err(|_| Failure::internal("the server store is poisoned"))
    }

    fn accounts(&self) -> Result<MutexGuard<'_, Accounts>, Failure> {
        self.accounts
            .lock()
            .map_err(|_| Failure::internal("the account map is poisoned"))
    }

    /// The database of an account, opened if needed.
    pub(crate) fn account(&self, name: &str) -> Result<Arc<Mutex<AccountStore>>, Failure> {
        let mut accounts = self.accounts()?;
        if accounts.closing.contains(name) {
            return Err(Failure::status(
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::Internal,
                format!("the account {name} is being deleted"),
            ));
        }
        let now = std::time::Instant::now();
        if let Some(open) = accounts.open.get_mut(name) {
            open.used = now;
            return Ok(open.store.clone());
        }
        let store = AccountStore::open(&account_path(&self.data, name))
            .map_err(|e| Failure::internal(e.to_string()))?;
        for note in &store.migration().notes {
            crate::calendar::say(&format!("{name}: {note}"));
        }
        let store = Arc::new(Mutex::new(store));
        accounts.calendar_work.remove(name);
        accounts.open.insert(
            name.to_string(),
            OpenAccount {
                store: store.clone(),
                used: now,
            },
        );
        Ok(store)
    }

    /// Whether the calendar loops should look into an account: it is open,
    /// or it was never closed since the start, or it was closed with work
    /// left. Asking does not open it.
    pub(crate) fn has_calendar_work(&self, name: &str) -> bool {
        let Ok(accounts) = self.accounts() else {
            return true;
        };
        if accounts.closing.contains(name) {
            return false;
        }
        accounts.open.contains_key(name) || accounts.calendar_work.get(name) != Some(&false)
    }

    /// Whether the account is open now.
    #[cfg(test)]
    pub(crate) fn is_open(&self, name: &str) -> bool {
        self.accounts().is_ok_and(|a| a.open.contains_key(name))
    }

    /// Whether the account is being deleted: a stream of `/events` ends.
    pub(crate) fn is_closing(&self, name: &str) -> bool {
        self.accounts().is_ok_and(|a| a.closing.contains(name))
    }

    /// Closes every account that has not been asked for since `IDLE`
    /// before `now` and that nothing holds: no request runs in it, no
    /// `/events` stream listens. The names closed.
    pub(crate) fn close_idle(&self, now: std::time::Instant) -> Vec<String> {
        let Ok(mut accounts) = self.accounts() else {
            return Vec::new();
        };
        let idle: Vec<String> = accounts
            .open
            .iter()
            .filter(|(_, open)| {
                now.saturating_duration_since(open.used) >= IDLE
                    && Arc::strong_count(&open.store) == 1
            })
            .map(|(name, _)| name.clone())
            .collect();
        for name in &idle {
            let Some(open) = accounts.open.remove(name) else {
                continue;
            };
            // Nobody else holds it, so nobody holds its lock either.
            let work = open
                .store
                .lock()
                .ok()
                .and_then(|store| store.has_calendar_work().ok())
                .unwrap_or(true);
            accounts.calendar_work.insert(name.clone(), work);
        }
        idle
    }

    /// Deletes an account while the server runs (ADR 0099 §5): no request
    /// opens it any more, the running ones end, then its files go and its
    /// row with the sessions. Refused when a request still holds it after
    /// [`DELETE_WAIT`]; then nothing is deleted.
    pub(crate) async fn delete_account(&self, name: &str) -> Result<(), Failure> {
        let taken = {
            let mut accounts = self.accounts()?;
            if !accounts.closing.insert(name.to_string()) {
                return Err(Failure::status(
                    StatusCode::CONFLICT,
                    ErrorCode::BadRequest,
                    format!("the account {name} is being deleted already"),
                ));
            }
            accounts.calendar_work.remove(name);
            accounts.open.remove(name)
        };
        let deleted = self.delete_taken(name, taken).await;
        if let Ok(mut accounts) = self.accounts() {
            accounts.closing.remove(name);
        }
        deleted
    }

    /// Removes an account just made whose making failed: nothing else
    /// knows it yet, so nothing waits.
    pub(crate) fn forget_account(&self, name: &str) -> Result<(), Failure> {
        let taken = {
            let mut accounts = self.accounts()?;
            accounts.calendar_work.remove(name);
            accounts.open.remove(name)
        };
        drop(taken);
        AccountStore::remove(&account_path(&self.data, name))?;
        self.server()?.delete_account(name)?;
        Ok(())
    }

    async fn delete_taken(&self, name: &str, taken: Option<OpenAccount>) -> Result<(), Failure> {
        if let Some(open) = taken {
            let start = std::time::Instant::now();
            while Arc::strong_count(&open.store) > 1 {
                if start.elapsed() >= DELETE_WAIT {
                    return Err(Failure::status(
                        StatusCode::SERVICE_UNAVAILABLE,
                        ErrorCode::Internal,
                        format!("the account {name} is still in use; try again"),
                    ));
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
        let path = account_path(&self.data, name);
        // Files first, under the account's lock, as the command line does:
        // a refusal leaves the account whole. Removing a few files and a
        // row is quick enough for an async thread.
        AccountStore::remove(&path)?;
        self.server()?.delete_account(name)?;
        Ok(())
    }
}

/// Closes idle accounts every minute, for as long as the server runs.
pub(crate) async fn close_idle_accounts(state: Arc<AppState>) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        state.close_idle(std::time::Instant::now());
    }
}

/// Deletes expired demo accounts at the start and then hourly (ADR 0099 §4).
pub(crate) async fn expire_demo_accounts(state: Arc<AppState>) {
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(3600));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        state.expire_demo_accounts(Timestamp::now()).await;
    }
}

/// Where an account's database lives.
pub(crate) fn account_path(data: &Path, name: &str) -> PathBuf {
    data.join("accounts").join(format!("{name}.db"))
}

/// A failed request, answered as an [`ApiError`].
#[derive(Debug)]
pub(crate) struct Failure {
    status: StatusCode,
    error: ApiError,
}

impl Failure {
    /// What went wrong, for the log.
    pub(crate) fn message(&self) -> &str {
        &self.error.message
    }

    fn new(status: StatusCode, error: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            status,
            error: ApiError {
                error,
                message: message.into(),
            },
        }
    }

    pub(crate) fn status(status: StatusCode, error: ErrorCode, message: impl Into<String>) -> Self {
        Self::new(status, error, message)
    }

    pub(crate) fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, ErrorCode::Unauthorized, message)
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, ErrorCode::NotFound, message)
    }

    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, ErrorCode::BadRequest, message)
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            message,
        )
    }
}

impl From<samu_store::Error> for Failure {
    fn from(error: samu_store::Error) -> Self {
        Self::internal(error.to_string())
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        (self.status, Json(self.error)).into_response()
    }
}

/// The session a signed-in request carries.
#[derive(Debug, Clone)]
pub(crate) struct Session {
    pub(crate) account: String,
    pub(crate) device: String,
    /// A person or an agent (ADR 0039 §2).
    pub(crate) kind: AuthorKind,
    pub(crate) expires: Timestamp,
    pub(crate) token: String,
    /// What a restricted session may reach (ADR 0095 §1).
    pub(crate) grant: Option<samu_store::core::api::Grant>,
}

impl Session {
    /// Who writes through this session: the author of what it makes and
    /// the kind in the logbook (ADR 0039 §1).
    pub(crate) fn author(&self) -> Author {
        Author {
            device: self.device.clone(),
            kind: self.kind,
        }
    }
}

/// The token of a request: bearer first, then the cookie.
pub(crate) fn token_of(headers: &HeaderMap) -> Option<String> {
    if let Some(value) = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok())
        && let Some(token) = value.strip_prefix("Bearer ")
    {
        return Some(token.trim().to_string());
    }
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == COOKIE_NAME)
        .map(|(_, value)| value.to_string())
}

/// Lets a request through only with a live session (ADR 0019 §3).
async fn require_session(
    State(state): State<Arc<AppState>>,
    mut request: Request,
    next: Next,
) -> Result<Response, Failure> {
    let token =
        token_of(request.headers()).ok_or_else(|| Failure::unauthorized("not signed in"))?;
    let row = {
        let mut server = state.server()?;
        let row = server
            .session(&token, Timestamp::now())?
            .ok_or_else(|| Failure::unauthorized("the session is unknown or has expired"))?;
        state.touch(&row.account, &mut server)?;
        row
    };
    // A restricted session never reaches a forbidden route (ADR 0095 §4).
    if row.grant.is_some() {
        let path = request
            .extensions()
            .get::<axum::extract::MatchedPath>()
            .map(|p| p.as_str().to_string())
            .unwrap_or_default();
        let method = request.method().as_str();
        let rule = samu_store::core::api::ROUTES
            .iter()
            .find(|route| route.method == method && route.path == path)
            .and_then(crate::grant::restriction);
        // A route outside the table is refused too: what is not known to
        // be safe is not.
        if !matches!(
            rule,
            Some(crate::grant::Rule::Filtered | crate::grant::Rule::Free)
        ) {
            return Err(crate::grant::forbidden(format!(
                "{method} {path} is not open to a restricted session (ADR 0095)"
            )));
        }
    }
    request.extensions_mut().insert(Session {
        account: row.account,
        device: row.device,
        kind: row.kind,
        expires: row.expires,
        token,
        grant: row.grant,
    });
    Ok(next.run(request).await)
}

#[derive(Serialize)]
pub(crate) struct Health {
    status: &'static str,
    version: &'static str,
    api: &'static str,
}

/// The version this binary was built as: `SAMU_VERSION` from the build
/// (like `0.1.0+git42.abc1234`), else the package's. It is what a deploy
/// script compares against (ADR 0009 §4).
pub(crate) const VERSION: &str = match option_env!("SAMU_VERSION") {
    Some(version) => version,
    None => env!("CARGO_PKG_VERSION"),
};

pub(crate) async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        version: VERSION,
        api: samu_store::core::api::VERSION,
    })
}

const WRONG: &str = "wrong account or password";

/// 429 with `Retry-After` for a held try (ADR 0097 §1). It carries
/// `unauthorized`, so a client sees a failed sign-in.
fn held(wait: std::time::Duration) -> Response {
    let (seconds, message) = crate::brake::message(wait);
    let mut response = Failure::status(
        StatusCode::TOO_MANY_REQUESTS,
        ErrorCode::Unauthorized,
        message,
    )
    .into_response();
    response
        .headers_mut()
        .insert(axum::http::header::RETRY_AFTER, HeaderValue::from(seconds));
    response
}

/// `change` with the device of its stamp set to the session's: who made a
/// change is the session's word, never the client's (ADR 0039 §1). Set,
/// not required: a replica passes on what its local clients stamped, and
/// an outbox outlives a sign-in under another name; refusing those would
/// drop them. The time and the counter stay as the client gave them.
fn bound_to(change: Change, device: &str) -> Change {
    match change.hlc() {
        Some(hlc) if hlc.device != device => {
            let stamp = Hlc::new(hlc.wall, hlc.counter, device);
            restamp(change, stamp)
        }
        _ => change,
    }
}

async fn login(
    State(state): State<Arc<AppState>>,
    Json(login): Json<Login>,
) -> Result<Response, Failure> {
    let device = login.device.trim().to_string();
    let accounts = state.clone();
    if device.is_empty() {
        return Err(Failure::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::BadRequest,
            "a session needs a device name",
        ));
    }
    if samu_store::is_reserved_device(&device) {
        return Err(Failure::bad_request(samu_store::RESERVED_DEVICE));
    }
    let kind = login.kind.unwrap_or_default();
    if kind == AuthorKind::Unknown {
        return Err(Failure::new(
            StatusCode::BAD_REQUEST,
            ErrorCode::BadRequest,
            "a session is a person's (human) or an agent's (agent)",
        ));
    }
    let key = crate::brake::Brake::key(&login.account);
    // A public password is no secret to guess (ADR 0098 §1).
    let braked = !state.unbraked.contains(&key);
    if braked && let Some(wait) = state.sign_ins.wait(&key, std::time::Instant::now()) {
        return Ok(held(wait));
    }
    // Argon2 is slow on purpose; off the async threads, and only so many
    // at once (ADR 0097 §1).
    let only = login.only.clone();
    if only
        .as_ref()
        .is_some_and(|g| g.access == samu_store::core::api::Access::Unknown)
    {
        return Err(Failure::bad_request(
            "a grant is read-write (rw) or read-only (ro)",
        ));
    }
    let check = accounts
        .checks
        .acquire()
        .await
        .map_err(|e| Failure::internal(e.to_string()))?;
    let signed_in = tokio::task::spawn_blocking(move || {
        let mut server = state.server()?;
        if !server.has_active_account()? {
            return Err(Failure::unauthorized(
                "there is no account yet: create one with `samu-server account add`",
            ));
        }
        Ok(server.sign_in(
            &login.account,
            &login.password,
            &device,
            kind,
            login.only.as_ref(),
            Timestamp::now(),
        )?)
    })
    .await
    .map_err(|e| Failure::internal(e.to_string()))??;
    drop(check);
    let Some((token, row)) = signed_in else {
        if braked {
            accounts.sign_ins.failed(&key, std::time::Instant::now());
        }
        return Err(Failure::unauthorized(WRONG));
    };
    accounts.sign_ins.succeeded(&key);
    // Only after the password: whether the grant names live folders and
    // projects of the account (ADR 0095 §1). A wrong one leaves no session
    // behind, and tells nothing to someone without the password.
    if let Some(only) = &only
        && let Err(refused) = check_grant(&accounts, &row.account, only)
    {
        accounts.server()?.sign_out(&token)?;
        return Err(refused);
    }
    Ok(signed_in_answer(token, row, &accounts))
}

/// Whether every id of a grant is a live folder or project of the account.
fn check_grant(
    state: &AppState,
    account: &str,
    grant: &samu_store::core::api::Grant,
) -> Result<(), Failure> {
    let store = state.account(account)?;
    let store = lock(&store)?;
    for id in &grant.objects {
        let found = store.get(*id)?.map(AnyObject::from);
        if !matches!(found, Some(AnyObject::Folder(_) | AnyObject::Project(_))) {
            return Err(Failure::bad_request(format!(
                "{id} is no folder or project of this account"
            )));
        }
    }
    Ok(())
}

/// The answer to a sign-in, by password or by pairing code (ADR 0031 §3):
/// the session with the account's settings, and the cookie for a browser.
fn signed_in_answer(token: String, row: samu_store::SessionRow, state: &AppState) -> Response {
    let cookie = session_cookie(&token);
    let body = with_settings(
        SessionInfo {
            account: row.account,
            device: row.device,
            kind: Some(row.kind),
            expires: row.expires,
            token: Some(token),
            zone: None,
            due_soon: None,
            remind_time: None,
            replica: None,
            grant: row.grant,
            demo_until: None,
        },
        state,
    );
    let mut response = Json(body).into_response();
    if let Ok(value) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(SET_COOKIE, value);
    }
    response
}

/// The cookie a browser keeps a session in.
pub(crate) fn session_cookie(token: &str) -> String {
    format!(
        "{COOKIE_NAME}={token}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age={}",
        30 * 24 * 3600
    )
}

/// The cookie that takes it away again.
pub(crate) fn gone_cookie() -> String {
    format!("{COOKIE_NAME}=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0")
}

/// A pairing code for a phone, after the password once more (ADR 0031 §1).
async fn pair_code(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    headers: HeaderMap,
    Json(request): Json<PairCodeRequest>,
) -> Result<Json<PairCode>, Failure> {
    let server = origin(&headers, state.trust_proxy)?;
    let account = session.account.clone();
    let made = tokio::task::spawn_blocking(move || {
        state
            .server()?
            .pairing_code(
                &account,
                &request.password,
                &request.device,
                Timestamp::now(),
            )
            .map_err(|e| match e {
                samu_store::Error::Unsupported { what } => {
                    Failure::new(StatusCode::BAD_REQUEST, ErrorCode::BadRequest, what)
                }
                other => other.into(),
            })
    })
    .await
    .map_err(|e| Failure::internal(e.to_string()))??;
    let made = made.ok_or_else(|| Failure::unauthorized("wrong password"))?;
    let uri = PairingUri {
        server,
        account: made.account.clone(),
        device: made.device.clone(),
        code: made.code.clone(),
    }
    .to_string();
    Ok(Json(PairCode {
        code: made.code,
        account: made.account,
        device: made.device,
        expires: made.expires,
        uri,
    }))
}

/// Where the phone finds this server: the address the request came to
/// (ADR 0031 §4).
///
/// The server speaks plain HTTP itself, so on its own it is `http://` and
/// the `Host` header. `X-Forwarded-Host` and `X-Forwarded-Proto` count only
/// with `trust_proxy`, behind a reverse proxy that sets them: anyone who
/// reaches the server directly can send them too, and the address ends up
/// in the pairing code a phone signs in with.
pub(crate) fn origin(headers: &HeaderMap, trust_proxy: bool) -> Result<String, Failure> {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
    };
    let forwarded = |name: &str| header(name).filter(|_| trust_proxy);
    let host = forwarded("x-forwarded-host")
        .or_else(|| header("host"))
        .filter(|h| !h.is_empty() && !h.contains(['/', ' ', '@']))
        .ok_or_else(|| {
            Failure::new(
                StatusCode::BAD_REQUEST,
                ErrorCode::BadRequest,
                "the request names no host",
            )
        })?;
    let scheme = if forwarded("x-forwarded-proto") == Some("https") {
        "https"
    } else {
        "http"
    };
    Ok(format!("{scheme}://{host}"))
}

/// A pairing code traded for a session, once (ADR 0031 §3).
async fn pair(
    State(state): State<Arc<AppState>>,
    Json(request): Json<PairRequest>,
) -> Result<Response, Failure> {
    let accounts = state.clone();
    // All codes under one key: a code names no account (ADR 0097 §1).
    if let Some(wait) = state.pairings.wait("", std::time::Instant::now()) {
        return Ok(held(wait));
    }
    let redeemed = tokio::task::spawn_blocking(move || {
        Ok::<_, Failure>(
            state
                .server()?
                .redeem(request.code.trim(), Timestamp::now())?,
        )
    })
    .await
    .map_err(|e| Failure::internal(e.to_string()))??;
    let Some((token, row)) = redeemed else {
        accounts.pairings.failed("", std::time::Instant::now());
        return Err(Failure::unauthorized(
            "the pairing code is unknown, used or expired",
        ));
    };
    Ok(signed_in_answer(token, row, &accounts))
}

async fn logout(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
) -> Result<Response, Failure> {
    state.server()?.sign_out(&session.token)?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    if let Ok(value) = HeaderValue::from_str(&gone_cookie()) {
        response.headers_mut().insert(SET_COOKIE, value);
    }
    Ok(response)
}

async fn session(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
) -> Json<SessionInfo> {
    Json(with_settings(
        SessionInfo {
            account: session.account,
            device: session.device,
            kind: Some(session.kind),
            expires: session.expires,
            token: None,
            zone: None,
            due_soon: None,
            remind_time: None,
            replica: None,
            grant: session.grant,
            demo_until: None,
        },
        &state,
    ))
}

/// Only a person's session of the whole account sees and ends sessions
/// (ADR 0095 §7).
fn may_manage_sessions(session: &Session) -> Result<(), Failure> {
    if session.grant.is_some() || session.kind != AuthorKind::Human {
        return Err(crate::grant::forbidden(
            "only a person's session of the whole account manages sessions (ADR 0095 §7)",
        ));
    }
    Ok(())
}

/// `GET /sessions`: the live sessions of the account (ADR 0095 §7).
async fn list_sessions(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
) -> Result<Json<Vec<samu_store::core::api::SessionSummary>>, Failure> {
    may_manage_sessions(&session)?;
    Ok(Json(state.server()?.sessions(
        &session.account,
        &session.token,
        Timestamp::now(),
    )?))
}

/// `POST /sessions/{id}/end`: ends one of them by the start of its id.
async fn end_session(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    UrlPath(id): UrlPath<String>,
) -> Result<StatusCode, Failure> {
    may_manage_sessions(&session)?;
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Failure::bad_request(format!(
            "{id:?} is not a session id (hex)"
        )));
    }
    match state.server()?.end_session(&session.account, &id)? {
        samu_store::Ended::Ended => Ok(StatusCode::NO_CONTENT),
        samu_store::Ended::Unknown => Err(Failure::not_found(format!("no session {id}"))),
        samu_store::Ended::Ambiguous => Err(Failure::bad_request(format!(
            "more than one session starts with {id}"
        ))),
    }
}

/// A session answer with the account's zone, due-soon span and reminder
/// time (ADR 0028 §6, ADR 0030 §3); without them when the account cannot be
/// read, which the client survives.
fn with_settings(mut info: SessionInfo, state: &AppState) -> SessionInfo {
    let Ok(store) = state.account(&info.account) else {
        return info;
    };
    let Ok(settings) = lock(&store).and_then(|s| Ok(s.settings()?)) else {
        return info;
    };
    info.zone = settings.zone.iana_name().map(str::to_string);
    info.due_soon = Some(Period(settings.due_soon));
    info.remind_time = Some(settings.remind_time);
    info.demo_until = demo_until(state, &info.account, &settings.zone);
    info
}

/// The day a registered demo account goes unless it is used before
/// (ADR 0099 §4); none for any other.
fn demo_until(
    state: &AppState,
    account: &str,
    zone: &jiff::tz::TimeZone,
) -> Option<jiff::civil::Date> {
    let info = state.server().ok()?.account_info(account).ok()??;
    if info.registered != samu_store::Registered::Demo {
        return None;
    }
    let span = jiff::SignedDuration::from_hours(24 * i64::from(state.demo_days));
    Some(
        info.last_active
            .checked_add(span)
            .ok()?
            .to_zoned(zone.clone())
            .date(),
    )
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>, Failure> {
    mutex
        .lock()
        .map_err(|_| Failure::internal("an account store is poisoned"))
}

#[derive(Debug, Deserialize)]
struct ChangesQuery {
    /// `server`: the server stamps the changes itself (ADR 0019 §4).
    stamp: Option<String>,
}

/// `POST /changes`: a list of changes in, one outcome per change out, in
/// order (ADR 0016).
async fn post_changes(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    Query(query): Query<ChangesQuery>,
    Json(changes): Json<Vec<Change>>,
) -> Result<Json<Vec<ChangeOutcome>>, Failure> {
    let by_server = match query.stamp.as_deref() {
        None => false,
        Some("server") => true,
        Some(other) => return Err(Failure::bad_request(format!("stamp={other} is not known"))),
    };
    let store = state.account(&session.account)?;
    let mut store = lock(&store)?;
    let mut reach = session_reach(&session, &store)?;
    let mut outcomes = Vec::with_capacity(changes.len());
    for change in changes {
        // A restricted session's change outside its grant (ADR 0095 §3).
        if let Some(reach) = &reach
            && !crate::grant::may_apply(reach, &store, &change)?
        {
            outcomes.push(ChangeOutcome::Rejected {
                reason: crate::grant::OUTSIDE.into(),
            });
            continue;
        }
        let now = Timestamp::now();
        let change = if by_server {
            let seen = store.observed()?;
            restamp(
                change,
                next_stamp(&session.device, None, seen.as_ref(), now),
            )
        } else {
            bound_to(change, &session.device)
        };
        let outcome = store.apply(&change, &session.author(), now)?;
        if let Some(reach) = &mut reach
            && matches!(outcome, ChangeOutcome::Applied { .. })
        {
            crate::grant::admit(reach, &store, &change)?;
        }
        outcomes.push(outcome);
    }
    drop(store);
    // An appointment changed in a writable calendar goes back at once.
    crate::writer::kick(&state, &session.account);
    Ok(Json(outcomes))
}

#[derive(Debug, Deserialize)]
struct AfterQuery {
    #[serde(default)]
    after: u64,
}

/// `GET /changes?after=N`: everything changed since (ADR 0004 §5).
async fn get_changes(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    Query(query): Query<AfterQuery>,
) -> Result<Json<ChangesPage>, Failure> {
    let store = state.account(&session.account)?;
    let store = lock(&store)?;
    let reach = session_reach(&session, &store)?;
    let mut page = ChangesPage {
        seq: store.seq()?,
        ..ChangesPage::default()
    };
    // Read at once, not one by one: after 0 that is every object.
    let mut live: HashMap<Id, AnyObject> = store
        .live_after(query.after)?
        .into_iter()
        .map(|object| (object.id(), AnyObject::from(object)))
        .collect();
    for changed in store.changed_after(query.after)? {
        if changed.deleted {
            page.gone.push(changed.id);
        } else if let Some(object) = live.remove(&changed.id) {
            // What a restricted session does not see now is gone for it,
            // also what was moved out of its grant (ADR 0095 §5).
            if reach.as_ref().is_none_or(|reach| reach.sees(&object)) {
                page.objects.push(object);
            } else {
                page.gone.push(changed.id);
            }
        }
    }
    Ok(Json(page))
}

/// `GET /export`: the whole account (ADR 0074 §5); the files go by
/// `/blobs`.
async fn get_export(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
) -> Result<Json<Export>, Failure> {
    let store = state.account(&session.account)?;
    let export = lock(&store)?.export(&session.account, Timestamp::now())?;
    Ok(Json(export))
}

/// `GET /objects/{id}`.
async fn get_object(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    UrlPath(id): UrlPath<String>,
) -> Result<Json<AnyObject>, Failure> {
    let id: Id = id
        .parse()
        .map_err(|_| Failure::bad_request(format!("{id:?} is not an id")))?;
    let store = state.account(&session.account)?;
    let store = lock(&store)?;
    let object = store
        .get(id)?
        .map(AnyObject::from)
        .filter(|object| {
            session_reach(&session, &store)
                .ok()
                .flatten()
                .is_none_or(|reach| reach.sees(object))
        })
        .ok_or_else(|| Failure::not_found(format!("no object {id}")))?;
    Ok(Json(object))
}

/// `POST /objects/{id}/delete`: the object and what hangs on it, computed
/// in samu-core and written in one transaction (ADR 0036). With `preview`,
/// the same counts and nothing written.
async fn delete_object(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    UrlPath(id): UrlPath<String>,
    Json(request): Json<DeleteRequest>,
) -> Result<Json<DeleteResponse>, Failure> {
    let id: Id = id
        .parse()
        .map_err(|_| Failure::bad_request(format!("{id:?} is not an id")))?;
    let store = state.account(&session.account)?;
    let mut store = lock(&store)?;
    let reach = session_reach(&session, &store)?;
    if let Some(reach) = &reach
        && !crate::grant::sees_stored(reach, &store, id)?
    {
        return Err(Failure::not_found(format!("no object {id}")));
    }
    match store.deleted(id)? {
        None => return Err(Failure::not_found(format!("no object {id}"))),
        // Deleted before: applied, and nothing changes (ADR 0036 §1).
        Some(true) => return Ok(Json(DeleteResponse::default())),
        Some(false) => {}
    }
    let snapshot = store.snapshot()?;
    let plan =
        samu_store::core::delete::plan(&snapshot.snapshot(), &snapshot.scopes, id, request.tasks)
            .map_err(|e| Failure::bad_request(e.to_string()))?;
    // Everything the delete would do must stay inside the grant: a
    // project's tasks to the inbox, a folder's content up and out.
    if let Some(reach) = &reach {
        for change in &plan.changes {
            if !crate::grant::may_apply(reach, &store, change)? {
                return Err(crate::grant::forbidden(format!(
                    "the delete reaches {}",
                    crate::grant::OUTSIDE
                )));
            }
        }
    }
    let mut answer = DeleteResponse {
        deleted: plan.deleted,
        moved: plan.moved,
        untagged: plan.untagged,
        seq: None,
    };
    if request.preview {
        return Ok(Json(answer));
    }
    let now = Timestamp::now();
    let seen = store.observed()?;
    let mut own: Option<Hlc> = None;
    let changes: Vec<Change> = plan
        .changes
        .into_iter()
        .map(|change| {
            let stamp = next_stamp(&session.device, own.as_ref(), seen.as_ref(), now);
            own = Some(stamp.clone());
            restamp(change, stamp)
        })
        .collect();
    // One logbook entry for all of it (ADR 0123 §2).
    let counts = serde_json::json!({
        "deleted": answer.deleted,
        "moved": answer.moved,
        "untagged": answer.untagged,
    });
    match store.apply_delete(&changes, &session.author(), now, id, &counts)? {
        Ok(outcomes) => {
            answer.seq = outcomes.iter().rev().find_map(|outcome| match outcome {
                ChangeOutcome::Applied { seq } => Some(*seq),
                _ => None,
            });
            Ok(Json(answer))
        }
        Err((index, outcome)) => Err(Failure::internal(format!(
            "delete not applied, change {index}: {outcome:?}"
        ))),
    }
}

/// `POST /objects/{id}/to-project`: a new project takes the task's title,
/// note, dates and children; the task stays and goes in last (ADR 0126).
/// Computed in samu-core and written in one transaction; with `preview`,
/// the same answer and nothing written.
async fn to_project(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    UrlPath(id): UrlPath<String>,
    Json(request): Json<ToProjectRequest>,
) -> Result<Json<ToProjectResponse>, Failure> {
    let id: Id = id
        .parse()
        .map_err(|_| Failure::bad_request(format!("{id:?} is not an id")))?;
    let store = state.account(&session.account)?;
    let mut store = lock(&store)?;
    let reach = session_reach(&session, &store)?;
    if let Some(reach) = &reach
        && !crate::grant::sees_stored(reach, &store, id)?
    {
        return Err(Failure::not_found(format!("no object {id}")));
    }
    if store.deleted(id)?.is_none_or(|deleted| deleted) {
        return Err(Failure::not_found(format!("no object {id}")));
    }
    let snapshot = store.snapshot()?;
    let now = Timestamp::now();
    let plan = samu_store::core::convert::plan(&snapshot.snapshot(), id, Id::new(), &request, now)
        .map_err(|e| Failure::bad_request(e.to_string()))?;
    if let Some(reach) = &reach {
        for change in &plan.changes {
            if !crate::grant::may_apply(reach, &store, change)? {
                return Err(crate::grant::forbidden(format!(
                    "the project reaches {}",
                    crate::grant::OUTSIDE
                )));
            }
        }
    }
    let mut answer = ToProjectResponse {
        project: plan.project,
        moved: plan.moved,
        seq: None,
    };
    if request.preview {
        return Ok(Json(answer));
    }
    let seen = store.observed()?;
    let mut own: Option<Hlc> = None;
    let changes: Vec<Change> = plan
        .changes
        .into_iter()
        .map(|change| {
            let stamp = next_stamp(&session.device, own.as_ref(), seen.as_ref(), now);
            own = Some(stamp.clone());
            restamp(change, stamp)
        })
        .collect();
    match store.apply_all(&changes, &session.author(), now)? {
        Ok(outcomes) => {
            answer.seq = outcomes.iter().rev().find_map(|outcome| match outcome {
                ChangeOutcome::Applied { seq } => Some(*seq),
                _ => None,
            });
            Ok(Json(answer))
        }
        Err((index, outcome)) => Err(Failure::internal(format!(
            "project not made, change {index}: {outcome:?}"
        ))),
    }
}

/// `POST /objects/{id}/close`: complete or drop a task, and with it, lifted
/// out or not yet what is open below it (ADR 0128). Computed in samu-core
/// and written in one transaction; with `ask` and open tasks below, or
/// with `preview`, nothing is written and the answer counts them.
async fn close_task(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    UrlPath(id): UrlPath<String>,
    Json(request): Json<CloseRequest>,
) -> Result<Json<CloseResponse>, Failure> {
    let id: Id = id
        .parse()
        .map_err(|_| Failure::bad_request(format!("{id:?} is not an id")))?;
    let store = state.account(&session.account)?;
    let mut store = lock(&store)?;
    let reach = session_reach(&session, &store)?;
    if let Some(reach) = &reach
        && !crate::grant::sees_stored(reach, &store, id)?
    {
        return Err(Failure::not_found(format!("no object {id}")));
    }
    if store.deleted(id)?.is_none_or(|deleted| deleted) {
        return Err(Failure::not_found(format!("no object {id}")));
    }
    let snapshot = store.snapshot()?;
    let now = Timestamp::now();
    let plan = samu_store::core::close::plan(&snapshot.snapshot(), id, &request, now)
        .map_err(|e| Failure::bad_request(e.to_string()))?;
    if let Some(reach) = &reach {
        for change in &plan.changes {
            if !crate::grant::may_apply(reach, &store, change)? {
                return Err(crate::grant::forbidden(format!(
                    "closing reaches {}",
                    crate::grant::OUTSIDE
                )));
            }
        }
    }
    let mut answer = CloseResponse {
        open: plan.open,
        closed: plan.closed,
        lifted: plan.lifted,
        seq: None,
    };
    if request.preview || plan.changes.is_empty() {
        return Ok(Json(answer));
    }
    let seen = store.observed()?;
    let mut own: Option<Hlc> = None;
    let changes: Vec<Change> = plan
        .changes
        .into_iter()
        .map(|change| {
            let stamp = next_stamp(&session.device, own.as_ref(), seen.as_ref(), now);
            own = Some(stamp.clone());
            restamp(change, stamp)
        })
        .collect();
    match store.apply_all(&changes, &session.author(), now)? {
        Ok(outcomes) => {
            answer.seq = outcomes.iter().rev().find_map(|outcome| match outcome {
                ChangeOutcome::Applied { seq } => Some(*seq),
                _ => None,
            });
            Ok(Json(answer))
        }
        Err((index, outcome)) => Err(Failure::internal(format!(
            "task not closed, change {index}: {outcome:?}"
        ))),
    }
}

/// `GET /objects/{id}/logbook`: what happened to it, oldest first
/// (ADR 0024 §7).
async fn get_logbook(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    UrlPath(id): UrlPath<String>,
) -> Result<Json<Vec<LogEntry>>, Failure> {
    let id: Id = id
        .parse()
        .map_err(|_| Failure::bad_request(format!("{id:?} is not an id")))?;
    let store = state.account(&session.account)?;
    let store = lock(&store)?;
    if let Some(reach) = session_reach(&session, &store)?
        && !crate::grant::sees_stored(&reach, &store, id)?
    {
        return Err(Failure::not_found(format!("no object {id}")));
    }
    let entries = store
        .logbook(id)?
        .ok_or_else(|| Failure::not_found(format!("no object {id}")))?;
    Ok(Json(entries))
}

#[derive(Debug, Deserialize)]
struct LogbookQuery {
    since: Option<Timestamp>,
    until: Option<Timestamp>,
    /// Comma-separated kinds, like `deleted,moved`.
    kind: Option<String>,
    device: Option<String>,
    limit: Option<usize>,
}

/// `GET /logbook`: the account's logbook, newest first (ADR 0123 §4); the
/// last 7 days and 200 entries unless asked otherwise. A session with a
/// grant sees the entries of what it reaches.
async fn get_account_logbook(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    Query(query): Query<LogbookQuery>,
) -> Result<Json<Vec<samu_store::core::api::AccountLogEntry>>, Failure> {
    let until = query.until.unwrap_or_else(Timestamp::now);
    let since = query.since.unwrap_or_else(|| {
        until
            .checked_sub(jiff::SignedDuration::from_hours(7 * 24))
            .unwrap_or(until)
    });
    let mut kinds = Vec::new();
    for name in query
        .kind
        .iter()
        .flat_map(|k| k.split(','))
        .filter(|k| !k.is_empty())
    {
        match serde_json::from_value::<LogKind>(serde_json::Value::String(name.trim().into())) {
            Ok(LogKind::Unknown) | Err(_) => {
                return Err(Failure::bad_request(format!("{name:?} is no logbook kind")));
            }
            Ok(kind) => kinds.push(kind),
        }
    }
    let filter = samu_store::LogFilter {
        since,
        until,
        kinds,
        device: query.device,
        limit: query.limit.unwrap_or(200).clamp(1, 1000),
    };
    let store = state.account(&session.account)?;
    let store = lock(&store)?;
    let mut entries = store.logbook_all(&filter)?;
    if let Some(reach) = session_reach(&session, &store)? {
        let mut seen = Vec::with_capacity(entries.len());
        for entry in entries {
            if crate::grant::sees_stored(&reach, &store, entry.entry.object)? {
                seen.push(entry);
            }
        }
        entries = seen;
    }
    Ok(Json(entries))
}

#[derive(Debug, Deserialize)]
struct AvailableQuery {
    /// Only available tasks.
    #[serde(default)]
    available: bool,
    /// Only tasks planned for today or earlier.
    #[serde(default)]
    today: bool,
    /// Only tasks of this project.
    project: Option<Id>,
    /// Completed and dropped tasks too.
    #[serde(default)]
    all: bool,
    /// Only what this scope shows, and what is due outside it marked
    /// `outside` (ADR 0035 §5/§6).
    scope: Option<Id>,
    /// Only what this perspective shows, inside the scope (ADR 0090 §2).
    perspective: Option<Id>,
    /// Also what another own scope hides, marked (ADR 0124 §2).
    #[serde(default)]
    elsewhere: bool,
    /// Without the tasks themselves: `BareStatus`, for a client that has
    /// them from `GET /changes` (ADR 0065).
    #[serde(default)]
    bare: bool,
}

impl From<ViewError> for Failure {
    fn from(error: ViewError) -> Self {
        match error {
            ViewError::NoScope(_) | ViewError::NoPerspective(_) | ViewError::NoHabit(_) => {
                Failure::not_found(error.to_string())
            }
            ViewError::BadRequest(message) => Failure::bad_request(message),
            ViewError::Unprocessable(message) => unprocessable(message),
        }
    }
}

/// The objects of the session's account as the session sees them, and the
/// account's settings.
fn read(state: &AppState, session: &Session) -> Result<(Arc<OwnedSnapshot>, Settings), Failure> {
    let store = state.account(&session.account)?;
    let store = lock(&store)?;
    Ok((seen_snapshot(session, &store)?, store.settings()?))
}

/// The grant of a restricted session read against its account now; none
/// for a session of the whole account.
pub(crate) fn session_reach(
    session: &Session,
    store: &AccountStore,
) -> Result<Option<samu_store::core::grant::Reach>, Failure> {
    session
        .grant
        .as_ref()
        .map(|grant| crate::grant::reach(store, grant))
        .transpose()
}

/// The account's objects, restricted to what the session sees (ADR 0095
/// §2): the same views then answer only with that.
fn seen_snapshot(session: &Session, store: &AccountStore) -> Result<Arc<OwnedSnapshot>, Failure> {
    let objects = store.snapshot()?;
    Ok(match session_reach(session, store)? {
        Some(reach) => Arc::new(reach.restrict(&objects)),
        None => objects,
    })
}

/// `GET /available`: tasks with their computed status, computed in
/// samu-core and only there (ADR 0006, ADR 0017).
async fn available(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    Query(query): Query<AvailableQuery>,
) -> Result<Response, Failure> {
    let (objects, settings) = read(&state, &session)?;
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

#[derive(Debug, Deserialize)]
pub(crate) struct BlocksQuery {
    /// The perspective whose blocks to show (ADR 0116 §4).
    pub(crate) perspective: Id,
    /// Only what this scope shows (ADR 0035 §5).
    pub(crate) scope: Option<Id>,
}

/// `GET /blocks`: what a perspective shows, block by block, computed in
/// samu-core (ADR 0116 §3-4).
async fn blocks(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    Query(query): Query<BlocksQuery>,
) -> Result<Response, Failure> {
    let (objects, settings) = read(&state, &session)?;
    let sections = views::blocks(
        &objects,
        &settings,
        Timestamp::now(),
        query.scope,
        query.perspective,
    )?;
    Ok(Json(sections).into_response())
}

#[derive(Debug, Deserialize)]
pub(crate) struct HabitsQuery {
    /// Only what this scope shows (ADR 0035 §5).
    pub(crate) scope: Option<Id>,
    /// With the history and the entries from this day on (ADR 0091 §4).
    pub(crate) from: Option<jiff::civil::Date>,
}

/// `GET /habits`: the open habits of the scope with their streaks, computed
/// in samu-core (ADR 0091 §3, §4).
async fn habits(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    Query(query): Query<HabitsQuery>,
) -> Result<Response, Failure> {
    let (objects, settings) = read(&state, &session)?;
    let habits = views::habits(
        &objects,
        &settings,
        Timestamp::now(),
        query.scope,
        query.from,
    )?;
    Ok(Json(habits).into_response())
}

/// `GET /habits/{id}`: one habit, also a completed one (ADR 0091 §4).
async fn habit(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    UrlPath(id): UrlPath<String>,
    Query(query): Query<HabitsQuery>,
) -> Result<Response, Failure> {
    let id: Id = id
        .parse()
        .map_err(|_| Failure::bad_request(format!("{id:?} is not an id")))?;
    let (objects, settings) = read(&state, &session)?;
    let habit = views::habit(&objects, &settings, Timestamp::now(), id, query.from)?;
    Ok(Json(habit).into_response())
}

#[derive(Debug, Deserialize)]
struct ForecastQuery {
    /// Only what this scope shows (ADR 0035 §5).
    scope: Option<Id>,
}

/// `GET /forecast`: the sections of Forecast as samu-core computes them
/// with the account's settings (ADR 0045).
async fn forecast(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    Query(query): Query<ForecastQuery>,
) -> Result<Json<Vec<samu_store::core::api::ForecastSection>>, Failure> {
    let store = state.account(&session.account)?;
    let (objects, settings, appointments) = {
        let store = lock(&store)?;
        // A restricted session sees no appointments (ADR 0095 §2).
        let appointments = if session.grant.is_some() {
            Vec::new()
        } else {
            store.appointments()?
        };
        (
            seen_snapshot(&session, &store)?,
            store.settings()?,
            appointments,
        )
    };
    Ok(Json(views::forecast(
        &objects,
        &appointments,
        &settings,
        Timestamp::now(),
        query.scope,
    )?))
}

#[derive(Debug, Deserialize)]
pub(crate) struct AlertsQuery {
    from: Option<Timestamp>,
    to: Option<Timestamp>,
    #[serde(default = "yes")]
    tasks: bool,
    #[serde(default = "yes")]
    appointments: bool,
}

impl AlertsQuery {
    pub(crate) fn view(&self) -> views::AlertsQuery {
        views::AlertsQuery {
            from: self.from,
            to: self.to,
            wanted: samu_store::core::reminders::Wanted {
                tasks: self.tasks,
                appointments: self.appointments,
            },
        }
    }
}

const fn yes() -> bool {
    true
}

/// `GET /alarms`: the reminders of tasks and appointments that go off in
/// a window (ADR 0101 §5). Scopes do not count, as for every reminder.
async fn alerts(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    Query(query): Query<AlertsQuery>,
) -> Result<Json<Vec<samu_store::core::api::Alert>>, Failure> {
    let store = state.account(&session.account)?;
    let (objects, settings, appointments) = {
        let store = lock(&store)?;
        (
            seen_snapshot(&session, &store)?,
            store.settings()?,
            store.appointments()?,
        )
    };
    Ok(Json(views::alerts(
        &objects,
        &appointments,
        &settings,
        Timestamp::now(),
        query.view(),
    )?))
}

#[derive(Debug, Deserialize)]
struct ClockReportQuery {
    /// The first day; today in the account's zone without it.
    from: Option<jiff::civil::Date>,
    /// The last day, included; `from` without it.
    to: Option<jiff::civil::Date>,
    #[serde(default)]
    by: samu_store::core::api::ClockReportBy,
    /// Only clocks on what this scope shows (ADR 0035 §5).
    scope: Option<Id>,
}

/// `GET /clocks/report`: the time clocked over days, as samu-core computes
/// it in the account's zone (ADR 0046 §5).
async fn clock_report(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    Query(query): Query<ClockReportQuery>,
) -> Result<Json<samu_store::core::api::ClockReport>, Failure> {
    let (objects, settings) = read(&state, &session)?;
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
    )?))
}

#[derive(Debug, Deserialize)]
struct ReportQuery {
    from: Option<jiff::civil::Date>,
    to: Option<jiff::civil::Date>,
    #[serde(default)]
    by: samu_store::core::api::ClockReportBy,
    scope: Option<Id>,
    /// The filter, the search language with `autor:` (ADR 0080 §2).
    #[serde(default)]
    q: String,
    /// `done,worked,time`; all without it.
    parts: Option<String>,
    #[serde(default)]
    dropped: bool,
}

impl ReportQuery {
    fn view(self) -> Result<views::ReportQuery, ViewError> {
        Ok(views::ReportQuery {
            from: self.from,
            to: self.to,
            by: self.by,
            scope: self.scope,
            q: self.q,
            parts: views::report_parts(self.parts.as_deref())?,
            dropped: self.dropped,
        })
    }
}

/// `GET /report`: done, worked on and time over days (ADR 0080 §4). The
/// logbook is read from the span's start until now, so a completion
/// written later for the span still tells its author.
async fn report(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    Query(query): Query<ReportQuery>,
) -> Result<Json<samu_store::core::api::Report>, Failure> {
    let now = Timestamp::now();
    let query = query.view()?;
    let (objects, settings) = read(&state, &session)?;
    let (start, end) = views::report_span(&settings, now, &query)?;
    let (log, stamps) = {
        let store = state.account(&session.account)?;
        let store = lock(&store)?;
        let mut log = store.logbook_between(start, now.max(end))?;
        let mut stamps = store.stamps_between(start, end)?;
        // Only what the session sees (ADR 0095 §2); the objects are already.
        if session.grant.is_some() {
            let seen = seen_ids(&objects);
            log.retain(|entry| seen.contains(&entry.object));
            stamps.retain(|stamp| seen.contains(&stamp.object));
        }
        (log, stamps)
    };
    let agents: std::collections::HashSet<String> = state
        .server()?
        .agent_devices(&session.account)?
        .into_iter()
        .collect();
    let events = samu_store::core::report::Events {
        log: &log,
        comments: &objects.comments,
        stamps: &stamps,
        agent_devices: &agents,
        account: &session.account,
    };
    Ok(Json(views::report(
        &objects, &settings, now, &query, &events,
    )?))
}

/// The ids of objects in a snapshot that something can be logged on.
fn seen_ids(objects: &OwnedSnapshot) -> std::collections::HashSet<Id> {
    let mut ids = std::collections::HashSet::new();
    ids.extend(objects.folders.iter().map(|o| o.id));
    ids.extend(objects.projects.iter().map(|o| o.id));
    ids.extend(objects.tasks.iter().map(|o| o.id));
    ids.extend(objects.tags.iter().map(|o| o.id));
    ids.extend(objects.links.iter().map(|o| o.id));
    ids.extend(objects.clocks.iter().map(|o| o.id));
    ids.extend(objects.reminders.iter().map(|o| o.id));
    ids.extend(objects.comments.iter().map(|o| o.id));
    ids.extend(objects.attachments.iter().map(|o| o.id));
    ids.extend(objects.habit_entries.iter().map(|o| o.id));
    ids
}

/// `POST /render`: notes and comments as document trees (ADR 0058 §4),
/// each in the format it looks like or the one asked for, with `task` links
/// looked up in the account: `samu:` by id, `id:` by `org_id`, then by id.
async fn render(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    Json(request): Json<RenderRequest>,
) -> Result<Json<RenderResponse>, Failure> {
    let mut documents: Vec<_> = request
        .texts
        .iter()
        .map(|text| samu_store::core::render::render_text(&text.text, text.format))
        .collect();
    // The account is read only when a link asks for a task.
    if documents.iter().any(|d| views::has_task_link(&d.blocks)) {
        let (objects, _) = read(&state, &session)?;
        views::resolve_task_links(&mut documents, &objects);
    }
    Ok(Json(RenderResponse { documents }))
}

/// `POST /capture`: parse a line of quick capture, resolve its names and
/// create the task, new tags first and each tag as an `add` (ADR 0027).
/// With `preview`, the same answer and nothing written.
async fn capture(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    Json(request): Json<CaptureRequest>,
) -> Result<Json<CaptureResponse>, Failure> {
    let store = state.account(&session.account)?;
    let mut store = lock(&store)?;
    let (objects, settings) = (seen_snapshot(&session, &store)?, store.settings()?);
    let placeholder = Hlc {
        wall: 0,
        counter: 0,
        device: session.device.clone(),
    };
    let run = |request: &CaptureRequest| {
        views::capture(
            &objects,
            &settings,
            request,
            Timestamp::now(),
            &placeholder,
            &mut Id::new,
        )
        .map_err(Failure::from)
    };
    let views::Capture {
        mut answer,
        changes,
    } = run(&request)?;
    // A preview makes no changes; for a restricted session the grant is
    // checked against the ones writing would make, so a preview is refused
    // like the capture itself.
    if let Some(reach) = session_reach(&session, &store)? {
        let writing = if request.preview {
            let mut writing = request.clone();
            writing.preview = false;
            run(&writing)?.changes
        } else {
            changes.clone()
        };
        check_capture(reach, &objects, &writing)?;
    }
    if request.preview {
        return Ok(Json(answer));
    }
    write_capture(&session, &mut store, changes)?;
    if let Some(stored) = store.get(answer.task.id)?
        && let AnyObject::Task(stored) = AnyObject::from(stored)
    {
        answer.task = stored;
    }
    answer.created = request.task.is_none();
    Ok(Json(answer))
}

/// `POST /capture/lines`: several lines of quick capture, an indented one
/// under the line above it, written all or none (ADR 0129).
async fn capture_lines(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    Json(request): Json<CaptureLinesRequest>,
) -> Result<Json<CaptureLinesResponse>, Failure> {
    let store = state.account(&session.account)?;
    let mut store = lock(&store)?;
    let (objects, settings) = (seen_snapshot(&session, &store)?, store.settings()?);
    let placeholder = Hlc {
        wall: 0,
        counter: 0,
        device: session.device.clone(),
    };
    // The changes writing would make, also for a preview: a preview is
    // refused like the capture itself.
    let mut writing = request.clone();
    writing.preview = false;
    let views::CaptureLines {
        mut answer,
        changes,
    } = views::capture_lines(
        &objects,
        &settings,
        &writing,
        Timestamp::now(),
        &placeholder,
        &mut Id::new,
    )?;
    if let Some(reach) = session_reach(&session, &store)? {
        check_capture(reach, &objects, &changes)?;
    }
    if request.preview {
        return Ok(Json(answer));
    }
    write_capture(&session, &mut store, changes)?;
    for made in &mut answer.tasks {
        if let Some(stored) = store.get(made.task.id)?
            && let AnyObject::Task(stored) = AnyObject::from(stored)
        {
            made.task = stored;
        }
    }
    Ok(Json(answer))
}

/// A restricted session captures only into its grant: not into the inbox,
/// never with a new tag (ADR 0095 §4). The changes build on each other, so
/// they are checked against what the earlier ones make.
fn check_capture(
    mut reach: samu_store::core::grant::Reach,
    objects: &OwnedSnapshot,
    changes: &[Change],
) -> Result<(), Failure> {
    let mut laid = (*objects).clone();
    for change in changes {
        let Some(id) = crate::grant::object_of(change) else {
            return Err(crate::grant::forbidden(crate::grant::OUTSIDE));
        };
        let before = laid.get(id);
        samu_store::core::overlay::apply(&mut laid, change)
            .map_err(|e| Failure::internal(e.to_string()))?;
        let after = laid.get(id);
        if !after.is_some_and(|after| reach.may_write(before.as_ref(), &after)) {
            return Err(crate::grant::forbidden(format!(
                "a capture there is {}",
                crate::grant::OUTSIDE
            )));
        }
        if let Some(after) = laid.get(id) {
            reach.admit(&after);
        }
    }
    Ok(())
}

/// Write a capture's changes, stamped by the server like
/// `POST /changes?stamp=server`.
fn write_capture(
    session: &Session,
    store: &mut AccountStore,
    changes: Vec<Change>,
) -> Result<(), Failure> {
    for change in changes {
        let now = Timestamp::now();
        let seen = store.observed()?;
        let change = restamp(
            change,
            next_stamp(&session.device, None, seen.as_ref(), now),
        );
        match store.apply(&change, &session.author(), now)? {
            ChangeOutcome::Applied { .. } => {}
            other => {
                return Err(Failure::internal(format!("capture not applied: {other:?}")));
            }
        }
    }
    Ok(())
}

/// Why an edit of an appointment makes no changes, as an answer: 404 for
/// no such appointment, 422 for the rest.
pub(crate) fn edit_failure(e: &samu_store::core::calendar::edit::EditError) -> Failure {
    use samu_store::core::calendar::edit::EditError;
    match e {
        EditError::NoAppointment(_) => Failure::not_found(e.to_string()),
        _ => unprocessable(e.to_string()),
    }
}

/// `POST /appointments/{id}/edit`: an edit of an appointment or of one
/// occurrence, turned into changes by samu-core and applied all or none
/// (ADR 0068 §5). The writer takes them back to the calendar at once.
async fn edit_appointment(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    UrlPath(id): UrlPath<String>,
    Json(edit): Json<samu_store::core::api::AppointmentEdit>,
) -> Result<Json<samu_store::core::api::AppointmentEditResponse>, Failure> {
    let id: Id = id
        .parse()
        .map_err(|_| Failure::bad_request(format!("{id:?} is not an id")))?;
    let store = state.account(&session.account)?;
    let mut store = lock(&store)?;
    let (appointments, objects, zone) = (
        store.appointments()?,
        store.snapshot()?,
        store.settings()?.zone,
    );
    let now = Timestamp::now();
    let drafts = samu_store::core::calendar::edit::edit_appointment(
        &appointments,
        &objects.calendars,
        id,
        &edit,
        &zone,
        now,
        Id::new,
    )
    .map_err(|e| edit_failure(&e))?;
    let seen = store.observed()?;
    let mut own: Option<Hlc> = None;
    let changes: Vec<Change> = drafts
        .into_iter()
        .map(|draft| {
            let stamp = next_stamp(&session.device, own.as_ref(), seen.as_ref(), now);
            own = Some(stamp.clone());
            draft.stamped(stamp)
        })
        .collect();
    let outcomes = match store.apply_all(&changes, &session.author(), now)? {
        Ok(outcomes) => outcomes,
        Err((index, ChangeOutcome::Rejected { reason })) => {
            return Err(unprocessable(format!("change {index}: {reason}")));
        }
        Err((index, outcome)) => {
            return Err(Failure::internal(format!("change {index}: {outcome:?}")));
        }
    };
    drop(store);
    crate::writer::kick(&state, &session.account);
    Ok(Json(samu_store::core::api::AppointmentEditResponse {
        results: changes
            .into_iter()
            .zip(outcomes)
            .map(|(change, outcome)| samu_store::core::api::EditResult { change, outcome })
            .collect(),
    }))
}

/// A request that is well formed but names what cannot be (422).
fn unprocessable(message: String) -> Failure {
    Failure::status(
        StatusCode::UNPROCESSABLE_ENTITY,
        ErrorCode::BadRequest,
        message,
    )
}

#[derive(Debug, Deserialize)]
struct ProjectsQuery {
    /// Only stuck projects.
    #[serde(default)]
    stuck: bool,
    /// Only projects due for review (ADR 0088 §4).
    #[serde(default)]
    review: bool,
    /// Completed and dropped projects too.
    #[serde(default)]
    all: bool,
    /// Only the projects this scope shows (ADR 0035 §5).
    scope: Option<Id>,
    /// Also what another own scope hides, marked (ADR 0124 §2).
    #[serde(default)]
    elsewhere: bool,
}

/// `GET /available/projects`: projects with their computed status, from
/// the same evaluation as `GET /available`, in project order.
async fn available_projects(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    Query(query): Query<ProjectsQuery>,
) -> Result<Json<Vec<ProjectAvailability>>, Failure> {
    let (objects, settings) = read(&state, &session)?;
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
    )?))
}

/// `GET /scopes/scheduled`: what the schedules of the scopes say now
/// (ADR 0043 §7), computed in samu-core; for clients without it.
async fn scheduled_scope(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
) -> Result<Json<samu_store::core::api::ScheduledScope>, Failure> {
    let (objects, settings) = read(&state, &session)?;
    Ok(Json(views::scheduled(
        &objects,
        &settings,
        Timestamp::now(),
    )))
}

#[derive(Debug, Deserialize)]
struct FoldersQuery {
    /// Only the folders this scope shows (ADR 0035 §5).
    scope: Option<Id>,
}

/// `GET /available/folders`: the folders the views show (ADR 0037 §4).
async fn available_folders(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    Query(query): Query<FoldersQuery>,
) -> Result<Json<Vec<Folder>>, Failure> {
    let (objects, settings) = read(&state, &session)?;
    Ok(Json(views::folders(
        &objects,
        &settings,
        Timestamp::now(),
        query.scope,
    )?))
}

/// More objects than this in one notice become a reset (catastr ADR 0095 §3).
const NOTICE_LIMIT: usize = 500;

/// What changed after `after`, as a notice or a reset.
/// For a restricted session, what it does not see now is gone, as in
/// `GET /changes` (ADR 0095 §5).
fn notice(
    store: &AccountStore,
    after: u64,
    grant: Option<&samu_store::core::api::Grant>,
) -> Result<Option<Event>, Failure> {
    let seq = store.seq()?;
    if seq <= after {
        return Ok(None);
    }
    let changed = store.changed_after(after)?;
    if changed.len() > NOTICE_LIMIT {
        return Ok(Some(reset_event(seq)));
    }
    let mut notice = ChangeNotice {
        seq,
        ..ChangeNotice::default()
    };
    let unseen: std::collections::HashSet<Id> = match grant {
        Some(grant) => {
            let reach = crate::grant::reach(store, grant)?;
            store
                .live_after(after)?
                .into_iter()
                .filter_map(|object| {
                    let id = object.id();
                    (!reach.sees(&AnyObject::from(object))).then_some(id)
                })
                .collect()
        }
        None => std::collections::HashSet::new(),
    };
    for c in changed {
        if c.deleted || unseen.contains(&c.id) {
            notice.gone.push(c.id);
        } else if c.created {
            notice.added.push(c.id);
        } else {
            notice.changed.push(c.id);
        }
    }
    let data = serde_json::to_string(&notice).map_err(|e| Failure::internal(e.to_string()))?;
    Ok(Some(
        Event::default()
            .event("changed")
            .id(seq.to_string())
            .data(data),
    ))
}

fn reset_event(seq: u64) -> Event {
    let data = serde_json::to_string(&ResetNotice { seq }).unwrap_or_default();
    Event::default()
        .event("reset")
        .id(seq.to_string())
        .data(data)
}

/// `GET /events`: SSE like catastr ADR 0095. The account's change number
/// is asked once a second, only while someone listens. Reconnecting with a
/// `Last-Event-ID` behind the current number gets a reset, not a replay.
async fn events(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    headers: HeaderMap,
) -> Result<Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>>, Failure> {
    let store = state.account(&session.account)?;
    let current = lock(&store)?.seq()?;
    let last: Option<u64> = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok());
    // Without a first event the head of the answer waits for one, and a
    // client sees nothing until the first change. A comment, which
    // `EventSource` ignores, sends it at once.
    let first = Some(last.filter(|last| *last < current).map_or_else(
        || Event::default().comment("connected"),
        |_| reset_event(current),
    ));
    let grant = session.grant.clone();
    let account = session.account.clone();
    let stream = futures_util::stream::unfold(
        (store, current, first),
        move |(store, mut seen, mut pending)| {
            let grant = grant.clone();
            let state = state.clone();
            let account = account.clone();
            async move {
                loop {
                    if let Some(event) = pending.take() {
                        return Some((Ok(event), (store, seen, None)));
                    }
                    tokio::time::sleep(POLL).await;
                    // The stream holds the account open; a deletion waits
                    // for it to let go.
                    if state.is_closing(&account) {
                        return None;
                    }
                    let Ok(guard) = store.lock() else {
                        return None;
                    };
                    let found = notice(&guard, seen, grant.as_ref()).ok().flatten();
                    if found.is_some() {
                        seen = guard.seq().unwrap_or(seen);
                    }
                    drop(guard);
                    pending = found;
                }
            }
        },
    );
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// How often `/events` asks for the change number.
const POLL: std::time::Duration = std::time::Duration::from_secs(1);

/// The whole API. Every route here is in `samu_api::ROUTES`, and a test
/// holds the two against each other.
pub(crate) fn router(state: Arc<AppState>) -> Router {
    let signed_in = Router::new()
        .route("/logout", post(logout))
        .route("/session", get(session))
        .route("/changes", post(post_changes).get(get_changes))
        .route("/objects/{id}", get(get_object))
        .route("/objects/{id}/logbook", get(get_logbook))
        .route("/logbook", get(get_account_logbook))
        .route("/objects/{id}/delete", post(delete_object))
        .route("/objects/{id}/to-project", post(to_project))
        .route("/objects/{id}/close", post(close_task))
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
        .route("/capture/lines", post(capture_lines))
        .route("/render", post(render))
        .route(
            "/calendars/{id}/source",
            axum::routing::put(crate::calendar::put_source),
        )
        .route(
            "/calendars/{id}/refresh",
            post(crate::calendar::post_refresh),
        )
        .route("/appointments", get(crate::calendar::appointments))
        .route("/appointments/{id}/edit", post(edit_appointment))
        .route("/calendars/discover", post(crate::calendar::discover))
        .route(
            "/calendars/{id}/discover",
            post(crate::calendar::discover_from_calendar),
        )
        .route(
            "/calendars/{id}/import",
            post(crate::import::import).layer(axum::extract::DefaultBodyLimit::max(
                crate::import::MAX_IMPORT_BYTES,
            )),
        )
        .route("/events", get(events))
        .route("/export", get(get_export))
        .route("/hooks", get(crate::hooks::list).post(crate::hooks::change))
        .route("/pair/code", post(pair_code))
        .route("/sessions", get(list_sessions))
        .route("/sessions/{id}/end", post(end_session))
        .route("/account/delete", post(crate::register::delete_own))
        .route("/demo", post(crate::register::add_demo))
        .route(
            "/blobs/{sha256}",
            get(crate::blobs::get).put(crate::blobs::put),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_session,
        ));
    Router::new()
        // The web UI, without a session (ADR 0025 §2).
        .route("/", get(crate::web::server_index))
        .route("/o/{id}", get(crate::web::server_object_page))
        .route("/v/{view}", get(crate::web::server_view_page))
        .route("/{name}", get(crate::web::root_file))
        .route("/lib/{name}", get(crate::web::lib_file))
        .route("/health", get(health))
        .route("/login", post(login))
        .route("/pair", post(pair))
        .route(
            "/register",
            get(crate::register::info).post(crate::register::register),
        )
        // No session: the signature is the sign-in (ADR 0022 §4).
        .route(
            "/hooks/{name}",
            post(crate::hooks::deliver).layer(axum::extract::DefaultBodyLimit::max(
                samu_store::core::hook::INDEX_BODY_LIMIT,
            )),
        )
        .merge(signed_in)
        .with_state(state)
        .layer(tower_http::compression::CompressionLayer::new().compress_when(Text))
}

/// What is sent gzipped to a client that takes it (ADR 0065 §6): JSON and
/// the files of the web UI above a size worth it. Not `/events`, which must
/// leave line by line, and not blobs, which are images and sound.
#[derive(Clone, Copy)]
pub(crate) struct Text;

impl tower_http::compression::Predicate for Text {
    fn should_compress<B>(&self, response: &axum::http::Response<B>) -> bool
    where
        B: axum::body::HttpBody,
    {
        let header = |name| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
        };
        let kind = header(axum::http::header::CONTENT_TYPE).unwrap_or_default();
        let text = kind.starts_with("application/json")
            || kind.starts_with("image/svg+xml")
            || (kind.starts_with("text/") && !kind.starts_with("text/event-stream"));
        // A blob of such a type is still a file someone put there; every
        // blob is answered with its own policy. The web UI has a policy
        // too, another one, and is text to compress.
        let blob =
            header(axum::http::header::CONTENT_SECURITY_POLICY) == Some(crate::blobs::BLOB_POLICY);
        // What is still coming in pieces has no size yet and is taken.
        let long = response
            .body()
            .size_hint()
            .exact()
            .is_none_or(|length| length >= 1024);
        text && !blob && long
    }
}
