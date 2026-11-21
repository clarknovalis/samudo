// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Transcripts of audio attachments (ADR 0140): the one optional service
//! the server sends a recording to, the route that asks for a transcript,
//! and the work in the background.
//!
//! The service speaks the format of OpenAI's `POST /v1/audio/transcriptions`
//! (§1). Without one nothing leaves the server: a request that came with a
//! new attachment is dropped, and the route answers 503.

use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::{Extension, Path as UrlPath, State};
use axum::http::StatusCode;
use jiff::Timestamp;
use samu_store::core::api::{AnyObject, Attachment, ErrorCode, Id};
use samu_store::{Asked, BlobDir, Object, Transcribed};

use crate::app::{AppState, Failure, Session, account_path, lock, session_reach};
use crate::calendar::say;

/// How long the service may take for one recording: a model on a small
/// machine needs longer than the recording lasts.
const TIMEOUT: Duration = Duration::from_secs(15 * 60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// An answer is a text, not a file.
const MAX_ANSWER_BYTES: usize = 1024 * 1024;
/// What of a refusal's body is kept as the reason.
const REASON_CHARS: usize = 200;
/// The model named when the settings name none; a self-hosted service
/// usually ignores it or has it as an alias.
const DEFAULT_MODEL: &str = "whisper-1";

pub(crate) const NO_SERVICE: &str = "the server has no transcript service (SAMU_TRANSCRIBE_URL)";

/// The transcript service of this server (ADR 0140 §1).
#[derive(Debug, Clone)]
pub(crate) struct Service {
    url: String,
    model: String,
    token: Option<String>,
    language: Option<String>,
    client: reqwest::Client,
}

impl Service {
    pub(crate) fn new(
        url: &str,
        model: Option<&str>,
        token: Option<&str>,
        language: Option<&str>,
    ) -> anyhow::Result<Self> {
        let url = url.trim();
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            anyhow::bail!("SAMU_TRANSCRIBE_URL is not an http or https address");
        }
        let filled = |value: Option<&str>| {
            value
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };
        Ok(Self {
            url: url.to_string(),
            model: filled(model).unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            token: filled(token),
            language: filled(language),
            client: reqwest::Client::builder()
                .timeout(TIMEOUT)
                .connect_timeout(CONNECT_TIMEOUT)
                .build()?,
        })
    }

    /// Sends one recording and reads the answer. Never an error: what goes
    /// wrong is the reason of a failed transcript.
    pub(crate) async fn transcribe(
        &self,
        file: Vec<u8>,
        name: &str,
        media_type: &str,
    ) -> Transcribed {
        let boundary = format!("samu-{}", Id::new());
        let body = self.form(&boundary, file, name, media_type);
        let mut request = self
            .client
            .post(&self.url)
            .header(
                reqwest::header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .header(reqwest::header::ACCEPT, "application/json")
            .body(body);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let mut response = match request.send().await {
            Ok(response) => response,
            Err(e) => {
                return Transcribed::Failed(format!(
                    "the transcript service did not answer: {}",
                    e.without_url()
                ));
            }
        };
        let status = response.status();
        let mut answer = Vec::new();
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    if answer.len() + chunk.len() > MAX_ANSWER_BYTES {
                        return Transcribed::Failed(format!(
                            "the transcript service answered more than {MAX_ANSWER_BYTES} bytes"
                        ));
                    }
                    answer.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(e) => {
                    return Transcribed::Failed(format!(
                        "the transcript service broke off: {}",
                        e.without_url()
                    ));
                }
            }
        }
        read_answer(status.as_u16(), &answer)
    }

    /// The form of `POST /v1/audio/transcriptions`, written by hand: three
    /// parts need no multipart library. No `response_format`: JSON is what
    /// every such service answers by default, and not every one takes the
    /// field.
    fn form(&self, boundary: &str, file: Vec<u8>, name: &str, media_type: &str) -> Vec<u8> {
        let mut body = Vec::with_capacity(file.len() + 1024);
        let mut field = |name: &str, value: &str| {
            body.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
                )
                .as_bytes(),
            );
        };
        field("model", &header_safe(&self.model));
        if let Some(language) = &self.language {
            field("language", &header_safe(language));
        }
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\n\
                 Content-Type: {}\r\n\r\n",
                header_safe(name),
                header_safe(media_type)
            )
            .as_bytes(),
        );
        body.extend_from_slice(&file);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        body
    }
}

/// A value that may stand inside a part's header: no quote, no line break.
fn header_safe(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c == '"' || c == '\\' || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect()
}

/// What an answer of the service means.
fn read_answer(status: u16, body: &[u8]) -> Transcribed {
    if !(200..300).contains(&status) {
        let said: String = String::from_utf8_lossy(body)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(REASON_CHARS)
            .collect();
        return Transcribed::Failed(if said.is_empty() {
            format!("the transcript service answered {status}")
        } else {
            format!("the transcript service answered {status}: {said}")
        });
    }
    match serde_json::from_slice::<serde_json::Value>(body) {
        Ok(value) => match value.get("text").and_then(serde_json::Value::as_str) {
            Some(text) => Transcribed::Text(text.to_string()),
            None => Transcribed::Failed("the transcript service answered without `text`".into()),
        },
        Err(_) => Transcribed::Failed("the transcript service did not answer with JSON".into()),
    }
}

/// Works off what waits in an account, in the background: after an
/// attachment came that asks for its transcript, and after the route.
pub(crate) fn kick(state: &Arc<AppState>, account: &str) {
    let state = state.clone();
    let account = account.to_string();
    tokio::spawn(async move { work_off(&state, &account).await });
}

/// Once at the start: what a server that stopped left waiting (§3).
pub(crate) async fn resume(state: Arc<AppState>) {
    let accounts = match state.server().and_then(|s| Ok(s.accounts()?)) {
        Ok(accounts) => accounts,
        Err(e) => {
            say(&format!("transcripts: {}", e.message()));
            return;
        }
    };
    for account in accounts.into_iter().filter(|a| !a.disabled) {
        work_off(&state, &account.name).await;
    }
}

/// Every waiting attachment of an account, one after the other. One
/// recording at a time over all accounts: the service is a small machine
/// more often than not.
pub(crate) async fn work_off(state: &AppState, account: &str) {
    let _one = state.transcribing().lock().await;
    if let Err(e) = work_off_locked(state, account).await {
        say(&format!("transcripts {account}: {}", e.message()));
    }
}

async fn work_off_locked(state: &AppState, account: &str) -> Result<(), Failure> {
    let store = state.account(account)?;
    let waiting = lock(&store)?.pending_transcripts()?;
    for id in waiting {
        let Some(service) = state.transcriber() else {
            lock(&store)?.drop_transcript_request(id, Timestamp::now())?;
            continue;
        };
        // Read again: it may have been deleted while another was heard.
        let Some(Object::Attachment(attachment)) = lock(&store)?.get(id)? else {
            continue;
        };
        let path = BlobDir::beside(&account_path(state.data(), account)).path(&attachment.blob);
        let file = match path {
            Some(path) => tokio::fs::read(&path).await.ok(),
            None => None,
        };
        let outcome = match file {
            Some(file) => {
                service
                    .transcribe(file, &attachment.name, &attachment.media_type)
                    .await
            }
            None => Transcribed::Failed("the recording is not on the server".into()),
        };
        if let Transcribed::Failed(reason) = &outcome {
            say(&format!("transcript {id} of {account}: {reason}"));
        }
        lock(&store)?.store_transcript(id, &outcome, Timestamp::now())?;
    }
    Ok(())
}

/// `POST /attachments/{id}/transcribe`: asks for the transcript of an
/// audio attachment and answers at once with the attachment as it waits
/// (ADR 0140 §3). 503 without a service.
pub(crate) async fn post_transcribe(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    UrlPath(raw): UrlPath<String>,
) -> Result<Json<Attachment>, Failure> {
    let id: Id = raw
        .parse()
        .map_err(|_| Failure::bad_request(format!("{raw:?} is not an id")))?;
    if state.transcriber().is_none() {
        return Err(Failure::status(
            StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::Internal,
            NO_SERVICE,
        ));
    }
    let missing = || Failure::not_found(format!("no attachment {id}"));
    let asked = {
        let store = state.account(&session.account)?;
        let mut store = lock(&store)?;
        // A restricted session asks only where it may write (ADR 0095 §3).
        if let Some(reach) = session_reach(&session, &store)? {
            let Some(object) = store.get(id)?.map(AnyObject::from) else {
                return Err(missing());
            };
            if !reach.sees(&object) {
                return Err(missing());
            }
            if !reach.may_write(Some(&object), &object) {
                return Err(crate::grant::forbidden(crate::grant::OUTSIDE));
            }
        }
        store.ask_transcript(id, Timestamp::now())?
    };
    match asked {
        Asked::Pending(attachment) => {
            kick(&state, &session.account);
            Ok(Json(*attachment))
        }
        Asked::NotAudio => Err(Failure::bad_request(format!(
            "attachment {id} is not audio"
        ))),
        Asked::Missing => Err(missing()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests may unwrap")]

    use super::*;

    #[test]
    fn an_answer_with_text_is_the_transcript() {
        assert_eq!(
            read_answer(200, br#"{"text": " Buy milk. ", "language": "en"}"#),
            Transcribed::Text(" Buy milk. ".into())
        );
        assert_eq!(
            read_answer(200, br#"{"text": ""}"#),
            Transcribed::Text(String::new())
        );
    }

    #[test]
    fn anything_else_is_a_failure_with_its_reason() {
        assert_eq!(
            read_answer(503, b""),
            Transcribed::Failed("the transcript service answered 503".into())
        );
        let Transcribed::Failed(reason) = read_answer(400, "bad\n  file ".repeat(60).as_bytes())
        else {
            panic!("a refusal is a failure");
        };
        assert!(reason.starts_with("the transcript service answered 400: bad file bad"));
        assert!(reason.chars().count() <= 40 + REASON_CHARS);
        assert!(matches!(
            read_answer(200, b"<html>"),
            Transcribed::Failed(_)
        ));
        assert!(matches!(
            read_answer(200, br#"{"segments": []}"#),
            Transcribed::Failed(_)
        ));
    }

    #[test]
    fn the_form_names_model_format_language_and_file() {
        let service = Service::new(
            "http://127.0.0.1:1/v1/audio/transcriptions",
            Some("large-v3"),
            None,
            Some("de"),
        )
        .unwrap();
        let body = service.form("b", b"AUDIO".to_vec(), "a \"note\".m4a", "audio/mp4");
        let body = String::from_utf8(body).unwrap();
        assert_eq!(
            body,
            "--b\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\nlarge-v3\r\n\
             --b\r\nContent-Disposition: form-data; name=\"language\"\r\n\r\nde\r\n\
             --b\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a _note_.m4a\"\r\n\
             Content-Type: audio/mp4\r\n\r\nAUDIO\r\n--b--\r\n"
        );
    }

    #[test]
    fn the_address_has_to_be_http() {
        assert!(Service::new("ftp://example.org", None, None, None).is_err());
        let service = Service::new(" https://example.org/v1 ", Some(" "), Some(""), None).unwrap();
        assert_eq!(
            (service.model.as_str(), service.token),
            (DEFAULT_MODEL, None)
        );
    }
}
