// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Making an account over HTTP (ADR 0099 §1–3) and deleting one's own
//! (§5).

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::header::{ACCEPT_LANGUAGE, RETRY_AFTER, SET_COOKIE};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use jiff::Timestamp;
use samu_store::core::api::{
    AccountDelete, AuthorKind, DemoAdded, DemoRequest, ErrorCode, Register, Registered,
    RegistrationInfo, RegistrationMode,
};
use samu_store::{Registered as Origin, valid_account_name};

use crate::app::{AppState, Failure, Session};
use crate::demo::Lang;

/// Registrations in the whole server per [`WINDOW`] (ADR 0099 §3).
pub(crate) const PER_WINDOW: u32 = 10;
pub(crate) const WINDOW: Duration = Duration::from_secs(3600);
/// Registered accounts at the same time, unless `SAMU_REGISTRATION_MAX`
/// says otherwise (ADR 0099 §3).
pub(crate) const MAX_ACCOUNTS: u64 = 100;

/// Shortest password taken, as the store has it.
const MIN_PASSWORD: usize = 8;

/// The languages of the demo data.
const LANGUAGES: [&str; 2] = ["de", "en"];

/// How registration is set up in this server.
#[derive(Debug)]
pub(crate) struct Registration {
    pub(crate) mode: RegistrationMode,
    pub(crate) max: u64,
    /// Every registration counts, under one key.
    made: crate::brake::Brake,
}

impl Registration {
    pub(crate) fn new(mode: RegistrationMode, max: Option<u64>) -> Self {
        Self {
            mode,
            max: max.unwrap_or(MAX_ACCOUNTS),
            made: crate::brake::Brake::new(PER_WINDOW, WINDOW),
        }
    }
}

impl Default for Registration {
    fn default() -> Self {
        Self::new(RegistrationMode::Off, None)
    }
}

/// `GET /register`: what the sign-in page offers.
pub(crate) async fn info(State(state): State<Arc<AppState>>) -> Json<RegistrationInfo> {
    let mode = state.registration().mode;
    Json(RegistrationInfo {
        mode,
        languages: if mode == RegistrationMode::Off {
            Vec::new()
        } else {
            LANGUAGES.iter().map(ToString::to_string).collect()
        },
    })
}

fn forbidden(message: impl Into<String>) -> Failure {
    Failure::status(StatusCode::FORBIDDEN, ErrorCode::Forbidden, message)
}

fn conflict(message: impl Into<String>) -> Failure {
    Failure::status(StatusCode::CONFLICT, ErrorCode::BadRequest, message)
}

/// The language of the demo data: asked for, else the first of
/// `Accept-Language` there is, else English.
fn language(asked: Option<&str>, headers: &HeaderMap) -> Result<Lang, Failure> {
    if let Some(asked) = asked {
        return match asked.trim() {
            "de" => Ok(Lang::De),
            "en" => Ok(Lang::En),
            other => Err(Failure::bad_request(format!(
                "no demo data in {other:?}: de or en"
            ))),
        };
    }
    let accepted = headers
        .get(ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    for range in accepted.split(',') {
        let tag = range.split(';').next().unwrap_or("").trim().to_lowercase();
        match tag.split('-').next() {
            Some("de") => return Ok(Lang::De),
            Some("en") => return Ok(Lang::En),
            _ => {}
        }
    }
    Ok(Lang::En)
}

/// So many letters and digits, at random.
fn random(chars: usize) -> Result<String, Failure> {
    let token = samu_store::new_token().map_err(Failure::from)?;
    Ok(token.chars().take(chars).collect())
}

/// What a request asks to be made, checked.
struct Asked {
    name: Option<String>,
    password: String,
    /// The password goes back in the answer: the server made it.
    made_password: bool,
    demo: Option<Lang>,
    origin: Origin,
    device: String,
}

fn check(mode: RegistrationMode, request: Register, headers: &HeaderMap) -> Result<Asked, Failure> {
    let device = request.device.trim().to_string();
    if device.is_empty() {
        return Err(Failure::bad_request("a session needs a device name"));
    }
    let name = request
        .name
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty());
    if let Some(name) = &name
        && !valid_account_name(name)
    {
        return Err(Failure::bad_request(
            "an account name is 1 to 32 of a-z, 0-9, '_' and '-'",
        ));
    }
    let password = request.password.filter(|p| !p.is_empty());
    if password
        .as_ref()
        .is_some_and(|p| p.chars().count() < MIN_PASSWORD)
    {
        return Err(Failure::bad_request(format!(
            "a password needs at least {MIN_PASSWORD} characters"
        )));
    }
    let demo = request.demo.filter(|d| !d.trim().is_empty());
    match mode {
        RegistrationMode::Off | RegistrationMode::Unknown => {
            Err(forbidden("this server makes no accounts over HTTP"))
        }
        RegistrationMode::Demo => {
            let demo = Some(language(demo.as_deref(), headers)?);
            match (name, password) {
                (None, None) => Ok(Asked {
                    name: None,
                    password: random(20)?,
                    made_password: true,
                    demo,
                    origin: Origin::Demo,
                    device,
                }),
                (Some(name), Some(password)) => Ok(Asked {
                    name: Some(name),
                    password,
                    made_password: false,
                    demo,
                    origin: Origin::Demo,
                    device,
                }),
                (None, Some(_)) => Err(Failure::bad_request(
                    "a password without a name: give both, or neither to try it out",
                )),
                (Some(_), None) => Err(Failure::bad_request("an account needs a password")),
            }
        }
        RegistrationMode::Open => {
            let (Some(name), Some(password)) = (name, password) else {
                return Err(Failure::bad_request(
                    "an account needs a name and a password",
                ));
            };
            Ok(Asked {
                name: Some(name),
                password,
                made_password: false,
                demo: demo.map(|d| language(Some(&d), headers)).transpose()?,
                origin: Origin::Open,
                device,
            })
        }
    }
}

/// `POST /register` (ADR 0099 §2): the account, signed in.
pub(crate) async fn register(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<Register>,
) -> Result<Response, Failure> {
    let registration = state.registration();
    let asked = check(registration.mode, request, &headers)?;
    if let Some(wait) = registration.made.wait("", Instant::now()) {
        let (seconds, message) = crate::brake::message(wait);
        let mut response = Failure::status(
            StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::Forbidden,
            format!("too many new accounts just now; {message}"),
        )
        .into_response();
        response
            .headers_mut()
            .insert(RETRY_AFTER, HeaderValue::from(seconds));
        return Ok(response);
    }
    // Argon2 is slow on purpose; only so many at once (ADR 0097 §1).
    let check = state
        .checks()
        .acquire()
        .await
        .map_err(|e| Failure::internal(e.to_string()))?;
    let making = state.clone();
    let made = tokio::task::spawn_blocking(move || make(&making, asked))
        .await
        .map_err(|e| Failure::internal(e.to_string()))??;
    drop(check);
    state.registration().made.failed("", Instant::now());
    let (registered, token) = made;
    let mut response = Json(&registered).into_response();
    if let Ok(value) = HeaderValue::from_str(&crate::app::session_cookie(&token)) {
        response.headers_mut().insert(SET_COOKIE, value);
    }
    Ok(response)
}

/// Makes the account, its demo data and its first session.
fn make(state: &AppState, asked: Asked) -> Result<(Registered, String), Failure> {
    let now = Timestamp::now();
    let name = {
        let mut server = state.server()?;
        if server.registered_count()? >= state.registration().max {
            return Err(Failure::status(
                StatusCode::SERVICE_UNAVAILABLE,
                ErrorCode::Internal,
                "this server takes no more new accounts for now",
            ));
        }
        let name = match &asked.name {
            Some(name) => {
                if server.account_info(name)?.is_some() {
                    return Err(conflict(format!("there is already an account {name:?}")));
                }
                name.clone()
            }
            // A free name, without saying which ones are taken.
            None => {
                let mut found = None;
                for _ in 0..8 {
                    let name = format!("demo-{}", random(6)?);
                    if server.account_info(&name)?.is_none() {
                        found = Some(name);
                        break;
                    }
                }
                found.ok_or_else(|| Failure::internal("no free demo name found"))?
            }
        };
        server
            .add_account_as(&name, &asked.password, asked.origin, now)
            .map_err(|e| match e {
                samu_store::Error::Unsupported { what } => Failure::bad_request(what),
                other => other.into(),
            })?;
        name
    };
    // Made now, as `account add` does, so it is there before any request.
    if let Err(e) = state
        .account(&name)
        .and_then(|_| asked.demo.map_or(Ok(()), |lang| seed(state, &name, lang)))
    {
        // Half an account is none: it goes again.
        crate::calendar::say(&format!("registering {name}: {}", e.message()));
        if let Err(gone) = state.forget_account(&name) {
            crate::calendar::say(&format!("removing {name} again: {}", gone.message()));
        }
        return Err(e);
    }
    let (token, row) = state
        .server()?
        .open_first_session(&name, &asked.device, now)?
        .ok_or_else(|| Failure::internal("the new account is gone"))?;
    let registered = Registered {
        account: name,
        password: asked.made_password.then_some(asked.password),
        token: token.clone(),
        expires: row.expires,
    };
    Ok((registered, token))
}

fn seed(state: &AppState, name: &str, lang: Lang) -> Result<(), Failure> {
    let store = state.account(name)?;
    let mut store = crate::app::lock(&store)?;
    let blobs = samu_store::BlobDir::beside(&crate::app::account_path(state.data(), name));
    crate::demo::seed(&mut store, &blobs, lang).map_err(|e| Failure::internal(format!("{e:#}")))?;
    Ok(())
}

/// `POST /demo` (ADR 0099 §7): demo data into the session's account,
/// everything into an empty one, else under the folder `Demo`.
pub(crate) async fn add_demo(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    Json(request): Json<DemoRequest>,
) -> Result<Json<DemoAdded>, Failure> {
    if session.grant.is_some() {
        return Err(forbidden(
            "only a session of the whole account adds demo data",
        ));
    }
    let lang = language(Some(&request.lang), &HeaderMap::new())?;
    let store = state.account(&session.account)?;
    let mut store = crate::app::lock(&store)?;
    let blobs =
        samu_store::BlobDir::beside(&crate::app::account_path(state.data(), &session.account));
    let (whole, counts) =
        crate::demo::seed_into(&mut store, &blobs, lang).map_err(|e| match e.downcast_ref() {
            Some(crate::demo::Taken(why)) => conflict(why.clone()),
            None => Failure::internal(format!("{e:#}")),
        })?;
    Ok(Json(DemoAdded {
        whole,
        counts: counts
            .0
            .into_iter()
            .map(|(k, n)| (k.to_string(), n))
            .collect(),
    }))
}

/// `POST /account/delete` (ADR 0099 §5): a registered account deletes
/// itself, after its password once more.
pub(crate) async fn delete_own(
    State(state): State<Arc<AppState>>,
    Extension(session): Extension<Session>,
    Json(request): Json<AccountDelete>,
) -> Result<Response, Failure> {
    if session.grant.is_some() || session.kind != AuthorKind::Human {
        return Err(forbidden(
            "only a person's session of the whole account deletes it",
        ));
    }
    let info = state
        .server()?
        .account_info(&session.account)?
        .ok_or_else(|| Failure::unauthorized("the account is gone"))?;
    if info.registered == Origin::Cli {
        return Err(forbidden(
            "this account was made by whoever runs the server; ask them to delete it",
        ));
    }
    let key = crate::brake::Brake::key(&session.account);
    if let Some(wait) = state.sign_ins().wait(&key, Instant::now()) {
        let (seconds, message) = crate::brake::message(wait);
        let mut response = Failure::status(
            StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::Unauthorized,
            message,
        )
        .into_response();
        response
            .headers_mut()
            .insert(RETRY_AFTER, HeaderValue::from(seconds));
        return Ok(response);
    }
    let check = state
        .checks()
        .acquire()
        .await
        .map_err(|e| Failure::internal(e.to_string()))?;
    let checking = state.clone();
    let account = session.account.clone();
    let right = tokio::task::spawn_blocking(move || {
        Ok::<_, Failure>(
            checking
                .server()?
                .check_password(&account, &request.password)?,
        )
    })
    .await
    .map_err(|e| Failure::internal(e.to_string()))??;
    drop(check);
    if !right {
        state.sign_ins().failed(&key, Instant::now());
        return Err(Failure::unauthorized("wrong password"));
    }
    state.delete_account(&session.account).await?;
    crate::calendar::say(&format!("account {} deleted by itself", session.account));
    let mut response = StatusCode::NO_CONTENT.into_response();
    if let Ok(value) = HeaderValue::from_str(&crate::app::gone_cookie()) {
        response.headers_mut().insert(SET_COOKIE, value);
    }
    Ok(response)
}
