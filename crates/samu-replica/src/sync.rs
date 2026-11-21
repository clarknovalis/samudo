// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Reading the mirror with the outbox over it, and the sync with the server
//! (ADR 0028 §4, §5).

use std::sync::{Mutex, MutexGuard};

use jiff::Timestamp;
use jiff::tz::TimeZone;
use samu_api::{Change, ChangeOutcome, ChangesPage, Id, SessionInfo};
use samu_client::{Answer, Client};
use samu_core::availability::Settings;
use samu_core::overlay::{self, Objects};

use crate::{Cache, Error, object_of};

/// How `last_error` starts when the server refused changes.
const REFUSED: &str = "refused: ";

/// What one sync did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Changes the server took.
    pub pushed: u32,
    /// Changes the server refused; they left the outbox.
    pub rejected: u32,
    /// Objects that came in.
    pub pulled: u32,
    /// The mirror was loaded anew.
    pub reset: bool,
}

fn lock(cache: &Mutex<Cache>) -> Result<MutexGuard<'_, Cache>, Error> {
    cache
        .lock()
        .map_err(|_| Error::Store("the database lock is poisoned".into()))
}

/// The account's settings as the server last said them, else this
/// machine's zone; and whether they came from the server.
pub fn settings(cache: &Cache) -> Result<(Settings, bool), Error> {
    let from_server = cache
        .meta("zone")?
        .and_then(|name| TimeZone::get(&name).ok());
    let known = from_server.is_some();
    let mut settings = Settings::new(from_server.unwrap_or_else(TimeZone::system));
    if let Some(span) = cache.meta("due_soon")?.and_then(|s| s.parse().ok()) {
        settings.due_soon = span;
    }
    if let Some(time) = cache.meta("remind_time")?.and_then(|s| s.parse().ok()) {
        settings.remind_time = time;
    }
    Ok((settings, known))
}

/// Keeps what the server says about the session: the account's settings
/// and which kind of author it is (ADR 0039 §3).
pub fn remember_session(cache: &Cache, session: &SessionInfo) -> Result<(), Error> {
    cache.set_meta("zone", session.zone.as_deref())?;
    cache.set_meta(
        "due_soon",
        session.due_soon.map(|p| p.0.to_string()).as_deref(),
    )?;
    cache.set_meta(
        "remind_time",
        session.remind_time.map(|t| t.to_string()).as_deref(),
    )?;
    cache.set_meta(
        "kind",
        match session.kind {
            Some(samu_api::AuthorKind::Agent) => Some("agent"),
            _ => None,
        },
    )?;
    cache.set_meta("transcribe", session.transcribe.then_some("1"))?;
    Ok(())
}

/// Whether the server has a transcript service (ADR 0140 §3), as it said
/// at the last sync.
pub fn transcribes(cache: &Cache) -> Result<bool, Error> {
    Ok(cache.meta("transcribe")?.as_deref() == Some("1"))
}

/// The mirror with the outbox laid over it, and which objects the outbox
/// touches.
pub fn view(cache: &Cache) -> Result<(Objects, Vec<Id>), Error> {
    let mut objects = Objects::from_any(cache.mirror()?);
    let mut touched = Vec::new();
    for (_, change) in cache.outbox()? {
        // What cannot be laid over waits for the server's word.
        if overlay::apply(&mut objects, &change).is_ok()
            && let Some(id) = object_of(&change)
        {
            touched.push(id);
        }
    }
    Ok((objects, touched))
}

/// An answer, with a lost session forgotten and any other failure but
/// being offline kept in `last_error`.
fn checked(cache: &Mutex<Cache>, answer: samu_client::Result<Answer>) -> Result<Answer, Error> {
    match answer {
        Err(samu_client::Error::Unauthorized(reason)) => {
            lock(cache)?.set_meta("token", None)?;
            Err(Error::SignedOut(reason))
        }
        Err(other) => {
            let error = Error::from(other);
            if !matches!(error, Error::Offline(_))
                && let Ok(cache) = lock(cache)
            {
                let _ = cache.set_meta("last_error", Some(&error.to_string()));
            }
            Err(error)
        }
        Ok(answer) => Ok(answer),
    }
}

/// Reads the file of a waiting blob by its hash; where the files lie is the
/// client's business.
pub type ReadBlob<'a> = &'a dyn Fn(&str) -> Option<Vec<u8>>;

/// Sends every waiting file up (ADR 0039 §6), before the outbox. A file the
/// server refuses leaves the queue and is said; without a server the sync
/// stops here and nothing is lost.
pub fn push_blobs(
    cache: &Mutex<Cache>,
    client: &Client,
    read: ReadBlob<'_>,
) -> Result<Vec<String>, Error> {
    let waiting = lock(cache)?.blob_outbox()?;
    let mut refused = Vec::new();
    for (hash, media_type, _) in waiting {
        let Some(bytes) = read(&hash) else {
            refused.push(format!("the file {hash} is gone from this device"));
            lock(cache)?.drop_blob(&hash)?;
            continue;
        };
        let answer = client.call_bytes("PUT", &format!("/blobs/{hash}"), &media_type, &[], &bytes);
        match checked(cache, answer) {
            Ok(_) => lock(cache)?.drop_blob(&hash)?,
            Err(Error::Refused(reason)) => {
                refused.push(format!("file {}: {reason}", &hash[..8.min(hash.len())]));
                lock(cache)?.drop_blob(&hash)?;
            }
            Err(other) => return Err(other),
        }
    }
    Ok(refused)
}

fn page(answer: Answer) -> Result<ChangesPage, Error> {
    serde_json::from_value(answer.json()?).map_err(|e| Error::Refused(e.to_string()))
}

/// Push the outbox, then pull what changed (ADR 0028 §5). A refused change
/// leaves the outbox and is remembered in `last_error`. Without a server
/// nothing is lost.
pub fn sync(
    cache: &Mutex<Cache>,
    client: &Client,
    read: ReadBlob<'_>,
) -> Result<SyncReport, Error> {
    let mut report = SyncReport::default();
    // Files first: an attachment in the outbox needs its blob there.
    let mut refused = push_blobs(cache, client, read)?;
    let outbox = lock(cache)?.outbox()?;
    if !outbox.is_empty() {
        let changes: Vec<&Change> = outbox.iter().map(|(_, c)| c).collect();
        let body = serde_json::to_value(&changes).map_err(|e| Error::Store(e.to_string()))?;
        // From here on `withdraw` leaves it (ADR 0104 §4).
        lock(cache)?.sending(outbox.last().map_or(0, |(n, _)| *n));
        let answer = checked(cache, client.call("POST", "/changes", Some(&body)))?;
        let outcomes: Vec<ChangeOutcome> =
            serde_json::from_value(answer.json()?).map_err(|e| Error::Refused(e.to_string()))?;
        let mut sent = Vec::new();
        for ((n, _), outcome) in outbox.iter().zip(&outcomes) {
            match outcome {
                ChangeOutcome::Applied { .. } | ChangeOutcome::Superseded { .. } => {
                    report.pushed += 1;
                }
                ChangeOutcome::Rejected { reason } => {
                    report.rejected += 1;
                    refused.push(reason.clone());
                }
                ChangeOutcome::Unknown => {
                    report.rejected += 1;
                    refused.push("an answer this client does not know".into());
                }
            }
            sent.push(*n);
        }
        lock(cache)?.drop_sent(&sent)?;
    }
    if !outbox.is_empty() || !refused.is_empty() {
        lock(cache)?.set_meta(
            "last_error",
            (!refused.is_empty())
                .then(|| format!("{REFUSED}{}", refused.join("; ")))
                .as_deref(),
        )?;
    }

    // A mirror pulled by a client built against another samu-api lost what
    // that one could not read: kinds it did not know and fields it dropped
    // on storing. The server sends them only when they change again, so
    // the mirror loads everything once.
    let same_api = lock(cache)?.meta("api")?.as_deref() == Some(samu_api::VERSION);
    let after = if same_api { lock(cache)?.seq()? } else { 0 };
    let mut pulled = page(checked(
        cache,
        client.call("GET", &format!("/changes?after={after}"), None),
    )?)?;
    if pulled.reset {
        report.reset = true;
        pulled = page(checked(
            cache,
            client.call("GET", "/changes?after=0", None),
        )?)?;
    }
    report.pulled = u32::try_from(pulled.objects.len()).unwrap_or(u32::MAX);
    {
        let mut cache = lock(cache)?;
        cache.apply_page(
            &pulled.objects,
            &pulled.gone,
            pulled.seq,
            report.reset || after == 0,
        )?;
        cache.set_meta("api", Some(samu_api::VERSION))?;
    }
    if let Ok(answer) = client.call("GET", "/session", None)
        && let Ok(json) = answer.json()
        && let Ok(session) = serde_json::from_value::<SessionInfo>(json)
    {
        remember_session(&*lock(cache)?, &session)?;
    }
    {
        let cache = lock(cache)?;
        // A failure on the way (a 502 while the server restarts) is over
        // once a sync went through; what the server refused stays until
        // the next push says otherwise.
        if cache
            .meta("last_error")?
            .is_some_and(|e| !e.starts_with(REFUSED))
        {
            cache.set_meta("last_error", None)?;
        }
        cache.set_meta("last_sync", Some(&Timestamp::now().to_string()))?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests may unwrap")]

    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;

    use super::*;

    /// A server that answers each request with the next of `answers`
    /// (status, body), whatever it asks.
    fn fake(answers: Vec<(u16, &'static str)>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for (status, body) in answers {
                let (mut stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                while reader.read_line(&mut line).unwrap() > 2 {
                    line.clear();
                }
                write!(
                    stream,
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
            }
        });
        format!("http://{address}")
    }

    const PAGE: &str = r#"{"seq": 1, "objects": [], "gone": []}"#;

    /// Regression (11.11.2026): a 502 while the server restarted stayed in
    /// `last_error` after the next sync went through.
    #[test]
    fn a_passing_failure_leaves_with_the_next_good_sync() {
        let cache = Mutex::new(Cache::in_memory().unwrap());
        let none = |_: &str| None;
        let url = fake(vec![(502, "bad gateway"), (200, PAGE), (401, "{}")]);
        let client = Client::new(&url, Some("t".into())).unwrap();
        assert!(sync(&cache, &client, &none).is_err());
        assert!(cache.lock().unwrap().meta("last_error").unwrap().is_some());
        sync(&cache, &client, &none).unwrap();
        assert_eq!(cache.lock().unwrap().meta("last_error").unwrap(), None);
    }

    #[test]
    fn a_refusal_stays_after_a_good_sync() {
        let cache = Mutex::new(Cache::in_memory().unwrap());
        cache
            .lock()
            .unwrap()
            .set_meta("last_error", Some("refused: no such task"))
            .unwrap();
        let none = |_: &str| None;
        let url = fake(vec![(200, PAGE), (401, "{}")]);
        let client = Client::new(&url, Some("t".into())).unwrap();
        sync(&cache, &client, &none).unwrap();
        assert_eq!(
            cache.lock().unwrap().meta("last_error").unwrap().as_deref(),
            Some("refused: no such task")
        );
    }
}
