// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `GET /export`, `samu-server export` and `samu-server restore`
//! (ADR 0074 §5, §6), against a running server. Synthetic data only.

use samu_store::core::api::{Export, ObjectKind, kind_of};
use samu_store::{AccountStore, ServerStore};
use serde_json::json;

use super::{PASSWORD, TempDir, call, create, new_task, sign_in, start};

#[tokio::test]
async fn the_route_answers_with_the_whole_account() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let (id, task) = new_task("Milch");
    let answer = call(
        address,
        "POST",
        "/changes",
        Some(&token),
        Some(&json!([create(&id, &task)])),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);

    let answer = call(address, "GET", "/export", Some(&token), None).await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    let export: Export = serde_json::from_str(&answer.body).unwrap();
    assert_eq!(export.manifest.account, "privat");
    let task = export
        .objects
        .iter()
        .find(|o| kind_of(&o.object) == ObjectKind::Task)
        .unwrap();
    assert!(task.sync.stamps.contains_key("@create"));
    assert_eq!(task.sync.author.as_ref().unwrap().device, "laptop");

    // Without a session, nothing.
    let answer = call(address, "GET", "/export", None, None).await;
    assert_eq!(answer.status, 401);
}

/// Exported while the server holds the account, restored into a fresh data
/// directory, exported again: the same files but the time.
#[tokio::test]
async fn an_export_beside_the_running_server_restores_elsewhere() {
    let (address, dir) = start(true).await;
    let token = sign_in(address).await;
    for title in ["Milch", "Brot"] {
        let (id, task) = new_task(title);
        let answer = call(
            address,
            "POST",
            "/changes",
            Some(&token),
            Some(&json!([create(&id, &task)])),
        )
        .await;
        assert_eq!(answer.status, 200, "{}", answer.body);
    }
    // The server holds the lock now.
    assert!(AccountStore::open(&crate::app::account_path(&dir.0, "privat")).is_err());

    let out = TempDir::new();
    crate::export::export(&dir.0, "privat", &out.0).unwrap();
    let first = crate::export::read(&out.0).unwrap();
    assert_eq!(
        first
            .objects
            .iter()
            .filter(|o| kind_of(&o.object) == ObjectKind::Task)
            .count(),
        2
    );

    // A second export of the same state writes nothing new but the time.
    let tasks = std::fs::metadata(out.0.join("tasks.json"))
        .unwrap()
        .modified()
        .unwrap();
    crate::export::export(&dir.0, "privat", &out.0).unwrap();
    assert_eq!(
        std::fs::metadata(out.0.join("tasks.json"))
            .unwrap()
            .modified()
            .unwrap(),
        tasks
    );

    // Into a data directory without the account: refused.
    let other = TempDir::new();
    let err = crate::export::restore(&other.0, "privat", &out.0).unwrap_err();
    assert!(err.to_string().contains("account add"), "{err}");

    ServerStore::open(&other.0.join("server.db"))
        .unwrap()
        .add_account("privat", PASSWORD, jiff::Timestamp::now())
        .unwrap();
    crate::export::restore(&other.0, "privat", &out.0).unwrap();
    // Twice is refused: the account is not empty any more.
    let err = crate::export::restore(&other.0, "privat", &out.0).unwrap_err();
    assert!(err.to_string().contains("not empty"), "{err}");

    let again = TempDir::new();
    crate::export::export(&other.0, "privat", &again.0).unwrap();
    for name in ["tasks.json", "settings.json", "logbook.json", "files.json"] {
        assert_eq!(
            std::fs::read_to_string(out.0.join(name)).unwrap(),
            std::fs::read_to_string(again.0.join(name)).unwrap(),
            "{name}"
        );
    }
    // Asked of the database.
    let conn = rusqlite::Connection::open(crate::app::account_path(&other.0, "privat")).unwrap();
    let tasks: i64 = conn
        .query_row("SELECT count(*) FROM task", [], |r| r.get(0))
        .unwrap();
    assert_eq!(tasks, 2);
}
