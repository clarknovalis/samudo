//! The command line against a real `samu-server`, started from its binary
//! with a data directory of its own and stopped by its PID, never by name:
//! a kill by name would hit every samu-server on the machine. Exit codes
//! are part of the interface (ADR 0019 §7).

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

const PASSWORD: &str = "correct horse";

/// A number unique in this test process: tests run as threads of one
/// process, and the clock can give two of them the same directory.
fn unique() -> u32 {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// `samu-server` beside `samu`: `cargo test --workspace` builds both.
fn server_binary() -> PathBuf {
    let path = Path::new(env!("CARGO_BIN_EXE_samu"))
        .parent()
        .unwrap()
        .join("samu-server");
    assert!(
        path.exists(),
        "{} is missing: run `cargo build -p samu-server` (make check does)",
        path.display()
    );
    path
}

struct Server {
    child: Child,
    url: String,
    dir: PathBuf,
}

impl Server {
    /// A server with one account `privat`, or none.
    fn start(with_account: bool) -> Self {
        let dir =
            std::env::temp_dir().join(format!("samu-cli-{}-{}", std::process::id(), unique()));
        std::fs::create_dir_all(&dir).unwrap();
        if with_account {
            let mut add = Command::new(server_binary())
                .args(["--data"])
                .arg(dir.join("data"))
                .args(["account", "add", "privat", "--password-stdin"])
                .stdin(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            writeln!(add.stdin.take().unwrap(), "{PASSWORD}").unwrap();
            let added = add.wait_with_output().unwrap();
            assert!(
                added.status.success(),
                "account add: {}",
                String::from_utf8_lossy(&added.stderr)
            );
        }
        let mut child = Command::new(server_binary())
            .arg("--data")
            .arg(dir.join("data"))
            .args(["serve", "--listen", "127.0.0.1:0"])
            // 32 bytes of 3: seals calendar sources (ADR 0047 §3).
            .env(
                "SAMU_SECRET_KEY",
                "AwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwM=",
            )
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        // The server says where it listens on its first line.
        let mut line = String::new();
        BufReader::new(child.stderr.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let address = line.trim().rsplit(' ').next().unwrap().to_string();
        Self {
            child,
            url: format!("http://{address}"),
            dir,
        }
    }

    fn samu(&self, args: &[&str], stdin: Option<&str>) -> Output {
        run_samu(&self.url, &self.dir, args, stdin)
    }

    /// As `samu`, on a computer in the zone `tz`.
    fn samu_in_zone(&self, tz: &str, args: &[&str]) -> Output {
        run_samu_env(&self.url, &self.dir, args, None, &[("TZ", tz)])
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        // By its PID, never by name: catastr's real daemons run here.
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn run_samu(url: &str, dir: &Path, args: &[&str], stdin: Option<&str>) -> Output {
    run_samu_env(url, dir, args, stdin, &[])
}

fn run_samu_env(
    url: &str,
    dir: &Path,
    args: &[&str],
    stdin: Option<&str>,
    env: &[(&str, &str)],
) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_samu"))
        .args(args)
        .envs(env.iter().copied())
        .env("SAMU_SERVER", url)
        .env("SAMU_ACCOUNT", "privat")
        .env("SAMU_DEVICE", "test")
        .env("SAMU_STATE", dir.join("state"))
        .env("SAMU_CONFIG", dir.join("no-config.toml"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(text) = stdin {
        writeln!(child.stdin.take().unwrap(), "{text}").unwrap();
    } else {
        drop(child.stdin.take());
    }
    child.wait_with_output().unwrap()
}

fn code(output: &Output) -> i32 {
    output.status.code().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn health_answers() {
    let server = Server::start(false);
    let out = server.samu(&["health", "--format", "json"], None);
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let records: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    assert_eq!(records[0]["status"], "ok");
}

#[test]
fn login_session_logout() {
    let server = Server::start(true);
    let login = server.samu(&["login", "--password-stdin"], Some(PASSWORD));
    assert_eq!(
        code(&login),
        0,
        "{}",
        String::from_utf8_lossy(&login.stderr)
    );

    let token = server.dir.join("state/privat.token");
    let mode =
        std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&token).unwrap().permissions());
    assert_eq!(mode & 0o777, 0o600, "the token is the user's alone");

    let session = server.samu(&["session", "--format", "tsv"], None);
    assert_eq!(code(&session), 0);
    assert!(
        stdout(&session).starts_with("privat\ttest\t"),
        "{}",
        stdout(&session)
    );

    assert_eq!(code(&server.samu(&["logout"], None)), 0);
    assert!(!token.exists());
    assert_eq!(
        code(&server.samu(&["session"], None)),
        6,
        "signed out is exit 6"
    );
}

/// ADR 0095 §7: `samu sessions` lists the account's sessions without
/// tokens, `samu session end` ends one by its id; an unknown id is exit 3,
/// and an agent may do neither.
#[test]
fn sessions_are_listed_and_ended() {
    let server = Server::start(true);
    assert_eq!(
        code(&server.samu(&["login", "--password-stdin"], Some(PASSWORD))),
        0
    );
    let agent_dir = server.dir.join("agent");
    let agent = |args: &[&str], stdin| run_samu(&server.url, &agent_dir, args, stdin);
    assert_eq!(
        code(&agent(
            &["login", "--agent", "--password-stdin"],
            Some(PASSWORD)
        )),
        0
    );
    assert_eq!(code(&agent(&["sessions"], None)), 1, "not an agent's");

    let listed = json_out(&server.samu(&["sessions", "--format", "json"], None));
    assert_eq!(listed.as_array().unwrap().len(), 2, "{listed}");
    let token = std::fs::read_to_string(server.dir.join("state/privat.token")).unwrap();
    assert!(!listed.to_string().contains(token.trim()));
    let theirs = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["current"] == false)
        .unwrap();
    assert_eq!(theirs["kind"], "agent");
    let id = theirs["id"].as_str().unwrap();

    assert_eq!(
        code(&server.samu(&["session", "end", "ffffffffffff"], None)),
        3
    );
    assert_eq!(code(&agent(&["session"], None)), 0, "the probe");
    let ended = server.samu(&["session", "end", id], None);
    assert_eq!(
        code(&ended),
        0,
        "{}",
        String::from_utf8_lossy(&ended.stderr)
    );
    assert_eq!(code(&agent(&["session"], None)), 6);
    let tsv = stdout(&server.samu(&["sessions", "--format", "tsv"], None));
    assert_eq!(tsv.lines().count(), 1, "{tsv}");
}

/// `samu login --only` (ADR 0095 §8): a path is looked up before, the
/// session sees only its grant, `samu session` shows it, and the session
/// used for the lookup is gone afterwards.
#[test]
fn a_login_only_for_a_folder_sees_only_that() {
    let server = signed_in_server();
    let made = |args: &[&str]| {
        json_out(&server.samu(args, None))[0]["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    made(&["folder", "add", "tec", "--format", "json"]);
    let ai = made(&["folder", "add", "ai", "--parent", "tec", "--format", "json"]);
    // The second folder's title alone is ambiguous; its path is not.
    made(&["folder", "add", "priv", "--format", "json"]);
    made(&[
        "folder", "add", "ai", "--parent", "priv", "--format", "json",
    ]);
    let inside = made(&[
        "project", "add", "Werkbank", "--folder", &ai, "--format", "json",
    ]);
    made(&["project", "add", "Draußen", "--format", "json"]);

    let agent_dir = server.dir.join("agent");
    let agent = |args: &[&str], stdin| run_samu(&server.url, &agent_dir, args, stdin);
    let ambiguous = agent(
        &["login", "--agent", "--only", "ai", "--password-stdin"],
        Some(PASSWORD),
    );
    assert_eq!(
        code(&ambiguous),
        1,
        "{}",
        String::from_utf8_lossy(&ambiguous.stderr)
    );
    assert_eq!(
        code(&agent(
            &[
                "login",
                "--agent",
                "--only",
                "tec/nichts",
                "--password-stdin"
            ],
            Some(PASSWORD)
        )),
        3
    );
    let login = agent(
        &[
            "login",
            "--agent",
            "--only",
            "tec/ai",
            "--read-only",
            "--password-stdin",
        ],
        Some(PASSWORD),
    );
    assert_eq!(
        code(&login),
        0,
        "{}",
        String::from_utf8_lossy(&login.stderr)
    );
    assert!(
        String::from_utf8_lossy(&login.stderr).contains("only tec/ai (read-only)"),
        "{}",
        String::from_utf8_lossy(&login.stderr)
    );

    let session = json_out(&agent(&["session", "--format", "json"], None));
    assert_eq!(session[0]["access"], "ro", "{session}");
    assert_eq!(session[0]["only"], serde_json::json!([ai]), "{session}");
    assert_eq!(session[0]["grant"], "read-only: ai", "{session}");
    let projects = json_out(&agent(&["projects", "--format", "json"], None));
    let ids: Vec<&str> = projects
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, [inside.as_str()], "{projects}");
    assert_eq!(
        code(&agent(&["add", "Hobeln", "--project", &inside], None)),
        1,
        "read-only"
    );

    // The person's own and the agent's: no session left from the lookups.
    let sessions = json_out(&server.samu(&["sessions", "--format", "json"], None));
    assert_eq!(sessions.as_array().unwrap().len(), 2, "{sessions}");
    // A whole account's session shows no grant.
    let own = json_out(&server.samu(&["session", "--format", "json"], None));
    assert!(
        own[0]["access"].is_null() && own[0].get("grant").is_none(),
        "{own}"
    );
}

#[test]
fn a_wrong_password_is_exit_6() {
    let server = Server::start(true);
    let out = server.samu(&["login", "--password-stdin"], Some("wrong wrong"));
    assert_eq!(code(&out), 6);
    assert!(String::from_utf8_lossy(&out.stderr).contains("wrong account or password"));
}

#[test]
fn a_session_the_server_forgot_is_exit_6() {
    let server = Server::start(true);
    assert_eq!(
        code(&server.samu(&["login", "--password-stdin"], Some(PASSWORD))),
        0
    );
    std::fs::write(server.dir.join("state/privat.token"), "0000").unwrap();
    assert_eq!(code(&server.samu(&["session"], None)), 6);
}

#[test]
fn no_server_is_exit_4() {
    let dir = std::env::temp_dir().join(format!("samu-cli-none-{}", std::process::id()));
    // Port 9 (discard) on loopback: nothing listens there.
    let out = run_samu("http://127.0.0.1:9", &dir, &["health"], None);
    assert_eq!(code(&out), 4, "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn a_wrong_invocation_is_exit_2() {
    let dir = std::env::temp_dir();
    assert_eq!(
        code(&run_samu(
            "http://127.0.0.1:9",
            &dir,
            &["no-such-command"],
            None
        )),
        2
    );
}

fn signed_in_server() -> Server {
    let server = Server::start(true);
    let login = server.samu(&["login", "--password-stdin"], Some(PASSWORD));
    assert_eq!(
        code(&login),
        0,
        "{}",
        String::from_utf8_lossy(&login.stderr)
    );
    server
}

fn json_out(output: &Output) -> serde_json::Value {
    assert_eq!(
        code(output),
        0,
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_str(&stdout(output)).unwrap()
}

/// ADR 0033: `done` on a repetition names the next task on stderr, and
/// the server holds it.
#[test]
fn done_on_a_repetition_names_the_next_task() {
    let server = signed_in_server();
    let added = json_out(&server.samu(
        &[
            "add",
            "Müll raus due:2026-11-10 every:1w",
            "--format",
            "json",
        ],
        None,
    ));
    let id = added[0]["id"].as_str().unwrap().to_string();
    // Before: the list says when the next one would come.
    let listed = json_out(&server.samu(&["list", "--format", "json"], None));
    let row = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == id.as_str())
        .unwrap();
    assert_eq!(row["next_repeat"], "2026-11-17", "{row}");
    let done = server.samu(&["done", &id], None);
    assert_eq!(code(&done), 0);
    let said = String::from_utf8_lossy(&done.stderr).to_string();
    assert!(said.contains("next: "), "{said}");
    assert!(said.contains("Müll raus due 2026-11-17"), "{said}");

    let shown = json_out(&server.samu(&["changes", "--format", "json"], None));
    let next: Vec<&serde_json::Value> = shown
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["object"]["repeat_of"] == id.as_str())
        .collect();
    assert_eq!(next.len(), 1, "{shown}");
    assert_eq!(next[0]["object"]["due"]["date"], "2026-11-17");

    // A task that does not repeat says nothing of a next one.
    let plain = json_out(&server.samu(&["add", "Brot", "--format", "json"], None));
    let done = server.samu(&["done", plain[0]["id"].as_str().unwrap()], None);
    assert_eq!(code(&done), 0);
    assert!(!String::from_utf8_lossy(&done.stderr).contains("next:"));
}

#[test]
fn add_show_set_done_delete() {
    let server = signed_in_server();
    let added = json_out(&server.samu(&["add", "Milch", "kaufen", "--format", "json"], None));
    let id = added[0]["id"].as_str().unwrap().to_string();
    let short = added[0]["short"].as_str().unwrap().to_string();

    // By the end of the id, like git.
    let shown = json_out(&server.samu(&["show", &short, "--format", "json"], None));
    assert_eq!(shown[0]["title"], "Milch kaufen");
    assert_eq!(shown[0]["id"], id.as_str());

    assert_eq!(
        code(&server.samu(&["set", &short, "flagged", "true"], None)),
        0
    );
    assert_eq!(
        code(&server.samu(&["set", &short, "note", "zwei Liter"], None)),
        0
    );
    assert_eq!(code(&server.samu(&["done", &id], None)), 0);
    let shown = json_out(&server.samu(&["show", &id, "--format", "json"], None));
    assert_eq!(shown[0]["object"]["flagged"], true);
    assert_eq!(shown[0]["object"]["note"], "zwei Liter");
    assert!(shown[0]["object"]["completed_at"].is_string());

    // A wrong type is the server's refusal, exit 1.
    assert_eq!(
        code(&server.samu(&["set", &id, "flagged", "\"ja\""], None)),
        1
    );

    assert_eq!(code(&server.samu(&["delete", &id], None)), 0);
    assert_eq!(
        code(&server.samu(&["show", &id], None)),
        3,
        "deleted is not found"
    );
    let changes = server.samu(&["changes", "--format", "tsv"], None);
    assert!(
        stdout(&changes).contains(&format!("gone\t{short}")),
        "{}",
        stdout(&changes)
    );
}

/// ADR 0023: a property is set and removed key by key.
#[test]
fn set_and_unset_a_property() {
    let server = signed_in_server();
    let added = json_out(&server.samu(&["add", "Rauchmelder", "--format", "json"], None));
    let id = added[0]["id"].as_str().unwrap().to_string();
    for (key, value) in [("org_id", "3f0c2a1e"), ("source", "org-import")] {
        let field = format!("properties.{key}");
        assert_eq!(code(&server.samu(&["set", &id, &field, value], None)), 0);
    }
    let shown = json_out(&server.samu(&["show", &id, "--format", "json"], None));
    assert_eq!(
        shown[0]["object"]["properties"],
        serde_json::json!({"org_id": "3f0c2a1e", "source": "org-import"})
    );

    assert_eq!(
        code(&server.samu(&["unset", &id, "properties.org_id"], None)),
        0
    );
    let shown = json_out(&server.samu(&["show", &id, "--format", "json"], None));
    assert_eq!(
        shown[0]["object"]["properties"],
        serde_json::json!({"source": "org-import"})
    );

    // Not a key: the server refuses, exit 1.
    assert_eq!(
        code(&server.samu(&["set", &id, "properties.Org ID", "x"], None)),
        1
    );
}

/// ADR 0054: a task inherits from its project, `samu prop` shows both,
/// a template makes a value a link, and `samu open` runs this device's
/// command or `xdg-open`; the handler for `samu-open:` finds the value
/// itself.
#[test]
fn properties_inherit_lead_somewhere_and_open() {
    let server = signed_in_server();
    let project = "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6b78";
    let create = format!(
        r#"{{"op":"create","object":"{project}","kind":"project","value":{{"id":"{project}","created":"2026-11-09T08:00:00Z","modified":"2026-11-09T08:00:00Z","title":"Haus","kind":"parallel","status":"active","properties":{{"jira":"HAUS-12","team":"rot"}},"position":0}},"hlc":{{"wall":1,"counter":0,"device":"test"}}}}"#
    );
    assert_eq!(code(&server.samu(&["apply", "-"], Some(&create))), 0);
    let added = json_out(&server.samu(
        &["add", "Rinne", "--project", project, "--format", "json"],
        None,
    ));
    let id = added[0]["id"].as_str().unwrap().to_string();
    assert_eq!(
        code(&server.samu(&["set", &id, "properties.team", "blau"], None)),
        0
    );
    assert_eq!(
        code(&server.samu(
            &[
                "settings",
                "link",
                "jira",
                "https://jira.example.org/browse/{value}"
            ],
            None
        )),
        0
    );
    // A template without {value} is refused.
    assert_eq!(
        code(&server.samu(&["settings", "link", "team", "https://x.example/"], None)),
        1
    );
    let links = json_out(&server.samu(&["settings", "link", "--format", "json"], None));
    assert_eq!(links.as_array().unwrap().len(), 1, "{links:#}");

    let props = json_out(&server.samu(&["prop", &id, "--format", "json"], None));
    let by_key = |key: &str| {
        props
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["key"] == key)
            .cloned()
            .unwrap_or_else(|| panic!("no {key} in {props:#}"))
    };
    assert_eq!(by_key("team")["value"], "blau");
    assert!(by_key("team")["from"].is_null(), "its own");
    assert_eq!(by_key("jira")["value"], "HAUS-12");
    assert_eq!(by_key("jira")["from"], &project[project.len() - 8..]);
    assert_eq!(
        by_key("jira")["link"],
        "https://jira.example.org/browse/HAUS-12"
    );
    assert!(by_key("team")["link"].is_null(), "no template, no URI");

    // Without a command: xdg-open on the template.
    let print = json_out(&server.samu(&["open", &id, "jira", "--print", "--format", "json"], None));
    assert_eq!(
        print[0]["command"],
        serde_json::json!(["xdg-open", "https://jira.example.org/browse/HAUS-12"])
    );
    // No template, no command: leads nowhere, exit 3; a missing key too.
    assert_eq!(
        code(&server.samu(&["open", &id, "team", "--print"], None)),
        3
    );
    assert_eq!(
        code(&server.samu(&["open", &id, "nichts", "--print"], None)),
        3
    );

    // This device's command wins, runs as argv, and gets the value.
    let out = server.dir.join("opened.txt");
    std::fs::write(
        server.dir.join("no-config.toml"),
        format!(
            "[open]\nteam = [\"sh\", \"-c\", \"printf %s \\\"$0\\\" > {}\", \"{{value}}\"]\n",
            out.display()
        ),
    )
    .unwrap();
    let uri = format!("samu-open:{id}/team");
    let ran = server.samu(&["open-uri", &uri], None);
    assert_eq!(code(&ran), 0, "{}", String::from_utf8_lossy(&ran.stderr));
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "blau");
    assert_eq!(
        code(&server.samu(&["open-uri", "samu-open:nope/team"], None)),
        1
    );
}

#[test]
fn not_found_is_exit_3() {
    let server = signed_in_server();
    assert_eq!(code(&server.samu(&["show", "deadbeef"], None)), 3);
    let missing = "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6c99";
    assert_eq!(code(&server.samu(&["show", missing], None)), 3);
}

#[test]
fn apply_sends_raw_changes_with_their_own_stamps() {
    let server = signed_in_server();
    let id = "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6c42";
    let task = serde_json::json!({
        "id": id, "created": "2026-11-07T08:00:00Z", "modified": "2026-11-07T08:00:00Z",
        "title": "aus dem Ausgang", "flagged": false, "sequential": false,
        "complete_with_last_child": false, "position": 0
    });
    let lines = format!(
        "{}\n{}\n{}\n",
        serde_json::json!({"op": "create", "object": id, "kind": "task", "value": task,
                           "hlc": {"wall": 1000, "counter": 0, "device": "pixel"}}),
        serde_json::json!({"op": "set", "object": id, "field": "title", "value": "neu",
                           "hlc": {"wall": 3000, "counter": 0, "device": "pixel"}}),
        serde_json::json!({"op": "set", "object": id, "field": "title", "value": "alt",
                           "hlc": {"wall": 2000, "counter": 0, "device": "pixel"}}),
    );
    let out = server.samu(&["apply", "-", "--format", "tsv"], Some(&lines));
    assert_eq!(code(&out), 1, "one change was superseded");
    let text = stdout(&out);
    let rows: Vec<&str> = text.lines().collect();
    assert_eq!(rows.len(), 3);
    assert!(rows[2].starts_with("superseded"), "{rows:?}");
    let shown = json_out(&server.samu(&["show", id, "--format", "json"], None));
    assert_eq!(shown[0]["title"], "neu");
}

#[test]
fn list_shows_availability_and_reasons() {
    let server = signed_in_server();
    let free = json_out(&server.samu(&["add", "frei", "--format", "json"], None));
    let later = json_out(&server.samu(&["add", "später", "--format", "json"], None));
    let later_id = later[0]["id"].as_str().unwrap();
    assert_eq!(
        code(&server.samu(
            &["set", later_id, "defer", r#"{"date":"2099-01-01"}"#],
            None
        )),
        0
    );

    let all = json_out(&server.samu(&["list", "--format", "json"], None));
    assert_eq!(all.as_array().unwrap().len(), 2);
    let deferred = all
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == later_id)
        .unwrap();
    assert_eq!(deferred["available"], false);
    assert_eq!(deferred["reasons"], "deferred");

    let available = json_out(&server.samu(&["list", "--available", "--format", "json"], None));
    assert_eq!(available.as_array().unwrap().len(), 1);
    assert_eq!(available[0]["id"], free[0]["id"]);

    let plain = server.samu(&["list"], None);
    assert!(stdout(&plain).contains("später"), "{}", stdout(&plain));

    // ADR 0065: the same tasks, the id and what was computed, no task.
    let bare = json_out(&server.samu(&["list", "--bare", "--format", "json"], None));
    assert_eq!(bare.as_array().unwrap().len(), 2);
    let deferred = bare
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == later_id)
        .unwrap();
    assert_eq!(deferred["available"], false);
    assert_eq!(deferred["reasons"][0]["reason"], "deferred");
    assert!(deferred.get("task").is_none() && deferred.get("title").is_none());
}

/// `add` goes through quick capture (ADR 0027): `--dry-run` writes nothing,
/// a capture sets project, tag and date, `--literal` keeps the text as it
/// is, and a warning goes to stderr with exit 0.
#[test]
fn add_parses_quick_capture() {
    let server = signed_in_server();
    let id = "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6b78";
    let create = format!(
        r#"{{"op":"create","object":"{id}","kind":"project","value":{{"id":"{id}","created":"2026-11-07T08:00:00Z","modified":"2026-11-07T08:00:00Z","title":"Neues Bad","kind":"parallel","status":"active","flagged":false,"position":0}},"hlc":{{"wall":1,"counter":0,"device":"test"}}}}"#
    );
    assert_eq!(code(&server.samu(&["apply", "-"], Some(&create))), 0);
    let text = "Fliesen kaufen #neu @baumarkt d:2026-11-20";

    let preview = server.samu(&["add", text, "--dry-run", "--format", "json"], None);
    assert_eq!(code(&preview), 0);
    let preview = json_out(&preview);
    assert_eq!(preview[0]["created"], false);
    assert_eq!(preview[0]["title"], "Fliesen kaufen");
    assert_eq!(preview[0]["new_tags"], serde_json::json!(["baumarkt"]));
    // Asked of the server: nothing was made.
    let listed = json_out(&server.samu(&["list", "--all", "--format", "json"], None));
    assert!(listed.as_array().unwrap().is_empty(), "{listed:#}");

    let made = json_out(&server.samu(&["add", text, "--flag", "--format", "json"], None));
    assert_eq!(made[0]["created"], true);
    let task_id = made[0]["id"].as_str().unwrap();
    let shown = json_out(&server.samu(&["show", task_id, "--format", "json"], None));
    let task = &shown[0]["object"];
    assert_eq!(task["title"], "Fliesen kaufen");
    assert_eq!(task["project"], id);
    assert_eq!(task["due"], serde_json::json!({"date": "2026-11-20"}));
    assert_eq!(task["flagged"], true);
    assert_eq!(task["tags"].as_array().unwrap().len(), 1);
    let tags = stdout(&server.samu(&["tag", "list", "--format", "tsv"], None));
    assert!(tags.contains("baumarkt"), "{tags}");

    let literal = json_out(&server.samu(
        &["add", "#neu bleibt", "--literal", "--format", "json"],
        None,
    ));
    let shown = json_out(&server.samu(
        &[
            "show",
            literal[0]["id"].as_str().unwrap(),
            "--format",
            "json",
        ],
        None,
    ));
    assert_eq!(shown[0]["object"]["title"], "#neu bleibt");

    let warned = server.samu(&["add", "Anruf #gibtsnicht"], None);
    assert_eq!(code(&warned), 0);
    let stderr = String::from_utf8_lossy(&warned.stderr);
    assert!(stderr.contains("#gibtsnicht"), "{stderr}");
}

/// `edit` applies a line of marks to a task (ADR 0078 §4): `--preview`
/// writes nothing, an edit adds the tag and sets the priority, free text
/// fails with exit 1 and writes nothing.
#[test]
fn edit_applies_a_line_to_a_task() {
    let server = signed_in_server();
    let made = json_out(&server.samu(&["add", "Fliesen", "--format", "json"], None));
    let id = made[0]["id"].as_str().unwrap().to_string();
    let short = &id[id.len() - 8..];

    let preview = server.samu(
        &[
            "edit",
            short,
            "@baumarkt",
            "!A",
            "--preview",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(code(&preview), 0);
    let preview = json_out(&preview);
    assert_eq!(preview[0]["created"], false);
    assert_eq!(preview[0]["task"]["priority"], "A");
    let shown = json_out(&server.samu(&["show", &id, "--format", "json"], None));
    assert!(shown[0]["object"].get("priority").is_none(), "{shown:#}");

    let edited = server.samu(&["edit", short, "@baumarkt !A // mehr"], None);
    assert_eq!(code(&edited), 0);
    let stderr = String::from_utf8_lossy(&edited.stderr);
    assert!(stderr.contains("made the tag baumarkt"), "{stderr}");
    // Asked of the server, not of the answer.
    let shown = json_out(&server.samu(&["show", &id, "--format", "json"], None));
    let task = &shown[0]["object"];
    assert_eq!(task["title"], "Fliesen");
    assert_eq!(task["priority"], "A");
    assert_eq!(task["note"], "mehr");
    assert_eq!(task["tags"].as_array().unwrap().len(), 1);

    let refused = server.samu(&["edit", short, "morgen", "!B"], None);
    assert_eq!(code(&refused), 1);
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(stderr.contains("no mark"), "{stderr}");
    let shown = json_out(&server.samu(&["show", &id, "--format", "json"], None));
    assert_eq!(shown[0]["object"]["priority"], "A");
}

/// `add --parent` puts the new task under another, in its project (ADR 0062
/// §3); a parent that is closed fails with exit 1.
#[test]
fn add_under_a_task() {
    let server = signed_in_server();
    let id = "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6b7a";
    let create = format!(
        r#"{{"op":"create","object":"{id}","kind":"project","value":{{"id":"{id}","created":"2026-11-07T08:00:00Z","modified":"2026-11-07T08:00:00Z","title":"Bad","kind":"parallel","status":"active","flagged":false,"position":0}},"hlc":{{"wall":1,"counter":0,"device":"test"}}}}"#
    );
    assert_eq!(code(&server.samu(&["apply", "-"], Some(&create))), 0);
    let group = json_out(&server.samu(&["add", "Fliesen #bad", "--format", "json"], None));
    let group = group[0]["id"].as_str().unwrap().to_string();
    let short = &group[group.len() - 8..];

    let made = json_out(&server.samu(
        &["add", "bestellen", "--parent", short, "--format", "json"],
        None,
    ));
    // Asked of the server, not of the answer.
    let shown = json_out(&server.samu(
        &["show", made[0]["id"].as_str().unwrap(), "--format", "json"],
        None,
    ));
    assert_eq!(shown[0]["object"]["parent"], group.as_str());
    assert_eq!(shown[0]["object"]["project"], id);

    assert_eq!(code(&server.samu(&["done", &group], None)), 0);
    let refused = server.samu(&["add", "noch eins", "--parent", &group], None);
    assert_eq!(code(&refused), 1);
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(stderr.contains("closed"), "{stderr}");
}

/// `list` shows state key, flag, due, project title, tags and the first
/// reason in words, and filters by tag (and below), flag and search; a
/// project may be named by title.
#[test]
fn list_shows_more_and_filters() {
    let server = signed_in_server();
    let id = "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6b79";
    let create = format!(
        r#"{{"op":"create","object":"{id}","kind":"project","value":{{"id":"{id}","created":"2026-11-07T08:00:00Z","modified":"2026-11-07T08:00:00Z","title":"Neues Bad","kind":"sequential","status":"active","flagged":false,"position":0}},"hlc":{{"wall":1,"counter":0,"device":"test"}}}}"#
    );
    assert_eq!(code(&server.samu(&["apply", "-"], Some(&create))), 0);
    for args in [
        &["tag", "add", "Status", "--exclusive", "--default-workflow"][..],
        &["tag", "add", "Wartet", "--parent", "Status", "--key", "w"][..],
        &["tag", "add", "ort"][..],
    ] {
        assert_eq!(code(&server.samu(args, None)), 0, "{args:?}");
    }
    let first = json_out(&server.samu(
        &[
            "add",
            "Fliesen aussuchen #neu @ort/baumarkt ! d:2026-11-20 // Muster ansehen",
            "--format",
            "json",
        ],
        None,
    ));
    let first_id = first[0]["id"].as_str().unwrap();
    assert_eq!(
        code(&server.samu(&["add", "Fliesenleger anrufen", "--project", "neues"], None)),
        0
    );
    assert_eq!(code(&server.samu(&["add", "Milch"], None)), 0);
    assert_eq!(code(&server.samu(&["state", first_id, "w"], None)), 0);

    let tsv = stdout(&server.samu(&["list", "--format", "tsv"], None));
    let row = tsv
        .lines()
        .find(|l| l.contains("Fliesen aussuchen"))
        .unwrap_or_else(|| panic!("{tsv}"));
    let cells: Vec<&str> = row.split('\t').collect();
    assert_eq!(
        cells[1..7],
        [
            "w",
            "!",
            "2026-11-20",
            "Neues Bad",
            "Fliesen aussuchen",
            "@ort/baumarkt"
        ]
    );
    let waiting = tsv.lines().find(|l| l.contains("Fliesenleger")).unwrap();
    assert!(
        waiting.ends_with("waits for: Fliesen aussuchen"),
        "{waiting}"
    );

    let titles = |args: &[&str]| -> Vec<String> {
        json_out(&server.samu(&[args, &["--format", "json"]].concat(), None))
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["title"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        titles(&["list", "--tag", "ort"]),
        ["Fliesen aussuchen"],
        "a tag finds its children"
    );
    assert_eq!(titles(&["list", "--flagged"]), ["Fliesen aussuchen"]);
    assert_eq!(
        titles(&["list", "--search", "MUSTER"]),
        ["Fliesen aussuchen"],
        "the note counts"
    );
    let end = &first_id[first_id.len() - 8..];
    assert_eq!(
        titles(&["list", "--search", end]),
        ["Fliesen aussuchen"],
        "the end of the id counts"
    );
    // ADR 0056: the priority is a field, set and cleared like any.
    assert_eq!(
        code(&server.samu(&["set", first_id, "priority", "A"], None)),
        0
    );
    let listed = json_out(&server.samu(&["list", "--format", "json"], None));
    let priority = |listed: &serde_json::Value| {
        listed
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["title"] == "Fliesen aussuchen")
            .unwrap()["priority"]
            .clone()
    };
    assert_eq!(priority(&listed), "A");
    assert_ne!(
        code(&server.samu(&["set", first_id, "priority", "D"], None)),
        0
    );
    assert_eq!(
        code(&server.samu(&["unset", first_id, "priority"], None)),
        0
    );
    let listed = json_out(&server.samu(&["list", "--format", "json"], None));
    assert_eq!(priority(&listed), serde_json::Value::Null);
    // ADR 0057: quick capture through the server takes `!C`.
    assert_eq!(code(&server.samu(&["add", "Prio-Probe", "!c"], None)), 0);
    let listed = json_out(&server.samu(&["list", "--format", "json"], None));
    let probe = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["title"] == "Prio-Probe")
        .unwrap();
    assert_eq!(probe["priority"], "C");
    assert_eq!(
        code(&server.samu(&["drop", probe["id"].as_str().unwrap()], None)),
        0
    );
    assert_eq!(titles(&["list", "--project", "Neues Bad"]).len(), 2);
    assert_eq!(titles(&["list"]).len(), 3);
    assert_eq!(
        code(&server.samu(&["list", "--tag", "gibtsnicht"], None)),
        3
    );
}

/// The finding of 08.11.2026: `samu tag add Ort/Büro` made one tag titled
/// „Ort/Büro“ at the top. A path puts the new tag under its parent (ADR
/// 0027 §3); the other tag commands take paths too.
#[test]
fn tag_add_takes_a_path() {
    let server = signed_in_server();
    for args in [
        &["tag", "add", "Ort"][..],
        &["tag", "add", "Ort/Büro"][..],
        &["tag", "add", "Firma"][..],
        &["tag", "add", "Büro", "--parent", "Firma"][..],
        &["tag", "add", "Raum 3", "--parent", "Ort/Büro"][..],
    ] {
        assert_eq!(code(&server.samu(args, None)), 0, "{args:?}");
    }
    let paths = || -> Vec<String> {
        json_out(&server.samu(&["tag", "list", "--format", "json"], None))
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["path"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        paths(),
        ["Ort", "Ort/Büro", "Ort/Büro/Raum 3", "Firma", "Firma/Büro"]
    );
    // Missing above: not found, nothing made; twice: refused.
    assert_eq!(code(&server.samu(&["tag", "add", "Zeit/Abends"], None)), 3);
    assert_eq!(code(&server.samu(&["tag", "add", "ort/büro"], None)), 1);
    assert_eq!(
        code(&server.samu(&["tag", "rename", "Firma/Büro", "a/b"], None)),
        1
    );
    assert_eq!(paths().len(), 5);
    // Rename and move find a tag by its path, where the title alone is two.
    assert_eq!(
        code(&server.samu(&["tag", "rename", "Firma/Büro", "Zentrale"], None)),
        0
    );
    assert_eq!(
        code(&server.samu(&["tag", "move", "Ort/Büro/Raum 3", "--top"], None)),
        0
    );
    assert_eq!(
        paths(),
        ["Ort", "Ort/Büro", "Firma", "Firma/Zentrale", "Raum 3"]
    );
}

/// zsh completion the way the registered function asks (`COMPLETE=zsh`,
/// `_CLAP_COMPLETE_INDEX`): tags in the text of `add` come from the server,
/// task ids carry their title; without a server there are no candidates
/// and no error.
#[test]
fn zsh_completion_offers_tags_and_tasks() {
    let server = signed_in_server();
    assert_eq!(code(&server.samu(&["tag", "add", "telefon"], None)), 0);
    assert_eq!(code(&server.samu(&["tag", "add", "termin"], None)), 0);
    let made = json_out(&server.samu(&["add", "Milch kaufen", "--format", "json"], None));
    let short = made[0]["short"].as_str().unwrap().to_string();

    let complete = |url: &str, index: &str, words: &[&str]| -> (String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_samu"))
            .arg("--")
            .args(words)
            .env("COMPLETE", "zsh")
            .env("_CLAP_COMPLETE_INDEX", index)
            .env("SAMU_SERVER", url)
            .env("SAMU_ACCOUNT", "privat")
            .env("SAMU_STATE", server.dir.join("state"))
            .env("SAMU_CONFIG", server.dir.join("no-config.toml"))
            .output()
            .unwrap();
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let (tags, _) = complete(&server.url, "2", &["samu", "add", "Anruf @te"]);
    assert_eq!(
        tags.lines().collect::<Vec<_>>(),
        ["Anruf @telefon", "Anruf @termin"]
    );
    let (ids, _) = complete(&server.url, "2", &["samu", "done", ""]);
    assert!(ids.contains(&format!("{short}:Milch kaufen")), "{ids}");
    let (commands, _) = complete(&server.url, "1", &["samu", "pro"]);
    assert!(
        commands.lines().any(|line| line.starts_with("projects:")),
        "{commands}"
    );

    let (none, errors) = complete("http://127.0.0.1:9", "2", &["samu", "add", "@te"]);
    assert!(
        none.trim().is_empty() && errors.trim().is_empty(),
        "{none} {errors}"
    );
}

/// `projects --stuck` finds the project without a next action, and loses
/// it once it has one.
#[test]
fn projects_names_the_stuck_ones() {
    let server = signed_in_server();
    let id = "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6b77";
    let create = format!(
        r#"{{"op":"create","object":"{id}","kind":"project","value":{{"id":"{id}","created":"2026-11-07T08:00:00Z","modified":"2026-11-07T08:00:00Z","title":"Garten","kind":"parallel","status":"active","flagged":false,"position":0}},"hlc":{{"wall":1,"counter":0,"device":"test"}}}}"#
    );
    assert_eq!(code(&server.samu(&["apply", "-"], Some(&create))), 0);

    let stuck = json_out(&server.samu(&["projects", "--stuck", "--format", "json"], None));
    assert_eq!(stuck.as_array().unwrap().len(), 1, "{stuck:#}");
    assert_eq!(stuck[0]["title"], "Garten");
    assert_eq!(stuck[0]["stuck"], "stuck");
    let plain = stdout(&server.samu(&["projects"], None));
    assert!(
        plain.contains("Garten") && plain.contains("stuck"),
        "{plain}"
    );

    assert_eq!(
        code(&server.samu(&["add", "Rasen mähen", "--project", id], None)),
        0
    );
    let stuck = json_out(&server.samu(&["projects", "--stuck", "--format", "json"], None));
    assert!(stuck.as_array().unwrap().is_empty(), "{stuck:#}");
    let all = json_out(&server.samu(&["projects", "--format", "json"], None));
    assert_eq!(all[0]["available"], true);
}

/// `projects --review` lists the project due for review (ADR 0088);
/// `samu review` marks it reviewed by title, and it leaves the list.
#[test]
fn review_marks_a_due_project_reviewed() {
    let server = signed_in_server();
    let id = "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6b78";
    let create = format!(
        r#"{{"op":"create","object":"{id}","kind":"project","value":{{"id":"{id}","created":"2020-01-01T08:00:00Z","modified":"2020-01-01T08:00:00Z","title":"Keller","kind":"parallel","status":"active","review_interval":"P1W","next_review":"2020-03-01","flagged":false,"position":0}},"hlc":{{"wall":1,"counter":0,"device":"test"}}}}"#
    );
    assert_eq!(code(&server.samu(&["apply", "-"], Some(&create))), 0);

    let due = json_out(&server.samu(&["projects", "--review", "--format", "json"], None));
    assert_eq!(due.as_array().unwrap().len(), 1, "{due:#}");
    assert_eq!(due[0]["review"], "2020-03-01");
    assert_eq!(due[0]["review_due"], "due");
    let plain = stdout(&server.samu(&["projects"], None));
    assert!(plain.contains("2020-03-01"), "{plain}");

    assert_eq!(code(&server.samu(&["review", "Keller"], None)), 0);
    let due = json_out(&server.samu(&["projects", "--review", "--format", "json"], None));
    assert!(due.as_array().unwrap().is_empty(), "{due:#}");
    let shown = json_out(&server.samu(&["show", id, "--format", "json"], None));
    assert!(shown[0]["object"]["last_reviewed"].is_string(), "{shown:#}");
    assert!(shown[0]["object"].get("next_review").is_none(), "{shown:#}");

    assert_ne!(code(&server.samu(&["review"], None)), 0);
}

/// `samu set` and `samu unset` take a project by title, like `samu review`;
/// a reference that is neither an id nor a title is still not found.
#[test]
fn set_takes_a_project_by_title() {
    let server = signed_in_server();
    let id = "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6b79";
    let create = format!(
        r#"{{"op":"create","object":"{id}","kind":"project","value":{{"id":"{id}","created":"2020-01-01T08:00:00Z","modified":"2020-01-01T08:00:00Z","title":"Keller","kind":"parallel","status":"active","flagged":false,"position":0}},"hlc":{{"wall":1,"counter":0,"device":"test"}}}}"#
    );
    assert_eq!(code(&server.samu(&["apply", "-"], Some(&create))), 0);

    assert_eq!(
        code(&server.samu(&["set", "Keller", "review_interval", "P2W"], None)),
        0
    );
    let shown = json_out(&server.samu(&["show", id, "--format", "json"], None));
    assert_eq!(shown[0]["object"]["review_interval"], "P2W", "{shown:#}");

    assert_eq!(
        code(&server.samu(&["unset", "kel", "review_interval"], None)),
        0
    );
    let shown = json_out(&server.samu(&["show", id, "--format", "json"], None));
    assert!(
        shown[0]["object"].get("review_interval").is_none(),
        "{shown:#}"
    );

    let missing = server.samu(&["set", "Dachboden", "flagged", "true"], None);
    assert_eq!(
        code(&missing),
        3,
        "{}",
        String::from_utf8_lossy(&missing.stderr)
    );
}

#[test]
fn events_follow_a_change() {
    let server = signed_in_server();
    let mut child = Command::new(env!("CARGO_BIN_EXE_samu"))
        .args(["events", "--count", "1", "--format", "jsonl"])
        .env("SAMU_SERVER", &server.url)
        .env("SAMU_ACCOUNT", "privat")
        .env("SAMU_STATE", server.dir.join("state"))
        .env("SAMU_CONFIG", server.dir.join("no-config.toml"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let added = json_out(&server.samu(&["add", "Milch", "--format", "json"], None));

    let pid = child.id();
    let stdout_pipe = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let _ = BufReader::new(stdout_pipe).read_line(&mut line);
        let _ = tx.send(line);
    });
    let line = rx.recv_timeout(std::time::Duration::from_secs(10));
    if line.is_err() {
        // By its PID, never by name.
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
    let status = child.wait().unwrap();
    let line = line.expect("no event within ten seconds");
    let event: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(event["event"], "changed");
    assert_eq!(event["data"]["added"][0], added[0]["id"]);
    assert!(status.success(), "--count 1 ends after one event");
}

/// ADR 0010, 0022: a hook made from the command line takes what
/// `samu hook send` signs, once per delivery id, into the inbox; a wrong
/// secret makes nothing and is exit 6.
#[test]
fn a_hook_takes_a_delivery_into_the_inbox_once() {
    let server = signed_in_server();
    let added = json_out(&server.samu(&["hook", "add", "ring", "--format", "json"], None));
    assert_eq!(added[0]["name"], "ring");
    let secret = added[0]["secret"].as_str().unwrap().to_string();
    assert_eq!(secret.len(), 64);

    let listed = json_out(&server.samu(&["hook", "list", "--format", "json"], None));
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert!(listed[0].get("secret").is_none());

    let wrong = server.samu(
        &[
            "hook", "send", "ring", "--secret", "wrong", "--text", "Milch",
        ],
        None,
    );
    assert_eq!(
        code(&wrong),
        6,
        "{}",
        String::from_utf8_lossy(&wrong.stderr)
    );
    let tasks = json_out(&server.samu(&["list", "--format", "json"], None));
    assert_eq!(tasks.as_array().unwrap().len(), 0);

    let send = |delivery: &str| {
        server.samu(
            &[
                "hook",
                "send",
                "ring",
                "--secret",
                &secret,
                "--text",
                "Milch kaufen\nund Brot",
                "--delivery",
                delivery,
                "--at",
                "2026-11-07T08:30:00Z",
                "--format",
                "json",
            ],
            None,
        )
    };
    let first = json_out(&send("rec-1"));
    assert_eq!(first[0]["created"], true);
    let again = json_out(&send("rec-1"));
    assert_eq!(again[0]["created"], false);
    assert_eq!(again[0]["task"], first[0]["task"]);

    let tasks = json_out(&server.samu(&["list", "--format", "json"], None));
    let tasks = tasks.as_array().unwrap();
    assert_eq!(tasks.len(), 1, "{tasks:?}");
    let shown = json_out(&server.samu(
        &[
            "show",
            first[0]["task"].as_str().unwrap(),
            "--format",
            "json",
        ],
        None,
    ));
    assert_eq!(shown[0]["object"]["title"], "Milch kaufen");
    assert_eq!(shown[0]["object"]["note"], "Milch kaufen\nund Brot");
    assert_eq!(shown[0]["object"]["created"], "2026-11-07T08:30:00Z");
    assert!(shown[0]["object"].get("project").is_none(), "inbox");

    let test = json_out(&server.samu(
        &[
            "hook", "send", "ring", "--secret", &secret, "--test", "--format", "json",
        ],
        None,
    ));
    assert_eq!(test[0]["test"], true);

    assert_eq!(code(&server.samu(&["hook", "remove", "ring"], None)), 0);
    assert_eq!(code(&server.samu(&["hook", "remove", "ring"], None)), 3);
}

/// ADR 0002 §4, ADR 0051: the open tasks of an Org directory. A second
/// run over it makes nothing; another directory, with an inbox.org of its
/// own and a new heading in a file of the same name, comes in beside it.
/// Synthetic files in `testdata/org`.
#[test]
fn org_directories_are_imported_without_doubles() {
    let server = signed_in_server();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/org");
    let dir = dir.to_str().unwrap();

    let dry = server.samu(
        &["import", "org", dir, "--dry-run", "--zone", "Europe/Berlin"],
        None,
    );
    assert_eq!(code(&dry), 0, "{}", String::from_utf8_lossy(&dry.stderr));
    let lines: Vec<serde_json::Value> = stdout(&dry)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let count = |kind: &str| lines.iter().filter(|c| c["kind"] == kind).count();
    // inbox 2, haus 5 (+1 project heading), sub/eigen 1.
    assert_eq!((count("task"), count("project"), count("tag")), (8, 3, 4));
    let listed = json_out(&server.samu(&["list", "--format", "json"], None));
    assert_eq!(
        listed.as_array().unwrap().len(),
        0,
        "a dry run sends nothing"
    );

    let real = json_out(&server.samu(
        &[
            "import",
            "org",
            dir,
            "--zone",
            "Europe/Berlin",
            "--format",
            "json",
        ],
        None,
    ));
    assert_eq!(real[0]["tasks"], 8);
    assert_eq!(real[0]["with_org_id"], 2);
    let listed = json_out(&server.samu(&["list", "--format", "json"], None));
    assert_eq!(listed.as_array().unwrap().len(), 8, "{listed:#}");

    let again = json_out(&server.samu(&["import", "org", dir, "--format", "json"], None));
    assert_eq!(again[0]["tasks"], 0, "{again:#}");
    assert_eq!(again[0]["projects"], 0);
    // 8 tasks and the :prj: heading.
    assert_eq!(again[0]["known"], 9);
    let listed = json_out(&server.samu(&["list", "--format", "json"], None));
    assert_eq!(listed.as_array().unwrap().len(), 8, "nothing twice");

    // Another directory: its inbox.org is not the first one's.
    let other = server.dir.join("org-agenda-work");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(
        other.join("inbox.org"),
        "* TODO Zahnarzt anrufen\n* TODO Bericht lesen\n",
    )
    .unwrap();
    std::fs::write(other.join("haus.org"), "* TODO Dienstreise buchen\n").unwrap();
    let second = json_out(&server.samu(
        &["import", "org", other.to_str().unwrap(), "--format", "json"],
        None,
    ));
    assert_eq!(second[0]["tasks"], 3, "{second:#}");
    assert_eq!(second[0]["projects"], 1, "its own haus");
    assert_eq!(second[0]["known"], 0);
    let listed = json_out(&server.samu(&["list", "--format", "json"], None));
    assert_eq!(listed.as_array().unwrap().len(), 11, "{listed:#}");
    let again = json_out(&server.samu(
        &["import", "org", other.to_str().unwrap(), "--format", "json"],
        None,
    ));
    assert_eq!(
        (again[0]["tasks"].clone(), again[0]["known"].clone()),
        (0.into(), 3.into())
    );
}

/// ADR 0024: a workflow made with `samu tag add`, a task cycled through it
/// round to no state; the on-hold state keeps it out of the available
/// list; the logbook holds every switch with its note.
#[test]
fn a_task_runs_through_a_workflow() {
    let server = signed_in_server();
    let ok = |out: Output| {
        assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
        out
    };
    ok(server.samu(
        &["tag", "add", "Status", "--exclusive", "--default-workflow"],
        None,
    ));
    ok(server.samu(
        &[
            "tag",
            "add",
            "In Arbeit",
            "--parent",
            "Status",
            "--key",
            "a",
        ],
        None,
    ));
    ok(server.samu(
        &[
            "tag",
            "add",
            "Wartet",
            "--parent",
            "Status",
            "--on-hold",
            "--key",
            "w",
            "--ask-note",
        ],
        None,
    ));
    let listed = stdout(&ok(server.samu(&["tag", "list", "--format", "tsv"], None)));
    assert!(
        listed.contains("Status\tactive\t\tworkflow,default"),
        "{listed}"
    );
    assert!(
        listed.contains("Status/Wartet\ton_hold\tw\tnote"),
        "{listed}"
    );

    let added = json_out(&server.samu(&["add", "Dach", "--format", "json"], None));
    let id = added[0]["id"].as_str().unwrap().to_string();
    let state = |args: &[&str]| {
        let mut all = vec!["state", id.as_str()];
        all.extend_from_slice(args);
        all.extend_from_slice(&["--format", "json"]);
        json_out(&server.samu(&all, None))[0]["state"].clone()
    };
    assert_eq!(state(&["--next"]), "In Arbeit");
    assert_eq!(state(&["w", "--note", "Angebot angefragt"]), "Wartet");
    let all = json_out(&server.samu(&["list", "--format", "json"], None));
    assert_eq!(all[0]["state"], "Wartet", "{all:#}");

    let available = json_out(&server.samu(&["list", "--available", "--format", "json"], None));
    assert!(
        available.as_array().unwrap().is_empty(),
        "waiting is on hold: {available:#}"
    );

    assert_eq!(state(&["--next"]), "", "after the last state comes none");
    let available = json_out(&server.samu(&["list", "--available", "--format", "json"], None));
    assert_eq!(available.as_array().unwrap().len(), 1);

    let unknown = server.samu(&["state", &id, "Fertig"], None);
    assert_eq!(code(&unknown), 1);
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("In Arbeit, Wartet"));

    assert_eq!(
        code(&server.samu(&["done", &id, "--note", "dicht"], None)),
        0
    );
    let log = json_out(&server.samu(&["log", &id, "--format", "json"], None));
    let kinds: Vec<&str> = log
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["state", "state", "state", "completed"]);
    assert_eq!(log[1]["note"], "Angebot angefragt");
    assert_eq!(log[3]["note"], "dicht");
    assert_eq!(log[3]["device"], "test");
}

/// `samu tag rename|move|delete`: a tag moves under another but never under
/// its own child; a tag with children stays; a deleted tag comes off the
/// task that carried it.
#[test]
fn tags_are_renamed_moved_and_deleted() {
    let server = signed_in_server();
    let ok = |out: Output| {
        assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
        out
    };
    let list = || stdout(&ok(server.samu(&["tag", "list", "--format", "tsv"], None)));
    ok(server.samu(&["tag", "add", "Orte"], None));
    ok(server.samu(&["tag", "add", "Telefon"], None));
    ok(server.samu(&["tag", "move", "Telefon", "--parent", "Orte"], None));
    assert!(list().contains("\tOrte/Telefon\t"), "{}", list());
    let cycle = server.samu(&["tag", "move", "Orte", "--parent", "Telefon"], None);
    assert_eq!(code(&cycle), 1);
    ok(server.samu(&["tag", "rename", "Telefon", "Anrufen"], None));
    assert!(list().contains("\tOrte/Anrufen\t"), "{}", list());
    assert_eq!(code(&server.samu(&["tag", "delete", "Orte"], None)), 1);
    ok(server.samu(&["tag", "move", "Anrufen", "--top"], None));
    assert!(list().contains("\tAnrufen\t"), "{}", list());

    ok(server.samu(
        &["tag", "add", "Status", "--exclusive", "--default-workflow"],
        None,
    ));
    ok(server.samu(&["tag", "add", "Wartet", "--parent", "Status"], None));
    let added = json_out(&server.samu(&["add", "Dach", "--format", "json"], None));
    let id = added[0]["id"].as_str().unwrap().to_string();
    ok(server.samu(&["state", &id, "Wartet"], None));
    let tags = || {
        let task = json_out(&server.samu(&["show", &id, "--format", "json"], None));
        task[0]["object"]["tags"].as_array().map_or(0, Vec::len)
    };
    assert_eq!(tags(), 1, "the probe must find the tag first");
    ok(server.samu(&["tag", "delete", "Wartet"], None));
    assert!(!list().contains("Wartet"), "{}", list());
    assert_eq!(tags(), 0);
}

/// ADR 0030: reminders are set on a task, listed, and deleted like any
/// object; a reminder on a tag is the server's refusal.
#[test]
fn reminders_are_set_listed_and_deleted() {
    let server = signed_in_server();
    let task = json_out(&server.samu(&["add", "Steuer", "--format", "json"], None));
    let task_id = task[0]["id"].as_str().unwrap().to_string();
    let other = json_out(&server.samu(&["add", "Anderes", "--format", "json"], None));
    let other_id = other[0]["id"].as_str().unwrap().to_string();

    let before = json_out(&server.samu(
        &["remind", &task_id, "--before", "2h", "--format", "json"],
        None,
    ));
    let before_id = before[0]["id"].as_str().unwrap().to_string();
    let at = json_out(&server.samu(
        &[
            "remind",
            &task_id,
            "--at",
            "2026-11-20T07:30:00Z",
            "--format",
            "json",
        ],
        None,
    ));
    assert_eq!(at[0]["anchor"], "at");
    json_out(&server.samu(
        &["remind", &other_id, "--of", "planned", "--format", "json"],
        None,
    ));

    // Asked of the server, not of what `remind` printed.
    let shown = json_out(&server.samu(&["show", &before_id, "--format", "json"], None));
    assert_eq!(shown[0]["kind"], "reminder");
    assert_eq!(shown[0]["object"]["owner"], task_id.as_str());
    assert_eq!(shown[0]["object"]["before"], "PT2H");

    let all = json_out(&server.samu(&["reminders", "--format", "json"], None));
    assert_eq!(all.as_array().unwrap().len(), 3);
    let mine = json_out(&server.samu(&["reminders", &task_id, "--format", "json"], None));
    let whens: Vec<&str> = mine
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["when"].as_str().unwrap())
        .collect();
    assert_eq!(whens, ["PT2H before due", "2026-11-20T07:30:00Z"]);
    assert!(
        mine.as_array()
            .unwrap()
            .iter()
            .all(|r| r["title"] == "Steuer")
    );

    let plain = stdout(&server.samu(&["reminders", &task_id], None));
    assert!(
        plain
            .lines()
            .any(|l| l.contains("PT2H before due") && l.ends_with("Steuer")),
        "{plain}"
    );

    // Not a length of time: the command line's own error, nothing sent.
    assert_eq!(
        code(&server.samu(&["remind", &task_id, "--before", "bald"], None)),
        1
    );

    assert_eq!(code(&server.samu(&["delete", &before_id], None)), 0);
    let mine = json_out(&server.samu(&["reminders", &task_id, "--format", "json"], None));
    assert_eq!(mine.as_array().unwrap().len(), 1);

    let session = json_out(&server.samu(&["session", "--format", "json"], None));
    assert_eq!(session[0]["remind_time"], "09:00:00");
}

/// ADR 0031: `samu pair` makes a code after the password once more,
/// `samu login --code` on another device trades it once.
#[test]
fn a_pairing_code_signs_in_another_device_once() {
    let server = signed_in_server();
    assert_eq!(
        code(&server.samu(
            &["pair", "pixel", "--password-stdin"],
            Some("wrong password")
        )),
        6,
        "the session alone is not enough"
    );
    let made = json_out(&server.samu(
        &["pair", "pixel", "--password-stdin", "--format", "json"],
        Some(PASSWORD),
    ));
    let pairing = made[0]["code"].as_str().unwrap().to_string();
    assert!(
        made[0]["uri"]
            .as_str()
            .unwrap()
            .starts_with("samu-pair:?server=http")
    );
    assert_eq!(made[0]["device"], "pixel");

    // Another device: a state directory of its own, no session yet.
    let phone = server.dir.join("phone");
    let other = |args: &[&str]| run_samu(&server.url, &phone, args, None);
    assert_eq!(code(&other(&["session"])), 6, "not signed in yet");
    let paired = json_out(&other(&["login", "--code", &pairing, "--format", "json"]));
    assert_eq!(paired[0]["device"], "pixel");
    let session = json_out(&other(&["session", "--format", "json"]));
    assert_eq!(session[0]["device"], "pixel");
    assert_eq!(code(&other(&["login", "--code", &pairing])), 6, "once");
}

/// ADR 0035: `samu scope` makes, changes, chooses and deletes a scope;
/// `list` and `projects` go through the chosen one, `--no-scope` around it,
/// and what is due outside comes as a hint on stderr. An unknown scope is
/// exit 3.
#[test]
fn a_scope_is_made_chosen_and_narrows_list_and_projects() {
    let server = signed_in_server();
    let ok = |out: Output| {
        assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
        out
    };
    let project = |id: &str, title: &str| {
        format!(
            r#"{{"op":"create","object":"{id}","kind":"project","value":{{"id":"{id}","created":"2026-11-07T08:00:00Z","modified":"2026-11-07T08:00:00Z","title":"{title}","kind":"parallel","status":"active","flagged":false,"position":0}},"hlc":{{"wall":1,"counter":0,"device":"test"}}}}"#
        )
    };
    let haus = "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6b81";
    let garten = "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6b82";
    ok(server.samu(&["apply", "-"], Some(&project(haus, "Haus"))));
    ok(server.samu(&["apply", "-"], Some(&project(garten, "Garten"))));
    ok(server.samu(&["tag", "add", "IT"], None));
    ok(server.samu(&["add", "Kabel ziehen @IT", "--project", haus], None));
    ok(server.samu(
        &["add", "Server patchen @IT d:2000-01-01", "--project", haus],
        None,
    ));
    ok(server.samu(&["add", "Rasen mähen", "--project", garten], None));
    ok(server.samu(&["add", "Milch"], None));
    let titles = |args: &[&str]| -> Vec<String> {
        let mut all = vec!["list", "--format", "json"];
        all.extend_from_slice(args);
        let rows = json_out(&server.samu(&all, None));
        let mut titles: Vec<String> = rows
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["title"].as_str().unwrap().to_owned())
            .collect();
        titles.sort();
        titles
    };
    // The probe that must find something: every task before a scope.
    assert_eq!(titles(&[]).len(), 4, "{:?}", titles(&[]));

    let made = json_out(&server.samu(
        &[
            "scope", "add", "Ohne IT", "--except", "tag:IT", "--format", "json",
        ],
        None,
    ));
    assert!(made[0]["id"].is_string());
    ok(server.samu(&["scope", "use", "ohne it"], None));
    assert_eq!(titles(&[]), vec!["Milch", "Rasen mähen"]);
    let out = ok(server.samu(&["list"], None));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("1 due outside the scope"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(titles(&["--no-scope"]).len(), 4);

    ok(server.samu(&["scope", "mode", "Ohne IT", "only"], None));
    ok(server.samu(&["scope", "rename", "Ohne IT", "Nur IT"], None));
    // ADR 0043 §1: with `only`, the inbox only when ticked.
    assert_eq!(titles(&[]), vec!["Kabel ziehen", "Server patchen"]);
    ok(server.samu(&["scope", "inbox", "Nur IT", "on"], None));
    assert_eq!(titles(&[]), vec!["Kabel ziehen", "Milch", "Server patchen"]);
    ok(server.samu(&["scope", "member", "remove", "Nur IT", "IT"], None));
    ok(server.samu(&["scope", "member", "add", "Nur IT", "Garten"], None));
    assert_eq!(titles(&[]), vec!["Milch", "Rasen mähen"]);
    let projects = json_out(&server.samu(&["projects", "--format", "json"], None));
    let names: Vec<&str> = projects
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["title"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["Garten"]);

    // ADR 0043 §2/§3: a filter in the search language, AND the members.
    ok(server.samu(&["scope", "filter", "Nur IT", "#inbox ~ohne"], None));
    assert_eq!(titles(&[]), vec!["Milch"]);
    let filter = json_out(&server.samu(&["scope", "filter", "Nur IT", "--format", "json"], None));
    assert_eq!(filter[0]["filter"], "#Inbox ~ohne");
    assert_eq!(filter[0]["chips"][0]["value"], "inbox");
    // ADR 0056 §4: `prio:-` is `prio:ohne`; nobody here has one.
    ok(server.samu(&["scope", "filter", "Nur IT", "#inbox prio:-"], None));
    assert_eq!(titles(&[]), vec!["Milch"]);
    ok(server.samu(&["scope", "filter", "Nur IT", "#inbox prio:a"], None));
    assert!(titles(&[]).is_empty());
    // ADR 0054 §8: the property chip, read by samu-core as the web reads
    // it; nobody here has a `jira`, so it hides both and its negation none.
    ok(server.samu(&["scope", "filter", "Nur IT", "prop:Jira"], None));
    assert!(titles(&[]).is_empty());
    let filter = json_out(&server.samu(&["scope", "filter", "Nur IT", "--format", "json"], None));
    assert_eq!(filter[0]["filter"], "prop:jira");
    ok(server.samu(&["scope", "filter", "Nur IT", "-prop:jira=HAUS-1"], None));
    assert_eq!(titles(&[]), vec!["Milch", "Rasen mähen"]);
    // ADR 0105 §3: English keys and values, kept in German.
    ok(server.samu(&["scope", "filter", "Nur IT", "is:open prio:none"], None));
    let filter = json_out(&server.samu(&["scope", "filter", "Nur IT", "--format", "json"], None));
    assert_eq!(filter[0]["filter"], "ist:offen prio:ohne");
    assert_eq!(titles(&[]), vec!["Milch", "Rasen mähen"]);
    ok(server.samu(&["scope", "filter", "Nur IT", "-#Inbox"], None));
    assert_eq!(titles(&[]), vec!["Rasen mähen"]);
    assert_ne!(
        code(&server.samu(&["scope", "filter", "Nur IT", "fällig:bald"], None)),
        0
    );
    assert_ne!(
        code(&server.samu(&["scope", "filter", "Nur IT", "Milch"], None)),
        0
    );
    assert_eq!(
        code(&server.samu(&["scope", "filter", "Nur IT", "@Nichts"], None)),
        3
    );
    assert_eq!(
        titles(&[]),
        vec!["Rasen mähen"],
        "a refused filter changes nothing"
    );
    ok(server.samu(&["scope", "filter", "Nur IT", "--clear"], None));
    assert_eq!(titles(&[]), vec!["Milch", "Rasen mähen"]);

    // ADR 0043 §6: a capture while the scope is chosen gets its project and
    // tags, unless the line names its own or --no-scope says so.
    ok(server.samu(
        &[
            "scope",
            "capture",
            "Nur IT",
            "--project",
            "Garten",
            "--tag",
            "IT",
        ],
        None,
    ));
    let capture = json_out(&server.samu(&["scope", "capture", "Nur IT", "--format", "json"], None));
    assert_eq!(
        (capture[0]["project"].as_str(), capture[0]["tags"].as_str()),
        (Some("Garten"), Some("IT"))
    );
    let made = json_out(&server.samu(
        &["add", "Zaun streichen", "--dry-run", "--format", "json"],
        None,
    ));
    assert_eq!(made[0]["task"]["project"], garten, "{made}");
    assert_eq!(
        made[0]["task"]["tags"].as_array().map(Vec::len),
        Some(1),
        "{made}"
    );
    let made = json_out(&server.samu(
        &[
            "add",
            "Zaun streichen",
            "--dry-run",
            "--no-scope",
            "--format",
            "json",
        ],
        None,
    ));
    assert!(made[0]["task"]["project"].is_null(), "{made}");
    ok(server.samu(&["scope", "capture", "Nur IT", "--clear"], None));
    let made = json_out(&server.samu(
        &["add", "Zaun streichen", "--dry-run", "--format", "json"],
        None,
    ));
    assert!(made[0]["task"]["project"].is_null(), "{made}");

    let list = stdout(&ok(server.samu(&["scope", "list", "--format", "tsv"], None)));
    assert!(
        list.contains("*\t") && list.contains("Nur IT\tonly\tinbox\tGarten"),
        "{list}"
    );
    let shown = json_out(&server.samu(&["scope", "show", "Nur IT", "--format", "json"], None));
    assert_eq!(shown[0]["kind"], "project");

    assert_eq!(code(&server.samu(&["list", "--scope", "Keiner"], None)), 3);
    assert_eq!(
        code(&server.samu(&["scope", "add", "x", "--only", "Nichts"], None)),
        3
    );
    assert_eq!(code(&server.samu(&["scope", "add", "x"], None)), 2);

    // ADR 0043 §7: a schedule, and a device that follows it.
    ok(server.samu(&["scope", "schedule", "Nur IT", "mo-so 00:00-00:00"], None));
    let windows =
        json_out(&server.samu(&["scope", "schedule", "Nur IT", "--format", "json"], None));
    assert_eq!(windows[0]["window"], "mo,di,mi,do,fr,sa,so 00:00-00:00");
    let now = json_out(&server.samu(&["scope", "now", "--format", "json"], None));
    assert_eq!(now[0]["title"], "Nur IT", "{now}");
    ok(server.samu(&["scope", "use", "--none"], None));
    assert_eq!(titles(&[]).len(), 4, "the probe: nothing chosen");
    ok(server.samu(&["scope", "use", "auto"], None));
    assert_eq!(
        titles(&[]),
        vec!["Milch", "Rasen mähen"],
        "the schedule chose Nur IT"
    );
    assert_ne!(
        code(&server.samu(&["scope", "schedule", "Nur IT", "xx 08:00-09:00"], None)),
        0
    );
    ok(server.samu(&["scope", "schedule", "Nur IT", "--clear"], None));
    assert_eq!(titles(&[]).len(), 4, "no window holds now");

    ok(server.samu(&["scope", "delete", "Nur IT"], None));
    assert_eq!(titles(&[]).len(), 4, "the deleted scope no longer narrows");
    assert!(
        stdout(&ok(server.samu(&["scope", "list"], None)))
            .trim()
            .is_empty()
    );
}

/// ADR 0036: projects and folders are made and renamed here; deleting a
/// project sends its tasks to the inbox or into another project, a dry run
/// writes nothing, a folder hands its projects up, and a choice for the
/// tasks of something that is no project is exit 1.
#[test]
fn projects_and_folders_are_made_renamed_and_deleted() {
    let server = signed_in_server();
    let ok = |out: Output| {
        assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
        out
    };
    let made = |args: &[&str]| {
        let out = json_out(&ok(server.samu(args, None)));
        out[0]["id"].as_str().unwrap().to_string()
    };
    let home = made(&["folder", "add", "Haus", "--format", "json"]);
    let garden = made(&[
        "folder", "add", "Garten", "--parent", "Haus", "--format", "json",
    ]);
    let folders = stdout(&ok(
        server.samu(&["folder", "list", "--format", "tsv"], None)
    ));
    assert!(folders.contains("\tHaus/Garten\t"), "{folders}");
    ok(server.samu(&["folder", "rename", "Garten", "Beete"], None));

    let old = made(&[
        "project", "add", "Rasen", "--folder", "Beete", "--format", "json",
    ]);
    let new = made(&["project", "add", "Hecke", "--format", "json"]);
    ok(server.samu(&["project", "rename", &old, "Wiese"], None));
    ok(server.samu(&["add", "Mähen", "--project", &old], None));
    ok(server.samu(&["add", "Vertikutieren", "--project", &old], None));

    let in_project = |id: &str| {
        let list = json_out(&ok(
            server.samu(&["list", "--project", id, "--format", "json"], None)
        ));
        list.as_array().unwrap().len()
    };
    assert_eq!(in_project(&old), 2, "the probe finds both");
    let dry = json_out(&ok(server.samu(
        &[
            "delete",
            &old,
            "--tasks",
            "Hecke",
            "--dry-run",
            "--format",
            "json",
        ],
        None,
    )));
    assert_eq!(dry[0]["moved"], 2, "{dry:#}");
    assert_eq!(in_project(&old), 2, "a dry run writes nothing");
    ok(server.samu(&["delete", &old, "--tasks", "Hecke"], None));
    assert_eq!(in_project(&new), 2);

    let wrong = server.samu(&["delete", &home, "--tasks", "inbox"], None);
    assert_eq!(
        code(&wrong),
        1,
        "{}",
        String::from_utf8_lossy(&wrong.stderr)
    );

    let third = made(&[
        "project", "add", "Teich", "--folder", "Beete", "--format", "json",
    ]);
    ok(server.samu(&["delete", &garden], None));
    let shown = json_out(&ok(server.samu(&["show", &third, "--format", "json"], None)));
    assert_eq!(shown[0]["object"]["folder"], home.as_str(), "{shown:#}");

    ok(server.samu(&["delete", &new], None));
    let inbox = json_out(&ok(server.samu(&["list", "--format", "json"], None)));
    assert_eq!(inbox.as_array().unwrap().len(), 2, "{inbox:#}");
}

/// ADR 0059, 0060: icons on projects and folders, a colour on a scope; set,
/// printed back, cleared; a wrong colour is a usage error.
#[test]
fn icons_and_a_scope_colour_are_set_and_cleared() {
    let server = signed_in_server();
    let ok = |out: Output| {
        assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
        out
    };
    let made = |args: &[&str]| {
        let out = json_out(&ok(server.samu(args, None)));
        out[0]["id"].as_str().unwrap().to_string()
    };
    let folder = made(&["folder", "add", "Haus", "--format", "json"]);
    let project = made(&["project", "add", "Garten", "--format", "json"]);
    ok(server.samu(&["folder", "icon", "Haus", "🏠"], None));
    ok(server.samu(&["project", "icon", &project, "🌻"], None));
    let shown = |id: &str| {
        json_out(&ok(server.samu(&["show", id, "--format", "json"], None)))[0]["object"].clone()
    };
    assert_eq!(shown(&folder)["icon"], "🏠");
    assert_eq!(shown(&project)["icon"], "🌻");
    let printed = stdout(&ok(
        server.samu(&["project", "icon", &project, "--format", "tsv"], None)
    ));
    assert_eq!(printed.trim(), "🌻");
    ok(server.samu(&["project", "icon", &project, "--clear"], None));
    assert!(shown(&project).get("icon").is_none());

    ok(server.samu(&["scope", "add", "wrk", "--only", "Haus"], None));
    ok(server.samu(&["scope", "color", "wrk", "blue"], None));
    let list = json_out(&ok(
        server.samu(&["scope", "list", "--format", "json"], None)
    ));
    assert_eq!(list[0]["color"], "blue", "{list:#}");
    let wrong = server.samu(&["scope", "color", "wrk", "black"], None);
    assert_eq!(
        code(&wrong),
        2,
        "{}",
        String::from_utf8_lossy(&wrong.stderr)
    );
    ok(server.samu(&["scope", "color", "wrk", "--clear"], None));
    let printed = json_out(&ok(
        server.samu(&["scope", "color", "wrk", "--format", "json"], None)
    ));
    assert!(printed[0]["color"].is_null(), "{printed:#}");
}

/// ADR 0037: a project moves into a folder and becomes a single-action
/// list on hold; a folder never goes below itself (exit 1); an empty
/// folder is listed.
#[test]
fn projects_and_folders_move_and_projects_are_set() {
    let server = signed_in_server();
    let ok = |out: Output| {
        assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
        out
    };
    let made = |args: &[&str]| {
        let out = json_out(&ok(server.samu(args, None)));
        out[0]["id"].as_str().unwrap().to_string()
    };
    made(&["folder", "add", "GTD", "--format", "json"]);
    made(&["folder", "add", "Haus", "--format", "json"]);
    made(&[
        "folder", "add", "Garten", "--parent", "Haus", "--format", "json",
    ]);
    let project = made(&["project", "add", "Sammelsurium", "--format", "json"]);
    let folders = stdout(&ok(
        server.samu(&["folder", "list", "--format", "tsv"], None)
    ));
    assert!(
        folders.contains("\tGTD\t"),
        "the empty folder is listed: {folders}"
    );

    ok(server.samu(&["project", "move", &project, "--folder", "GTD"], None));
    ok(server.samu(
        &[
            "project",
            "set",
            &project,
            "--kind",
            "single-actions",
            "--status",
            "on-hold",
            "--review",
            "2w",
        ],
        None,
    ));
    let shown = json_out(&ok(
        server.samu(&["show", &project, "--format", "json"], None)
    ));
    let object = &shown[0]["object"];
    let gtd = stdout(&ok(
        server.samu(&["folder", "list", "--format", "json"], None)
    ));
    let gtd: serde_json::Value = serde_json::from_str(&gtd).unwrap();
    let gtd_id = gtd
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == "GTD")
        .unwrap()["id"]
        .clone();
    assert_eq!(object["folder"], gtd_id, "{object:#}");
    assert_eq!(object["kind"], "single_actions");
    assert_eq!(object["status"], "on_hold");
    assert_eq!(object["review_interval"], "P2W");

    ok(server.samu(&["project", "set", &project, "--status", "completed"], None));
    let shown = json_out(&ok(
        server.samu(&["show", &project, "--format", "json"], None)
    ));
    assert!(shown[0]["object"]["completed_at"].is_string(), "{shown:#}");
    ok(server.samu(&["project", "set", &project, "--status", "active"], None));
    let shown = json_out(&ok(
        server.samu(&["show", &project, "--format", "json"], None)
    ));
    assert!(
        shown[0]["object"].get("completed_at").is_none(),
        "{shown:#}"
    );

    let cycle = server.samu(&["folder", "move", "Haus", "--parent", "Garten"], None);
    assert_eq!(
        code(&cycle),
        1,
        "{}",
        String::from_utf8_lossy(&cycle.stderr)
    );
    ok(server.samu(&["folder", "move", "Garten", "--top"], None));
    ok(server.samu(&["folder", "move", "Haus", "--parent", "Garten"], None));
    let folders = stdout(&ok(
        server.samu(&["folder", "list", "--format", "tsv"], None)
    ));
    assert!(folders.contains("\tGarten/Haus\t"), "{folders}");
    let nothing = server.samu(&["project", "set", &project], None);
    assert_eq!(code(&nothing), 1);
}

/// ADR 0039: an agent signs in as one, comments and attaches; the server
/// records it as the author, and the file comes back as it went up.
#[test]
fn an_agent_comments_and_attaches_a_file() {
    let server = Server::start(true);
    let login = server.samu(&["login", "--agent", "--password-stdin"], Some(PASSWORD));
    assert_eq!(
        code(&login),
        0,
        "{}",
        String::from_utf8_lossy(&login.stderr)
    );
    assert!(String::from_utf8_lossy(&login.stderr).contains("(agent)"));
    let session = json_out(&server.samu(&["session", "--format", "json"], None));
    assert_eq!(session[0]["kind"], "agent");

    let task = json_out(&server.samu(&["add", "Tafel", "--format", "json"], None));
    let task_id = task[0]["id"].as_str().unwrap().to_string();
    let first = json_out(&server.samu(
        &["comment", &task_id, "Erster Stand", "--format", "json"],
        None,
    ));
    let first_id = first[0]["id"].as_str().unwrap().to_string();
    json_out(&server.samu(
        &["comment", &task_id, "-", "--format", "json"],
        Some("Zweiter\nmit zwei Zeilen"),
    ));
    // Empty text is the command line's own error.
    assert_eq!(code(&server.samu(&["comment", &task_id, " "], None)), 1);

    // Asked of the server: the author is the session's.
    let shown = json_out(&server.samu(&["show", &first_id, "--format", "json"], None));
    assert_eq!(shown[0]["kind"], "comment");
    assert_eq!(
        shown[0]["object"]["created_by"],
        serde_json::json!({"device": "test", "kind": "agent"})
    );
    let listed = json_out(&server.samu(&["comments", &task_id, "--format", "json"], None));
    let bodies: Vec<&str> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["body"].as_str().unwrap())
        .collect();
    assert_eq!(bodies, ["Erster Stand", "Zweiter\nmit zwei Zeilen"]);
    assert_eq!(listed[0]["author"], "test (agent)");

    // A file on the task and one on the first comment.
    let file = server.dir.join("tafel.png");
    let bytes: Vec<u8> = (0..300_000u32).map(|i| (i % 253) as u8).collect();
    std::fs::write(&file, &bytes).unwrap();
    let attached = json_out(&server.samu(
        &[
            "attach",
            &task_id,
            file.to_str().unwrap(),
            "--format",
            "json",
        ],
        None,
    ));
    assert_eq!(attached[0]["media_type"], "image/png");
    assert_eq!(attached[0]["size"], 300_000);
    let hash = attached[0]["blob"].as_str().unwrap().to_string();
    let note = server.dir.join("memo.webm");
    std::fs::write(&note, b"not really sound").unwrap();
    json_out(&server.samu(
        &[
            "attach",
            &task_id,
            note.to_str().unwrap(),
            "--to-comment",
            &first_id,
            "--name",
            "Sprachnotiz",
            "--format",
            "json",
        ],
        None,
    ));
    // A comment of another task is refused.
    let other = json_out(&server.samu(&["add", "Anderes", "--format", "json"], None));
    let other_id = other[0]["id"].as_str().unwrap().to_string();
    assert_eq!(
        code(&server.samu(
            &[
                "attach",
                &other_id,
                note.to_str().unwrap(),
                "--to-comment",
                &first_id
            ],
            None
        )),
        1
    );

    let all = json_out(&server.samu(&["attachments", &task_id, "--format", "json"], None));
    let names: Vec<(&str, &str)> = all
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            (
                a["name"].as_str().unwrap(),
                a["media_type"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        names,
        [("tafel.png", "image/png"), ("Sprachnotiz", "audio/webm")]
    );
    let listed = json_out(&server.samu(&["comments", &task_id, "--format", "json"], None));
    assert_eq!(listed[0]["attachments"], 1);

    // The file comes back unchanged, to stdout and to a file.
    let fetched = server.samu(&["blob", "get", &hash], None);
    assert_eq!(code(&fetched), 0);
    assert!(fetched.stdout == bytes, "the file came back changed");
    let copy = server.dir.join("copy.png");
    assert_eq!(
        code(&server.samu(&["blob", "get", &hash, "-o", copy.to_str().unwrap()], None)),
        0
    );
    assert_eq!(std::fs::read(&copy).unwrap(), bytes);
    let missing = "0".repeat(64);
    assert_eq!(code(&server.samu(&["blob", "get", &missing], None)), 3);
}

/// ADR 0045: `samu forecast` shows what the server computes, each row with
/// its reason; `samu settings` shows and sets the account's settings, and
/// Forecast follows them.
#[test]
fn forecast_follows_the_settings() {
    let server = signed_in_server();
    let ok = |out: Output| {
        assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
        out
    };
    ok(server.samu(&["add", "Steuer abschicken ! d:morgen"], None));
    ok(server.samu(&["add", "Milch"], None));
    let rows = |server: &Server| -> Vec<(String, String, String)> {
        let rows = json_out(&server.samu(&["forecast", "--format", "json"], None));
        rows.as_array()
            .unwrap()
            .iter()
            .map(|r| {
                (
                    r["section"].as_str().unwrap().to_owned(),
                    r["why"].as_str().unwrap().to_owned(),
                    r["title"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    };
    let got = rows(&server);
    // Flagged and due tomorrow: under Markiert and on its day.
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!(got[0].0, "markiert");
    assert_eq!(
        (got[1].1.as_str(), got[1].2.as_str()),
        ("fällig", "Steuer abschicken")
    );

    let shown = json_out(&server.samu(&["settings", "--format", "json"], None));
    let value = |key: &str| {
        shown
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["key"] == key)
            .map(|r| r["value"].as_str().unwrap().to_owned())
    };
    assert_eq!(value("forecast-days").as_deref(), Some("7"));
    assert_eq!(value("forecast-flagged").as_deref(), Some("on"));
    ok(server.samu(&["settings", "set", "forecast-flagged", "off"], None));
    let got = rows(&server);
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!(got[0].1, "fällig");
    assert_ne!(
        code(&server.samu(&["settings", "set", "forecast-days", "0"], None)),
        0
    );
    assert_ne!(
        code(&server.samu(&["settings", "set", "farbe", "rot"], None)),
        0
    );
    assert_eq!(
        code(&server.samu(&["settings", "set", "forecast-tag", "Nichts"], None)),
        3
    );

    // ADR 0079 §2: the key bindings as one JSON object.
    assert_eq!(value("keys").as_deref(), Some("{}"));
    ok(server.samu(
        &[
            "settings",
            "set",
            "keys",
            r#"{"complete":"y","delete":null}"#,
        ],
        None,
    ));
    let shown = json_out(&server.samu(&["settings", "--format", "json"], None));
    let keys = shown
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["key"] == "keys")
        .map(|r| r["value"].as_str().unwrap().to_owned());
    assert_eq!(keys.as_deref(), Some(r#"{"complete":"y","delete":null}"#));
    assert_eq!(
        code(&server.samu(&["settings", "set", "keys", r#"{"complete":5}"#], None)),
        1
    );
    ok(server.samu(&["settings", "set", "keys", "{}"], None));
}

/// ADR 0083 §2: as JSON a Forecast row carries what `list` shows of the
/// task and says which section it stands in; plain stays as it was.
#[test]
fn forecast_json_carries_what_a_widget_draws() {
    let server = signed_in_server();
    let ok = |out: Output| {
        assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
        out
    };
    ok(server.samu(&["project", "add", "Garten"], None));
    ok(server.samu(&["add", "Rasen mähen #Garten !A d:morgen @draußen"], None));
    let rows = json_out(&server.samu(&["forecast", "--format", "json"], None));
    let row = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["title"] == "Rasen mähen")
        .unwrap();
    assert_eq!(row["kind"], "task");
    assert_eq!(row["section_kind"], "day");
    assert_eq!(row["day"], row["section"]);
    assert_eq!(row["project"], "Garten");
    assert_eq!(row["priority"], "A");
    assert_eq!(row["tags"], "@draußen");
    assert_eq!(row["due"], "soon");
    assert!(
        row["due_text"]
            .as_str()
            .unwrap()
            .starts_with(row["day"].as_str().unwrap())
    );

    let plain = ok(server.samu(&["forecast"], None));
    let plain = String::from_utf8_lossy(&plain.stdout);
    assert!(plain.contains("Rasen mähen"), "{plain}");
    assert!(!plain.contains("Garten"), "{plain}");
}

/// ADR 0046: `samu clock` starts, stops, adds and reports; a second `in`
/// ends the first, `done` stops the clock on the task, `out` without a
/// running clock is exit 3.
#[test]
fn clocks_start_stop_and_report() {
    let server = signed_in_server();
    let steuer = json_out(&server.samu(&["add", "Steuer", "--format", "json"], None));
    let steuer = steuer[0]["id"].as_str().unwrap().to_string();
    let garten = json_out(&server.samu(&["add", "Garten", "--format", "json"], None));
    let garten = garten[0]["id"].as_str().unwrap().to_string();

    // Nothing runs: status prints nothing, out is exit 3.
    assert!(stdout(&server.samu(&["clock", "status"], None)).is_empty());
    assert_eq!(code(&server.samu(&["clock", "out"], None)), 3);

    let first = json_out(&server.samu(
        &[
            "clock",
            "in",
            &steuer,
            "--at",
            "2026-11-09T07:00:00Z",
            "--format",
            "json",
        ],
        None,
    ));
    assert_eq!(first[0]["running"], true);
    let first_id = first[0]["id"].as_str().unwrap().to_string();
    let second = server.samu(
        &["clock", "in", &garten, "--at", "2026-11-09T08:00:00Z"],
        None,
    );
    assert_eq!(code(&second), 0);
    assert!(
        String::from_utf8_lossy(&second.stderr).contains("stopped: Steuer (1:00)"),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    // Asked of the server: the first ended where the second began.
    let shown = json_out(&server.samu(&["show", &first_id, "--format", "json"], None));
    assert_eq!(shown[0]["object"]["end"], "2026-11-09T08:00:00Z");
    let status = json_out(&server.samu(&["clock", "status", "--format", "json"], None));
    assert_eq!(status[0]["title"], "Garten");

    let out = json_out(&server.samu(
        &[
            "clock",
            "out",
            "--at",
            "2026-11-09T08:30:00Z",
            "--format",
            "json",
        ],
        None,
    ));
    assert_eq!(out[0]["length"], "0:30");
    assert!(stdout(&server.samu(&["clock", "status"], None)).is_empty());

    json_out(&server.samu(
        &[
            "clock",
            "add",
            &steuer,
            "--start",
            "2026-11-09T09:00:00Z",
            "--end",
            "2026-11-09T09:45:00Z",
            "--format",
            "json",
        ],
        None,
    ));
    // The end before the start: refused before anything is sent.
    assert_eq!(
        code(&server.samu(
            &[
                "clock",
                "add",
                &steuer,
                "--start",
                "2026-11-09T10:00:00Z",
                "--end",
                "2026-11-09T09:00:00Z",
            ],
            None,
        )),
        1
    );
    let mine = json_out(&server.samu(&["clock", "list", &steuer, "--format", "json"], None));
    assert_eq!(mine.as_array().unwrap().len(), 2);

    // Done stops the clock on it.
    json_out(&server.samu(
        &[
            "clock",
            "in",
            &steuer,
            "--at",
            "2026-11-09T11:00:00Z",
            "--format",
            "json",
        ],
        None,
    ));
    assert_eq!(code(&server.samu(&["done", &steuer], None)), 0);
    assert!(stdout(&server.samu(&["clock", "status"], None)).is_empty());

    let report = json_out(&server.samu(
        &[
            "clock",
            "report",
            "--from",
            "2026-11-09",
            "--to",
            "2026-11-09",
            "--format",
            "json",
        ],
        None,
    ));
    let rows = report.as_array().unwrap();
    assert_eq!(rows[0]["what"], "Steuer");
    assert_eq!(rows[1]["what"], "Garten");
    assert_eq!(rows[1]["length"], "0:30");
    assert_eq!(rows.last().unwrap()["total"], true);
    let by_day = stdout(&server.samu(
        &[
            "clock",
            "report",
            "--by",
            "day",
            "--from",
            "2026-11-09",
            "--format",
            "tsv",
        ],
        None,
    ));
    assert!(by_day.contains("2026-11-09"), "{by_day}");
}

/// A calendar file served over plain HTTP from a thread, counted from
/// today so the server's window holds it: tomorrow at nine, and a trip over
/// today and the next two days. Any other path is a 404.
fn ics_server() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let zone = jiff::tz::TimeZone::get("Europe/Berlin").unwrap();
    let today = jiff::Timestamp::now().to_zoned(zone).date();
    let day = |d: jiff::civil::Date| d.strftime("%Y%m%d").to_string();
    let tomorrow = day(today.tomorrow().unwrap());
    let ics = [
        "BEGIN:VCALENDAR".to_string(),
        "BEGIN:VEVENT".into(),
        "UID:dentist@example.org".into(),
        "SUMMARY:Zahnarzt".into(),
        "LOCATION:Hauptstraße 1".into(),
        format!("DTSTART;TZID=Europe/Berlin:{tomorrow}T090000"),
        format!("DTEND;TZID=Europe/Berlin:{tomorrow}T094500"),
        "BEGIN:VALARM".into(),
        "ACTION:DISPLAY".into(),
        "TRIGGER:-PT15M".into(),
        "END:VALARM".into(),
        "END:VEVENT".into(),
        "BEGIN:VEVENT".into(),
        "UID:trip@example.org".into(),
        "SUMMARY:Reise".into(),
        format!("DTSTART;VALUE=DATE:{}", day(today)),
        format!(
            "DTEND;VALUE=DATE:{}",
            day(today.checked_add(jiff::ToSpan::days(3)).unwrap())
        ),
        "END:VEVENT".into(),
        "END:VCALENDAR".into(),
        String::new(),
    ]
    .join("\r\n");
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            reader.read_line(&mut first).unwrap_or_default();
            // The rest of the head.
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 2 {
                line.clear();
            }
            let answer = if first.starts_with("GET /cal.ics ") {
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/calendar\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{ics}",
                    ics.len()
                )
            } else {
                "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into()
            };
            let _ = stream.write_all(answer.as_bytes());
        }
    });
    format!("http://{address}")
}

/// ADR 0047, 0048: a calendar is added with its source, fetched, listed,
/// changed and deleted; its appointments come unfolded; an error after a
/// refresh is exit 1, a calendar that is not there exit 3.
#[test]
fn calendars_are_added_fetched_listed_and_deleted() {
    let server = signed_in_server();
    let ics = ics_server();
    let added = json_out(&server.samu(
        &[
            "calendar",
            "add",
            "Familie",
            "--url",
            &format!("{ics}/cal.ics"),
            "--format",
            "json",
        ],
        None,
    ));
    assert_eq!(added[0]["host"], "127.0.0.1", "{added}");
    assert!(added[0]["error"].is_null(), "{added}");
    // The address stays on the server.
    let listed = server.samu(&["calendar", "list", "--format", "json"], None);
    assert!(!stdout(&listed).contains("cal.ics"), "{}", stdout(&listed));
    assert_eq!(json_out(&listed)[0]["title"], "Familie");

    let zone = jiff::tz::TimeZone::get("Europe/Berlin").unwrap();
    let today = jiff::Timestamp::now().to_zoned(zone).date();
    let to = today
        .checked_add(jiff::ToSpan::days(6))
        .unwrap()
        .to_string();
    let from = today.to_string();
    let week = json_out(&server.samu(
        &[
            "appointments",
            "--from",
            &from,
            "--to",
            &to,
            "--format",
            "json",
        ],
        None,
    ));
    let titles: Vec<&str> = week
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["title"].as_str().unwrap())
        .collect();
    assert_eq!(titles, vec!["Reise", "Zahnarzt"], "{week}");
    assert_eq!(week[0]["all_day"], true);
    assert_eq!(week[0]["calendar"], "Familie");
    assert_eq!(week[1]["location"], "Hauptstraße 1");

    // ADR 0101 §5: its VALARM goes off 15 minutes before; a window over
    // seven days is a usage error of the server (exit 1).
    let two_days = today
        .checked_add(jiff::ToSpan::days(2))
        .unwrap()
        .to_string();
    let alarms = json_out(&server.samu(
        &[
            "alarms", "--from", &from, "--to", &two_days, "--format", "json",
        ],
        None,
    ));
    assert_eq!(alarms.as_array().unwrap().len(), 1, "{alarms}");
    assert_eq!(alarms[0]["kind"], "appointment");
    assert_eq!(alarms[0]["title"], "Zahnarzt");
    assert_eq!(alarms[0]["calendar"], "Familie");
    assert!(
        alarms[0]["at"].as_str().unwrap().ends_with(":45"),
        "{alarms}"
    );
    let quiet = json_out(&server.samu(
        &[
            "alarms",
            "--from",
            &from,
            "--to",
            &two_days,
            "--no-appointments",
            "--format",
            "json",
        ],
        None,
    ));
    assert_eq!(quiet, serde_json::json!([]));
    let long = today
        .checked_add(jiff::ToSpan::days(9))
        .unwrap()
        .to_string();
    assert_eq!(
        code(&server.samu(&["alarms", "--from", &from, "--to", &long], None)),
        1
    );

    let set = json_out(&server.samu(
        &[
            "calendar", "set", "familie", "color", "#ff0000", "--format", "json",
        ],
        None,
    ));
    assert_eq!(set[0]["color"], "#ff0000");
    assert_eq!(
        code(&server.samu(&["calendar", "set", "Familie", "host", "x"], None)),
        1,
        "the host is the server's"
    );
    assert_eq!(
        code(&server.samu(&["calendar", "refresh", "Familie"], None)),
        0
    );

    // A source that answers 404: said on refresh, exit 1.
    let moved = server.samu(
        &["calendar", "source", "Familie", &format!("{ics}/gone.ics")],
        None,
    );
    assert_eq!(
        code(&moved),
        0,
        "{}",
        String::from_utf8_lossy(&moved.stderr)
    );
    let refresh = server.samu(&["calendar", "refresh", "Familie"], None);
    assert_eq!(code(&refresh), 1);
    assert!(
        String::from_utf8_lossy(&refresh.stderr).contains("404"),
        "{}",
        String::from_utf8_lossy(&refresh.stderr)
    );
    // Its appointments stay meanwhile.
    let still = json_out(&server.samu(
        &[
            "appointments",
            "--from",
            &from,
            "--to",
            &to,
            "--format",
            "json",
        ],
        None,
    ));
    assert_eq!(still.as_array().unwrap().len(), 2);

    assert_eq!(
        code(&server.samu(&["calendar", "refresh", "Kein"], None)),
        3
    );
    // Two of one name: the error names them by id, and the id finds one.
    let twin = json_out(&server.samu(&["calendar", "add", "Familie", "--format", "json"], None));
    let twin = twin[0]["short"].as_str().unwrap().to_string();
    let both = server.samu(&["calendar", "set", "Familie", "color", "#000000"], None);
    assert_eq!(code(&both), 1);
    let said = String::from_utf8_lossy(&both.stderr).to_string();
    assert!(said.contains(&twin), "{said}");
    assert_eq!(code(&server.samu(&["calendar", "delete", &twin], None)), 0);
    // Forecast shows them before the tasks, unless switched off.
    let forecast = json_out(&server.samu(&["forecast", "--format", "json"], None));
    let mine: Vec<&str> = forecast
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["why"] == "termin")
        .map(|r| r["title"].as_str().unwrap())
        .collect();
    assert_eq!(
        mine,
        vec!["Reise", "Reise", "Zahnarzt", "Reise"],
        "{forecast}"
    );
    assert_eq!(
        code(&server.samu(&["settings", "set", "forecast-events", "off"], None)),
        0
    );
    let quiet = json_out(&server.samu(&["forecast", "--format", "json"], None));
    assert!(
        quiet
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r["why"] != "termin"),
        "{quiet}"
    );
    assert_eq!(
        code(&server.samu(&["calendar", "delete", "Familie"], None)),
        0
    );
    let none = json_out(&server.samu(
        &[
            "appointments",
            "--from",
            &from,
            "--to",
            &to,
            "--format",
            "json",
        ],
        None,
    ));
    assert_eq!(none, serde_json::json!([]));
}

/// ADR 0052 §7: `samu bulk` does one action on many tasks in one write;
/// `-` reads the ids from stdin, a picked child goes along with its parent.
#[test]
fn bulk_acts_on_many_tasks() {
    let server = signed_in_server();
    let ok = |out: Output| {
        assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
        out
    };
    let add = |title: &str| {
        json_out(&server.samu(&["add", title, "--format", "json"], None))[0]["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let show = |id: &str| {
        json_out(&server.samu(&["show", id, "--format", "json"], None))[0]["object"].clone()
    };
    ok(server.samu(
        &["tag", "add", "Status", "--exclusive", "--default-workflow"],
        None,
    ));
    ok(server.samu(
        &["tag", "add", "Läuft", "--parent", "Status", "--key", "l"],
        None,
    ));
    ok(server.samu(
        &["tag", "add", "Wartet", "--parent", "Status", "--ask-note"],
        None,
    ));
    ok(server.samu(&["tag", "add", "Telefon"], None));
    let project = json_out(&ok(
        server.samu(&["project", "add", "Haus", "--format", "json"], None)
    ))[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let a = add("Dach");
    let b = add("Ziegel");
    let c = add("Rinne");
    ok(server.samu(&["set", &b, "parent", &a], None));

    // b is below a: it goes along and keeps its parent.
    let moved = ok(server.samu(&["bulk", "move", "--to", "Haus", &b, &a, &c], None));
    assert!(String::from_utf8_lossy(&moved.stderr).contains("2 of 2 tasks moved"));
    assert_eq!(show(&a)["project"], project.as_str());
    assert_eq!(show(&b)["project"], project.as_str());
    assert_eq!(show(&b)["parent"], a.as_str());
    assert_eq!(show(&c)["project"], project.as_str());
    assert!(show(&c)["position"].as_i64().unwrap() > show(&a)["position"].as_i64().unwrap());

    ok(server.samu(&["bulk", "flag", &a, &c], None));
    ok(server.samu(&["bulk", "tag", "--add", "Telefon", &a, &b], None));
    assert_eq!(show(&a)["flagged"], true);
    assert_eq!(show(&b)["tags"].as_array().unwrap().len(), 1);
    let refused = server.samu(&["bulk", "tag", "--add", "Läuft", &a], None);
    assert_eq!(code(&refused), 1, "a state goes through bulk state");

    ok(server.samu(&["bulk", "state", "--to", "l", &a, &c], None));
    let all = json_out(&server.samu(&["list", "--format", "json"], None));
    let states: Vec<&str> = all
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["state"] == "Läuft")
        .map(|r| r["title"].as_str().unwrap())
        .collect();
    assert_eq!(states.len(), 2, "{all:#}");
    assert_eq!(
        code(&server.samu(&["bulk", "state", "--to", "Wartet", &a], None)),
        1
    );

    ok(server.samu(&["bulk", "date", "due", "2026-12-01", &a, &b, &c], None));
    assert_eq!(show(&c)["due"]["date"], "2026-12-01");
    ok(server.samu(&["bulk", "date", "due", "none", &c], None));
    assert!(show(&c).get("due").is_none());
    // Today is the account's, whatever this computer's zone: these two
    // are on different dates at any instant, so before the fix they
    // differed.
    let mut dues = Vec::new();
    for tz in ["Etc/GMT+12", "Pacific/Kiritimati"] {
        ok(server.samu_in_zone(tz, &["bulk", "date", "due", "today", &c]));
        dues.push(show(&c)["due"]["date"].clone());
    }
    assert_eq!(dues[0], dues[1], "{dues:?}");
    ok(server.samu(&["bulk", "date", "due", "none", &c], None));

    // From stdin, as `samu list --format tsv` prints it.
    let listed = stdout(&ok(server.samu(&["list", "--format", "tsv"], None)));
    let done = ok(server.samu(
        &["bulk", "done", "-", "--format", "json"],
        Some(listed.trim_end()),
    ));
    assert_eq!(json_out(&done).as_array().unwrap().len(), 3);
    assert!(show(&c)["completed_at"].is_string());
    ok(server.samu(&["bulk", "reopen", &c], None));
    assert!(show(&c).get("completed_at").is_none());

    let dry = json_out(&ok(server.samu(
        &["bulk", "delete", "--dry-run", &a, &b, "--format", "json"],
        None,
    )));
    assert_eq!(dry.as_array().unwrap().len(), 1, "b goes with a");
    assert_eq!(dry[0]["deleted"], 2);
    assert_eq!(show(&b)["title"], "Ziegel", "a dry run writes nothing");
    ok(server.samu(&["bulk", "delete", &a, &c], None));
    assert_eq!(code(&server.samu(&["show", &b], None)), 3);
    assert_eq!(code(&server.samu(&["bulk", "flag", "zzzzzzzz"], None)), 3);
}

/// `samu render` (ADR 0058 §4): the format read off the text or asked for,
/// the tree with `--format json`, from stdin or a file.
#[test]
fn render_reads_org_and_markdown() {
    let server = signed_in_server();
    let org = server.samu(&["render"], Some("#+title: x\n- [ ] [[https://x.org][a]]"));
    assert_eq!(code(&org), 0);
    assert_eq!(stdout(&org).trim(), "org");

    let markdown = json_out(&server.samu(
        &["render", "--format", "json"],
        Some("# Plan\n- [x] **fertig**"),
    ));
    assert_eq!(markdown[0]["format"], "markdown");
    let blocks = &markdown[0]["document"]["blocks"];
    assert_eq!(blocks[0]["type"], "heading");
    assert_eq!(blocks[1]["items"][0]["checked"], true);

    let forced = server.samu(&["render", "--as", "markdown"], Some("#+title: x"));
    assert_eq!(stdout(&forced).trim(), "markdown");

    let file = server.dir.join("note.org");
    std::fs::write(&file, ":PROPERTIES:\n:ID: a\n:END:\n").unwrap();
    let from_file = server.samu(&["render", file.to_str().unwrap()], None);
    assert_eq!(stdout(&from_file).trim(), "org");

    let missing = server.samu(&["render", "/nonexistent/samu-note.md"], None);
    assert_ne!(code(&missing), 0);
}

/// ADR 0068: `samu appointment` reaches its routes and says no with the
/// right exit codes where samu may not write: an ICS calendar is read-only
/// (exit 1), an appointment that is not there is exit 3, an edit without a
/// field and an add without a calendar are exit 1. Writing itself is
/// tested against a CalDAV server in samu-server.
#[test]
fn appointments_of_a_read_only_calendar_are_refused() {
    let server = signed_in_server();
    let ics = ics_server();
    let added = server.samu(
        &[
            "calendar",
            "add",
            "Familie",
            "--url",
            &format!("{ics}/cal.ics"),
        ],
        None,
    );
    assert_eq!(
        code(&added),
        0,
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    let listed = json_out(&server.samu(&["calendar", "list", "--format", "json"], None));
    assert_eq!(listed[0]["writable"], false, "ICS is never writable");

    let zone = jiff::tz::TimeZone::get("Europe/Berlin").unwrap();
    let today = jiff::Timestamp::now().to_zoned(zone).date();
    let to = today
        .checked_add(jiff::ToSpan::days(6))
        .unwrap()
        .to_string();
    let week = json_out(&server.samu(
        &[
            "appointments",
            "--from",
            &today.to_string(),
            "--to",
            &to,
            "--format",
            "json",
        ],
        None,
    ));
    let one = week[0]["appointment"].as_str().unwrap().to_string();
    assert!(
        !one.is_empty(),
        "the probe must find an appointment: {week}"
    );

    let edit = server.samu(&["appointment", "edit", &one, "--title", "anders"], None);
    assert_eq!(code(&edit), 1, "{}", String::from_utf8_lossy(&edit.stderr));
    assert!(
        String::from_utf8_lossy(&edit.stderr).contains("not writable"),
        "{}",
        String::from_utf8_lossy(&edit.stderr)
    );
    let delete = server.samu(&["appointment", "delete", &one], None);
    assert_eq!(code(&delete), 1);
    let add = server.samu(
        &[
            "appointment",
            "add",
            "Neu",
            "--start",
            &today.to_string(),
            "--calendar",
            "Familie",
        ],
        None,
    );
    assert_eq!(code(&add), 1, "{}", String::from_utf8_lossy(&add.stderr));
    let no_calendar = server.samu(
        &["appointment", "add", "Neu", "--start", &today.to_string()],
        None,
    );
    assert_eq!(code(&no_calendar), 1);
    assert_eq!(
        code(&server.samu(&["appointment", "edit", &one], None)),
        1,
        "no field"
    );
    assert_eq!(
        code(&server.samu(&["appointment", "edit", "ffffffff", "--title", "x"], None)),
        3
    );
    // Nothing changed.
    let again = json_out(&server.samu(
        &[
            "appointments",
            "--from",
            &today.to_string(),
            "--to",
            &to,
            "--format",
            "json",
        ],
        None,
    ));
    assert_eq!(again, week);
}

/// ADR 0094: what has an own scope is hidden without a scope and in every
/// other one, shown in its own scope also with „alles außer“, and counts
/// for the hint when due; `samu set … own_scope <scope>` names the scope by
/// title, `samu unset` takes it away; `scope_only` is no longer set.
#[test]
fn an_own_scope_hides_a_folder_and_a_tag_outside_it() {
    let server = signed_in_server();
    let ok = |out: Output| {
        assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
        out
    };
    let made = |args: &[&str]| {
        let out = json_out(&ok(server.samu(args, None)));
        out[0]["id"].as_str().unwrap().to_string()
    };
    let titles = |args: &[&str]| -> Vec<String> {
        let mut all = vec!["list", "--format", "json"];
        all.extend_from_slice(args);
        let rows = json_out(&ok(server.samu(&all, None)));
        let mut titles: Vec<String> = rows
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["title"].as_str().unwrap().to_owned())
            .collect();
        titles.sort();
        titles
    };
    let habits = made(&["folder", "add", "Habits", "--format", "json"]);
    made(&[
        "project",
        "add",
        "Gewohnheiten",
        "--folder",
        "Habits",
        "--format",
        "json",
    ]);
    ok(server.samu(&["tag", "add", "habit"], None));
    ok(server.samu(
        &["add", "Rauchfrei d:2000-01-01", "--project", "Gewohnheiten"],
        None,
    ));
    ok(server.samu(&["add", "Dehnen @habit"], None));
    ok(server.samu(&["add", "Milch"], None));
    // The probe that must find something: all three before the mark.
    assert_eq!(titles(&[]), vec!["Dehnen", "Milch", "Rauchfrei"]);

    ok(server.samu(&["scope", "add", "Eingang", "--only", "--inbox"], None));
    // The case of 13.11.2026: everything but the inbox.
    ok(server.samu(&["scope", "add", "Gewohnt", "--except", "--inbox"], None));
    let refused = server.samu(&["set", &habits, "scope_only", "true"], None);
    assert_ne!(code(&refused), 0, "scope_only is derived");
    let missing = server.samu(&["set", &habits, "own_scope", "Nirgends"], None);
    assert_ne!(code(&missing), 0, "no such scope");
    ok(server.samu(&["set", &habits, "own_scope", "gewohnt"], None));
    let tags = json_out(&ok(server.samu(&["tag", "list", "--format", "json"], None)));
    let habit = tags.as_array().unwrap()[0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    ok(server.samu(&["set", &habit, "own_scope", "Gewohnt"], None));
    let marks = stdout(&ok(server.samu(&["tag", "list", "--format", "tsv"], None)));
    assert!(marks.contains("only:Gewohnt"), "{marks}");
    let folders = stdout(&ok(
        server.samu(&["folder", "list", "--format", "tsv"], None)
    ));
    assert!(
        !folders.contains("Habits"),
        "hidden without a scope: {folders}"
    );
    let folders = stdout(&ok(server.samu(
        &["folder", "list", "--scope", "Gewohnt", "--format", "tsv"],
        None,
    )));
    assert!(folders.contains("only:Gewohnt"), "{folders}");

    // Without a scope: the inbox task with the tag is gone too (ADR 0069
    // §5), and the overdue one counts for the hint (§7).
    assert_eq!(titles(&[]), vec!["Milch"]);
    let out = ok(server.samu(&["list"], None));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("1 due, hidden"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Another scope does not show it; its own does, with except: what is
    // not ticked (Habits) is in, the ticked inbox is out, tag or not.
    assert_eq!(titles(&["--scope", "Eingang"]), vec!["Milch"]);
    assert_eq!(titles(&["--scope", "Gewohnt"]), vec!["Rauchfrei"]);
    ok(server.samu(&["scope", "inbox", "Gewohnt", "off"], None));
    assert_eq!(
        titles(&["--scope", "Gewohnt"]),
        vec!["Dehnen", "Milch", "Rauchfrei"]
    );

    // Off again: everything is back without a scope.
    ok(server.samu(&["unset", &habits, "own_scope"], None));
    ok(server.samu(&["unset", &habit, "own_scope"], None));
    assert_eq!(titles(&[]), vec!["Dehnen", "Milch", "Rauchfrei"]);
}

/// ADR 0072 §5: `samu scope layout` sets the inspector's parts and the
/// note, prints them, keeps one when the other changes, refuses an unknown
/// part and takes the layout off.
#[test]
fn a_scope_layout_is_set_printed_and_cleared() {
    let server = signed_in_server();
    let ok = |out: Output| {
        assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
        out
    };
    ok(server.samu(&["scope", "add", "Gewohnt", "--only", "--inbox"], None));
    let printed = |server: &Server| {
        json_out(&ok(
            server.samu(&["scope", "layout", "Gewohnt", "--format", "json"], None)
        ))
    };
    // The probe: nothing yet.
    assert_eq!(printed(&server)[0]["inspector"], "");

    ok(server.samu(
        &[
            "scope",
            "layout",
            "Gewohnt",
            "--inspector",
            "note,repeat,planned,reminders",
            "--note",
            "rendered",
        ],
        None,
    ));
    let layout = printed(&server);
    assert_eq!(layout[0]["inspector"], "note,repeat,planned,reminders");
    assert_eq!(layout[0]["note"], "rendered");

    // Only the note: the parts stay.
    ok(server.samu(&["scope", "layout", "Gewohnt", "--note", "default"], None));
    let layout = printed(&server);
    assert_eq!(layout[0]["inspector"], "note,repeat,planned,reminders");
    assert_eq!(layout[0]["note"], "default");

    let bad = server.samu(
        &["scope", "layout", "Gewohnt", "--inspector", "note,sidebar"],
        None,
    );
    assert_ne!(code(&bad), 0);
    assert!(String::from_utf8_lossy(&bad.stderr).contains("no part of the inspector"));

    ok(server.samu(&["scope", "layout", "Gewohnt", "--clear"], None));
    let scopes = json_out(&ok(
        server.samu(&["scope", "list", "--format", "json"], None)
    ));
    assert!(
        scopes[0].get("layout").is_none_or(|l| l.is_null()),
        "{scopes}"
    );
    assert_eq!(printed(&server)[0]["inspector"], "");
}

/// ADR 0073: `samu template` makes and changes a template; `samu add` calls
/// it as `/name` or with `--template`, and the task gets its marks and
/// reminders; a `/name` that is no template stays in the title.
#[test]
fn a_template_fills_in_a_capture_and_gives_its_reminders() {
    let server = signed_in_server();
    let ok = |out: Output| {
        assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
        out
    };
    ok(server.samu(
        &["project", "add", "Gewohnheiten", "--format", "json"],
        None,
    ));
    ok(server.samu(
        &[
            "template",
            "add",
            "habit",
            "#Gewohnheiten",
            "@habit",
            "p:heute",
            "21:00",
            "every+:1d",
            "--remind",
            "planned",
        ],
        None,
    ));
    let list = json_out(&ok(
        server.samu(&["template", "list", "--format", "json"], None)
    ));
    assert_eq!(list[0]["call"], "/habit");
    assert_eq!(list[0]["reminders"], "planned");
    // Free text in a template is refused by the server.
    assert_ne!(
        code(&server.samu(&["template", "text", "habit", "Rauchfrei", "#x"], None)),
        0
    );

    let made = json_out(&ok(
        server.samu(&["add", "Rauchfrei /habit", "--format", "json"], None)
    ));
    let task = &made[0]["task"];
    assert_eq!(task["title"], "Rauchfrei");
    assert!(task["project"].is_string(), "{task}");
    assert!(
        task["planned"]["at"]
            .as_str()
            .unwrap()
            .contains("T21:00:00"),
        "{task}"
    );
    assert_eq!(task["repeat"]["catch_up"], true);
    let id = made[0]["id"].as_str().unwrap().to_owned();
    // The server holds the reminder, not only the answer.
    let reminders = json_out(&ok(
        server.samu(&["reminders", &id, "--format", "json"], None)
    ));
    assert_eq!(reminders.as_array().unwrap().len(), 1, "{reminders}");
    assert_eq!(reminders[0]["anchor"], "planned");

    // The same with --template, and the line's project wins.
    ok(server.samu(&["project", "add", "Anderes", "--format", "json"], None));
    let other = json_out(&ok(server.samu(
        &[
            "add",
            "Dehnen #Anderes",
            "--template",
            "habit",
            "--format",
            "json",
        ],
        None,
    )));
    assert_eq!(other[0]["task"]["title"], "Dehnen");
    assert_ne!(other[0]["task"]["project"], task["project"]);

    // An unknown one stays text, with a warning.
    let out = ok(server.samu(&["add", "Pfad /gibtsnicht", "--format", "json"], None));
    assert_eq!(json_out(&out)[0]["task"]["title"], "Pfad /gibtsnicht");
    assert!(String::from_utf8_lossy(&out.stderr).contains("/gibtsnicht: no template"));

    ok(server.samu(&["template", "rename", "habit", "gewohnheit"], None));
    ok(server.samu(&["template", "remind", "gewohnheit", "--clear"], None));
    let list = json_out(&ok(
        server.samu(&["template", "list", "--format", "json"], None)
    ));
    assert_eq!(list[0]["call"], "/gewohnheit");
    assert_eq!(list[0]["reminders"], "");
    ok(server.samu(&["template", "delete", "gewohnheit"], None));
    let list = json_out(&ok(
        server.samu(&["template", "list", "--format", "json"], None)
    ));
    assert!(list.as_array().unwrap().is_empty());
}

/// ADR 0084 §4: `samu template key` gives a template the key that calls it
/// after the capture prefix; a taken or malformed key is refused, `h` and
/// `H` are two, and `--none` takes it off.
#[test]
fn a_template_gets_a_key() {
    let server = signed_in_server();
    let ok = |out: Output| {
        assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
        out
    };
    ok(server.samu(&["template", "add", "habit", "@habit", "--key", "h"], None));
    ok(server.samu(&["template", "add", "inbox", "@inbox"], None));
    let list = json_out(&ok(
        server.samu(&["template", "list", "--format", "json"], None)
    ));
    assert_eq!(list[0]["key"], "h");
    assert!(list[1]["key"].is_null());

    let taken = server.samu(&["template", "key", "inbox", "h"], None);
    assert_ne!(code(&taken), 0);
    assert!(String::from_utf8_lossy(&taken.stderr).contains("\"habit\""));
    assert_ne!(
        code(&server.samu(&["template", "key", "inbox", "ib"], None)),
        0
    );
    ok(server.samu(&["template", "key", "inbox", "H"], None));
    // Its own key again is fine.
    ok(server.samu(&["template", "key", "habit", "h"], None));
    let tsv = ok(server.samu(&["template", "list", "--format", "tsv"], None));
    let tsv = String::from_utf8_lossy(&tsv.stdout);
    assert!(tsv.lines().any(|l| l.contains("\tH\t/inbox")), "{tsv}");

    ok(server.samu(&["template", "key", "habit", "--none"], None));
    let list = json_out(&ok(
        server.samu(&["template", "list", "--format", "json"], None)
    ));
    assert!(list[0]["key"].is_null());
    assert_eq!(list[1]["key"], "H");
}

/// ADR 0074: `samu export` writes the directory with the files of
/// attachments and `samu.org`; `samu export-org` reads it without a
/// server; `samu-server export` beside the running server gives the same
/// files, and `samu-server restore` makes the account again elsewhere.
#[test]
fn an_account_is_exported_read_without_samu_and_restored() {
    let server = signed_in_server();
    let added = json_out(&server.samu(&["add", "Milch", "kaufen", "!A", "--format", "json"], None));
    let id = added[0]["id"].as_str().unwrap().to_string();
    let file = server.dir.join("tafel.png");
    let bytes: Vec<u8> = (0..3_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&file, &bytes).unwrap();
    let attached = json_out(&server.samu(
        &["attach", &id, file.to_str().unwrap(), "--format", "json"],
        None,
    ));
    let hash = attached[0]["blob"].as_str().unwrap().to_string();

    let out = server.dir.join("export");
    let export = server.samu(&["export", out.to_str().unwrap(), "--org"], None);
    assert_eq!(
        code(&export),
        0,
        "{}",
        String::from_utf8_lossy(&export.stderr)
    );
    assert!(stdout(&export).is_empty(), "messages go to stderr");
    let tasks = std::fs::read_to_string(out.join("tasks.json")).unwrap();
    assert!(tasks.contains("\"title\": \"Milch kaufen\""), "{tasks}");
    assert_eq!(std::fs::read(out.join("files").join(&hash)).unwrap(), bytes);
    let org = std::fs::read_to_string(out.join("samu.org")).unwrap();
    assert!(org.contains("** TODO [#A] Milch kaufen\n"), "{org}");
    assert!(
        org.contains(&format!("[[file:files/{hash}][tafel.png]]")),
        "{org}"
    );

    // Without a server: from the directory alone.
    let printed = run_samu(
        "http://127.0.0.1:9",
        &server.dir,
        &["export-org", out.to_str().unwrap(), "-o", "-"],
        None,
    );
    assert_eq!(
        code(&printed),
        0,
        "{}",
        String::from_utf8_lossy(&printed.stderr)
    );
    assert_eq!(stdout(&printed), org);
    let empty = server.dir.join("leer");
    std::fs::create_dir_all(&empty).unwrap();
    assert_eq!(
        code(&server.samu(&["export-org", empty.to_str().unwrap()], None)),
        1
    );

    // The server's own export, beside the running server.
    let data = server.dir.join("data");
    let beside = server.dir.join("beside");
    let run = |args: &[&str]| {
        Command::new(server_binary())
            .arg("--data")
            .arg(args[0])
            .args(&args[1..])
            .output()
            .unwrap()
    };
    let done = run(&[
        data.to_str().unwrap(),
        "export",
        "--account",
        "privat",
        beside.to_str().unwrap(),
    ]);
    assert!(
        done.status.success(),
        "{}",
        String::from_utf8_lossy(&done.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(beside.join("tasks.json")).unwrap(),
        tasks
    );

    // Restored into a fresh data directory, exported again: the same.
    let fresh = server.dir.join("fresh");
    let mut add = Command::new(server_binary())
        .arg("--data")
        .arg(&fresh)
        .args(["account", "add", "privat", "--password-stdin"])
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(add.stdin.take().unwrap(), "{PASSWORD}").unwrap();
    assert!(add.wait_with_output().unwrap().status.success());
    let done = run(&[
        fresh.to_str().unwrap(),
        "restore",
        "--account",
        "privat",
        beside.to_str().unwrap(),
    ]);
    assert!(
        done.status.success(),
        "{}",
        String::from_utf8_lossy(&done.stderr)
    );
    let again = server.dir.join("again");
    let done = run(&[
        fresh.to_str().unwrap(),
        "export",
        "--account",
        "privat",
        again.to_str().unwrap(),
    ]);
    assert!(
        done.status.success(),
        "{}",
        String::from_utf8_lossy(&done.stderr)
    );
    for name in [
        "tasks.json",
        "attachments.json",
        "logbook.json",
        "files.json",
    ] {
        assert_eq!(
            std::fs::read_to_string(beside.join(name)).unwrap(),
            std::fs::read_to_string(again.join(name)).unwrap(),
            "{name}"
        );
    }
    assert_eq!(
        std::fs::read(again.join("files").join(&hash)).unwrap(),
        bytes
    );
    // A second restore is refused.
    let refused = run(&[
        fresh.to_str().unwrap(),
        "restore",
        "--account",
        "privat",
        beside.to_str().unwrap(),
    ]);
    assert!(!refused.status.success());
    assert!(String::from_utf8_lossy(&refused.stderr).contains("not empty"));
}

/// ADR 0075 §2: `samu:<uuid>` opens `<web>/o/<uuid>`, `web` from the config
/// file, else the server; no session needed. A broken address is exit 1.
#[test]
fn a_samu_address_opens_the_object_in_the_web_ui() {
    let dir = std::env::temp_dir().join(format!("samu-cli-uri-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let id = "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6c01";
    let command = |out: &Output| -> Vec<String> {
        assert_eq!(code(out), 0, "{}", String::from_utf8_lossy(&out.stderr));
        let rows: serde_json::Value = serde_json::from_str(&stdout(out)).unwrap();
        serde_json::from_value(rows[0]["command"].clone()).unwrap()
    };
    let uri = format!("samu:{id}");
    let args = ["open-uri", uri.as_str(), "--print", "--format", "json"];
    let out = run_samu("https://samu.example.org/", &dir, &args, None);
    assert_eq!(
        command(&out),
        ["xdg-open", &format!("https://samu.example.org/o/{id}")]
    );
    std::fs::write(
        dir.join("no-config.toml"),
        "web = \"http://127.0.0.1:8490\"\n",
    )
    .unwrap();
    let out = run_samu("https://samu.example.org", &dir, &args, None);
    assert_eq!(
        command(&out),
        ["xdg-open", &format!("http://127.0.0.1:8490/o/{id}")]
    );
    // samu:// as some programs write it, and any case of the scheme.
    let out = run_samu(
        "https://samu.example.org",
        &dir,
        &[
            "open-uri",
            &format!("SAMU://{id}/"),
            "--print",
            "--format",
            "json",
        ],
        None,
    );
    assert_eq!(command(&out)[1], format!("http://127.0.0.1:8490/o/{id}"));
    for broken in ["samu:nope", "samu:"] {
        assert_eq!(
            code(&run_samu(
                "https://samu.example.org",
                &dir,
                &["open-uri", broken],
                None
            )),
            1,
            "{broken}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// ADR 0080 §6, ADR 0081 §5: `samu add` with `z:` and `x:`, `samu clock
/// add` with the time as after `z:`, and `samu report` over the last days,
/// asked of the server's clocks.
#[test]
fn report_shows_done_worked_and_booked_time() {
    let server = signed_in_server();
    let made = json_out(&server.samu(&["add", "Review Angebot z:1h x:", "--format", "json"], None));
    let review = made[0]["id"].as_str().unwrap().to_string();
    let steuer = json_out(&server.samu(&["add", "Steuer", "--format", "json"], None));
    let steuer = steuer[0]["id"].as_str().unwrap().to_string();
    let booked =
        json_out(&server.samu(&["clock", "add", &steuer, "30m", "--format", "json"], None));
    assert_eq!(booked[0]["seconds"], 1800);
    assert_eq!(booked[0]["owner"], steuer.as_str());
    // Neither a time nor --start/--end; and a time that is none.
    assert_ne!(code(&server.samu(&["clock", "add", &steuer], None)), 0);
    assert_ne!(
        code(&server.samu(&["clock", "add", &steuer, "bald"], None)),
        0
    );
    // Asked of the server: two clocks, an hour and half an hour.
    let clocks = json_out(&server.samu(&["clock", "list", "--format", "json"], None));
    let mut seconds: Vec<i64> = clocks
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["seconds"].as_i64().unwrap())
        .collect();
    seconds.sort_unstable();
    assert_eq!(seconds, [1800, 3600]);

    let report = json_out(&server.samu(&["report", "--days", "2", "--format", "json"], None));
    let rows = report.as_array().unwrap();
    let of = |part: &str| -> Vec<&serde_json::Value> {
        rows.iter().filter(|r| r["part"] == part).collect()
    };
    assert_eq!(of("done").len(), 1);
    assert_eq!(of("done")[0]["id"], review.as_str());
    let worked = of("worked");
    assert_eq!(worked.len(), 2);
    let review_row = worked.iter().find(|r| r["id"] == review.as_str()).unwrap();
    assert_eq!(review_row["reasons"], "clocked 1, completed 1");
    let total = of("time").into_iter().find(|r| r["total"] == true).unwrap();
    assert_eq!(total["seconds"], 5400);

    let only = json_out(&server.samu(
        &[
            "report",
            "--part",
            "done",
            "--q",
            "autor:agent",
            "--format",
            "json",
        ],
        None,
    ));
    assert!(only.as_array().unwrap().is_empty());
    // Today is the account's (Europe/Berlin), whatever this computer's
    // zone: at any instant one of these two is on another date than
    // Berlin, so before the fix one of them missed the completion.
    for tz in ["Etc/GMT+12", "Pacific/Kiritimati"] {
        let done = json_out(&server.samu_in_zone(
            tz,
            &[
                "report", "--days", "1", "--part", "done", "--format", "json",
            ],
        ));
        assert_eq!(done.as_array().unwrap().len(), 1, "TZ={tz}: {done}");
    }
    let tsv = stdout(&server.samu(&["report", "--part", "done", "--format", "tsv"], None));
    assert!(tsv.contains("Review Angebot"), "{tsv}");
    // A person's work is named by the account: the same as without a chip.
    let mine = json_out(&server.samu(
        &[
            "report",
            "--days",
            "2",
            "--q",
            "autor:privat",
            "--format",
            "json",
        ],
        None,
    ));
    assert_eq!(mine, report);
    assert_ne!(
        code(&server.samu(&["report", "--q", "autor:jemand"], None)),
        0
    );

    assert_eq!(
        code(&server.samu(&["settings", "set", "report-worked", "fields"], None)),
        0
    );
    let shown = json_out(&server.samu(&["settings", "--format", "json"], None));
    assert!(
        shown
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["key"] == "report-worked" && r["value"] == "fields")
    );
    assert_eq!(
        code(&server.samu(&["settings", "set", "report-worked", "alles"], None)),
        1
    );
}

/// The command line with a configuration file and only the environment
/// given, not the test server's `SAMU_SERVER` and `SAMU_ACCOUNT`.
fn run_with_config(
    dir: &Path,
    config: &str,
    env: &[(&str, &str)],
    args: &[&str],
    stdin: Option<&str>,
) -> Output {
    let path = dir.join("config.toml");
    std::fs::write(&path, config).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_samu"));
    command
        .args(args)
        .env_remove("SAMU_SERVER")
        .env_remove("SAMU_ACCOUNT")
        .env_remove("SAMU_PROFILE")
        .env_remove("SAMU_WEB")
        .env("SAMU_DEVICE", "test")
        .env("SAMU_STATE", dir.join("state"))
        .env("SAMU_CONFIG", &path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        command.env(key, value);
    }
    let mut child = command.spawn().unwrap();
    if let Some(text) = stdin {
        writeln!(child.stdin.take().unwrap(), "{text}").unwrap();
    } else {
        drop(child.stdin.take());
    }
    child.wait_with_output().unwrap()
}

impl Server {
    fn port(&self) -> u16 {
        self.url.rsplit(':').next().unwrap().parse().unwrap()
    }
}

/// A port nothing listens on: bound once, then let go.
fn dead_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn a_profile_gives_server_and_account_and_names_the_token() {
    let server = Server::start(true);
    let config = format!(
        "[profile.lokal]\naccount = \"privat\"\nport = {}\n",
        server.port()
    );
    let run = |args: &[&str], stdin| run_with_config(&server.dir, &config, &[], args, stdin);

    let login = run(
        &["--profile", "lokal", "login", "--password-stdin"],
        Some(PASSWORD),
    );
    assert_eq!(
        code(&login),
        0,
        "{}",
        String::from_utf8_lossy(&login.stderr)
    );
    assert!(server.dir.join("state/lokal.token").exists());
    assert!(
        !server.dir.join("state/privat.token").exists(),
        "the profile names the file, not the account (ADR 0086 §5)"
    );

    let session = run(&["--profile", "lokal", "session", "--format", "tsv"], None);
    assert_eq!(
        code(&session),
        0,
        "{}",
        String::from_utf8_lossy(&session.stderr)
    );
    assert!(
        stdout(&session).starts_with("privat\ttest\t"),
        "{}",
        stdout(&session)
    );

    // The same through the environment.
    let session = run_with_config(
        &server.dir,
        &config,
        &[("SAMU_PROFILE", "lokal")],
        &["session"],
        None,
    );
    assert_eq!(
        code(&session),
        0,
        "{}",
        String::from_utf8_lossy(&session.stderr)
    );
}

#[test]
fn the_default_profile_applies_only_when_no_server_is_named() {
    let server = Server::start(true);
    // The default points where nothing listens; naming the server, as an
    // agent does, must go around it (ADR 0067 §5).
    let config = format!(
        "default = \"weg\"\n[profile.weg]\naccount = \"privat\"\nport = {}\n",
        dead_port()
    );
    let named = [
        ("SAMU_SERVER", server.url.as_str()),
        ("SAMU_ACCOUNT", "privat"),
    ];
    let login = run_with_config(
        &server.dir,
        &config,
        &named,
        &["login", "--password-stdin"],
        Some(PASSWORD),
    );
    assert_eq!(
        code(&login),
        0,
        "{}",
        String::from_utf8_lossy(&login.stderr)
    );
    assert!(server.dir.join("state/privat.token").exists());

    let away = run_with_config(&server.dir, &config, &[], &["health"], None);
    assert_eq!(code(&away), 4, "the default profile's service is not there");
}

#[test]
fn a_profile_takes_over_the_token_of_the_same_server_and_account() {
    let server = Server::start(true);
    let plain = format!("server = \"{}\"\naccount = \"privat\"\n", server.url);
    let login = run_with_config(
        &server.dir,
        &plain,
        &[],
        &["login", "--password-stdin"],
        Some(PASSWORD),
    );
    assert_eq!(
        code(&login),
        0,
        "{}",
        String::from_utf8_lossy(&login.stderr)
    );

    let with_profile = format!(
        "{plain}default = \"p\"\n[profile.p]\naccount = \"privat\"\nport = {}\n",
        server.port()
    );
    let session = run_with_config(&server.dir, &with_profile, &[], &["session"], None);
    assert_eq!(
        code(&session),
        0,
        "{}",
        String::from_utf8_lossy(&session.stderr)
    );
    assert!(server.dir.join("state/p.token").exists());

    // Another server's token is not taken over.
    let other = format!(
        "{plain}[profile.q]\naccount = \"privat\"\nport = {}\n",
        dead_port()
    );
    let _ = run_with_config(
        &server.dir,
        &other,
        &[],
        &["--profile", "q", "session"],
        None,
    );
    assert!(!server.dir.join("state/q.token").exists());
}

#[test]
fn profile_list_says_what_runs_and_where_it_is_signed_in() {
    let server = Server::start(true);
    let config = format!(
        "default = \"lokal\"\n\
         [profile.lokal]\naccount = \"privat\"\nport = {}\n\
         [profile.home]\naccount = \"privat\"\nupstream = \"https://samu.example\"\nport = {}\n",
        server.port(),
        dead_port()
    );
    let login = run_with_config(
        &server.dir,
        &config,
        &[],
        &["login", "--password-stdin"],
        Some(PASSWORD),
    );
    assert_eq!(
        code(&login),
        0,
        "{}",
        String::from_utf8_lossy(&login.stderr)
    );

    let out = run_with_config(
        &server.dir,
        &config,
        &[],
        &["profile", "list", "--format", "json"],
        None,
    );
    assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let list: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
    let list = list.as_array().unwrap();
    assert_eq!(list.len(), 2);
    let lokal = list.iter().find(|p| p["name"] == "lokal").unwrap();
    assert_eq!(lokal["kind"], "local");
    assert_eq!(lokal["running"], true);
    assert_eq!(lokal["signed_in"], true);
    assert_eq!(lokal["default"], true);
    assert_eq!(
        lokal["web"],
        format!("http://lokal.samu.localhost:{}", server.port()),
        "its own name in the browser, without a front with the port (ADR 0087 §5)"
    );
    let home = list.iter().find(|p| p["name"] == "home").unwrap();
    assert_eq!(home["kind"], "online");
    assert_eq!(home["upstream"], "https://samu.example");
    assert_eq!(home["running"], false);
    assert_eq!(home["signed_in"], false);
}

/// ADR 0087 §5: a profile's `samu:` address goes through the front while
/// it answers, else straight to the profile's port; `SAMU_WEB` wins.
#[test]
fn a_profile_opens_through_the_front_while_it_answers() {
    let dir = std::env::temp_dir().join(format!("samu-cli-front-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let id = "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6c01";
    let uri = format!("samu:{id}");
    let args = [
        "--profile",
        "dienst",
        "open-uri",
        uri.as_str(),
        "--print",
        "--format",
        "json",
    ];
    let target = |config: &str, env: &[(&str, &str)]| -> String {
        let out = run_with_config(&dir, config, env, &args, None);
        assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
        let rows: serde_json::Value = serde_json::from_str(&stdout(&out)).unwrap();
        rows[0]["command"][1].as_str().unwrap().to_string()
    };
    let profile = "[profile.dienst]\naccount = \"work\"\nport = 8491\n";
    // A front that answers: the address has its port, here not 80.
    let front = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = front.local_addr().unwrap().port();
    assert_eq!(
        target(&format!("{profile}[front]\nport = {port}\n"), &[]),
        format!("http://dienst.samu.localhost:{port}/o/{id}")
    );
    // One that does not: straight to the profile.
    assert_eq!(
        target(&format!("{profile}[front]\nport = {}\n", dead_port()), &[]),
        format!("http://dienst.samu.localhost:8491/o/{id}")
    );
    assert_eq!(
        target(profile, &[("SAMU_WEB", "http://elsewhere.example")]),
        format!("http://elsewhere.example/o/{id}")
    );
    drop(front);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_unknown_profile_is_a_usage_error() {
    let server = Server::start(false);
    let out = run_with_config(&server.dir, "", &[], &["--profile", "nix", "health"], None);
    assert_eq!(code(&out), 2);
    assert!(String::from_utf8_lossy(&out.stderr).contains("nix"));
}

/// ADR 0090: a perspective is made, changed and shown through the CLI; the
/// server computes what it holds, inside the scope.
#[test]
fn a_perspective_shows_what_its_search_meets() {
    let server = signed_in_server();
    let ok = |out: Output| {
        assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
        out
    };
    ok(server.samu(&["tag", "add", "Büro"], None));
    ok(server.samu(&["add", "Angebot schreiben @Büro"], None));
    ok(server.samu(&["add", "Angebot lesen"], None));
    ok(server.samu(&["add", "Steuer @Büro"], None));
    ok(server.samu(
        &[
            "perspective",
            "add",
            "Im Büro",
            "@Büro",
            "angebot",
            "--icon",
            "🏢",
            "--group",
            "none",
            "--sort",
            "title",
        ],
        None,
    ));
    let titles = |name: &str| -> Vec<String> {
        let shown = json_out(&ok(
            server.samu(&["list", "--perspective", name, "--format", "json"], None)
        ));
        let mut titles: Vec<String> = shown
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["title"].as_str().unwrap().to_owned())
            .collect();
        titles.sort();
        titles
    };
    // A probe that must find something: both tasks with the tag.
    let list = json_out(&ok(
        server.samu(&["perspective", "list", "--format", "json"], None)
    ));
    assert_eq!(list[0]["title"], "Im Büro");
    assert_eq!(list[0]["search"], "@Büro angebot");
    assert_eq!(list[0]["arrange"], "none · title ↑");
    assert_eq!(list[0]["perspective"]["icon"], "🏢");
    assert_eq!(titles("im büro"), ["Angebot schreiben"]);

    // Only the chips: both tasks of the office.
    ok(server.samu(&["perspective", "text", "Im Büro", "--clear"], None));
    assert_eq!(titles("Im Büro"), ["Angebot schreiben", "Steuer"]);
    // Only the free text: both offers.
    ok(server.samu(&["perspective", "search", "Im Büro", "angebot"], None));
    assert_eq!(titles("Im Büro"), ["Angebot lesen", "Angebot schreiben"]);
    // A chip after what is closed lifts the filter (ADR 0042 §4).
    ok(server.samu(&["perspective", "chips", "Im Büro", "ist:erledigt"], None));
    assert!(titles("Im Büro").is_empty());

    // Free text where chips go, and words the server does not know, are
    // refused; a perspective that is not there is not found.
    assert_ne!(
        code(&server.samu(&["perspective", "chips", "Im Büro", "angebot"], None)),
        0
    );
    assert_ne!(
        code(&server.samu(&["perspective", "filter", "Im Büro", "irgendwas"], None)),
        0
    );
    // ADR 0054 §8: a property chip in the search, a wrong one refused.
    ok(server.samu(
        &["perspective", "search", "Im Büro", "prop:jira angebot"],
        None,
    ));
    let list = json_out(&ok(
        server.samu(&["perspective", "list", "--format", "json"], None)
    ));
    assert_eq!(list[0]["search"], "prop:jira angebot");
    assert!(titles("Im Büro").is_empty());
    assert_ne!(
        code(&server.samu(&["perspective", "search", "Im Büro", "prop:1jira"], None)),
        0
    );
    assert_eq!(
        code(&server.samu(&["list", "--perspective", "Nirgends"], None)),
        3
    );
    ok(server.samu(&["perspective", "add", "Zweite"], None));
    ok(server.samu(&["perspective", "move", "Zweite", "1"], None));
    let list = json_out(&ok(
        server.samu(&["perspective", "list", "--format", "json"], None)
    ));
    assert_eq!(list[0]["title"], "Zweite");
    ok(server.samu(&["perspective", "delete", "Zweite"], None));
    let list = json_out(&ok(
        server.samu(&["perspective", "list", "--format", "json"], None)
    ));
    assert_eq!(list.as_array().unwrap().len(), 1);
}

/// ADR 0116 §7: a block perspective is made of other perspectives and
/// fixed views, `samu list --perspective` shows it block by block from
/// `/blocks`, a task once unless `dedupe` is off.
#[test]
fn a_block_perspective_lists_block_by_block() {
    let server = signed_in_server();
    let ok = |out: Output| {
        assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
        out
    };
    ok(server.samu(&["tag", "add", "Büro"], None));
    ok(server.samu(&["add", "Angebot schreiben @Büro"], None));
    ok(server.samu(&["add", "Steuer @Büro"], None));
    ok(server.samu(&["add", "Milch"], None));
    ok(server.samu(&["perspective", "add", "Büro", "@Büro"], None));
    ok(server.samu(
        &[
            "perspective",
            "add",
            "Morgen",
            "--icon",
            "🌅",
            "--blocks",
            "Büro=Kurz im Büro",
            ":inbox",
        ],
        None,
    ));
    let sections = |name: &str| -> Vec<(String, Vec<String>)> {
        let shown = json_out(&ok(
            server.samu(&["list", "--perspective", name, "--format", "json"], None)
        ));
        shown
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                let mut titles: Vec<String> = s["tasks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|t| t["title"].as_str().unwrap().to_owned())
                    .collect();
                titles.sort();
                (s["title"].as_str().unwrap().to_owned(), titles)
            })
            .collect()
    };
    let office = vec!["Angebot schreiben".to_owned(), "Steuer".to_owned()];
    assert_eq!(
        sections("Morgen"),
        [
            ("Kurz im Büro".to_owned(), office.clone()),
            ("inbox".to_owned(), vec!["Milch".to_owned()]),
        ]
    );
    let list = json_out(&ok(
        server.samu(&["perspective", "list", "--format", "json"], None)
    ));
    assert_eq!(list[1]["blocks"], "Büro=Kurz im Büro, :inbox");

    // Without dedupe the inbox shows the office's tasks again, as in org.
    ok(server.samu(&["perspective", "dedupe", "Morgen", "off"], None));
    assert_eq!(sections("Morgen")[1].1.len(), 3);

    // Plain: a heading before each block; tsv: the block first.
    let plain = ok(server.samu(&["list", "--perspective", "Morgen"], None));
    let plain = String::from_utf8_lossy(&plain.stdout);
    assert!(plain.contains("▸ Kurz im Büro (2)"), "{plain}");
    assert!(plain.contains("▸ inbox (3)"), "{plain}");
    let tsv = ok(server.samu(
        &["list", "--perspective", "Morgen", "--format", "tsv"],
        None,
    ));
    let tsv = String::from_utf8_lossy(&tsv.stdout);
    assert_eq!(tsv.lines().count(), 5);
    assert!(
        tsv.lines()
            .all(|l| l.starts_with("Kurz im Büro\t") || l.starts_with("inbox\t"))
    );

    // A plain perspective lists as before, without sections.
    let plain = json_out(&ok(
        server.samu(&["list", "--perspective", "Büro", "--format", "json"], None)
    ));
    assert_eq!(plain.as_array().unwrap().len(), office.len());
    assert!(plain[0]["tasks"].is_null());

    // What only a plain perspective has is refused on a block one; a block
    // of itself, of a block perspective, an unknown view, and blocks on a
    // perspective that is a block are refused.
    for refused in [
        vec!["perspective", "text", "Morgen", "angebot"],
        vec!["perspective", "filter", "Morgen", "all"],
        vec!["perspective", "blocks", "Morgen", "Morgen"],
        vec!["perspective", "blocks", "Morgen", ":forecast"],
        vec!["perspective", "blocks", "Morgen", "Büro="],
        vec!["perspective", "blocks", "Büro", ":today"],
        vec!["perspective", "dedupe", "Morgen", "vielleicht"],
        vec!["list", "--perspective", "Morgen", "--today"],
    ] {
        assert_eq!(code(&server.samu(&refused, None)), 1, "{refused:?}");
    }
    assert_eq!(
        code(&server.samu(&["perspective", "blocks", "Morgen", "Nirgends"], None)),
        3
    );
    // Its blocks printed, then cleared: a plain perspective again.
    let blocks = json_out(&ok(server.samu(
        &["perspective", "blocks", "Morgen", "--format", "json"],
        None,
    )));
    assert_eq!(blocks[1]["block"], ":inbox");
    ok(server.samu(&["perspective", "blocks", "Morgen", "--clear"], None));
    let list = json_out(&ok(
        server.samu(&["perspective", "list", "--format", "json"], None)
    ));
    assert_eq!(list[1]["blocks"], "");
}

/// ae5a3206: `samu block` lets a task wait for another, the server's
/// availability says so, and `samu unblock` takes it back. Itself, twice
/// and a circle are refused.
#[test]
fn block_and_unblock() {
    let server = signed_in_server();
    let id = |title: &str| {
        let out = json_out(&server.samu(&["add", title, "--format", "json"], None));
        out[0]["id"].as_str().unwrap().to_string()
    };
    let (dach, gerüst) = (id("Dach decken"), id("Gerüst stellen"));
    json_out(&server.samu(&["block", &dach, &gerüst, "--format", "json"], None));

    let listed = json_out(&server.samu(&["blockers", &dach, "--format", "json"], None));
    assert_eq!(listed[0]["id"], gerüst.as_str());
    assert_eq!(listed[0]["title"], "Gerüst stellen");
    assert_eq!(listed[0]["open"], true);
    let shown = json_out(&server.samu(&["list", "--all", "--format", "json"], None));
    let row = shown
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == dach.as_str())
        .expect("the task is listed");
    assert_eq!(row["available"], false, "{row}");

    assert_eq!(code(&server.samu(&["block", &dach, &gerüst], None)), 1);
    assert_eq!(code(&server.samu(&["block", &dach, &dach], None)), 1);
    assert_eq!(code(&server.samu(&["block", &gerüst, &dach], None)), 1);

    assert_eq!(code(&server.samu(&["unblock", &dach, &gerüst], None)), 0);
    let listed = json_out(&server.samu(&["blockers", &dach, "--format", "json"], None));
    assert_eq!(listed, serde_json::json!([]));
    assert_eq!(code(&server.samu(&["unblock", &dach, &gerüst], None)), 3);
}

/// ADR 0091: a habit is captured with `habit:`, entered as done, failed
/// and paused through the CLI; the server computes the streak.
#[test]
fn a_habit_is_entered_and_counted() {
    let server = signed_in_server();
    let ok = |out: Output| {
        assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
        out
    };
    let made = json_out(&ok(
        server.samu(&["add", "Wasser habit:3/d:Glas", "--format", "json"], None)
    ));
    let id = made[0]["id"].as_str().unwrap().to_owned();
    ok(server.samu(&["add", "Kein Habit"], None));

    // A probe that must find something: the habit, and only it.
    let list = json_out(&ok(
        server.samu(&["habit", "list", "--format", "json"], None)
    ));
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!(list[0]["title"], "Wasser");
    assert_eq!(list[0]["progress"], "0/3 Glas today");

    // Two taps, then the third reaches the goal; yesterday entered after.
    ok(server.samu(&["habit", "done", &id, "--amount", "2"], None));
    let shown = json_out(&ok(
        server.samu(&["habit", "show", &id, "--format", "json"], None)
    ));
    assert_eq!(shown[0]["today"], "partial");
    ok(server.samu(&["habit", "done", &id], None));
    ok(server.samu(
        &["habit", "done", &id, "--day", "gestern", "--amount", "3"],
        None,
    ));
    let shown = json_out(&ok(
        server.samu(&["habit", "show", &id, "--format", "json"], None)
    ));
    assert_eq!(shown[0]["today"], "done");
    assert_eq!(shown[0]["streak"], 2);
    assert!(shown[0]["marks"].as_str().unwrap().ends_with("##"));

    // A day in the future is refused before anything is sent.
    assert_ne!(
        code(&server.samu(&["habit", "done", &id, "--day", "2999-01-01"], None)),
        0
    );

    // The entries, and one taken back: today is partial again.
    let entries = json_out(&ok(
        server.samu(&["habit", "entries", &id, "--format", "json"], None)
    ));
    assert_eq!(entries.as_array().unwrap().len(), 3);
    // One day is the account's today, whatever this computer's zone: one
    // of these two is on another date than Berlin at any instant, so
    // before the fix it got yesterday's entry too or none at all.
    for tz in ["Etc/GMT+12", "Pacific/Kiritimati"] {
        let today = json_out(&ok(server.samu_in_zone(
            tz,
            &["habit", "entries", &id, "--days", "1", "--format", "json"],
        )));
        assert_eq!(today.as_array().unwrap().len(), 2, "TZ={tz}: {today}");
    }
    let last_tap = entries
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["amount"].is_null())
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    ok(server.samu(&["habit", "undo", &last_tap], None));
    let shown = json_out(&ok(
        server.samu(&["habit", "show", &id, "--format", "json"], None)
    ));
    assert_eq!(shown[0]["today"], "partial");

    // To avoid: a relapse today ends the run.
    ok(server.samu(&["habit", "set", &id, "avoid"], None));
    ok(server.samu(&["habit", "fail", &id], None));
    let shown = json_out(&ok(
        server.samu(&["habit", "show", &id, "--format", "json"], None)
    ));
    assert_eq!(shown[0]["today"], "fail");
    assert_eq!(shown[0]["progress"], "free 0 d");

    // A task that is no habit is not found; `none` makes a habit a task.
    let plain = json_out(&ok(server.samu(&["list", "--format", "json"], None)));
    let plain = plain
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["title"] == "Kein Habit")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(code(&server.samu(&["habit", "show", &plain], None)), 3);
    assert_ne!(code(&server.samu(&["habit", "set", &id, "oft"], None)), 0);
    ok(server.samu(&["habit", "set", &id, "none"], None));
    assert_eq!(code(&server.samu(&["habit", "show", &id], None)), 3);
}

/// ADR 0099 §7: `samu demo` fills an empty account; once is enough, the
/// second time is exit 1. Checked in the account's folders, not only in
/// the counts.
#[test]
fn demo_data_into_the_account() {
    let server = signed_in_server();
    let out = json_out(&server.samu(&["demo", "en", "--format", "json"], None));
    let tasks = out
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["kind"] == "tasks")
        .unwrap();
    assert_eq!(tasks["count"], 81);
    let projects = json_out(&server.samu(&["list", "--all", "--format", "json"], None));
    assert!(projects.as_array().unwrap().len() >= 81);

    // Now with content: under Demo, then a conflict.
    let again = server.samu(&["demo", "de"], None);
    assert_eq!(
        code(&again),
        0,
        "{}",
        String::from_utf8_lossy(&again.stderr)
    );
    assert!(String::from_utf8_lossy(&again.stderr).contains("under the folder"));
    let third = server.samu(&["demo", "de"], None);
    assert_eq!(
        code(&third),
        1,
        "{}",
        String::from_utf8_lossy(&third.stderr)
    );
    assert_eq!(
        code(&server.samu(&["demo", "fr"], None)),
        2,
        "clap refuses another language"
    );
}

/// ADR 0115 §8: `samu list --columns` shows the table's columns, an action
/// group's estimate and worked time count its subtree, and a table
/// perspective brings its own columns.
#[test]
fn list_shows_columns_and_a_table_perspective_its_own() {
    let server = signed_in_server();
    let ok = |out: Output| {
        assert_eq!(code(&out), 0, "{}", String::from_utf8_lossy(&out.stderr));
        out
    };
    let id_of = |out: Output| json_out(&ok(out))[0]["id"].as_str().unwrap().to_string();
    let group = id_of(server.samu(&["add", "Steuer ~1h", "--format", "json"], None));
    let child = id_of(server.samu(
        &["add", "Belege ~30m", "--parent", &group, "--format", "json"],
        None,
    ));
    ok(server.samu(&["set", &child, "properties.jira", "ST-7"], None));
    ok(server.samu(&["clock", "add", &child, "1h"], None));
    let later = id_of(server.samu(&["add", "Abgabe", "--format", "json"], None));
    ok(server.samu(&["block", &later, &child], None));

    let rows = json_out(&ok(server.samu(
        &[
            "list",
            "--columns",
            "estimate,worked,remaining,prop:jira,blocked_by",
            "--format",
            "json",
        ],
        None,
    )));
    let row = |id: &str| {
        rows.as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == id)
            .unwrap()["columns"]
            .clone()
    };
    // The group counts its subtree: 1h + 30m estimated, 1h worked below.
    assert_eq!(row(&group)["estimate"], 5400);
    assert_eq!(row(&group)["worked"], 3600);
    assert_eq!(row(&group)["remaining"], 1800);
    assert_eq!(row(&group)["prop:jira"], serde_json::Value::Null);
    assert_eq!(row(&group)["blocked_by"], serde_json::Value::Null);
    assert_eq!(
        row(&later)["blocked_by"],
        serde_json::json!([format!("samu:{child}")])
    );
    // Beyond the estimate: the rest is negative.
    assert_eq!(row(&child)["remaining"], -1800);
    assert_eq!(row(&child)["prop:jira"], "ST-7");

    // plain: a head, then short, the columns and the title.
    let plain = stdout(&ok(
        server.samu(&["list", "--columns", "worked,prop:jira"], None)
    ));
    let lines: Vec<&str> = plain.lines().collect();
    assert!(lines[0].starts_with("id"), "{plain}");
    assert!(lines[0].ends_with("title"), "{plain}");
    assert!(
        lines
            .iter()
            .any(|l| l.contains("1:00") && l.contains("ST-7") && l.ends_with("Belege")),
        "{plain}"
    );
    // tsv: no head, every column in place, empty ones too.
    let tsv = stdout(&ok(server.samu(
        &["list", "--columns", "priority,worked", "--format", "tsv"],
        None,
    )));
    assert!(
        tsv.lines().any(|l| l.ends_with("\t\t1:00\tBelege")),
        "{tsv}"
    );
    // Without a value: the default columns.
    let rows = json_out(&ok(
        server.samu(&["list", "--columns", "--format", "json"], None)
    ));
    let keys: Vec<&String> = rows[0]["columns"].as_object().unwrap().keys().collect();
    assert_eq!(keys, ["due", "estimate", "priority", "state", "worked"]);
    // Unknown and doubled columns are refused.
    assert_eq!(code(&server.samu(&["list", "--columns", "ist"], None)), 1);
    assert_eq!(
        code(&server.samu(&["list", "--columns", "due,due"], None)),
        1
    );

    // A table perspective: its columns on the server, used by list.
    ok(server.samu(
        &[
            "perspective",
            "add",
            "Zeit",
            "Belege",
            "--table",
            "--columns",
            "worked",
        ],
        None,
    ));
    let rows = json_out(&ok(
        server.samu(&["list", "--perspective", "Zeit", "--format", "json"], None)
    ));
    let keys: Vec<&String> = rows[0]["columns"]
        .as_object()
        .unwrap_or_else(|| panic!("{rows}"))
        .keys()
        .collect();
    assert_eq!(keys, ["worked"]);
    ok(server.samu(
        &[
            "perspective",
            "view",
            "Zeit",
            "table",
            "--columns",
            "estimate,prop:jira",
        ],
        None,
    ));
    let list = json_out(&ok(
        server.samu(&["perspective", "list", "--format", "json"], None)
    ));
    assert_eq!(list[0]["view"], "table");
    assert_eq!(list[0]["columns"], "estimate,prop:jira");
    assert_eq!(
        list[0]["perspective"]["table"]["columns"],
        serde_json::json!(["estimate", "prop:jira"])
    );
    // As a list it keeps its columns, and list shows no table.
    ok(server.samu(&["perspective", "view", "Zeit", "list"], None));
    let rows = json_out(&ok(
        server.samu(&["list", "--perspective", "Zeit", "--format", "json"], None)
    ));
    assert!(rows[0].get("columns").is_none());
    let list = json_out(&ok(
        server.samu(&["perspective", "list", "--format", "json"], None)
    ));
    assert_eq!(
        list[0]["perspective"]["table"]["columns"],
        serde_json::json!(["estimate", "prop:jira"])
    );
    assert_ne!(
        code(&server.samu(
            &["perspective", "view", "Zeit", "table", "--columns", "x"],
            None
        )),
        0
    );
}
