//! Open accounts close after a rest, and the running server deletes
//! accounts: expired demo accounts (ADR 0099 §4–5; 13.11.2026, to
//! `0b785034`).

use std::time::{Duration, Instant};

use jiff::{SignedDuration, Timestamp};
use samu_store::{Registered, ServerStore};

use super::{PASSWORD, call, sign_in, start_state, stream_until};
use crate::app::IDLE;

fn files_of(dir: &std::path::Path, name: &str) -> Vec<String> {
    std::fs::read_dir(dir.join("accounts"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|f| f.starts_with(&format!("{name}.")))
        .collect()
}

/// An account is open while it is asked for; after `IDLE` without a request
/// it closes, and the next request opens it again.
#[tokio::test]
async fn an_account_closes_after_a_rest_and_opens_again() {
    let (address, _dir, state) = start_state(true, |s| s).await;
    let token = sign_in(address).await;
    assert_eq!(
        call(address, "GET", "/session", Some(&token), None)
            .await
            .status,
        200
    );
    assert!(state.is_open("privat"));
    assert!(state.close_idle(Instant::now()).is_empty(), "used just now");
    assert_eq!(state.close_idle(Instant::now() + IDLE), vec!["privat"]);
    assert!(!state.is_open("privat"));
    let answer = call(address, "GET", "/changes?after=0", Some(&token), None).await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    assert!(state.is_open("privat"));
}

/// A closed account without calendars is not opened by the calendar loops;
/// one never closed since the start is looked into once.
#[tokio::test]
async fn the_calendar_loops_leave_a_closed_account_without_calendars_closed() {
    let (address, _dir, state) = start_state(true, |s| s).await;
    assert!(state.has_calendar_work("privat"), "not known yet");
    assert!(crate::calendar::due(&state).unwrap().is_empty());
    assert!(state.is_open("privat"), "looked into once");
    state.close_idle(Instant::now() + IDLE);
    assert!(!state.has_calendar_work("privat"));
    assert!(crate::calendar::due(&state).unwrap().is_empty());
    assert!(!state.is_open("privat"), "left closed");
    // Opened by a request, it is looked into again.
    let token = sign_in(address).await;
    call(address, "GET", "/session", Some(&token), None).await;
    assert!(state.has_calendar_work("privat"));
}

/// An open `/events` stream counts as use; deleting the account ends it,
/// then takes its files, row and sessions.
#[tokio::test]
async fn a_listening_stream_holds_an_account_until_it_is_deleted() {
    let (address, dir, state) = start_state(true, |s| s).await;
    let token = sign_in(address).await;
    let listening = {
        let token = token.clone();
        tokio::spawn(
            async move { stream_until(address, "/events", &token, "", "never comes").await },
        )
    };
    // The stream has opened the account once its first comment is out.
    let start = Instant::now();
    while !state.is_open("privat") && start.elapsed() < Duration::from_secs(5) {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        state.close_idle(Instant::now() + IDLE).is_empty(),
        "a stream listens"
    );
    assert!(!files_of(&dir.0, "privat").is_empty());

    state.delete_account("privat").await.unwrap();
    let seen = listening.await.unwrap();
    assert!(seen.contains("connected"), "{seen}");
    assert!(
        files_of(&dir.0, "privat").is_empty(),
        "{:?}",
        files_of(&dir.0, "privat")
    );
    let server = ServerStore::open(&dir.0.join("server.db")).unwrap();
    assert!(server.account_info("privat").unwrap().is_none());
    let answer = call(address, "GET", "/session", Some(&token), None).await;
    assert_eq!(answer.status, 401, "{}", answer.body);
}

/// ADR 0099 §4: a registered demo account goes `SAMU_DEMO_ACCOUNT_DAYS`
/// after its last use; a younger one and the CLI's stay.
#[tokio::test]
async fn an_unused_demo_account_expires() {
    let (address, dir, state) = start_state(true, |s| s.with_demo_days(Some(3))).await;
    let made = Timestamp::now();
    {
        let mut server = ServerStore::open(&dir.0.join("server.db")).unwrap();
        for name in ["demo-old", "demo-new"] {
            server
                .add_account_as(name, PASSWORD, Registered::Demo, made)
                .unwrap();
        }
        server
            .add_account_as("verein", PASSWORD, Registered::Open, made)
            .unwrap();
        let later = made + SignedDuration::from_hours(24);
        server.touch("demo-new", later).unwrap();
    }
    // Opened, so there are files to delete.
    for name in ["demo-old", "demo-new"] {
        state.account(name).unwrap();
    }
    let token = sign_in(address).await;

    let before = made + SignedDuration::from_hours(3 * 24 - 1);
    assert!(state.expire_demo_accounts(before).await.is_empty());
    let after = made + SignedDuration::from_hours(3 * 24);
    assert_eq!(state.expire_demo_accounts(after).await, vec!["demo-old"]);
    assert!(files_of(&dir.0, "demo-old").is_empty());
    assert!(!files_of(&dir.0, "demo-new").is_empty());

    let server = ServerStore::open(&dir.0.join("server.db")).unwrap();
    let names: Vec<String> = server
        .accounts()
        .unwrap()
        .into_iter()
        .map(|a| a.name)
        .collect();
    assert_eq!(names, vec!["demo-new", "privat", "verein"]);
    // Long after: every demo account goes, the others never.
    let much_later = made + SignedDuration::from_hours(100 * 24);
    assert_eq!(
        state.expire_demo_accounts(much_later).await,
        vec!["demo-new"]
    );
    assert_eq!(
        call(address, "GET", "/session", Some(&token), None)
            .await
            .status,
        200
    );
}

/// ADR 0099 §4: using an account moves `last_active` on.
#[tokio::test]
async fn using_an_account_moves_last_active_on() {
    let (address, dir, _state) = start_state(true, |s| s).await;
    let made = {
        let server = ServerStore::open(&dir.0.join("server.db")).unwrap();
        server.account_info("privat").unwrap().unwrap().last_active
    };
    let token = sign_in(address).await;
    {
        // Set back, as if the sign-in were long ago.
        let mut server = ServerStore::open(&dir.0.join("server.db")).unwrap();
        server.touch("privat", made).unwrap();
    }
    call(address, "GET", "/session", Some(&token), None).await;
    let server = ServerStore::open(&dir.0.join("server.db")).unwrap();
    assert!(server.account_info("privat").unwrap().unwrap().last_active > made);
}
