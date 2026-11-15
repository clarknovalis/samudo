//! `POST /calendars/{id}/import` (ADR 0100 §4): an iCalendar file into a
//! writable CalDAV calendar. samu-core cuts it into one resource per UID,
//! each goes to the collection as it came (VALARM, ATTENDEE and the rest
//! included); a UID the calendar holds replaces its resource. Then the
//! calendar is fetched, so the appointments come over `/changes` like any
//! fetched one. Nothing here holds the account's lock across a request.

use std::sync::Arc;

use axum::Extension;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::StatusCode;
use jiff::Timestamp;
use samu_store::core::api::{CalendarImport, ErrorCode, ImportIssue};
use samu_store::core::calendar::write::split_by_uid;
use samu_store::{KnownResource, StoredSource};
use serde::Deserialize;

use crate::app::{AppState, Failure, Session, lock};
use crate::calendar::{HOST_NOT_ALLOWED, NO_KEY, caldav_source, calendar_id, refresh, say};
use crate::writer::{address, new_href};

/// The largest file taken: a calendar of many years fits.
pub(crate) const MAX_IMPORT_BYTES: usize = 10 * 1024 * 1024;

#[derive(Debug, Deserialize)]
pub(crate) struct ImportQuery {
    /// The file's name, for the logbook.
    name: Option<String>,
}

/// What one resource came to.
enum Put {
    Created,
    Replaced,
    Failed(String),
}

pub(crate) async fn import(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    UrlPath(raw): UrlPath<String>,
    Query(query): Query<ImportQuery>,
    body: Bytes,
) -> Result<Json<CalendarImport>, Failure> {
    let id = calendar_id(&raw)?;
    let Some(key) = state.secret_key() else {
        return Err(Failure::status(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::Internal,
            NO_KEY,
        ));
    };
    let text = std::str::from_utf8(&body)
        .map_err(|_| Failure::bad_request("the file is not UTF-8 text"))?;
    let store = state.account(&session.account)?;
    let (source, known) = {
        let store = lock(&store)?;
        let source = caldav_source(&store, key, id)?;
        let writable = matches!(
            store.get(id)?,
            Some(samu_store::Object::Calendar(c)) if c.writable
        );
        if !writable {
            return Err(Failure::bad_request(format!(
                "calendar {id} is not writable; an import needs a CalDAV calendar samu may write to"
            )));
        }
        (source, store.resources_by_uid(id)?)
    };
    if !state.fetcher().allows(&source.url) {
        return Err(Failure::bad_request(HOST_NOT_ALLOWED));
    }
    let split = split_by_uid(text)
        .map_err(|e| Failure::bad_request(format!("not an iCalendar file: {e}")))?;
    let stored = StoredSource {
        source,
        etag: None,
        last_modified: None,
    };
    let mut answer = CalendarImport {
        skipped: split
            .left
            .into_iter()
            .map(|left| ImportIssue {
                uid: left.uid,
                reason: left.reason,
            })
            .collect(),
        ..CalendarImport::default()
    };
    for piece in split.pieces {
        let outcome = put(
            &state,
            &stored,
            known.get(&piece.uid),
            &piece.uid,
            piece.text,
        )
        .await;
        match outcome {
            Put::Created => answer.created += 1,
            Put::Replaced => answer.replaced += 1,
            Put::Failed(reason) => answer.failed.push(ImportIssue {
                uid: Some(piece.uid),
                reason,
            }),
        }
    }
    let note = format!(
        "{}: {} new, {} replaced, {} skipped, {} failed",
        query.name.as_deref().unwrap_or("a file"),
        answer.created,
        answer.replaced,
        answer.skipped.len(),
        answer.failed.len()
    );
    lock(&store)?.log_import(id, &note, &session.device, session.kind, Timestamp::now())?;
    if answer.created + answer.replaced > 0
        && let Err(e) = refresh(&state, &session.account, id).await
    {
        say(&format!(
            "calendar {id} of {}: after an import: {}",
            session.account,
            e.message()
        ));
    }
    Ok(Json(answer))
}

/// One resource to the collection: over the one samu knows for the UID
/// (`If-Match` with its ETag), else new (`If-None-Match: *`).
async fn put(
    state: &AppState,
    stored: &StoredSource,
    known: Option<&KnownResource>,
    uid: &str,
    text: String,
) -> Put {
    let collection = stored.source.url.as_str();
    if known.is_some_and(|k| k.pending) {
        return Put::Failed("a change from samu waits to be written; import again later".into());
    }
    let (url, replacing) = match known.and_then(|k| k.href.as_deref()) {
        Some(href) => match address(collection, href) {
            Some(url) => (url, true),
            None => return Put::Failed(format!("{href} is no address")),
        },
        None => match new_href(collection, uid) {
            Some((url, _)) => (url, false),
            None => return Put::Failed("the calendar's address is none".into()),
        },
    };
    let mut request = state
        .fetcher()
        .resource(reqwest::Method::PUT, url, stored)
        .header(
            reqwest::header::CONTENT_TYPE,
            "text/calendar; charset=utf-8",
        )
        .body(text);
    request = match (replacing, known.and_then(|k| k.etag.as_deref())) {
        (true, Some(etag)) => request.header(reqwest::header::IF_MATCH, etag),
        (true, None) => request,
        (false, _) => request.header(reqwest::header::IF_NONE_MATCH, "*"),
    };
    match request.send().await {
        Err(e) => Put::Failed(e.without_url().to_string()),
        Ok(r) if r.status().is_success() => {
            if replacing {
                Put::Replaced
            } else {
                Put::Created
            }
        }
        Ok(r) if r.status() == StatusCode::PRECONDITION_FAILED => Put::Failed(if replacing {
            "changed on the CalDAV server since samu fetched it; import again after a fetch".into()
        } else {
            "a resource of that name is there already; fetch the calendar and import again".into()
        }),
        Ok(r) => Put::Failed(format!("the CalDAV server answered {}", r.status())),
    }
}
