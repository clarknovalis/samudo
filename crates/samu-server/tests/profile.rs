//! `samu-server profile <name>` (ADR 0086 §2–§4): a local profile is a
//! server of its own on the loopback, an online one the replica; both
//! answer only to their own `Host`. Every process is stopped by its PID.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]
#![cfg(unix)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

const PASSWORD: &str = "correct horse";

/// A number unique in this test process: tests run as threads of one
/// process, and the clock can give two of them the same directory.
fn unique() -> u32 {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// A directory of its own for config and data, removed at the end.
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "samu-profile-{name}-{}-{}",
            std::process::id(),
            unique()
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

/// A child stopped by its PID, never by name.
struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A port nothing listens on: bound once, then let go.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn server(dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_samu-server"));
    command
        .env("SAMU_CONFIG", dir.join("config.toml"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .env_remove("SAMU_DATA")
        .env_remove("SAMU_SECRET_KEY");
    command
}

fn add_account(dir: &Path, profile: &str, account: &str) -> Output {
    let mut add = server(dir)
        .args([
            "profile",
            profile,
            "account",
            "add",
            account,
            "--password-stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(add.stdin.take().unwrap(), "{PASSWORD}").unwrap();
    add.wait_with_output().unwrap()
}

/// Starts a profile and waits for its first line, which says where it
/// listens.
fn start(dir: &Path, profile: &str) -> (Running, String) {
    let mut child = server(dir)
        .args(["profile", profile])
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut first = String::new();
    let mut lines = BufReader::new(child.stderr.take().unwrap());
    // A local profile without an account says so before it listens.
    loop {
        first.clear();
        lines.read_line(&mut first).unwrap();
        if first.starts_with("samu-server") || first.is_empty() {
            break;
        }
    }
    (Running(child), first)
}

/// The status of one request with the given `Host`.
fn status(port: u16, host: &str, method: &str, path: &str, body: &str) -> u16 {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    answer.split(' ').nth(1).unwrap().parse().unwrap()
}

#[test]
fn a_local_profile_is_a_server_of_its_own_on_the_loopback() {
    let dir = TempDir::new("local");
    let port = free_port();
    std::fs::write(
        dir.0.join("config.toml"),
        format!("[profile.lokal]\naccount = \"work\"\nport = {port}\n"),
    )
    .unwrap();

    let added = add_account(&dir.0, "lokal", "work");
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    assert!(
        dir.0.join("data/samu/lokal/server.db").exists(),
        "the profile's data lie under $XDG_DATA_HOME/samu/<name>"
    );
    assert!(dir.0.join("data/samu/lokal/accounts/work.db").exists());

    let (_server, first) = start(&dir.0, "lokal");
    assert!(first.contains(&format!("127.0.0.1:{port}")), "{first}");

    let own = format!("127.0.0.1:{port}");
    assert_eq!(status(port, &own, "GET", "/health", ""), 200);
    assert_eq!(
        status(port, &format!("localhost:{port}"), "GET", "/health", ""),
        200
    );
    let login = format!("{{\"account\":\"work\",\"password\":\"{PASSWORD}\",\"device\":\"t\"}}");
    assert_eq!(status(port, &own, "POST", "/login", &login), 200);
    assert_eq!(
        status(port, &format!("evil.example:{port}"), "GET", "/health", ""),
        421,
        "DNS rebinding: another host name is refused (ADR 0086 §3)"
    );
    assert_eq!(status(port, "127.0.0.1:1", "GET", "/health", ""), 421);
    // Its name in the browser, through the front or straight (ADR 0087 §3).
    assert_eq!(
        status(port, "lokal.samu.localhost", "GET", "/health", ""),
        200
    );
    assert_eq!(
        status(
            port,
            &format!("lokal.samu.localhost:{port}"),
            "GET",
            "/health",
            ""
        ),
        200
    );
    assert_eq!(
        status(port, "home.samu.localhost", "GET", "/health", ""),
        421
    );
}

#[test]
fn an_online_profile_is_the_replica_and_checks_the_host_too() {
    let dir = TempDir::new("online");
    let port = free_port();
    // Nothing listens upstream: the replica starts all the same (ADR 0066).
    std::fs::write(
        dir.0.join("config.toml"),
        format!(
            "[profile.home]\naccount = \"alice\"\nupstream = \"http://127.0.0.1:{}\"\nport = {port}\n",
            free_port()
        ),
    )
    .unwrap();

    let added = add_account(&dir.0, "home", "alice");
    assert!(!added.status.success());
    assert!(
        String::from_utf8_lossy(&added.stderr).contains("managed there"),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );

    let (_replica, first) = start(&dir.0, "home");
    assert!(first.contains("replica of"), "{first}");
    assert!(dir.0.join("data/samu/home").exists());
    assert_eq!(
        status(port, &format!("127.0.0.1:{port}"), "GET", "/health", ""),
        200
    );
    assert_eq!(
        status(port, &format!("evil.example:{port}"), "GET", "/health", ""),
        421
    );
    assert_eq!(
        status(port, "home.samu.localhost", "GET", "/health", ""),
        200
    );
}

#[test]
fn an_unknown_profile_fails() {
    let dir = TempDir::new("unknown");
    std::fs::write(dir.0.join("config.toml"), "").unwrap();
    let out = server(&dir.0).args(["profile", "nix"]).output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("nix"));
}

/// `deploy/samu-export-profile.sh` with this build's `samu-server` first
/// on the path, and no git configuration of the user.
fn export_script(dir: &Path, profile: &str) -> Output {
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../deploy/samu-export-profile.sh");
    let bin = Path::new(env!("CARGO_BIN_EXE_samu-server"))
        .parent()
        .unwrap();
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    Command::new("sh")
        .arg(script)
        .arg(profile)
        .env("PATH", path)
        .env("SAMU_CONFIG", dir.join("config.toml"))
        .env("XDG_DATA_HOME", dir.join("data"))
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap()
}

fn commits(repo: &Path) -> usize {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-list", "--count", "HEAD"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

#[test]
fn a_local_profile_is_kept_in_git_without_a_remote() {
    let dir = TempDir::new("export");
    std::fs::write(
        dir.0.join("config.toml"),
        format!(
            "[profile.lokal]\naccount = \"work\"\nport = {}\n\
             [profile.home]\naccount = \"alice\"\nupstream = \"https://samu.example\"\nport = {}\n",
            free_port(),
            free_port()
        ),
    )
    .unwrap();
    assert!(add_account(&dir.0, "lokal", "work").status.success());

    let first = export_script(&dir.0, "lokal");
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let repo = dir.0.join("data/samu/lokal/export");
    assert!(repo.join("manifest.json").exists());
    assert_eq!(commits(&repo), 1);

    // Nothing changed but the time: no second commit.
    let again = export_script(&dir.0, "lokal");
    assert!(
        again.status.success(),
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
    assert_eq!(commits(&repo), 1);

    let remotes = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .arg("remote")
        .output()
        .unwrap();
    assert!(remotes.stdout.is_empty(), "no remote (ADR 0086 §10)");

    let online = server(&dir.0)
        .args(["profile", "home", "export"])
        .output()
        .unwrap();
    assert!(!online.status.success());
    assert!(String::from_utf8_lossy(&online.stderr).contains("kept there"));
}

/// One answer through a port with the given `Host`: status and body.
fn get(port: u16, host: &str, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    let status = answer.split(' ').nth(1).unwrap().parse().unwrap();
    let body = answer
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    (status, body)
}

/// Starts the front and waits for its first line.
fn start_front(dir: &Path) -> (Running, String) {
    let mut child = server(dir)
        .arg("front")
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut first = String::new();
    let mut lines = BufReader::new(child.stderr.take().unwrap());
    lines.read_line(&mut first).unwrap();
    // It goes on saying things; a closed pipe would end it.
    std::thread::spawn(move || {
        for line in lines.lines().map_while(Result::ok) {
            eprintln!("front: {line}");
        }
    });
    (Running(child), first)
}

/// ADR 0087 §2: the front hands a connection to the profile its `Host`
/// names, lists the profiles under samu.localhost, says which profile is
/// not running, and reads the file again when it changes.
#[test]
fn the_front_hands_each_name_to_its_profile() {
    let dir = TempDir::new("front");
    let port = free_port();
    let front = free_port();
    let config =
        format!("[front]\nport = {front}\n[profile.lokal]\naccount = \"work\"\nport = {port}\n");
    std::fs::write(dir.0.join("config.toml"), &config).unwrap();
    assert!(add_account(&dir.0, "lokal", "work").status.success());
    let (_server, _) = start(&dir.0, "lokal");
    let (_front, first) = start_front(&dir.0);
    assert!(
        first.contains(&format!("front on 127.0.0.1:{front}")),
        "{first}"
    );

    let (status, body) = get(front, "lokal.samu.localhost", "/health");
    assert_eq!(status, 200, "{body}");
    let (status, _) = get(front, &format!("LOKAL.samu.localhost:{front}"), "/health");
    assert_eq!(status, 200, "the name in any case, with the port");

    let (status, page) = get(front, "samu.localhost", "/");
    assert_eq!(status, 200);
    assert!(
        page.contains(&format!("href=\"http://lokal.samu.localhost:{front}\"")),
        "{page}"
    );

    let (status, body) = get(front, "andere.samu.localhost", "/health");
    assert_eq!(status, 421, "{body}");
    let (status, _) = get(front, "evil.example", "/health");
    assert_eq!(status, 421);

    // A new profile, not running yet: the front knows it without a restart.
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(
        dir.0.join("config.toml"),
        format!(
            "{config}[profile.zwei]\naccount = \"b\"\nport = {}\n",
            free_port()
        ),
    )
    .unwrap();
    let (status, body) = get(front, "zwei.samu.localhost", "/health");
    assert_eq!(status, 502, "{body}");
    assert!(body.contains("systemctl --user start samu@zwei"), "{body}");
    let (status, _) = get(front, "lokal.samu.localhost", "/health");
    assert_eq!(status, 200);
}
