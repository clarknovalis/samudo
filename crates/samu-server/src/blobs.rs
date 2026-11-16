// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `PUT` and `GET /blobs/{sha256}`: the files of attachments (ADR 0039 §4).
//!
//! An upload goes straight to a temporary file and is hashed on the way, so
//! the server never holds a file in memory; a download is read in chunks.
//! A file is answered with headers that keep a browser from running it in
//! samu's origin: the web UI's cookie gets through to this route, and an
//! uploaded HTML page must not get at it.

use std::sync::Arc;

use axum::Extension;
use axum::Json;
use axum::body::Body;
use axum::extract::{Path as UrlPath, State};
use axum::http::header::{
    CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_SECURITY_POLICY, CONTENT_TYPE,
    X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use jiff::Timestamp;
use samu_store::core::api::{Blob, ErrorCode};
use samu_store::{BlobDir, BlobRefused, valid_hash};
use tokio::io::AsyncReadExt;

use crate::app::{AppState, Failure, Session, account_path, lock};

/// The type of an upload that names none.
const OCTET_STREAM: &str = "application/octet-stream";

/// The blob directory of a session's account.
fn blob_dir(state: &AppState, session: &Session) -> BlobDir {
    BlobDir::beside(&account_path(state.data(), &session.account))
}

pub(crate) fn checked_hash(hash: &str) -> Result<(), Failure> {
    if valid_hash(hash) {
        Ok(())
    } else {
        Err(Failure::bad_request(BlobRefused::NotAHash.to_string()))
    }
}

/// The media type of an upload from its `Content-Type`: `type/subtype` in
/// lower case, parameters kept (`audio/webm;codecs=opus`); none at all is
/// `application/octet-stream`.
pub(crate) fn media_type(headers: &HeaderMap) -> Result<String, Failure> {
    let Some(value) = headers.get(CONTENT_TYPE) else {
        return Ok(OCTET_STREAM.to_string());
    };
    let refused = || Failure::bad_request("Content-Type is not a media type");
    let text = value.to_str().map_err(|_| refused())?.trim();
    if text.is_empty() || text.len() > 255 || text.chars().any(|c| c.is_ascii_control()) {
        return Err(refused());
    }
    let (essence, parameters) = match text.split_once(';') {
        Some((essence, parameters)) => (essence.trim(), Some(parameters.trim())),
        None => (text, None),
    };
    let name = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$&-^_.+".contains(&b))
    };
    let Some((kind, subtype)) = essence.split_once('/') else {
        return Err(refused());
    };
    if !name(kind) || !name(subtype) {
        return Err(refused());
    }
    let essence = essence.to_ascii_lowercase();
    Ok(match parameters {
        Some(parameters) if !parameters.is_empty() => format!("{essence};{parameters}"),
        _ => essence,
    })
}

/// `PUT /blobs/{sha256}`: keeps the body under its hash and records it.
/// A wrong hash or more than 50 MB keeps nothing; the same file again is
/// answered the same.
pub(crate) async fn put(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    UrlPath(hash): UrlPath<String>,
    headers: HeaderMap,
    body: Body,
) -> Result<Json<Blob>, Failure> {
    checked_hash(&hash)?;
    // Only a restricted session that may write (ADR 0095 §4).
    if session
        .grant
        .as_ref()
        .is_some_and(|grant| grant.access != samu_store::core::api::Access::Rw)
    {
        return Err(crate::grant::forbidden(
            "a read-only session puts no files (ADR 0095)",
        ));
    }
    let media_type = media_type(&headers)?;
    let store = state.account(&session.account)?;
    let room = room(&state, &*lock(&store)?, &hash)?;
    let dir = blob_dir(&state, &session);
    let mut writer = dir.writer_up_to(room.limit)?;
    let mut stream = body.into_data_stream();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|e| Failure::bad_request(format!("the upload broke off: {e}")))?;
        // A chunk is small; writing it here keeps the upload out of memory
        // without a thread of its own per file.
        if let Err(refused) = writer.write(&chunk)? {
            return Err(room.refused(&refused));
        }
    }
    let expected = hash.clone();
    let written = tokio::task::spawn_blocking(move || writer.finish(&expected))
        .await
        .map_err(|e| Failure::internal(format!("writing the blob: {e}")))??
        .map_err(|refused| room.refused(&refused))?;
    let mut store = lock(&store)?;
    store.add_blob(&written.hash, written.size, &media_type, Timestamp::now())?;
    // The type it was first recorded with stays (ADR 0039 §4: a blob never
    // changes).
    let (size, media_type) = store
        .blob(&written.hash)?
        .ok_or_else(|| Failure::internal("the blob just recorded is gone"))?;
    Ok(Json(Blob {
        hash: written.hash,
        size,
        media_type,
    }))
}

/// How much an upload may hold (ADR 0097 §2): the limit per file, or
/// what the account has left, whichever is smaller. A file the account
/// has already counts nothing.
struct Room {
    limit: u64,
    /// The bytes of the account and its quota, when the quota is the
    /// tighter of the two.
    full: Option<(u64, u64)>,
}

impl Room {
    fn refused(&self, refused: &BlobRefused) -> Failure {
        match (refused, self.full) {
            (BlobRefused::TooLarge { .. }, Some((used, quota))) => Failure::status(
                StatusCode::PAYLOAD_TOO_LARGE,
                ErrorCode::BadRequest,
                format!("the account has {used} of {quota} bytes in files"),
            ),
            _ => refused_upload(refused),
        }
    }
}

fn room(state: &AppState, store: &samu_store::AccountStore, hash: &str) -> Result<Room, Failure> {
    let limits = state.blob_limits();
    let per_file = Room {
        limit: limits.per_file,
        full: None,
    };
    let Some(quota) = limits.per_account else {
        return Ok(per_file);
    };
    if store.blob(hash)?.is_some() {
        return Ok(per_file);
    }
    let used = store.blob_bytes()?;
    let left = quota.saturating_sub(used);
    Ok(if left < limits.per_file {
        Room {
            limit: left,
            full: Some((used, quota)),
        }
    } else {
        per_file
    })
}

pub(crate) fn refused_upload(refused: &BlobRefused) -> Failure {
    match refused {
        BlobRefused::TooLarge { .. } => Failure::status(
            StatusCode::PAYLOAD_TOO_LARGE,
            ErrorCode::BadRequest,
            refused.to_string(),
        ),
        _ => Failure::bad_request(refused.to_string()),
    }
}

/// Whether a browser may show a file of this type in place; anything else
/// is offered as a download.
fn shown_inline(media_type: &str) -> bool {
    ["image/", "audio/", "video/"]
        .iter()
        .any(|prefix| media_type.starts_with(prefix))
}

/// `GET /blobs/{sha256}`: the file with its media type.
pub(crate) async fn get(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    UrlPath(hash): UrlPath<String>,
) -> Result<Response, Failure> {
    checked_hash(&hash)?;
    let not_found = || Failure::not_found(format!("no blob {hash}"));
    let recorded = {
        let store = state.account(&session.account)?;
        let store = lock(&store)?;
        // A restricted session gets a file only through an attachment it
        // sees (ADR 0095 §4).
        if let Some(reach) = crate::app::session_reach(&session, &store)? {
            let snapshot = store.snapshot()?;
            let seen = snapshot.attachments.iter().any(|attachment| {
                attachment.blob == hash
                    && reach.sees(&samu_store::core::api::AnyObject::Attachment(
                        attachment.clone(),
                    ))
            });
            if !seen {
                return Err(not_found());
            }
        }
        store.blob(&hash)?
    };
    let (size, media_type) = recorded.ok_or_else(not_found)?;
    let file = blob_dir(&state, &session)
        .open(&hash)?
        .ok_or_else(not_found)?;
    let file = tokio::fs::File::from_std(file);
    let chunks = futures_util::stream::unfold(Some(file), |file| async move {
        let mut file = file?;
        let mut buffer = vec![0u8; 64 * 1024];
        match file.read(&mut buffer).await {
            Ok(0) => None,
            Ok(n) => {
                buffer.truncate(n);
                Some((Ok::<_, std::io::Error>(buffer), Some(file)))
            }
            Err(e) => Some((Err(e), None)),
        }
    });
    let mut response = Body::from_stream(chunks).into_response();
    file_headers(response.headers_mut(), &media_type, size);
    Ok(response)
}

/// The headers of a file answer: its type and size, cached for good, and
/// kept from running in samu's origin.
pub(crate) fn file_headers(headers: &mut HeaderMap, media_type: &str, size: u64) {
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(media_type).unwrap_or(HeaderValue::from_static(OCTET_STREAM)),
    );
    headers.insert(CONTENT_LENGTH, HeaderValue::from(size));
    // Named by its content, so it never changes.
    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; media-src 'self'; sandbox"),
    );
    if !shown_inline(media_type) {
        headers.insert(CONTENT_DISPOSITION, HeaderValue::from_static("attachment"));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests may unwrap")]

    use super::*;

    fn typed(value: &str) -> Result<String, u16> {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_str(value).unwrap());
        media_type(&headers).map_err(|_| 400)
    }

    #[test]
    fn the_media_type_comes_from_content_type() {
        assert_eq!(typed("image/JPEG").unwrap(), "image/jpeg");
        assert_eq!(
            typed("audio/webm; codecs=opus").unwrap(),
            "audio/webm;codecs=opus"
        );
        assert_eq!(media_type(&HeaderMap::new()).unwrap(), OCTET_STREAM);
        assert!(typed("jpeg").is_err());
        assert!(typed("image/").is_err());
        assert!(typed("im age/jpeg").is_err());
    }

    #[test]
    fn only_pictures_sound_and_film_are_shown_in_place() {
        assert!(shown_inline("image/png"));
        assert!(shown_inline("audio/webm;codecs=opus"));
        assert!(!shown_inline("text/html"));
        assert!(!shown_inline("application/pdf"));
    }
}
