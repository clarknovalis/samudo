// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Fetching calendars (ADR 0047 §7): a loop asks every account once a
//! minute which calendars are due, fetches them outside the account's lock,
//! and lays what came over what is stored.

use std::sync::Arc;
use std::time::Duration;

use axum::Extension;
use axum::Json;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::StatusCode;
use futures_util::StreamExt;
use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use samu_store::core::api::{
    AppointmentInstance, Calendar, CalendarKind, CalendarSource, DiscoveredCalendar, ErrorCode, Id,
};
use samu_store::core::calendar::{KEEP_AFTER, KEEP_BEFORE, Resource, read_ics, read_resources};
use samu_store::{Fetched, Object, SecretKey, StoredSource};
use serde::Deserialize;

use crate::app::{AppState, Failure, Session, lock};
use crate::caldav;

/// A line on stderr that never takes the server down: whoever started it
/// may have read the first line and closed the pipe, and `eprintln!` panics
/// on a closed one.
pub(crate) fn say(line: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr().lock(), "{line}");
}

/// How often the loop asks which calendars are due.
const TICK: Duration = Duration::from_secs(60);
/// How many calendars are fetched at once, over all accounts.
const AT_ONCE: usize = 4;
/// How long one fetch may take.
const TIMEOUT: Duration = Duration::from_secs(30);
/// More than this is no calendar anyone keeps.
const MAX_BYTES: usize = 20 * 1024 * 1024;

/// Fetches over HTTP; one client, kept for its connections.
#[derive(Clone)]
pub(crate) struct Fetcher {
    client: reqwest::Client,
    /// For finding calendars: follows no redirect by itself, because
    /// reqwest turns a redirected PROPFIND into a GET.
    manual: reqwest::Client,
    /// The hosts it may fetch from; any without (ADR 0097 §3).
    hosts: Option<Arc<[String]>>,
}

/// A calendar address on a host the server does not fetch from.
pub(crate) const HOST_NOT_ALLOWED: &str = "this server fetches no calendars from that host";

/// Whether `url` is on one of `hosts`; any is without a list.
fn allowed(hosts: Option<&[String]>, url: &reqwest::Url) -> bool {
    hosts.is_none_or(|hosts| {
        url.host_str()
            .is_some_and(|host| hosts.iter().any(|h| h.eq_ignore_ascii_case(host)))
    })
}

impl Fetcher {
    /// The client that follows redirects, for the writer.
    pub(crate) const fn client(&self) -> &reqwest::Client {
        &self.client
    }

    pub(crate) fn new() -> anyhow::Result<Self> {
        Self::on_hosts(None)
    }

    /// Only for these hosts, when given (ADR 0097 §3): also a redirect
    /// to another is not followed.
    pub(crate) fn on_hosts(hosts: Option<Vec<String>>) -> anyhow::Result<Self> {
        let hosts: Option<Arc<[String]>> = hosts.map(Into::into);
        let within = hosts.clone();
        let policy = reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() > HOPS {
                attempt.error(format!("more than {HOPS} redirects"))
            } else if !allowed(within.as_deref(), attempt.url()) {
                attempt.error(HOST_NOT_ALLOWED)
            } else {
                attempt.follow()
            }
        });
        let client = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(concat!("samu/", env!("CARGO_PKG_VERSION")))
            .redirect(policy)
            .build()?;
        let manual = reqwest::Client::builder()
            .timeout(TIMEOUT)
            .user_agent(concat!("samu/", env!("CARGO_PKG_VERSION")))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            client,
            manual,
            hosts,
        })
    }

    /// Whether the server may fetch from this address (ADR 0097 §3);
    /// `webcal://` counts as `https://`.
    pub(crate) fn allows(&self, address: &str) -> bool {
        let address = address.trim();
        let address = match address.strip_prefix("webcal://") {
            Some(rest) => format!("https://{rest}"),
            None => address.to_string(),
        };
        reqwest::Url::parse(&address).is_ok_and(|url| self.allows_url(&url))
    }

    fn allows_url(&self, url: &reqwest::Url) -> bool {
        allowed(self.hosts.as_deref(), url)
    }

    /// One fetch of an iCalendar address, conditional on what came last.
    /// An error never names the address: it may carry the secret.
    async fn fetch(&self, stored: &StoredSource, zone: &TimeZone) -> Fetched {
        if !self.allows(&stored.source.url) {
            return Fetched::Failed(HOST_NOT_ALLOWED.into());
        }
        let url = match stored.source.url.strip_prefix("webcal://") {
            Some(rest) => format!("https://{rest}"),
            None => stored.source.url.clone(),
        };
        let mut request = self.client.get(url);
        if let Some(user) = &stored.source.username {
            request = request.basic_auth(user, stored.source.password.as_deref());
        }
        if let Some(etag) = &stored.etag {
            request = request.header(reqwest::header::IF_NONE_MATCH, etag);
        }
        if let Some(modified) = &stored.last_modified {
            request = request.header(reqwest::header::IF_MODIFIED_SINCE, modified);
        }
        let mut response = match request.send().await {
            Ok(response) => response,
            Err(e) => return Fetched::Failed(e.without_url().to_string()),
        };
        let status = response.status();
        if status == StatusCode::NOT_MODIFIED {
            return Fetched::NotModified;
        }
        if !status.is_success() {
            return Fetched::Failed(status.to_string());
        }
        let header = |name: reqwest::header::HeaderName| {
            response
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        let etag = header(reqwest::header::ETAG);
        let last_modified = header(reqwest::header::LAST_MODIFIED);
        let body = match body_of(&mut response).await {
            Ok(body) => body,
            Err(reason) => return Fetched::Failed(reason),
        };
        match read_ics(&body, zone) {
            Ok(read) => Fetched::Read {
                read,
                etag,
                last_modified,
            },
            Err(e) => Fetched::Failed(e.to_string()),
        }
    }
}

impl Fetcher {
    /// One request of WebDAV with an XML body, signed in when the source
    /// has a user.
    fn dav(
        &self,
        method: &str,
        url: &str,
        depth: &str,
        body: String,
        stored: &StoredSource,
    ) -> reqwest::RequestBuilder {
        let method = reqwest::Method::from_bytes(method.as_bytes()).unwrap_or(reqwest::Method::GET);
        let mut request = self
            .client
            .request(method, url)
            .header("Depth", depth)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/xml; charset=utf-8",
            )
            .body(body);
        if let Some(user) = &stored.source.username {
            request = request.basic_auth(user, stored.source.password.as_deref());
        }
        request
    }

    /// A CalDAV collection (ADR 0047 §7): its ctag first, and when that is
    /// the one of the last fetch, nothing more; else every VEVENT with an
    /// occurrence in the window the server keeps, each with its href and
    /// ETag (ADR 0068 §6). The ctag is kept where an ICS keeps its ETag.
    /// Besides, whether samu may write there, when the server says.
    async fn fetch_caldav(
        &self,
        stored: &StoredSource,
        zone: &TimeZone,
    ) -> (Fetched, Option<bool>) {
        let url = stored.source.url.as_str();
        if !self.allows(url) {
            return (Fetched::Failed(HOST_NOT_ALLOWED.into()), None);
        }
        let mut writable = None;
        let version = match self
            .dav("PROPFIND", url, "0", caldav::PROPFIND.to_string(), stored)
            .send()
            .await
        {
            Err(e) => return (Fetched::Failed(e.without_url().to_string()), None),
            Ok(response) if response.status().as_u16() == 207 => {
                let mut response = response;
                match body_of(&mut response).await {
                    Ok(body) => {
                        writable = caldav::responses(&body)
                            .ok()
                            .and_then(|found| found.first().and_then(caldav::Found::writable));
                        caldav::version_of(&body)
                    }
                    Err(reason) => return (Fetched::Failed(reason), None),
                }
            }
            Ok(response)
                if matches!(
                    response.status(),
                    StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
                ) =>
            {
                return (Fetched::Failed(response.status().to_string()), None);
            }
            // A server without the property: fetched each time.
            Ok(_) => None,
        };
        if version.is_some() && version == stored.etag {
            return (Fetched::NotModified, writable);
        }
        let today = Timestamp::now().to_zoned(zone.clone()).date();
        let first = today.saturating_sub(jiff::ToSpan::days(KEEP_BEFORE));
        let last = today.saturating_add(jiff::ToSpan::days(KEEP_AFTER));
        let query = caldav::calendar_query(first, last);
        let mut response = match self.dav("REPORT", url, "1", query, stored).send().await {
            Ok(response) => response,
            Err(e) => return (Fetched::Failed(e.without_url().to_string()), writable),
        };
        if response.status().as_u16() != 207 {
            return (Fetched::Failed(response.status().to_string()), writable);
        }
        let body = match body_of(&mut response).await {
            Ok(body) => body,
            Err(reason) => return (Fetched::Failed(reason), writable),
        };
        let fetched = match caldav::responses(&body) {
            Ok(found) => {
                let resources: Vec<Resource> = found
                    .into_iter()
                    .filter_map(|f| {
                        Some(Resource {
                            text: f.data?,
                            href: f.href,
                            etag: f.etag,
                        })
                    })
                    .collect();
                Fetched::Read {
                    read: read_resources(&resources, zone),
                    etag: version,
                    last_modified: None,
                }
            }
            Err(reason) => Fetched::Failed(reason),
        };
        (fetched, writable)
    }
}

/// How many redirects one PROPFIND of discovery follows.
const HOPS: usize = 5;

impl Fetcher {
    /// A PROPFIND that follows redirects with its method kept: the address
    /// it ended at and the multistatus there. An error never names the
    /// address.
    async fn propfind(
        &self,
        start: reqwest::Url,
        depth: &str,
        body: &str,
        source: &CalendarSource,
    ) -> Result<(reqwest::Url, String), String> {
        let mut url = start;
        for _ in 0..=HOPS {
            if !self.allows_url(&url) {
                return Err(HOST_NOT_ALLOWED.into());
            }
            let method =
                reqwest::Method::from_bytes(b"PROPFIND").map_err(|e| format!("PROPFIND: {e}"))?;
            let mut request = self
                .manual
                .request(method, url.clone())
                .header("Depth", depth)
                .header(
                    reqwest::header::CONTENT_TYPE,
                    "application/xml; charset=utf-8",
                )
                .body(body.to_string());
            if let Some(user) = &source.username {
                request = request.basic_auth(user, source.password.as_deref());
            }
            let mut response = request
                .send()
                .await
                .map_err(|e| e.without_url().to_string())?;
            let status = response.status();
            if status.is_redirection() {
                let location = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| format!("{status} without a Location"))?;
                url = url
                    .join(location)
                    .map_err(|_| format!("{status} to an address that is none"))?;
                continue;
            }
            if status.as_u16() != 207 {
                return Err(status.to_string());
            }
            let body = body_of(&mut response).await?;
            return Ok((url, body));
        }
        Err(format!("more than {HOPS} redirects"))
    }

    /// The calendar collections that hold appointments, found from an
    /// address (ADR 0047 §8): `.well-known/caldav` when it names no path,
    /// then the principal, its calendar home, the collections in there. An
    /// address of a collection is that collection alone.
    pub(crate) async fn discover(
        &self,
        source: &CalendarSource,
    ) -> Result<Vec<DiscoveredCalendar>, String> {
        let given = reqwest::Url::parse(source.url.trim())
            .map_err(|_| "the address is not a URL".to_string())?;
        if !matches!(given.scheme(), "http" | "https") {
            return Err("the address must start with https://".into());
        }
        let mut starts = Vec::new();
        if matches!(given.path(), "" | "/")
            && let Ok(known) = given.join("/.well-known/caldav")
        {
            starts.push(known);
        }
        starts.push(given);
        let mut last_error = String::from("no CalDAV server there");
        for start in starts {
            match self.discover_from(start, source, false).await {
                Ok(found) => return Ok(found),
                Err(e) => last_error = e,
            }
        }
        Err(last_error)
    }

    /// The collections next to a calendar's own (ADR 0100 §1): from the
    /// root of its host over `.well-known/caldav`, else from the principal
    /// the collection names.
    pub(crate) async fn discover_around(
        &self,
        source: &CalendarSource,
    ) -> Result<Vec<DiscoveredCalendar>, String> {
        let given = reqwest::Url::parse(source.url.trim())
            .map_err(|_| "the address is not a URL".to_string())?;
        let mut last_error = String::from("no CalDAV server there");
        if let Ok(root) = given.join("/.well-known/caldav") {
            match self.discover_from(root, source, false).await {
                Ok(found) if !found.is_empty() => return Ok(found),
                Ok(_) => {}
                Err(e) => last_error = e,
            }
        }
        match self.discover_from(given, source, true).await {
            Ok(found) => Ok(found),
            Err(e) => Err(format!("{e}; before: {last_error}")),
        }
    }

    /// From one address; `around` goes on from a collection to its
    /// principal instead of stopping there.
    async fn discover_from(
        &self,
        start: reqwest::Url,
        source: &CalendarSource,
        around: bool,
    ) -> Result<Vec<DiscoveredCalendar>, String> {
        let (at, body) = self
            .propfind(start, "0", caldav::PROPFIND_START, source)
            .await?;
        let first = caldav::responses(&body)?
            .into_iter()
            .next()
            .ok_or("an empty answer")?;
        if first.calendar && !(around && first.principal.is_some()) {
            return Ok(collection(&at, &first).into_iter().collect());
        }
        let home = match (first.home, first.principal) {
            (Some(home), _) => home,
            (None, Some(principal)) => {
                let principal = at
                    .join(&principal)
                    .map_err(|_| "a principal that is no address".to_string())?;
                let (_, body) = self
                    .propfind(principal, "0", caldav::PROPFIND_START, source)
                    .await?;
                caldav::responses(&body)?
                    .into_iter()
                    .find_map(|f| f.home)
                    .ok_or("the principal names no calendar home")?
            }
            (None, None) => return Err("no CalDAV principal there".into()),
        };
        let home = at
            .join(&home)
            .map_err(|_| "a calendar home that is no address".to_string())?;
        let (at, body) = self
            .propfind(home, "1", caldav::PROPFIND_HOME, source)
            .await?;
        let mut out: Vec<DiscoveredCalendar> = caldav::responses(&body)?
            .iter()
            .filter_map(|found| collection(&at, found))
            .collect();
        out.dedup_by(|a, b| a.url == b.url);
        Ok(out)
    }
}

/// A found collection that holds appointments, its href made absolute
/// against the address that answered.
fn collection(at: &reqwest::Url, found: &caldav::Found) -> Option<DiscoveredCalendar> {
    if !found.holds_events() {
        return None;
    }
    let url = if found.href.is_empty() {
        at.clone()
    } else {
        at.join(&found.href).ok()?
    };
    let title = found.name.clone().unwrap_or_else(|| {
        url.path_segments()
            .and_then(|mut parts| parts.rfind(|p| !p.is_empty()).map(str::to_string))
            .unwrap_or_else(|| url.host_str().unwrap_or("Kalender").to_string())
    });
    Some(DiscoveredCalendar {
        url: url.to_string(),
        title,
        color: found.color.clone(),
        known: false,
    })
}

/// The body of an answer as text, at most [`MAX_BYTES`].
async fn body_of(response: &mut reqwest::Response) -> Result<String, String> {
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if body.len() + chunk.len() > MAX_BYTES {
                    return Err(format!("larger than {MAX_BYTES} bytes"));
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(e) => return Err(e.without_url().to_string()),
        }
    }
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// The loop, for as long as the server runs.
pub(crate) async fn run(state: Arc<AppState>) {
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        let due = match due(&state) {
            Ok(due) => due,
            Err(e) => {
                say(&format!("calendars: {}", e.message()));
                continue;
            }
        };
        futures_util::stream::iter(due)
            .for_each_concurrent(AT_ONCE, |(account, id)| {
                let state = state.clone();
                async move {
                    if let Err(e) = refresh(&state, &account, id).await {
                        say(&format!("calendar {id} of {account}: {}", e.message()));
                    }
                }
            })
            .await;
    }
}

/// Every due calendar of every active account.
pub(crate) fn due(state: &AppState) -> Result<Vec<(String, Id)>, Failure> {
    let accounts = state.server()?.accounts()?;
    let now = Timestamp::now();
    let mut out = Vec::new();
    // An account closed without calendars stays closed (ADR 0099).
    for account in accounts
        .into_iter()
        .filter(|a| !a.disabled && state.has_calendar_work(&a.name))
    {
        let store = state.account(&account.name)?;
        for id in lock(&store)?.due_calendars(now)? {
            out.push((account.name.clone(), id));
        }
    }
    Ok(out)
}

/// Fetches one calendar now and stores what came. `false` when it has no
/// source to fetch.
pub(crate) async fn refresh(state: &AppState, account: &str, id: Id) -> Result<bool, Failure> {
    let store = state.account(account)?;
    let (calendar, source, zone) = {
        let store = lock(&store)?;
        let Some(Object::Calendar(calendar)) = store.get(id)? else {
            return Err(Failure::not_found(format!("no calendar {id}")));
        };
        if !store.has_calendar_source(id)? {
            return Ok(false);
        }
        let source = match state.secret_key() {
            None => Err(NO_KEY.to_string()),
            Some(key) => match store.calendar_source(id, key) {
                Ok(Some(source)) => Ok(source),
                Ok(None) => return Ok(false),
                Err(_) => Err("the server's key does not open the source; set it again".into()),
            },
        };
        (calendar, source, store.settings()?.zone)
    };
    let (fetched, writable) = match (calendar.kind, source) {
        (_, Err(reason)) => (Fetched::Failed(reason), None),
        (CalendarKind::Ics, Ok(source)) => (state.fetcher().fetch(&source, &zone).await, None),
        (CalendarKind::Caldav, Ok(source)) => state.fetcher().fetch_caldav(&source, &zone).await,
        (CalendarKind::Unknown, Ok(_)) => (
            Fetched::Failed("a kind of calendar this server does not know".into()),
            None,
        ),
    };
    let mut store = lock(&store)?;
    let now = Timestamp::now();
    store.sync_calendar(id, fetched, now)?;
    if let Some(writable) = writable
        && writable != calendar.writable
    {
        store.set_writable(id, writable, now)?;
    }
    Ok(true)
}

pub(crate) const NO_KEY: &str = "the server has no SAMU_SECRET_KEY";

pub(crate) fn calendar_id(raw: &str) -> Result<Id, Failure> {
    raw.parse()
        .map_err(|_| Failure::bad_request(format!("{raw:?} is not an id")))
}

/// `PUT /calendars/{id}/source`: where a calendar comes from, sealed and
/// never answered back (ADR 0047 §3). The calendar is fetched at once.
pub(crate) async fn put_source(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    UrlPath(raw): UrlPath<String>,
    Json(source): Json<CalendarSource>,
) -> Result<StatusCode, Failure> {
    let id = calendar_id(&raw)?;
    if !state.fetcher().allows(&source.url) {
        return Err(Failure::bad_request(HOST_NOT_ALLOWED));
    }
    let Some(key) = state.secret_key() else {
        return Err(Failure::status(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::Internal,
            NO_KEY,
        ));
    };
    let store = state.account(&session.account)?;
    let source = {
        let store = lock(&store)?;
        with_credentials(&store, key, source)?
    };
    let outcome = lock(&store)?.set_calendar_source(id, &source, key, Timestamp::now())?;
    match outcome {
        Ok(()) => {}
        Err(reason) if reason.starts_with("no calendar") => {
            return Err(Failure::not_found(reason));
        }
        Err(reason) => return Err(Failure::bad_request(reason)),
    }
    let state = state.clone();
    let account = session.account.clone();
    tokio::spawn(async move {
        if let Err(e) = refresh(&state, &account, id).await {
            say(&format!("calendar {id} of {account}: {}", e.message()));
        }
    });
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /calendars/{id}/refresh`: fetch now and answer with the calendar
/// as it stands afterwards, its `error` included.
pub(crate) async fn post_refresh(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    UrlPath(raw): UrlPath<String>,
) -> Result<Json<Calendar>, Failure> {
    let id = calendar_id(&raw)?;
    if !refresh(&state, &session.account, id).await? {
        return Err(Failure::bad_request(format!(
            "calendar {id} has no source: PUT /calendars/{id}/source first"
        )));
    }
    let store = state.account(&session.account)?;
    match lock(&store)?.get(id)? {
        Some(Object::Calendar(calendar)) => Ok(Json(calendar)),
        _ => Err(Failure::not_found(format!("no calendar {id}"))),
    }
}

/// The source to keep: with `credentials_from`, user and password of that
/// CalDAV calendar on the same host, which never leave the server
/// (ADR 0100 §2). They are copied, not linked.
fn with_credentials(
    store: &samu_store::AccountStore,
    key: &SecretKey,
    source: CalendarSource,
) -> Result<CalendarSource, Failure> {
    let Some(from) = source.credentials_from else {
        return Ok(source);
    };
    if source.username.is_some() || source.password.is_some() {
        return Err(Failure::bad_request(
            "credentials_from takes user and password from the other calendar; give neither",
        ));
    }
    let stored = caldav_source(store, key, from)?;
    if !same_host(&stored.url, &source.url) {
        return Err(Failure::bad_request(format!(
            "calendar {from} is on another host; credentials go only to the host they are for"
        )));
    }
    Ok(CalendarSource {
        url: source.url,
        username: stored.username,
        password: stored.password,
        credentials_from: None,
    })
}

/// The opened source of a CalDAV calendar, or why there is none.
pub(crate) fn caldav_source(
    store: &samu_store::AccountStore,
    key: &SecretKey,
    id: Id,
) -> Result<CalendarSource, Failure> {
    let Some(Object::Calendar(calendar)) = store.get(id)? else {
        return Err(Failure::not_found(format!("no calendar {id}")));
    };
    if calendar.kind != CalendarKind::Caldav {
        return Err(Failure::bad_request(format!(
            "calendar {id} is no CalDAV calendar"
        )));
    }
    match store.calendar_source(id, key) {
        Ok(Some(stored)) => Ok(stored.source),
        Ok(None) => Err(Failure::bad_request(format!(
            "calendar {id} has no source: PUT /calendars/{id}/source first"
        ))),
        Err(_) => Err(Failure::bad_request(
            "the server's key does not open the source; set it again",
        )),
    }
}

/// Same host and port, so credentials go where they were given for.
fn same_host(a: &str, b: &str) -> bool {
    match (reqwest::Url::parse(a.trim()), reqwest::Url::parse(b.trim())) {
        (Ok(a), Ok(b)) => {
            a.host_str().map(str::to_ascii_lowercase) == b.host_str().map(str::to_ascii_lowercase)
                && a.port_or_known_default() == b.port_or_known_default()
        }
        _ => false,
    }
}

/// An address as compared for `known`: without a trailing slash.
fn address_key(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

/// Marks what a calendar of the account has as its source already
/// (ADR 0100 §1). Without the key nothing is marked.
fn mark_known(
    store: &samu_store::AccountStore,
    key: Option<&SecretKey>,
    found: &mut [DiscoveredCalendar],
) -> Result<(), Failure> {
    let Some(key) = key else {
        return Ok(());
    };
    let snapshot = store.snapshot()?;
    let known: Vec<String> = snapshot
        .calendars
        .iter()
        .filter_map(|c| store.calendar_source(c.id, key).ok().flatten())
        .map(|stored| address_key(&stored.source.url))
        .collect();
    for one in found {
        one.known = known.contains(&address_key(&one.url));
    }
    Ok(())
}

/// `POST /calendars/{id}/discover`: the collections on the server of a
/// CalDAV calendar, found with its stored source, those already
/// subscribed marked `known` (ADR 0100 §1).
pub(crate) async fn discover_from_calendar(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    UrlPath(raw): UrlPath<String>,
) -> Result<Json<Vec<DiscoveredCalendar>>, Failure> {
    let id = calendar_id(&raw)?;
    let Some(key) = state.secret_key() else {
        return Err(Failure::status(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::Internal,
            NO_KEY,
        ));
    };
    let store = state.account(&session.account)?;
    let source = caldav_source(&*lock(&store)?, key, id)?;
    if !state.fetcher().allows(&source.url) {
        return Err(Failure::bad_request(HOST_NOT_ALLOWED));
    }
    let mut found = state
        .fetcher()
        .discover_around(&source)
        .await
        .map_err(|reason| {
            Failure::status(
                StatusCode::BAD_GATEWAY,
                ErrorCode::BadRequest,
                format!("no calendars found: {reason}"),
            )
        })?;
    mark_known(&*lock(&store)?, Some(key), &mut found)?;
    Ok(Json(found))
}

/// `POST /calendars/discover`: the calendar collections at a CalDAV
/// address, with name and colour; makes nothing (ADR 0047 §8). The
/// password is used for this one call and kept nowhere.
pub(crate) async fn discover(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    Json(source): Json<CalendarSource>,
) -> Result<Json<Vec<DiscoveredCalendar>>, Failure> {
    if !state.fetcher().allows(&source.url) {
        return Err(Failure::bad_request(HOST_NOT_ALLOWED));
    }
    let mut found = state.fetcher().discover(&source).await.map_err(|reason| {
        Failure::status(
            StatusCode::BAD_GATEWAY,
            ErrorCode::BadRequest,
            format!("no calendars found: {reason}"),
        )
    })?;
    let store = state.account(&session.account)?;
    mark_known(&*lock(&store)?, state.secret_key(), &mut found)?;
    Ok(Json(found))
}

#[derive(Debug, Deserialize)]
pub(crate) struct AppointmentsQuery {
    /// The first day, in the account's zone; today by default.
    from: Option<Date>,
    /// The last day, included; `from` by default.
    to: Option<Date>,
    /// Through this scope: its `calendars` (ADR 0047 §12).
    scope: Option<Id>,
}

/// `GET /appointments`: the occurrences of some days, unfolded by
/// samu-core (ADR 0047 §5, ADR 0048).
pub(crate) async fn appointments(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    Query(query): Query<AppointmentsQuery>,
) -> Result<Json<Vec<AppointmentInstance>>, Failure> {
    let store = state.account(&session.account)?;
    let (snapshot, appointments, settings) = {
        let store = lock(&store)?;
        (store.snapshot()?, store.appointments()?, store.settings()?)
    };
    Ok(Json(samu_store::core::views::appointments(
        &snapshot,
        &appointments,
        &settings,
        Timestamp::now(),
        (query.from, query.to),
        query.scope,
    )?))
}

/// The key from `SAMU_SECRET_KEY`: 32 bytes in Base64.
pub(crate) fn parse_key(text: &str) -> anyhow::Result<SecretKey> {
    use base64ct::{Base64, Encoding};
    let bytes = Base64::decode_vec(text.trim())
        .map_err(|_| anyhow::anyhow!("SAMU_SECRET_KEY is not Base64"))?;
    let bytes: [u8; 32] = bytes.try_into().map_err(|_| {
        anyhow::anyhow!("SAMU_SECRET_KEY must be 32 bytes, like `openssl rand -base64 32`")
    })?;
    Ok(SecretKey::new(bytes))
}
