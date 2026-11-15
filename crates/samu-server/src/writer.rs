//! Writing appointments back to their CalDAV server (ADR 0068 §6–§9).
//!
//! A change a client made goes into the account's queue in the same
//! transaction (samu-store). Right after such a change, and once a minute
//! for whatever is due again, the writer takes each resource: it reads what
//! is there, lays samu's appointments over it with samu-core, and puts it
//! back with `If-Match`. Nothing here holds an account's lock across a
//! request.

use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;
use reqwest::{StatusCode, Url};
use samu_store::WriteJob;
use samu_store::core::api::Id;
use samu_store::core::calendar::write::{new_resource, patch_resource};
use samu_store::core::calendar::{Read, Resource, read_resources};

use crate::app::{AppState, Failure, lock};
use crate::calendar::say;

/// How often the loop looks for what is due again.
const TICK: Duration = Duration::from_secs(60);

/// What one write came to.
#[derive(Debug)]
pub(crate) enum Wrote {
    /// Written at this href, with this ETag if the server told.
    Put { href: String, etag: Option<String> },
    /// Deleted, or never there.
    Deleted,
    /// Changed elsewhere since samu read it: what is there now.
    Crossed(Read),
    /// Gone over there while samu changed it.
    Gone,
    /// Try again later.
    Retry(String),
    /// Give up: the server will not take it.
    Fail(String),
}

/// The loop, for as long as the server runs.
pub(crate) async fn run(state: Arc<AppState>) {
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tick.tick().await;
        match accounts(&state) {
            Ok(accounts) => {
                for account in accounts {
                    write_due(&state, &account).await;
                }
            }
            Err(e) => say(&format!("writer: {}", e.message())),
        }
    }
}

fn accounts(state: &AppState) -> Result<Vec<String>, Failure> {
    Ok(state
        .server()?
        .accounts()?
        .into_iter()
        // An account closed without calendars stays closed (ADR 0099).
        .filter(|a| !a.disabled && state.has_calendar_work(&a.name))
        .map(|a| a.name)
        .collect())
}

/// Writes now what is due in an account, in the background: after a
/// change a client made (ADR 0068 §6).
pub(crate) fn kick(state: &Arc<AppState>, account: &str) {
    let state = state.clone();
    let account = account.to_string();
    tokio::spawn(async move { write_due(&state, &account).await });
}

/// Every resource of an account whose time has come, one after the other.
/// One writer at a time over all accounts, so two never write the same.
pub(crate) async fn write_due(state: &AppState, account: &str) {
    let _one = state.writing().lock().await;
    let due = match due(state, account) {
        Ok(due) => due,
        Err(e) => {
            say(&format!("writer {account}: {}", e.message()));
            return;
        }
    };
    for (calendar, uid) in due {
        // A crossed write is settled and tried again at once, a few times.
        for _ in 0..3 {
            match write_one(state, account, calendar, &uid).await {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => {
                    say(&format!(
                        "writer {account} {calendar} {uid}: {}",
                        e.message()
                    ));
                    break;
                }
            }
        }
    }
}

fn due(state: &AppState, account: &str) -> Result<Vec<(Id, String)>, Failure> {
    let store = state.account(account)?;
    let due = lock(&store)?.due_writes(Timestamp::now())?;
    Ok(due)
}

/// One resource; `true` when it crossed a change from elsewhere and is to
/// be written again.
async fn write_one(
    state: &AppState,
    account: &str,
    calendar: Id,
    uid: &str,
) -> Result<bool, Failure> {
    let store = state.account(account)?;
    let Some(key) = state.secret_key() else {
        lock(&store)?.write_failed(
            calendar,
            uid,
            "the server has no SAMU_SECRET_KEY",
            true,
            Timestamp::now(),
        )?;
        return Ok(false);
    };
    let job = match lock(&store)?.write_job(calendar, uid, key)? {
        None => return Ok(false),
        Some(Err(reason)) => {
            lock(&store)?.write_failed(calendar, uid, &reason, true, Timestamp::now())?;
            return Ok(false);
        }
        Some(Ok(job)) => job,
    };
    let outcome = state.fetcher().write(&job).await;
    let now = Timestamp::now();
    let mut store = lock(&store)?;
    let again = matches!(outcome, Wrote::Crossed(_));
    match outcome {
        Wrote::Put { href, etag } => {
            store.wrote(calendar, uid, job.version, Some(&href), etag.as_deref())?;
        }
        Wrote::Deleted => store.wrote(calendar, uid, job.version, None, None)?,
        Wrote::Crossed(read) => store.crossed(calendar, uid, &read, now)?,
        Wrote::Gone => store.gone(calendar, uid, now)?,
        Wrote::Retry(reason) => store.write_failed(calendar, uid, &reason, true, now)?,
        Wrote::Fail(reason) => store.write_failed(calendar, uid, &reason, false, now)?,
    }
    Ok(again)
}

/// Whether to try again: the server or the way there failed, or it is
/// busy; anything else it will not take as it stands.
fn passing(status: StatusCode) -> bool {
    status.is_server_error()
        || matches!(
            status,
            StatusCode::LOCKED | StatusCode::TOO_MANY_REQUESTS | StatusCode::REQUEST_TIMEOUT
        )
}

fn refused(status: StatusCode) -> Wrote {
    if passing(status) {
        Wrote::Retry(status.to_string())
    } else {
        Wrote::Fail(status.to_string())
    }
}

/// A resource's address: its href against the collection, on the
/// collection's server; an href elsewhere is none (ADR 0097 §3).
pub(crate) fn address(collection: &str, href: &str) -> Option<Url> {
    let base = Url::parse(collection).ok()?;
    let url = base.join(href).ok()?;
    (url.origin() == base.origin()).then_some(url)
}

/// Where a new resource goes: the UID, made safe for a path, in the
/// collection.
pub(crate) fn new_href(collection: &str, uid: &str) -> Option<(Url, String)> {
    let mut base = Url::parse(collection).ok()?;
    if !base.path().ends_with('/') {
        let path = format!("{}/", base.path());
        base.set_path(&path);
    }
    let name: String = uid
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let url = base.join(&format!("{name}.ics")).ok()?;
    let href = url.path().to_string();
    Some((url, href))
}

fn etag_of(response: &reqwest::Response) -> Option<String> {
    response
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

impl crate::calendar::Fetcher {
    /// One request to a resource, signed in like the collection.
    pub(crate) fn resource(
        &self,
        method: reqwest::Method,
        url: Url,
        job_source: &samu_store::StoredSource,
    ) -> reqwest::RequestBuilder {
        let mut request = self.client().request(method, url);
        if let Some(user) = &job_source.source.username {
            request = request.basic_auth(user, job_source.source.password.as_deref());
        }
        request
    }

    /// The resource as it is: its text and ETag, `None` when it is gone.
    async fn read_resource(
        &self,
        url: &Url,
        job: &WriteJob,
    ) -> Result<Option<(String, Option<String>)>, Wrote> {
        let response = self
            .resource(reqwest::Method::GET, url.clone(), &job.source)
            .send()
            .await
            .map_err(|e| Wrote::Retry(e.without_url().to_string()))?;
        match response.status() {
            StatusCode::NOT_FOUND | StatusCode::GONE => Ok(None),
            status if status.is_success() => {
                let etag = etag_of(&response);
                let text = response
                    .text()
                    .await
                    .map_err(|e| Wrote::Retry(e.without_url().to_string()))?;
                Ok(Some((text, etag)))
            }
            status => Err(refused(status)),
        }
    }

    fn crossed(&self, href: &str, text: String, etag: Option<String>, job: &WriteJob) -> Wrote {
        let resource = Resource {
            href: href.to_string(),
            etag,
            text,
        };
        Wrote::Crossed(read_resources(&[resource], &job.zone))
    }

    /// Writes one resource (ADR 0068 §6, §7, §9).
    pub(crate) async fn write(&self, job: &WriteJob) -> Wrote {
        let now = Timestamp::now();
        let collection = job.source.source.url.as_str();
        // Not to a host the server does not fetch from (ADR 0097 §3); a
        // move leaves the old one where it is then.
        if !self.allows(collection) {
            return Wrote::Fail(crate::calendar::HOST_NOT_ALLOWED.into());
        }
        let outcome = if job.appointments.is_empty() {
            self.delete(job).await
        } else if let Some(href) = &job.href {
            self.update(job, href, now).await
        } else {
            self.create(job, collection, now).await
        };
        // A move: what was in the old calendar goes there once it is here.
        if let (Wrote::Put { .. }, Some((source, href))) = (&outcome, &job.from)
            && self.allows(&source.source.url)
            && let Some(url) = address(&source.source.url, href)
        {
            match self
                .resource(reqwest::Method::DELETE, url, source)
                .send()
                .await
            {
                Ok(r) if r.status().is_success() || r.status() == StatusCode::NOT_FOUND => {}
                Ok(r) => say(&format!(
                    "writer: the moved {} stays in its old calendar: {}",
                    job.uid,
                    r.status()
                )),
                Err(e) => say(&format!(
                    "writer: the moved {} stays in its old calendar: {}",
                    job.uid,
                    e.without_url()
                )),
            }
        }
        outcome
    }

    async fn create(&self, job: &WriteJob, collection: &str, now: Timestamp) -> Wrote {
        let Some((url, href)) = new_href(collection, &job.uid) else {
            return Wrote::Fail("the calendar's address is none".into());
        };
        let text = match new_resource(&job.appointments, now) {
            Ok(text) => text,
            Err(e) => return Wrote::Fail(e.to_string()),
        };
        let response = self
            .resource(reqwest::Method::PUT, url.clone(), &job.source)
            .header(
                reqwest::header::CONTENT_TYPE,
                "text/calendar; charset=utf-8",
            )
            .header(reqwest::header::IF_NONE_MATCH, "*")
            .body(text)
            .send()
            .await;
        match response {
            Err(e) => Wrote::Retry(e.without_url().to_string()),
            Ok(r) if r.status().is_success() => {
                let etag = match etag_of(&r) {
                    Some(etag) => Some(etag),
                    None => self
                        .read_resource(&url, job)
                        .await
                        .ok()
                        .flatten()
                        .and_then(|(_, e)| e),
                };
                Wrote::Put { href, etag }
            }
            Ok(r) if r.status() == StatusCode::PRECONDITION_FAILED => {
                Wrote::Fail(format!("{href} is there already"))
            }
            Ok(r) => refused(r.status()),
        }
    }

    async fn update(&self, job: &WriteJob, href: &str, now: Timestamp) -> Wrote {
        let Some(url) = address(&job.source.source.url, href) else {
            return Wrote::Fail(format!("{href} is no address"));
        };
        let (text, etag) = match self.read_resource(&url, job).await {
            Err(wrote) => return wrote,
            Ok(None) => return Wrote::Gone,
            Ok(Some(found)) => found,
        };
        // Changed elsewhere since samu read it: settle first (§7).
        if let (Some(known), Some(there)) = (&job.etag, &etag)
            && known != there
        {
            return self.crossed(href, text, etag, job);
        }
        let patched = match patch_resource(&text, &job.appointments, &job.zone, now) {
            Ok(patched) => patched,
            Err(e) => return Wrote::Fail(e.to_string()),
        };
        let mut request = self
            .resource(reqwest::Method::PUT, url.clone(), &job.source)
            .header(
                reqwest::header::CONTENT_TYPE,
                "text/calendar; charset=utf-8",
            )
            .body(patched);
        if let Some(etag) = &etag {
            request = request.header(reqwest::header::IF_MATCH, etag);
        }
        match request.send().await {
            Err(e) => Wrote::Retry(e.without_url().to_string()),
            Ok(r) if r.status().is_success() => {
                let etag = match etag_of(&r) {
                    Some(etag) => Some(etag),
                    None => self
                        .read_resource(&url, job)
                        .await
                        .ok()
                        .flatten()
                        .and_then(|(_, e)| e),
                };
                Wrote::Put {
                    href: href.to_string(),
                    etag,
                }
            }
            Ok(r) if r.status() == StatusCode::PRECONDITION_FAILED => {
                match self.read_resource(&url, job).await {
                    Ok(Some((text, etag))) => self.crossed(href, text, etag, job),
                    Ok(None) => Wrote::Gone,
                    Err(wrote) => wrote,
                }
            }
            Ok(r) if r.status() == StatusCode::NOT_FOUND => Wrote::Gone,
            Ok(r) => refused(r.status()),
        }
    }

    async fn delete(&self, job: &WriteJob) -> Wrote {
        let Some(href) = &job.href else {
            // Made and deleted before it was ever written.
            return Wrote::Deleted;
        };
        let Some(url) = address(&job.source.source.url, href) else {
            return Wrote::Fail(format!("{href} is no address"));
        };
        let mut request = self.resource(reqwest::Method::DELETE, url.clone(), &job.source);
        if let Some(etag) = &job.etag {
            request = request.header(reqwest::header::IF_MATCH, etag);
        }
        match request.send().await {
            Err(e) => Wrote::Retry(e.without_url().to_string()),
            Ok(r) if r.status().is_success() || r.status() == StatusCode::NOT_FOUND => {
                Wrote::Deleted
            }
            Ok(r) if r.status() == StatusCode::PRECONDITION_FAILED => {
                match self.read_resource(&url, job).await {
                    Ok(Some((text, etag))) => self.crossed(href, text, etag, job),
                    Ok(None) => Wrote::Deleted,
                    Err(wrote) => wrote,
                }
            }
            Ok(r) => refused(r.status()),
        }
    }
}

#[cfg(test)]
mod address_tests {
    #![allow(clippy::unwrap_used, reason = "tests may unwrap")]

    use super::address;

    /// ADR 0097 §3: an href is a resource on the collection's server.
    #[test]
    fn an_href_stays_on_the_collections_server() {
        let collection = "https://cal.example/dav/familie/";
        assert_eq!(
            address(collection, "/dav/familie/a.ics").unwrap().as_str(),
            "https://cal.example/dav/familie/a.ics"
        );
        assert_eq!(
            address(collection, "b.ics").unwrap().as_str(),
            "https://cal.example/dav/familie/b.ics"
        );
        assert!(address(collection, "http://10.0.0.1/x.ics").is_none());
        assert!(address(collection, "//elsewhere.example/x.ics").is_none());
    }
}
