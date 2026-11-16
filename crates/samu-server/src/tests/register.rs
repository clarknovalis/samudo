// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Making an account over HTTP and deleting one's own (ADR 0099 §1–5),
//! checked in the databases, not only in the answers.

use std::net::SocketAddr;

use samu_store::core::api::RegistrationMode;
use samu_store::{Registered, ServerStore};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{Answer, PASSWORD, TempDir, call, start_state};
use crate::register::Registration;

async fn start(mode: RegistrationMode, max: Option<u64>) -> (SocketAddr, TempDir) {
    let (address, dir, _) =
        start_state(true, |s| s.with_registration(Registration::new(mode, max))).await;
    (address, dir)
}

/// `POST /register` with an `Accept-Language`.
async fn register(address: SocketAddr, body: &Value, language: Option<&str>) -> Answer {
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let body = body.to_string();
    let language = language.map_or(String::new(), |l| format!("Accept-Language: {l}\r\n"));
    let request = format!(
        "POST /register HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{language}\
         Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).await.unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap();
    Answer {
        status: head.split(' ').nth(1).unwrap().parse().unwrap(),
        headers: head.to_string(),
        body: body.to_string(),
    }
}

/// Folder and project titles of an account, asked of its database.
fn projects(dir: &TempDir, account: &str) -> Vec<String> {
    let path = dir.0.join("accounts").join(format!("{account}.db"));
    let conn = rusqlite::Connection::open(path).unwrap();
    let mut statement = conn
        .prepare(
            "SELECT title FROM project JOIN object o ON o.id = project.object \
             WHERE o.deleted_at IS NULL UNION ALL SELECT title FROM folder \
             JOIN object o ON o.id = folder.object WHERE o.deleted_at IS NULL",
        )
        .unwrap();
    statement
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn origin(dir: &TempDir, account: &str) -> Option<Registered> {
    let server = ServerStore::open(&dir.0.join("server.db")).unwrap();
    server.account_info(account).unwrap().map(|a| a.registered)
}

#[tokio::test]
async fn off_says_so_and_makes_nothing() {
    let (address, dir) = start(RegistrationMode::Off, None).await;
    let info = call(address, "GET", "/register", None, None).await;
    assert_eq!(info.status, 200);
    assert_eq!(info.json(), json!({"mode": "off"}));
    let answer = register(address, &json!({"device": "web"}), None).await;
    assert_eq!(answer.status, 403, "{}", answer.body);
    let server = ServerStore::open(&dir.0.join("server.db")).unwrap();
    assert_eq!(server.accounts().unwrap().len(), 1);
}

/// „Ausprobieren“: a made name and password, signed in, demo data in the
/// language the browser asks for.
#[tokio::test]
async fn trying_it_out_makes_a_demo_account_with_data() {
    let (address, dir) = start(RegistrationMode::Demo, None).await;
    let info = call(address, "GET", "/register", None, None).await;
    assert_eq!(
        info.json(),
        json!({"mode": "demo", "languages": ["de", "en"]})
    );

    let answer = register(
        address,
        &json!({"device": "web"}),
        Some("de-DE,de;q=0.9,en;q=0.5"),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    assert!(
        answer.headers.contains("samu_session="),
        "{}",
        answer.headers
    );
    let made = answer.json();
    let account = made["account"].as_str().unwrap().to_string();
    assert!(
        account.starts_with("demo-") && account.len() == 11,
        "{account}"
    );
    let password = made["password"].as_str().unwrap();
    assert!(password.len() >= 16);
    assert_eq!(origin(&dir, &account), Some(Registered::Demo));
    assert!(projects(&dir, &account).contains(&"Scheibenwelt".to_string()));

    // The token works and the session knows when the account goes.
    let token = made["token"].as_str().unwrap();
    let session = call(address, "GET", "/session", Some(token), None).await;
    assert_eq!(session.status, 200, "{}", session.body);
    assert_eq!(session.json()["account"], account);
    assert!(session.json()["demo_until"].is_string(), "{}", session.body);
    // The password made is the account's.
    let login = call(
        address,
        "POST",
        "/login",
        None,
        Some(&json!({"account": account, "password": password, "device": "x"})),
    )
    .await;
    assert_eq!(login.status, 200, "{}", login.body);
}

#[tokio::test]
async fn a_named_demo_account_takes_the_asked_language() {
    let (address, dir) = start(RegistrationMode::Demo, None).await;
    let body = json!({"name": "probe", "password": PASSWORD, "demo": "en", "device": "web"});
    let answer = register(address, &body, Some("de")).await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    assert!(answer.json().get("password").is_none(), "{}", answer.body);
    assert!(projects(&dir, "probe").contains(&"Discworld".to_string()));
    // Taken now.
    let again = register(address, &body, None).await;
    assert_eq!(again.status, 409, "{}", again.body);
    // Without a language and without Accept-Language: English.
    let plain = json!({"name": "probe2", "password": PASSWORD, "device": "web"});
    assert_eq!(register(address, &plain, None).await.status, 200);
    assert!(projects(&dir, "probe2").contains(&"Discworld".to_string()));
}

#[tokio::test]
async fn open_makes_an_ordinary_account_with_data_only_if_asked() {
    let (address, dir) = start(RegistrationMode::Open, None).await;
    let bare = json!({"name": "verein", "password": PASSWORD, "device": "web"});
    let answer = register(address, &bare, Some("de")).await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    assert_eq!(origin(&dir, "verein"), Some(Registered::Open));
    assert!(projects(&dir, "verein").is_empty());
    let token = answer.json()["token"].as_str().unwrap().to_string();
    let session = call(address, "GET", "/session", Some(&token), None).await;
    assert!(
        session.json().get("demo_until").is_none(),
        "{}",
        session.body
    );

    let with = json!({"name": "familie", "password": PASSWORD, "demo": "de", "device": "web"});
    assert_eq!(register(address, &with, None).await.status, 200);
    assert!(projects(&dir, "familie").contains(&"Scheibenwelt".to_string()));
    // No name made up in `open`.
    assert_eq!(
        register(address, &json!({"device": "web"}), None)
            .await
            .status,
        400
    );
}

#[tokio::test]
async fn a_bad_name_or_short_password_is_refused() {
    let (address, dir) = start(RegistrationMode::Open, None).await;
    for body in [
        json!({"name": "Groß", "password": PASSWORD, "device": "web"}),
        json!({"name": "kurz", "password": "1234567", "device": "web"}),
        json!({"name": "ohne", "device": "web"}),
        json!({"name": "privat2", "password": PASSWORD, "demo": "fr", "device": "web"}),
        json!({"name": "geraet", "password": PASSWORD, "device": " "}),
    ] {
        let answer = register(address, &body, None).await;
        assert_eq!(answer.status, 400, "{body}: {}", answer.body);
    }
    // The CLI's account is taken too.
    let taken = json!({"name": "privat", "password": PASSWORD, "device": "web"});
    assert_eq!(register(address, &taken, None).await.status, 409);
    let server = ServerStore::open(&dir.0.join("server.db")).unwrap();
    assert_eq!(server.accounts().unwrap().len(), 1);
}

/// ADR 0099 §3: ten in an hour, then 429 with `Retry-After`.
#[tokio::test]
async fn the_eleventh_registration_in_an_hour_waits() {
    let (address, _dir) = start(RegistrationMode::Open, None).await;
    for n in 0..10 {
        let body = json!({"name": format!("konto{n}"), "password": PASSWORD, "device": "web"});
        assert_eq!(register(address, &body, None).await.status, 200, "{n}");
    }
    let body = json!({"name": "konto10", "password": PASSWORD, "device": "web"});
    let answer = register(address, &body, None).await;
    assert_eq!(answer.status, 429, "{}", answer.body);
    assert!(
        answer.headers.to_lowercase().contains("retry-after:"),
        "{}",
        answer.headers
    );
}

/// ADR 0099 §3: at most so many registered accounts; the CLI's do not
/// count.
#[tokio::test]
async fn above_the_most_accounts_the_server_takes_no_more() {
    let (address, dir) = start(RegistrationMode::Open, Some(2)).await;
    for name in ["eins", "zwei"] {
        let body = json!({"name": name, "password": PASSWORD, "device": "web"});
        assert_eq!(register(address, &body, None).await.status, 200, "{name}");
    }
    let body = json!({"name": "drei", "password": PASSWORD, "device": "web"});
    let answer = register(address, &body, None).await;
    assert_eq!(answer.status, 503, "{}", answer.body);
    assert!(origin(&dir, "drei").is_none());
}

/// ADR 0099 §5: a registered account deletes itself with its password; the
/// CLI's may not, nor a restricted or wrong one.
#[tokio::test]
async fn a_registered_account_deletes_itself() {
    let (address, dir) = start(RegistrationMode::Open, None).await;
    let body = json!({"name": "weg", "password": PASSWORD, "demo": "en", "device": "web"});
    let answer = register(address, &body, None).await;
    let token = answer.json()["token"].as_str().unwrap().to_string();

    let wrong = call(
        address,
        "POST",
        "/account/delete",
        Some(&token),
        Some(&json!({"password": "falsch!!"})),
    )
    .await;
    assert_eq!(wrong.status, 401, "{}", wrong.body);
    assert!(origin(&dir, "weg").is_some());

    let gone = call(
        address,
        "POST",
        "/account/delete",
        Some(&token),
        Some(&json!({"password": PASSWORD})),
    )
    .await;
    assert_eq!(gone.status, 204, "{}", gone.body);
    assert!(gone.headers.contains("Max-Age=0"), "{}", gone.headers);
    assert!(origin(&dir, "weg").is_none());
    let left: Vec<String> = std::fs::read_dir(dir.0.join("accounts"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|f| f.starts_with("weg."))
        .collect();
    assert!(left.is_empty(), "{left:?}");
    assert_eq!(
        call(address, "GET", "/session", Some(&token), None)
            .await
            .status,
        401
    );
    // The name is free again.
    assert_eq!(register(address, &body, None).await.status, 200);

    // The CLI's account is the operator's.
    let privat = super::sign_in(address).await;
    let refused = call(
        address,
        "POST",
        "/account/delete",
        Some(&privat),
        Some(&json!({"password": PASSWORD})),
    )
    .await;
    assert_eq!(refused.status, 403, "{}", refused.body);
    assert!(origin(&dir, "privat").is_some());
}

/// ADR 0099 §7: `POST /demo` fills an empty account, puts the demo under
/// `Demo` in one with content, and only once.
#[tokio::test]
async fn demo_data_afterwards() {
    let (address, dir) = start(RegistrationMode::Open, None).await;
    let body = json!({"name": "leer", "password": PASSWORD, "device": "web"});
    let token = register(address, &body, None).await.json()["token"]
        .as_str()
        .unwrap()
        .to_string();
    let bad = call(
        address,
        "POST",
        "/demo",
        Some(&token),
        Some(&json!({"lang": "fr"})),
    )
    .await;
    assert_eq!(bad.status, 400, "{}", bad.body);
    assert!(projects(&dir, "leer").is_empty());
    let whole = call(
        address,
        "POST",
        "/demo",
        Some(&token),
        Some(&json!({"lang": "en"})),
    )
    .await;
    assert_eq!(whole.status, 200, "{}", whole.body);
    assert_eq!(whole.json()["whole"], true);
    assert_eq!(whole.json()["counts"]["tasks"], 81);
    assert!(!projects(&dir, "leer").contains(&"Demo".to_string()));

    // An account of one's own, with a tag whose name starts like `Demo`.
    let privat = super::sign_in(address).await;
    let own = call(
        address,
        "POST",
        "/capture",
        Some(&privat),
        Some(&json!({"text": "Eigenes @Demonstration @Person"})),
    )
    .await;
    assert_eq!(own.status, 200, "{}", own.body);
    let shielded = call(
        address,
        "POST",
        "/demo",
        Some(&privat),
        Some(&json!({"lang": "de"})),
    )
    .await;
    assert_eq!(shielded.status, 200, "{}", shielded.body);
    assert_eq!(shielded.json()["whole"], false);
    let titles = projects(&dir, "privat");
    assert!(titles.contains(&"Demo".to_string()), "{titles:?}");
    assert!(titles.contains(&"Eingang".to_string()), "{titles:?}");
    // The own tags have no children from the demo.
    let path = dir.0.join("accounts").join("privat.db");
    let conn = rusqlite::Connection::open(path).unwrap();
    let under_own: i64 = conn
        .query_row(
            "SELECT count(*) FROM tag c JOIN object po ON po.uuid = c.parent JOIN tag p ON p.object = po.id \
             WHERE p.parent IS NULL AND p.title IN ('Demonstration', 'Person')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(under_own, 0);
    let probe: i64 = conn
        .query_row(
            "SELECT count(*) FROM tag c JOIN object po ON po.uuid = c.parent JOIN tag p ON p.object = po.id WHERE p.title = 'Demo'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(probe > 5, "the query finds children: {probe}");

    let again = call(
        address,
        "POST",
        "/demo",
        Some(&privat),
        Some(&json!({"lang": "de"})),
    )
    .await;
    assert_eq!(again.status, 409, "{}", again.body);
}
