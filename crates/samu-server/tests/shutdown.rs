// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The server ends on SIGTERM even while an `/events` stream is open
//! (regression, 11.11.2026: graceful shutdown waited for the stream
//! forever, and a restart waited for systemd's or Docker's kill).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]
#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const PASSWORD: &str = "correct horse";

/// A number unique in this test process: tests run as threads of one
/// process, and the clock can give two of them the same directory.
fn unique() -> u32 {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// A data directory of its own under the system's temp directory.
struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn request(address: &str, head: &str, body: &str) -> TcpStream {
    let mut stream = TcpStream::connect(address).unwrap();
    write!(
        stream,
        "{head}\r\nHost: {address}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    stream
}

#[test]
fn sigterm_ends_the_server_with_an_open_event_stream() {
    let dir = TempDir(std::env::temp_dir().join(format!(
        "samu-shutdown-{}-{}",
        std::process::id(),
        unique()
    )));
    std::fs::create_dir_all(&dir.0).unwrap();
    let binary = env!("CARGO_BIN_EXE_samu-server");
    let mut add = Command::new(binary)
        .args([
            "--data",
            dir.0.to_str().unwrap(),
            "account",
            "add",
            "privat",
        ])
        .arg("--password-stdin")
        .stdin(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    writeln!(add.stdin.take().unwrap(), "{PASSWORD}").unwrap();
    assert!(add.wait().unwrap().success());

    let mut server = Command::new(binary)
        .args(["--data", dir.0.to_str().unwrap(), "serve"])
        .args(["--listen", "127.0.0.1:0"])
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // The first line says where it listens.
    let mut first = String::new();
    BufReader::new(server.stderr.take().unwrap())
        .read_line(&mut first)
        .unwrap();
    let address = first.trim().rsplit(' ').next().unwrap().to_string();

    let mut login = request(
        &address,
        "POST /login HTTP/1.1\r\nConnection: close",
        &format!(r#"{{"account": "privat", "password": "{PASSWORD}", "device": "t"}}"#),
    );
    let mut answer = String::new();
    login.read_to_string(&mut answer).unwrap();
    let token = answer
        .split(r#""token":""#)
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("a token")
        .to_string();

    let mut events = request(
        &address,
        &format!("GET /events HTTP/1.1\r\nAuthorization: Bearer {token}"),
        "",
    );
    // The stream is open once its head came.
    let mut head = [0u8; 12];
    events.read_exact(&mut head).unwrap();
    assert!(head.starts_with(b"HTTP/1.1 200"));

    let terminated = Command::new("kill")
        .args(["-TERM", &server.id().to_string()])
        .status()
        .unwrap();
    assert!(terminated.success());
    let asked = Instant::now();
    loop {
        if server.try_wait().unwrap().is_some() {
            break;
        }
        if asked.elapsed() > Duration::from_secs(10) {
            // Only this process, by its id: a kill by name would hit every
            // samu-server on the machine.
            let _ = server.kill();
            panic!("the server was still running 10 s after SIGTERM");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    drop(events);
}
