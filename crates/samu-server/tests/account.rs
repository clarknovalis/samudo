//! `samu-server account …` (ADR 0019 §3, ADR 0099 §4–5): what `list`
//! shows and what `delete` leaves behind.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use samu_store::AccountStore;

const PASSWORD: &str = "correct horse";

/// A data directory of its own, removed at the end.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "samu-account-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn run(data: &Path, args: &[&str], stdin: Option<&str>) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_samu-server"))
        .arg("--data")
        .arg(data)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(text) = stdin {
        writeln!(child.stdin.take().unwrap(), "{text}").unwrap();
    }
    child.wait_with_output().unwrap()
}

fn listed(data: &Path) -> Vec<Vec<String>> {
    let out = run(data, &["account", "list"], None);
    assert!(out.status.success(), "{out:?}");
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|line| line.split('\t').map(String::from).collect())
        .collect()
}

/// ADR 0099 §5: `delete` takes the database, its blobs and the row, and
/// the name is free again; while the account is held it deletes nothing.
#[test]
fn delete_takes_the_account_with_its_files() {
    let dir = TempDir::new("delete");
    let data = &dir.0;
    for name in ["demo-de", "privat"] {
        let out = run(
            data,
            &["account", "add", name, "--password-stdin"],
            Some(PASSWORD),
        );
        assert!(out.status.success(), "{out:?}");
    }
    let rows = listed(data);
    assert_eq!(rows.len(), 2);
    // name, state, created, registered, last active (= created).
    assert_eq!(rows[0][0], "demo-de");
    assert_eq!(rows[0][1], "active");
    assert_eq!(rows[0][3], "cli");
    assert_eq!(rows[0][4], rows[0][2]);

    let db = data.join("accounts").join("demo-de.db");
    let blobs = db.with_extension("blobs");
    std::fs::create_dir_all(&blobs).unwrap();
    std::fs::write(blobs.join("file"), b"x").unwrap();

    // Held, as by a running server: refused, and nothing is gone.
    let held = AccountStore::open(&db).unwrap();
    let out = run(data, &["account", "delete", "demo-de"], None);
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("in use"),
        "{out:?}"
    );
    assert!(db.exists() && blobs.exists());
    assert_eq!(listed(data).len(), 2);
    drop(held);

    let out = run(data, &["account", "delete", "demo-de"], None);
    assert!(out.status.success(), "{out:?}");
    let mut left: Vec<String> = std::fs::read_dir(data.join("accounts"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("demo-de"))
        .collect();
    left.sort();
    assert!(left.is_empty(), "left behind: {left:?}");
    let rows = listed(data);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][0], "privat");

    // Unknown now: an error, and the name can be taken again.
    let out = run(data, &["account", "delete", "demo-de"], None);
    assert!(!out.status.success(), "{out:?}");
    let out = run(
        data,
        &["account", "add", "demo-de", "--password-stdin"],
        Some(PASSWORD),
    );
    assert!(out.status.success(), "{out:?}");
}

/// ADR 0099 §6: `add --demo` makes the account with the demo content in
/// it, as the database says, not the output.
#[test]
fn add_with_the_demo_fills_the_new_account() {
    let dir = TempDir::new("demo");
    let data = &dir.0;
    let out = run(
        data,
        &[
            "account",
            "add",
            "probe",
            "--password-stdin",
            "--demo",
            "en",
        ],
        Some(PASSWORD),
    );
    assert!(out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("81 tasks"),
        "{out:?}"
    );
    let conn = rusqlite::Connection::open(data.join("accounts").join("probe.db")).unwrap();
    let count = |kind: &str| -> i64 {
        conn.query_row(
            "SELECT count(*) FROM object WHERE kind = ?1 AND deleted_at IS NULL",
            [kind],
            |row| row.get(0),
        )
        .unwrap()
    };
    assert_eq!(count("task"), 81);
    assert_eq!(count("project"), 14);
    assert_eq!(count("perspective"), 7);
    // Without --caldav no calendar.
    assert_eq!(count("calendar"), 0);
}

/// `--caldav` without its password stops before an account is made.
#[test]
fn caldav_without_its_password_makes_nothing() {
    let dir = TempDir::new("caldav");
    let data = &dir.0;
    let mut command = Command::new(env!("CARGO_BIN_EXE_samu-server"));
    command
        .arg("--data")
        .arg(data)
        .args([
            "account",
            "add",
            "probe",
            "--password-stdin",
            "--demo",
            "de",
            "--caldav",
            "http://127.0.0.1:9/",
        ])
        .env_remove("SAMU_DEMO_CALDAV_PASSWORD")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    writeln!(child.stdin.take().unwrap(), "{PASSWORD}").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("SAMU_DEMO_CALDAV_PASSWORD"),
        "{out:?}"
    );
    assert!(listed(data).is_empty());
}
