// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The phone's side against a real `samu-server`, started from its binary
//! with a data directory of its own and stopped by its PID, never by name:
//! a kill by name would hit every samu-server on the machine. What the
//! server holds is asked of the server, not taken from the app's answers.

#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;

use samu_client::Client;
use samu_mobile::{MobileError, Samu};
use serde_json::{Value, json};

const PASSWORD: &str = "correct horse";

/// A number unique in this test process: tests run as threads of one
/// process, and the clock can give two of them the same directory.
fn unique() -> u32 {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// `samu-server` in the target directory: `cargo test --workspace` builds
/// it. This test runs from below it, from `deps/` or, with newer cargo,
/// from `build/<package>/<hash>/`; the first directory above that holds
/// the binary is the target directory.
fn server_binary() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.ancestors()
        .skip(1)
        .map(|dir| dir.join("samu-server"))
        .find(|path| path.is_file())
        .unwrap_or_else(|| {
            panic!(
                "samu-server is missing above {}: run `cargo build -p samu-server` (make check does)",
                exe.display()
            )
        })
}

struct Server {
    child: Child,
    url: String,
    dir: PathBuf,
}

impl Server {
    fn start() -> Self {
        Self::start_with_env(&[])
    }

    /// The same, with settings from the environment.
    fn start_with_env(env: &[(&str, &str)]) -> Self {
        let dir =
            std::env::temp_dir().join(format!("samu-mobile-{}-{}", std::process::id(), unique()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut add = Command::new(server_binary())
            .arg("--data")
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
        let mut child = Command::new(server_binary())
            .arg("--data")
            .arg(dir.join("data"))
            .args(["serve", "--listen", "127.0.0.1:0"])
            // 32 bytes of 3: seals calendar sources (ADR 0047 §3).
            .env(
                "SAMU_SECRET_KEY",
                "AwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwM=",
            )
            .envs(env.iter().copied())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
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

    /// A second session, as the web UI or the CLI would have it.
    fn other_client(&self) -> Client {
        let answer = Client::new(&self.url, None)
            .unwrap()
            .call(
                "POST",
                "/login",
                Some(&json!({"account": "privat", "password": PASSWORD, "device": "laptop"})),
            )
            .unwrap()
            .json()
            .unwrap();
        Client::new(
            &self.url,
            Some(answer["token"].as_str().unwrap().to_string()),
        )
        .unwrap()
    }

    fn stop(&mut self) {
        // By its PID, never by name: catastr's real daemons run here.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn phone(server: &Server) -> Arc<Samu> {
    let samu = Samu::open(server.dir.join("phone").display().to_string()).unwrap();
    samu.login(
        server.url.clone(),
        "privat".into(),
        PASSWORD.into(),
        "pixel".into(),
    )
    .unwrap();
    samu
}

/// Makes a task the way another client would, stamped by the server.
fn make_task(client: &Client, title: &str, extra: Value) -> String {
    let id = samu_api::Id::new().to_string();
    let mut value = json!({"id": id, "created": "2026-11-08T08:00:00Z",
        "modified": "2026-11-08T08:00:00Z", "title": title, "flagged": false,
        "sequential": false, "complete_with_last_child": false, "position": 0});
    value
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let change = json!([{"op": "create", "object": id, "kind": "task", "value": value,
                         "hlc": {"wall": 0, "counter": 0, "device": "x"}}]);
    let outcomes = client
        .call("POST", "/changes?stamp=server", Some(&change))
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(outcomes[0]["outcome"], "applied", "{outcomes}");
    id
}

/// Makes a tag the way another client would.
fn make_tag(client: &Client, title: &str, parent: Option<&str>) -> String {
    let id = samu_api::Id::new().to_string();
    let mut value = json!({"id": id, "created": "2026-11-08T08:00:00Z",
        "modified": "2026-11-08T08:00:00Z", "title": title, "status": "active",
        "exclusive": false, "position": 0});
    if let Some(parent) = parent {
        value["parent"] = json!(parent);
    }
    let change = json!([{"op": "create", "object": id, "kind": "tag", "value": value,
                         "hlc": {"wall": 0, "counter": 0, "device": "x"}}]);
    let outcomes = client
        .call("POST", "/changes?stamp=server", Some(&change))
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(outcomes[0]["outcome"], "applied", "{outcomes}");
    id
}

fn tagged(tag: &str) -> Value {
    json!({"tags": [{"tag": tag, "position": 0}]})
}

/// Today in the zone the phone computes in, and days from it.
fn day(samu: &Samu, days: i64) -> String {
    use jiff::ToSpan;
    let zone = jiff::tz::TimeZone::get(&samu.status().unwrap().zone).unwrap();
    let today = jiff::Timestamp::now().to_zoned(zone).date();
    today.checked_add(days.days()).unwrap().to_string()
}

fn titles(rows: &[samu_mobile::TaskRow]) -> Vec<&str> {
    rows.iter().map(|r| r.title.as_str()).collect()
}

fn stored(client: &Client, id: &str) -> Value {
    client
        .call("GET", &format!("/objects/{id}"), None)
        .unwrap()
        .json()
        .unwrap()["object"]
        .clone()
}

#[test]
fn sign_in_pull_and_the_zone_of_the_account() {
    let server = Server::start();
    let other = server.other_client();
    make_task(&other, "Milch kaufen", json!({}));
    let samu = phone(&server);
    let status = samu.status().unwrap();
    assert!(status.signed_in);
    assert!(status.zone_from_server, "{status:?}");
    assert_eq!(status.device.as_deref(), Some("pixel"));

    assert!(
        samu.inbox(None).unwrap().is_empty(),
        "nothing before the first sync"
    );
    let report = samu.sync().unwrap();
    // The task and the account's settings object (ADR 0045 §4).
    assert_eq!(report.pulled, 2);
    let inbox = samu.inbox(None).unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].rows[0].title, "Milch kaufen");
    assert!(inbox[0].rows[0].available && !inbox[0].rows[0].pending);
}

#[test]
fn completing_offline_waits_in_the_outbox_and_reaches_the_server() {
    let server = Server::start();
    let other = server.other_client();
    let id = make_task(&other, "Milch kaufen", json!({}));
    let samu = phone(&server);
    samu.sync().unwrap();

    samu.complete(id.clone()).unwrap();
    samu.set_flag(id.clone(), true).unwrap();
    assert_eq!(samu.outbox_count().unwrap(), 2);
    assert!(
        samu.available(None).unwrap().is_empty(),
        "the view shows it done"
    );
    let detail = samu.task(id.clone()).unwrap();
    assert!(detail.row.done && detail.row.pending && detail.row.flagged);
    // When it was captured (a1f43aed): the task's `created`.
    assert_eq!(
        detail.created_ms,
        stored(&other, &id)["created"]
            .as_str()
            .unwrap()
            .parse::<jiff::Timestamp>()
            .unwrap()
            .as_millisecond()
    );
    assert!(
        stored(&other, &id).get("completed_at").is_none(),
        "not on the server yet"
    );

    let report = samu.sync().unwrap();
    assert_eq!((report.pushed, report.rejected), (2, 0));
    assert_eq!(samu.outbox_count().unwrap(), 0);
    let object = stored(&other, &id);
    assert!(object["completed_at"].is_string(), "{object}");
    assert_eq!(object["flagged"], true);
    assert!(!samu.task(id).unwrap().row.pending);
}

#[test]
fn a_group_turns_sequential_offline_and_back() {
    let server = Server::start();
    let other = server.other_client();
    let group = make_task(&other, "Umzug", json!({}));
    let first = make_task(&other, "Kisten holen", json!({"parent": group}));
    let second = make_task(&other, "Kisten packen", json!({"parent": group}));
    let samu = phone(&server);
    samu.sync().unwrap();
    let row = |id: &String| samu.task(id.clone()).unwrap().row;
    assert!(row(&group).group && !row(&group).sequential);
    assert!(!row(&first).group, "a leaf is no group");
    assert!(row(&first).available && row(&second).available);

    samu.set_sequential(group.clone(), true).unwrap();
    assert!(row(&group).sequential && row(&group).pending);
    assert!(
        row(&first).available && !row(&second).available,
        "only the first child is available at once"
    );
    assert_eq!(stored(&other, &group)["sequential"], false, "not sent yet");
    let report = samu.sync().unwrap();
    assert_eq!((report.pushed, report.rejected), (1, 0));
    assert_eq!(stored(&other, &group)["sequential"], true);

    samu.set_sequential(group.clone(), false).unwrap();
    samu.sync().unwrap();
    assert_eq!(stored(&other, &group)["sequential"], false);
    assert!(row(&second).available);
}

#[test]
fn capture_offline_makes_tag_and_task_on_the_next_sync() {
    let server = Server::start();
    let other = server.other_client();
    let samu = phone(&server);
    samu.sync().unwrap();

    let preview = samu
        .preview("Batterien kaufen @einkauf d:2026-11-20 ! !b".into(), None)
        .unwrap();
    assert_eq!(preview.title, "Batterien kaufen");
    let kinds: Vec<&str> = preview.chips.iter().map(|c| c.kind.as_str()).collect();
    assert_eq!(kinds, ["new_tag", "flag", "priority", "due"]);
    assert_eq!(preview.chips[2].value, "B");
    assert_eq!(samu.outbox_count().unwrap(), 0, "a preview writes nothing");

    let id = samu
        .capture("Batterien kaufen @einkauf d:2026-11-20 ! !b".into(), None)
        .unwrap();
    assert_eq!(samu.outbox_count().unwrap(), 3, "tag, task, add");
    let row = &samu.inbox(None).unwrap()[0].rows[0];
    assert_eq!(row.title, "Batterien kaufen");
    assert_eq!(row.tags, ["einkauf"]);
    assert_eq!(row.due.as_deref(), Some("2026-11-20"));
    assert!(row.pending);

    samu.sync().unwrap();
    let object = stored(&other, &id);
    assert_eq!(object["title"], "Batterien kaufen");
    assert_eq!(object["due"], json!({"date": "2026-11-20"}));
    assert_eq!(object["priority"], "B", "ADR 0057");
    let tag_id = object["tags"][0]["tag"].as_str().unwrap();
    assert_eq!(stored(&other, tag_id)["title"], "einkauf");
}

#[test]
fn capture_chips_say_where_the_line_names_them() {
    let server = Server::start();
    let other = server.other_client();
    let gtd = make_project(&other, "GTD");
    make_project(&other, "Garten");
    let ort = make_tag(&other, "Ort", None);
    make_tag(&other, "Büro", Some(&ort));
    let samu = phone(&server);
    samu.sync().unwrap();

    // "Bür" before the marks: UTF-16 units differ from bytes.
    let text = "Bür #gtd @ort/büro @ort/neu d:2026-11-20 ! @Ort/Büro";
    let preview = samu.preview(text.into(), None).unwrap();
    let units: Vec<u16> = text.encode_utf16().collect();
    let said = |span: &samu_mobile::Span| {
        String::from_utf16(&units[span.start as usize..span.end as usize]).unwrap()
    };
    let chips: Vec<(&str, &str, Vec<String>)> = preview
        .chips
        .iter()
        .map(|c| {
            (
                c.kind.as_str(),
                c.value.as_str(),
                c.spans.iter().map(said).collect(),
            )
        })
        .collect();
    assert_eq!(
        chips,
        [
            ("project", "GTD", vec!["#gtd".to_owned()]),
            (
                "tag",
                "Ort/Büro",
                vec!["@ort/büro".to_owned(), "@Ort/Büro".to_owned()]
            ),
            ("new_tag", "Ort/neu", vec!["@ort/neu".to_owned()]),
            ("flag", "", vec!["!".to_owned()]),
            ("due", "2026-11-20", vec!["d:2026-11-20".to_owned()]),
        ]
    );

    // The project of the view is not in the line: nothing to take out.
    let preview = samu.preview("Bür".into(), Some(gtd)).unwrap();
    assert_eq!(preview.chips[0].kind, "project");
    assert!(preview.chips[0].spans.is_empty());
}

/// ADR 0073 §3–§5: `/name` in the line fills in from the template on the
/// phone too, offline, and the template's reminders come with the task.
#[test]
fn a_template_fills_in_offline_and_brings_its_reminders() {
    let server = Server::start();
    let other = server.other_client();
    let project = make_project(&other, "Gewohnheiten");
    let template = samu_api::Id::new().to_string();
    let change = json!([{"op": "create", "object": template, "kind": "template",
        "value": {"id": template, "created": "2026-11-08T08:00:00Z",
                  "modified": "2026-11-08T08:00:00Z", "title": "habit",
                  "text": "#Gewohnheiten @habit p:2026-11-20",
                  "reminders": [{"anchor": "planned"}], "position": 0},
        "hlc": {"wall": 0, "counter": 0, "device": "x"}}]);
    let outcomes = other
        .call("POST", "/changes?stamp=server", Some(&change))
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(outcomes[0]["outcome"], "applied", "{outcomes}");
    let samu = phone(&server);
    samu.sync().unwrap();
    let names: Vec<String> = samu
        .templates()
        .unwrap()
        .into_iter()
        .map(|t| t.title)
        .collect();
    assert_eq!(names, ["habit"]);

    let preview = samu.preview("Rauchfrei /habit".into(), None).unwrap();
    assert_eq!(preview.title, "Rauchfrei");
    assert_eq!(preview.project.as_deref(), Some("Gewohnheiten"));
    let kinds: Vec<&str> = preview.chips.iter().map(|c| c.kind.as_str()).collect();
    assert_eq!(kinds, ["template", "project", "new_tag", "planned"]);
    assert!(preview.warnings.is_empty());

    let unknown = samu.preview("Rauchfrei /nix".into(), None).unwrap();
    assert_eq!(unknown.title, "Rauchfrei /nix");
    assert_eq!(unknown.warnings[0].code, "unknown_template");

    let id = samu.capture("Rauchfrei /habit".into(), None).unwrap();
    assert_eq!(samu.outbox_count().unwrap(), 4, "tag, task, add, reminder");
    samu.sync().unwrap();
    let task = stored(&other, &id);
    assert_eq!(task["title"], "Rauchfrei");
    assert_eq!(task["project"], json!(project));
    assert_eq!(task["planned"], json!({"date": "2026-11-20"}));
    let page = other
        .call("GET", "/changes?after=0", None)
        .unwrap()
        .json()
        .unwrap();
    let reminders: Vec<(&str, &str)> = page["objects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| o["kind"] == "reminder")
        .map(|o| {
            (
                o["object"]["owner"].as_str().unwrap(),
                o["object"]["anchor"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(reminders, [(id.as_str(), "planned")]);
}

#[test]
fn capture_offers_projects_and_tags_from_the_first_sign() {
    let server = Server::start();
    let other = server.other_client();
    make_project(&other, "GTD");
    make_project(&other, "Garten");
    make_project(&other, "Umzug");
    make_project(&other, "Wohnung");
    let ort = make_tag(&other, "Ort", None);
    make_tag(&other, "Büro", Some(&ort));
    let samu = phone(&server);
    samu.sync().unwrap();
    let offer = |text: &str, cursor: usize| samu.capture_offer(text.into(), cursor as u32).unwrap();

    let all = offer("x #", 3).unwrap();
    assert_eq!(all.sign, "#");
    assert_eq!((all.start, all.end), (2, 3));
    assert_eq!(all.items, ["GTD", "Garten", "Umzug", "Wohnung"]);

    // Those that start with it first, then those that hold it.
    assert_eq!(
        offer("#g", 2).unwrap().items,
        ["GTD", "Garten", "Umzug", "Wohnung"]
    );
    assert_eq!(offer("#u", 2).unwrap().items, ["Umzug", "Wohnung"]);

    // The whole word, also behind the cursor; UTF-16 offsets.
    let tags = offer("Bür @o tag", 6).unwrap();
    assert_eq!((tags.start, tags.end, tags.sign.as_str()), (4, 6, "@"));
    assert_eq!(tags.items, ["Ort", "Ort/Büro"]);
    let mid = offer("@ortx", 2).unwrap();
    assert_eq!((mid.start, mid.end), (0, 5));
    assert_eq!(mid.items, ["Ort", "Ort/Büro"]);

    assert!(offer("#GTD", 4).is_none(), "typed already");
    assert!(offer("#zzz", 4).is_none(), "nothing fits");
    assert!(offer("x #g y", 6).is_none(), "cursor outside the word");
    assert!(offer("#\"Neues B", 9).is_none(), "quoted");
}

#[test]
fn a_lost_session_signs_out_and_keeps_the_outbox() {
    let server = Server::start();
    let other = server.other_client();
    let id = make_task(&other, "Milch kaufen", json!({}));
    let samu = phone(&server);
    samu.sync().unwrap();
    samu.complete(id).unwrap();

    // The session ends on the server (as after a password change).
    let mut setpass = Command::new(server_binary())
        .arg("--data")
        .arg(server.dir.join("data"))
        .args(["account", "passwd", "privat", "--password-stdin"])
        .stdin(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    writeln!(setpass.stdin.take().unwrap(), "{PASSWORD}").unwrap();
    assert!(setpass.wait().unwrap().success());

    assert!(matches!(samu.sync(), Err(MobileError::SignedOut { .. })));
    let status = samu.status().unwrap();
    assert!(!status.signed_in);
    assert_eq!(status.outbox, 1, "nothing lost");

    // Signing in again to the same account keeps it, and the next sync
    // delivers it.
    samu.login(
        server.url.clone(),
        "privat".into(),
        PASSWORD.into(),
        "pixel".into(),
    )
    .unwrap();
    assert_eq!(samu.outbox_count().unwrap(), 1);
    assert_eq!(samu.sync().unwrap().pushed, 1);
}

#[test]
fn another_account_is_refused_while_the_outbox_holds_changes() {
    let server = Server::start();
    let mut add = Command::new(server_binary())
        .arg("--data")
        .arg(server.dir.join("data"))
        .args(["account", "add", "dienst", "--password-stdin"])
        .stdin(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    writeln!(add.stdin.take().unwrap(), "{PASSWORD}").unwrap();
    assert!(add.wait().unwrap().success());
    let other = server.other_client();
    let id = make_task(&other, "Milch kaufen", json!({}));
    let samu = phone(&server);
    samu.sync().unwrap();
    samu.complete(id.clone()).unwrap();
    samu.logout().unwrap();

    // ADR 0107 §6: the outbox of privat is not thrown away for dienst.
    let refused = samu.login(
        server.url.clone(),
        "dienst".into(),
        PASSWORD.into(),
        "pixel".into(),
    );
    assert!(
        matches!(refused, Err(MobileError::Refused { .. })),
        "{refused:?}"
    );
    let status = samu.status().unwrap();
    assert_eq!(status.account.as_deref(), Some("privat"));
    assert_eq!(status.outbox, 1, "nothing lost");
    assert!(!status.signed_in);

    // Back to privat, the change goes out; then dienst is fine.
    samu.login(
        server.url.clone(),
        "privat".into(),
        PASSWORD.into(),
        "pixel".into(),
    )
    .unwrap();
    assert_eq!(samu.sync().unwrap().pushed, 1);
    assert!(stored(&other, &id)["completed_at"].is_string());
    samu.logout().unwrap();
    let status = samu
        .login(
            server.url.clone(),
            "dienst".into(),
            PASSWORD.into(),
            "pixel".into(),
        )
        .unwrap();
    assert_eq!(status.account.as_deref(), Some("dienst"));
    samu.sync().unwrap();
    assert!(
        samu.available(None).unwrap().is_empty(),
        "the mirror of privat is gone"
    );
}

#[test]
fn a_folder_that_never_signed_in_keeps_its_outbox_for_the_first_account() {
    // The app showed a new account's empty folder before its sign-in, and
    // something was captured there (ADR 0107, 15.11.2026).
    let server = Server::start();
    let samu = Samu::open(server.dir.join("fresh").display().to_string()).unwrap();
    samu.capture("Aus Versehen".into(), None).unwrap();
    assert_eq!(samu.outbox_count().unwrap(), 1);

    samu.login(
        server.url.clone(),
        "privat".into(),
        PASSWORD.into(),
        "pixel".into(),
    )
    .unwrap();
    assert_eq!(samu.outbox_count().unwrap(), 1, "nothing thrown away");
    assert_eq!(samu.sync().unwrap().pushed, 1);
}

#[test]
fn without_a_server_nothing_is_lost() {
    let mut server = Server::start();
    let other = server.other_client();
    let id = make_task(&other, "Milch kaufen", json!({}));
    let samu = phone(&server);
    samu.sync().unwrap();
    server.stop();

    samu.complete(id).unwrap();
    assert!(matches!(samu.sync(), Err(MobileError::Offline { .. })));
    let status = samu.status().unwrap();
    assert!(status.signed_in, "offline is not signed out");
    assert_eq!(status.outbox, 1);
    assert!(samu.available(None).unwrap().is_empty());
}

#[test]
fn a_refused_change_leaves_the_outbox_and_is_said() {
    let server = Server::start();
    let other = server.other_client();
    let id = make_task(&other, "Milch kaufen", json!({}));
    let samu = phone(&server);
    samu.sync().unwrap();
    samu.complete(id.clone()).unwrap();
    // Deleted elsewhere before the phone comes back.
    let delete = json!([{"op": "delete", "object": id,
                         "hlc": {"wall": 0, "counter": 0, "device": "x"}}]);
    other
        .call("POST", "/changes?stamp=server", Some(&delete))
        .unwrap();

    let report = samu.sync().unwrap();
    assert_eq!((report.pushed, report.rejected), (0, 1), "{report:?}");
    assert_eq!(samu.outbox_count().unwrap(), 0, "a refused change leaves");
    let said = samu.status().unwrap().last_error.unwrap_or_default();
    assert!(said.contains("is deleted"), "{said}");
    assert!(
        samu.inbox(None).unwrap().is_empty(),
        "the pull brings the deletion"
    );
}

/// ADR 0045: the phone's Forecast is samu-core's, sections in order, each
/// row with why it is there.
#[test]
fn forecast_by_day_with_the_overdue_first() {
    let server = Server::start();
    let other = server.other_client();
    let samu = phone(&server);
    let (today, soon, late) = (day(&samu, 0), day(&samu, 3), day(&samu, 10));
    make_task(&other, "überfällig", json!({"due": {"date": "2000-01-01"}}));
    make_task(&other, "heute", json!({"due": {"date": today}}));
    make_task(&other, "geplant", json!({"planned": {"date": soon}}));
    make_task(
        &other,
        "geplant früher, fällig bald",
        json!({"due": {"date": soon}, "planned": {"date": "2000-01-01"}}),
    );
    make_task(&other, "zu weit", json!({"due": {"date": late}}));
    make_task(&other, "ohne Termin", json!({}));
    make_task(
        &other,
        "erledigt",
        json!({"due": {"date": today}, "completed_at": "2026-11-08T09:00:00Z"}),
    );
    samu.sync().unwrap();

    let days = samu.forecast(None).unwrap();
    let kinds: Vec<(&str, Option<&str>)> = days
        .iter()
        .map(|d| (d.kind.as_str(), d.date.as_deref()))
        .collect();
    assert_eq!(
        kinds,
        [
            ("overdue", None),
            ("day", Some(today.as_str())),
            ("day", Some(soon.as_str())),
            ("later", None)
        ]
    );
    assert_eq!(titles(&days[0].rows), ["überfällig"]);
    // Planned in the past and not done stands today, as planned.
    let whys = |d: usize| -> Vec<(String, Option<String>)> {
        days[d]
            .rows
            .iter()
            .map(|r| (r.title.clone(), r.why.clone()))
            .collect()
    };
    assert_eq!(
        whys(1),
        [
            ("geplant früher, fällig bald".into(), Some("planned".into())),
            ("heute".into(), Some("due".into()))
        ]
    );
    let mut third = whys(2);
    third.sort_unstable();
    assert_eq!(
        third,
        [
            ("geplant".into(), Some("planned".into())),
            ("geplant früher, fällig bald".into(), Some("due".into()))
        ]
    );
    assert_eq!(titles(&days[3].rows), ["zu weit"]);
    let today_flags: Vec<bool> = days.iter().map(|d| d.today).collect();
    assert_eq!(today_flags, [false, true, false, false]);

    // The settings, offline: fewer days put the soon one under Später.
    assert!(samu.forecast_settings().unwrap().known);
    samu.set_forecast_setting("days".into(), "1".into())
        .unwrap();
    assert_eq!(samu.forecast_settings().unwrap().days, 1);
    let days = samu.forecast(None).unwrap();
    assert!(
        days.iter()
            .all(|d| d.date.as_deref() != Some(soon.as_str()))
    );
    assert!(matches!(
        samu.set_forecast_setting("days".into(), "0".into()),
        Err(MobileError::Input { .. })
    ));
    samu.sync().unwrap();
    assert_eq!(samu.outbox_count().unwrap(), 0, "the server took it");
}

#[test]
fn today_has_overdue_then_today_then_flagged_and_available() {
    let server = Server::start();
    let other = server.other_client();
    let samu = phone(&server);
    let (today, tomorrow) = (day(&samu, 0), day(&samu, 1));
    make_task(&other, "heute", json!({"due": {"date": today}}));
    make_task(&other, "überfällig", json!({"due": {"date": "2000-01-01"}}));
    make_task(&other, "morgen", json!({"due": {"date": tomorrow}}));
    make_task(&other, "markiert", json!({"flagged": true}));
    make_task(
        &other,
        "markiert und heute",
        json!({"flagged": true, "planned": {"date": today}}),
    );
    make_task(
        &other,
        "markiert, aber zurückgestellt",
        json!({"flagged": true, "defer": {"date": "2999-01-01"}}),
    );
    let done = make_task(&other, "gleich erledigt", json!({"due": {"date": today}}));
    make_task(&other, "ohne alles", json!({}));
    samu.sync().unwrap();

    let rows = samu.today().unwrap();
    assert_eq!(
        titles(&rows),
        [
            "überfällig",
            // Today's due ones before today's planned one (ADR 0045).
            "heute",
            "gleich erledigt",
            "markiert und heute",
            "markiert"
        ]
    );

    samu.complete(done).unwrap();
    assert_eq!(
        titles(&samu.today().unwrap()),
        ["überfällig", "heute", "markiert und heute", "markiert"],
        "done in the outbox leaves at once"
    );
}

#[test]
fn glance_counts_the_inbox_and_how_far_today_is() {
    let server = Server::start();
    let other = server.other_client();
    let samu = phone(&server);
    let today = day(&samu, 0);
    make_task(&other, "überfällig", json!({"due": {"date": "2000-01-01"}}));
    make_task(&other, "heute", json!({"due": {"date": today}}));
    let done = make_task(&other, "gleich erledigt", json!({"due": {"date": today}}));
    make_task(&other, "markiert", json!({"flagged": true}));
    samu.sync().unwrap();

    let g = samu.glance().unwrap();
    assert_eq!((g.overdue, g.today_open, g.today_done), (1, 4, 0));
    assert_eq!(g.inbox, 4, "all four sit in the inbox");

    samu.complete(done).unwrap();
    let g = samu.glance().unwrap();
    assert_eq!((g.overdue, g.today_open, g.today_done), (1, 3, 1));
    assert_eq!(g.inbox, 3, "done leaves the inbox");
}

#[test]
fn search_takes_every_word_in_title_or_note() {
    let server = Server::start();
    let other = server.other_client();
    make_task(&other, "Milch kaufen", json!({"note": "beim BIOLADEN"}));
    make_task(&other, "Brot kaufen", json!({}));
    make_task(
        &other,
        "Milch erledigt",
        json!({"completed_at": "2026-11-08T09:00:00Z"}),
    );
    let samu = phone(&server);
    samu.sync().unwrap();

    let hits = |text: &str| -> Vec<String> {
        samu.search(text.into(), None)
            .unwrap()
            .iter()
            .flat_map(|s| s.rows.iter().map(|r| r.title.clone()))
            .collect()
    };
    assert_eq!(hits("kaufen").len(), 2, "the probe must find something");
    assert_eq!(hits("milch bioladen"), ["Milch kaufen"]);
    assert_eq!(hits("KAUFEN brot"), ["Brot kaufen"]);
    assert!(hits("milch käse").is_empty(), "every word, not any");
    assert!(hits("  ").is_empty(), "no words, no hits");
    // The search language (a1f43aed): `make_task` stamps them all
    // captured on 08.11.2026.
    assert_eq!(hits("erstellt:2026-11-08").len(), 2, "the open ones");
    assert_eq!(hits("milch erstellt:2026-11-08"), ["Milch kaufen"]);
    assert!(hits("erstellt:2026-11-07").is_empty());
    assert_eq!(hits("milch ist:erledigt"), ["Milch erledigt"]);
    assert!(hits("kaufen @fehlt").is_empty(), "no such tag: free text");
}

#[test]
fn the_search_field_offers_makes_chips_and_remembers() {
    let server = Server::start();
    let other = server.other_client();
    make_project(&other, "Garten");
    make_project(&other, "Mein Umzug");
    let ort = make_tag(&other, "Ort", None);
    make_tag(&other, "Büro", Some(&ort));
    make_task(&other, "Milch kaufen", json!({"flagged": true}));
    make_task(&other, "Brot kaufen", json!({}));
    let samu = phone(&server);
    samu.sync().unwrap();

    // The offers of samu-core, chips with their word.
    let labels = |word: &str| -> Vec<String> {
        samu.search_offers(word.into(), false)
            .unwrap()
            .into_iter()
            .map(|o| o.label)
            .collect()
    };
    assert_eq!(labels("#"), ["#Garten", "#Inbox", "#Mein Umzug"]);
    let due = samu.search_offers("fällig:h".into(), false).unwrap();
    assert_eq!(due.len(), 1);
    let chip = due[0].chip.clone().unwrap();
    assert_eq!(
        (chip.kind.as_str(), chip.value.as_str(), chip.word.as_str()),
        ("due", "heute", "fällig:heute")
    );
    let tag = samu.search_offers("-@bü".into(), false).unwrap();
    let tag = tag[0].chip.clone().unwrap();
    assert!(tag.not);
    assert_eq!(
        (tag.word.as_str(), tag.label.as_str()),
        ("-@Ort/Büro", "-@Ort/Büro")
    );
    let kinds = samu.search_offers("fä".into(), false).unwrap();
    assert_eq!(kinds[0].insert.as_deref(), Some("fällig:"));
    assert!(kinds[0].chip.is_none());
    assert!(
        !labels("ist:").iter().any(|l| l == "ist:termin"),
        "no appointments here"
    );

    // Only finished words become chips; the one being typed stays.
    let split = samu.search_split("ist:markiert kauf".into()).unwrap();
    assert_eq!(split.chips.len(), 1);
    assert_eq!(split.chips[0].word, "ist:markiert");
    assert_eq!(split.rest, "kauf");
    let split = samu
        .search_split("milch #\"Mein Umzug\" fällig:heute".into())
        .unwrap();
    assert_eq!(
        split
            .chips
            .iter()
            .map(|c| c.label.as_str())
            .collect::<Vec<_>>(),
        ["#Mein Umzug"]
    );
    assert_eq!(split.chips[0].word, "#\"Mein Umzug\"");
    assert_eq!(split.rest, "milch fällig:heute");
    let split = samu.search_split("milch ".into()).unwrap();
    assert!(split.chips.is_empty());
    assert_eq!(split.rest, "milch ");

    // Chip words and the rest search as the text did.
    let hits = |text: &str| -> Vec<String> {
        samu.search(text.into(), None)
            .unwrap()
            .iter()
            .flat_map(|s| s.rows.iter().map(|r| r.title.clone()))
            .collect()
    };
    assert_eq!(hits("kaufen").len(), 2, "the probe must find something");
    let flagged = samu
        .search_chip("is".into(), "markiert".into(), false)
        .unwrap();
    assert_eq!(hits(&format!("{} kaufen", flagged.word)), ["Milch kaufen"]);
    let unflagged = samu
        .search_chip("is".into(), "markiert".into(), true)
        .unwrap();
    assert_eq!(unflagged.word, "-ist:markiert");
    assert_eq!(hits(&format!("{} kaufen", unflagged.word)), ["Brot kaufen"]);

    // `oder` between two groups (ADR 0121 §4): a chip of its own, offered
    // once a chip stands, and the search finds what meets either group.
    let or_offer = samu.search_offers("od".into(), true).unwrap();
    assert_eq!(or_offer[0].insert.as_deref(), Some("oder "));
    assert!(samu.search_offers("od".into(), false).unwrap().is_empty());
    let split = samu
        .search_split("ist:markiert oder -ist:markiert \"oder\" ".into())
        .unwrap();
    assert_eq!(
        split
            .chips
            .iter()
            .map(|c| (c.kind.as_str(), c.word.as_str()))
            .collect::<Vec<_>>(),
        [
            ("is", "ist:markiert"),
            ("or", "oder"),
            ("is", "-ist:markiert")
        ]
    );
    assert_eq!(split.rest, "\"oder\" ");
    assert_eq!(hits("ist:markiert oder -ist:markiert kaufen").len(), 2);
    assert_eq!(
        hits("ist:markiert -ist:markiert kaufen").len(),
        0,
        "one group: both cannot hold"
    );
    assert!(
        samu.search_chip("is".into(), "quatsch".into(), false)
            .is_err()
    );
    assert!(samu.search_chip("nope".into(), "x".into(), false).is_err());

    // The last five, the newest first, each once, none empty.
    assert!(samu.recent_searches().unwrap().is_empty());
    for text in ["a", "b", "  ", "c", "a", "d", "e", "f"] {
        samu.remember_search(text.into()).unwrap();
    }
    assert_eq!(samu.recent_searches().unwrap(), ["f", "e", "d", "a", "c"]);
    samu.remember_search(" milch   kaufen ".into()).unwrap();
    assert_eq!(samu.recent_searches().unwrap()[0], "milch kaufen");
}

#[test]
fn the_app_sets_the_language_and_the_search_follows_it() {
    let server = Server::start();
    let other = server.other_client();
    make_task(&other, "Milch kaufen", json!({"flagged": true}));
    let samu = phone(&server);
    samu.sync().unwrap();

    // German until the app says otherwise.
    let title = |key: &str| {
        samu.bar_views()
            .into_iter()
            .find(|e| e.key == key)
            .map(|e| e.title)
            .unwrap()
    };
    assert_eq!(title("today"), "Heute");
    samu.set_lang("en-GB".into());
    assert_eq!(title("today"), "Today");

    // English offers and words; the chip stays German, and both search.
    let due = samu.search_offers("due:to".into(), false).unwrap();
    let chip = due[0].chip.clone().unwrap();
    assert_eq!(
        (chip.value.as_str(), chip.word.as_str()),
        ("heute", "due:today")
    );
    let kinds = samu.search_offers("du".into(), false).unwrap();
    assert_eq!(kinds[0].insert.as_deref(), Some("due:"));
    let flagged = samu
        .search_chip("is".into(), "markiert".into(), false)
        .unwrap();
    assert_eq!(flagged.word, "is:flagged");
    let hits = |text: &str| samu.search(text.into(), None).unwrap().len();
    assert_eq!(hits("is:flagged milch"), 1, "the probe must find something");
    assert_eq!(hits("ist:markiert milch"), 1);

    samu.set_lang("de-DE".into());
    assert_eq!(
        samu.search_chip("is".into(), "markiert".into(), false)
            .unwrap()
            .word,
        "ist:markiert"
    );
}

#[test]
fn a_tag_filters_with_the_tags_below_and_chips_count() {
    let server = Server::start();
    let other = server.other_client();
    let haus = make_tag(&other, "Haus", None);
    let garten = make_tag(&other, "Garten", Some(&haus));
    let buero = make_tag(&other, "Büro", None);
    make_tag(&other, "leer", None);
    make_task(&other, "Hecke schneiden", tagged(&garten));
    make_task(&other, "Dach prüfen", tagged(&haus));
    make_task(&other, "Ablage", tagged(&buero));
    let mut done = tagged(&buero);
    done["completed_at"] = json!("2026-11-08T09:00:00Z");
    make_task(&other, "erledigt", done);
    let samu = phone(&server);
    samu.sync().unwrap();

    let shown = |tag: &str| -> Vec<String> {
        let mut out: Vec<String> = samu
            .available(Some(tag.into()))
            .unwrap()
            .iter()
            .flat_map(|s| s.rows.iter().map(|r| r.title.clone()))
            .collect();
        out.sort_unstable();
        out
    };
    assert_eq!(shown(&haus), ["Dach prüfen", "Hecke schneiden"]);
    assert_eq!(shown(&garten), ["Hecke schneiden"]);
    assert_eq!(samu.available(None).unwrap()[0].rows.len(), 3);
    assert!(
        samu.forecast(Some(buero.clone()))
            .unwrap()
            .iter()
            .all(|d| d.rows.is_empty())
    );

    let chips: Vec<(String, u32)> = samu
        .tags()
        .unwrap()
        .into_iter()
        .map(|c| (c.path, c.count))
        .collect();
    assert_eq!(
        chips,
        [
            ("Büro".into(), 1),
            ("Haus".into(), 2),
            ("Haus/Garten".into(), 1)
        ],
        "a tag nothing open carries is no chip"
    );

    samu.capture("Rasen mähen @Haus/Garten".into(), None)
        .unwrap();
    samu.capture("Neues @neu".into(), None).unwrap();
    let chips = samu.tags().unwrap();
    let count = |path: &str| chips.iter().find(|c| c.path == path).map(|c| c.count);
    assert_eq!(count("Haus/Garten"), Some(2), "the outbox counts");
    assert_eq!(count("neu"), Some(1), "a tag made offline is a chip");
}

#[test]
fn a_shared_text_becomes_a_task_with_its_links_on_the_next_sync() {
    let server = Server::start();
    let other = server.other_client();
    let project = samu_api::Id::new().to_string();
    let change = json!([{"op": "create", "object": project, "kind": "project",
        "value": {"id": project, "created": "2026-11-08T08:00:00Z",
                  "modified": "2026-11-08T08:00:00Z", "title": "Haus",
                  "kind": "parallel", "status": "active", "flagged": false, "position": 0},
        "hlc": {"wall": 0, "counter": 0, "device": "x"}}]);
    let outcomes = other
        .call("POST", "/changes?stamp=server", Some(&change))
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(outcomes[0]["outcome"], "applied", "{outcomes}");
    let samu = phone(&server);
    samu.sync().unwrap();

    let draft = samu.share_draft(
        None,
        "Dachdecker anrufen #nicht-geparst\nAngebot: catastr:01936f5e-8a00-7c3e-9b1a-3f2d4c5b6f00\n\
         https://example.org/dach"
            .into(),
    );
    assert_eq!(
        draft.title, "Dachdecker anrufen #nicht-geparst",
        "the shared text is not parsed"
    );
    assert_eq!(draft.links.len(), 2);
    let id = samu
        .capture_shared(
            draft.title.clone(),
            "#Haus @neu morgen früh // selbst getippt".into(),
            draft.note.clone(),
            draft.links.clone(),
            None,
        )
        .unwrap();
    assert_eq!(samu.outbox_count().unwrap(), 5, "tag, task, add, two links");
    samu.sync().unwrap();
    assert_eq!(samu.outbox_count().unwrap(), 0);

    let task = stored(&other, &id);
    assert_eq!(
        task["title"],
        "Dachdecker anrufen #nicht-geparst morgen früh"
    );
    assert_eq!(task["project"], json!(project));
    let note = task["note"].as_str().unwrap();
    assert!(
        note.starts_with("selbst getippt\n\nAngebot: catastr:"),
        "{note}"
    );
    let tag = task["tags"][0]["tag"].as_str().unwrap();
    assert_eq!(stored(&other, tag)["title"], "neu");

    let page = other
        .call("GET", "/changes?after=0", None)
        .unwrap()
        .json()
        .unwrap();
    let mut links: Vec<(String, String, String)> = page["objects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| o["kind"] == "link")
        .map(|o| {
            let l = &o["object"];
            (
                l["owner"].as_str().unwrap().to_string(),
                l["uri"].as_str().unwrap().to_string(),
                l["kind"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    links.sort();
    assert_eq!(
        links,
        [
            (
                id.clone(),
                "catastr:01936f5e-8a00-7c3e-9b1a-3f2d4c5b6f00".into(),
                "ref".into()
            ),
            (id.clone(), "https://example.org/dach".into(), "ref".into()),
        ]
    );
    assert!(
        samu.capture_shared(" ".into(), String::new(), None, Vec::new(), None)
            .is_err(),
        "no title, nothing made"
    );
}

fn ms(at: &str) -> i64 {
    at.parse::<jiff::Timestamp>().unwrap().as_millisecond()
}

fn owners(notices: &[samu_mobile::Notice]) -> Vec<(&str, i64)> {
    notices
        .iter()
        .map(|n| (n.title.as_str(), n.at_ms))
        .collect()
}

/// ADR 0030 §5, §6: one alarm at a time, each reminder shown once, later
/// and tomorrow only on the phone, a reminder made here reaches the
/// server, and what is done stays quiet.
#[test]
fn reminders_go_off_once_and_snooze_on_the_phone() {
    let server = Server::start();
    let other = server.other_client();
    // In January Berlin is UTC+1: 10:00 there is 09:00Z.
    let id = make_task(
        &other,
        "Zahnarzt",
        json!({"due": {"at": "2030-01-10T10:00:00+01:00[Europe/Berlin]"}}),
    );
    make_task(&other, "Altes", json!({"due": {"date": "2030-01-08"}}));
    make_task(&other, "Neues", json!({"due": {"date": "2030-01-19"}}));
    let samu = phone(&server);
    samu.sync().unwrap();

    // The first time nothing from before is shown.
    let t0 = ms("2030-01-10T08:00:00Z");
    assert!(samu.fire(t0).unwrap().is_empty());
    assert_eq!(
        samu.next_alarm(t0).unwrap(),
        Some(ms("2030-01-10T09:00:00Z"))
    );

    let reminder = samu
        .add_reminder(id.clone(), samu_mobile::Anchor::Due, None, Some(30))
        .unwrap();
    assert_eq!(
        samu.next_alarm(t0).unwrap(),
        Some(ms("2030-01-10T08:30:00Z"))
    );
    let rows = samu.reminders(id.clone()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].before_minutes, Some(30));
    assert_eq!(rows[0].goes_off_ms, Some(ms("2030-01-10T08:30:00Z")));

    // Asked of the server.
    samu.sync().unwrap();
    let object = stored(&other, &reminder);
    assert_eq!(
        (object["anchor"].as_str(), object["before"].as_str()),
        (Some("due"), Some("PT30M"))
    );
    assert_eq!(object["owner"], id.as_str());

    let fired = samu.fire(ms("2030-01-10T08:31:00Z")).unwrap();
    assert_eq!(owners(&fired), [("Zahnarzt", ms("2030-01-10T08:30:00Z"))]);
    assert!(fired[0].is_task);
    assert!(
        samu.fire(ms("2030-01-10T08:32:00Z")).unwrap().is_empty(),
        "shown once"
    );

    let due = ms("2030-01-10T09:00:00Z");
    assert_eq!(samu.next_alarm(due - 60_000).unwrap(), Some(due));
    assert_eq!(owners(&samu.fire(due).unwrap()), [("Zahnarzt", due)]);

    // Later: an hour on.
    samu.snooze(id.clone(), 60, due).unwrap();
    let hour = ms("2030-01-10T10:00:00Z");
    assert_eq!(samu.next_alarm(due).unwrap(), Some(hour));
    assert_eq!(owners(&samu.fire(hour).unwrap()), [("Zahnarzt", hour)]);

    // Tomorrow: 09:00 in Berlin.
    samu.snooze_tomorrow(id.clone(), hour).unwrap();
    let tomorrow = ms("2030-01-11T08:00:00Z");
    assert_eq!(samu.next_alarm(hour).unwrap(), Some(tomorrow));
    assert_eq!(samu.outbox_count().unwrap(), 0, "snoozes are not synced");

    // Done before it goes off: quiet.
    samu.complete(id.clone()).unwrap();
    assert!(samu.fire(tomorrow).unwrap().is_empty());

    // A week away: only the last day comes back.
    let fired = samu.fire(ms("2030-01-20T07:00:00Z")).unwrap();
    assert_eq!(owners(&fired), [("Neues", ms("2030-01-19T08:00:00Z"))]);

    samu.delete_reminder(reminder.clone()).unwrap();
    assert!(samu.reminders(id).unwrap().is_empty());
    samu.sync().unwrap();
    let gone = other.call("GET", &format!("/objects/{reminder}"), None);
    assert!(gone.is_err(), "the server no longer has it");
}

/// A reminder on nothing, or at an instant without one, is refused here.
#[test]
fn a_reminder_needs_its_owner_and_its_instant() {
    let server = Server::start();
    let other = server.other_client();
    let id = make_task(&other, "Etwas", json!({}));
    let samu = phone(&server);
    samu.sync().unwrap();
    let nobody = samu_api::Id::new().to_string();
    assert!(matches!(
        samu.add_reminder(nobody, samu_mobile::Anchor::Due, None, None),
        Err(MobileError::Input { .. })
    ));
    assert!(matches!(
        samu.add_reminder(id.clone(), samu_mobile::Anchor::At, None, None),
        Err(MobileError::Input { .. })
    ));
    assert!(matches!(
        samu.add_reminder(id, samu_mobile::Anchor::Due, None, Some(-5)),
        Err(MobileError::Input { .. })
    ));
    assert_eq!(samu.outbox_count().unwrap(), 0);
}

/// Regression: the very first alarm showed nothing, because the first call
/// of `fire` started its window at its own now (08.11.2026: a
/// reminder for "tedt", the alarm went off, no notification).
#[test]
fn the_first_alarm_ever_shows_its_reminder() {
    let server = Server::start();
    let other = server.other_client();
    let id = make_task(&other, "tedt", json!({}));
    let samu = phone(&server);
    samu.sync().unwrap();

    let at = ms("2030-01-10T02:35:00Z");
    samu.add_reminder(id, samu_mobile::Anchor::At, Some(at), None)
        .unwrap();
    // The app sets the alarm, and it goes off a few milliseconds late.
    assert_eq!(
        samu.next_alarm(ms("2030-01-10T02:33:36Z")).unwrap(),
        Some(at)
    );
    let fired = samu.fire(at + 9).unwrap();
    assert_eq!(owners(&fired), [("tedt", at)]);
}

/// ADR 0031: the phone pairs with the URI from the QR code, once; it then
/// syncs like a phone signed in by password.
#[test]
fn a_phone_pairs_with_the_code_from_the_qr_code_once() {
    let server = Server::start();
    let other = server.other_client();
    make_task(&other, "Milch kaufen", json!({}));
    let made = other
        .call(
            "POST",
            "/pair/code",
            Some(&json!({"device": "pixel", "password": PASSWORD})),
        )
        .unwrap()
        .json()
        .unwrap();
    let uri = made["uri"].as_str().unwrap().to_string();

    let samu = Samu::open(server.dir.join("paired").display().to_string()).unwrap();
    assert!(matches!(
        samu.pair("https://example.org".into()),
        Err(MobileError::Input { .. })
    ));
    let status = samu.pair(uri.clone()).unwrap();
    assert!(status.signed_in && status.zone_from_server, "{status:?}");
    assert_eq!(status.device.as_deref(), Some("pixel"));
    assert_eq!(status.server.as_deref(), Some(server.url.as_str()));
    // A task and the settings object (ADR 0045 §4).
    assert_eq!(samu.sync().unwrap().pulled, 2);

    let again = Samu::open(server.dir.join("again").display().to_string()).unwrap();
    assert!(matches!(
        again.pair(uri),
        Err(MobileError::SignedOut { .. })
    ));
}

#[test]
fn a_ring_recording_from_tasker_waits_offline_and_lands_in_the_inbox() {
    let server = Server::start();
    let other = server.other_client();
    let samu = phone(&server);
    samu.sync().unwrap();

    // 2026-11-08T07:15:00Z
    let recorded = 1_794_122_100_000;
    let id = samu
        .capture_ring(
            "Dachrinne prüfen morgen #haus\nvor dem Regen".into(),
            Some(recorded),
        )
        .unwrap();
    let silent = samu.capture_ring("  ".into(), None).unwrap();
    assert_eq!(samu.outbox_count().unwrap(), 2, "one create each");
    assert!(
        matches!(
            other.call("GET", &format!("/objects/{id}"), None),
            Err(samu_client::Error::NotFound(_))
        ),
        "not on the server before the sync"
    );

    let report = samu.sync().unwrap();
    assert_eq!((report.pushed, report.rejected), (2, 0));
    let task = stored(&other, &id);
    assert_eq!(task["title"], "Dachrinne prüfen morgen #haus");
    assert_eq!(task["note"], "Dachrinne prüfen morgen #haus\nvor dem Regen");
    assert_eq!(task["created"], "2026-11-08T07:15:00Z");
    assert!(task.get("project").is_none(), "into the inbox");
    assert!(
        task.get("due").is_none() && task.get("tags").is_none(),
        "not parsed"
    );
    assert_eq!(
        task["properties"],
        json!({"source": "index-ring", "via": "tasker"})
    );
    let silent = stored(&other, &silent);
    assert!(
        silent["title"]
            .as_str()
            .unwrap()
            .starts_with("Aufnahme vom Ring, "),
        "{silent}"
    );
    assert!(silent.get("note").is_none());

    samu.logout().unwrap();
    assert!(
        matches!(
            samu.capture_ring("nach dem Abmelden".into(), None),
            Err(MobileError::SignedOut { .. })
        ),
        "signed out, nothing filed"
    );
    assert_eq!(samu.outbox_count().unwrap(), 0);
}

/// ADR 0035 on the phone: a scope made offline narrows every list, the
/// projects and the widget at once; what is due outside is counted; the
/// choice stays on the phone; the scope reaches the server on the next
/// sync with its members, and one deleted elsewhere counts as none.
#[test]
fn a_scope_made_offline_narrows_the_lists_and_reaches_the_server() {
    let server = Server::start();
    let other = server.other_client();
    let create = |kind: &str, value: Value| {
        let id = value["id"].as_str().unwrap().to_owned();
        let change = json!([{"op": "create", "object": id, "kind": kind, "value": value,
                             "hlc": {"wall": 0, "counter": 0, "device": "x"}}]);
        let outcomes = other
            .call("POST", "/changes?stamp=server", Some(&change))
            .unwrap()
            .json()
            .unwrap();
        assert_eq!(outcomes[0]["outcome"], "applied", "{outcomes}");
        id
    };
    let at = "2026-11-08T08:00:00Z";
    let it = make_tag(&other, "IT", None);
    let privat = create(
        "folder",
        json!({"id": samu_api::Id::new(), "created": at, "modified": at,
               "title": "Privat", "status": "active", "position": 0}),
    );
    let project = |title: &str, tags: Value| {
        create(
            "project",
            json!({"id": samu_api::Id::new(), "created": at, "modified": at, "title": title,
                   "folder": privat, "kind": "parallel", "status": "active",
                   "flagged": false, "tags": tags, "position": 0}),
        )
    };
    let garten = project("Garten", json!([]));
    let homelab = project("Homelab", json!([{"tag": it, "position": 0}]));
    make_task(&other, "Rasen mähen", json!({"project": garten}));
    make_task(&other, "Kabel ziehen", json!({"project": homelab}));
    make_task(
        &other,
        "Backup prüfen",
        json!({"project": homelab, "due": {"date": "2000-01-01"}, "flagged": true}),
    );
    make_task(&other, "Milch", json!({}));
    let samu = phone(&server);
    samu.sync().unwrap();

    let shown = || -> Vec<String> {
        let mut out: Vec<String> = samu
            .available(None)
            .unwrap()
            .iter()
            .flat_map(|s| s.rows.iter().map(|r| r.title.clone()))
            .collect();
        out.sort_unstable();
        out
    };
    // The probe that must find something: all four before a scope.
    assert_eq!(shown().len(), 4);
    assert_eq!(samu.outside_due().unwrap(), 0);

    let scope = samu.create_scope("Ohne IT".into(), false).unwrap();
    samu.set_scope_member(scope.clone(), it.clone(), true)
        .unwrap();
    let ticked: Vec<(String, bool)> = samu
        .scope_members(scope.clone())
        .unwrap()
        .into_iter()
        .map(|m| (m.title, m.member))
        .collect();
    assert_eq!(
        ticked,
        [
            ("Privat".into(), false),
            ("Garten".into(), false),
            ("Homelab".into(), false),
            ("IT".into(), true)
        ]
    );
    assert!(samu.selected_scope().unwrap().is_none());
    samu.select_scope(Some(scope.clone())).unwrap();
    assert_eq!(samu.selected_scope().unwrap().unwrap().title, "Ohne IT");
    assert_eq!(
        shown(),
        ["Milch", "Rasen mähen"],
        "offline, before any sync"
    );
    assert_eq!(samu.outside_due().unwrap(), 1);
    let projects: Vec<String> = samu
        .project_tree()
        .unwrap()
        .into_iter()
        .filter(|p| p.kind == "project")
        .map(|p| p.title)
        .collect();
    assert_eq!(projects, ["Garten"]);
    assert!(
        titles(&samu.today().unwrap()).is_empty(),
        "the widget follows the scope: {:?}",
        titles(&samu.today().unwrap())
    );
    assert!(
        samu.forecast(None)
            .unwrap()
            .iter()
            .all(|d| d.rows.is_empty())
    );

    // On the server after the sync, members and all; the choice is not.
    samu.sync().unwrap();
    assert_eq!(samu.outbox_count().unwrap(), 0);
    let held = stored(&other, &scope);
    assert_eq!(held["title"], "Ohne IT");
    assert_eq!(held["mode"], "except");
    assert_eq!(held["members"], json!([it]));
    assert_eq!(shown(), ["Milch", "Rasen mähen"], "after the sync too");

    samu.set_scope_mode(scope.clone(), true).unwrap();
    samu.rename_scope(scope.clone(), "Nur IT".into()).unwrap();
    // ADR 0043 §1: with `only`, the inbox only when ticked.
    assert_eq!(shown(), ["Backup prüfen", "Kabel ziehen"]);
    samu.set_scope_inbox(scope.clone(), true).unwrap();
    assert!(samu.selected_scope().unwrap().unwrap().inbox);
    assert_eq!(shown(), ["Backup prüfen", "Kabel ziehen", "Milch"]);

    // ADR 0043 §2/§3: a filter from text, offline, AND the members.
    samu.set_scope_filter(scope.clone(), "#inbox".into())
        .unwrap();
    assert_eq!(shown(), ["Milch"]);
    assert_eq!(samu.scope_filter(scope.clone()).unwrap(), "#Inbox");
    assert!(matches!(
        samu.set_scope_filter(scope.clone(), "Milch".into()),
        Err(MobileError::Input { .. })
    ));
    assert_eq!(shown(), ["Milch"], "a refused text changes nothing");
    samu.sync().unwrap();
    assert_eq!(
        stored(&other, &scope)["filter"],
        json!([{"kind": "project", "value": "inbox"}])
    );
    samu.set_scope_filter(scope.clone(), " ".into()).unwrap();
    assert_eq!(shown(), ["Backup prüfen", "Kabel ziehen", "Milch"]);

    // ADR 0043 §6: offline, a capture in the chosen scope gets its project
    // and tags; what a line names itself wins.
    samu.set_scope_capture(scope.clone(), Some(homelab.clone()), vec![it.clone()])
        .unwrap();
    let capture = samu.scope_capture(scope.clone()).unwrap();
    assert_eq!(
        (capture.project.as_deref(), capture.tags.len()),
        (Some(homelab.as_str()), 1)
    );
    let patch = samu.capture("Patch einspielen".into(), None).unwrap();
    let own = samu
        .capture("Rosen schneiden #Garten".into(), None)
        .unwrap();
    samu.sync().unwrap();
    let patch = stored(&other, &patch);
    assert_eq!(patch["project"], json!(homelab));
    assert_eq!(patch["tags"][0]["tag"], json!(it));
    let own = stored(&other, &own);
    assert_eq!(own["project"], json!(garten));
    // It names a project, not a tag: the tag still comes from the scope.
    assert_eq!(own["tags"][0]["tag"], json!(it), "{own}");
    samu.set_scope_capture(scope.clone(), None, Vec::new())
        .unwrap();
    samu.sync().unwrap();
    assert!(stored(&other, &scope).get("capture").is_none());

    // ADR 0043 §7/§8: a phone that follows the schedule; the last event
    // wins; Tasker's word chooses.
    samu.set_scope_schedule(scope.clone(), "mo-so 00:00-00:00".into())
        .unwrap();
    assert_eq!(
        samu.scope_schedule(scope.clone()).unwrap(),
        "mo,di,mi,do,fr,sa,so 00:00-00:00"
    );
    assert!(matches!(
        samu.set_scope_schedule(scope.clone(), "xx 08:00-09:00".into()),
        Err(MobileError::Input { .. })
    ));
    samu.select_scope(None).unwrap();
    assert!(samu.selected_scope().unwrap().is_none(), "the probe");
    samu.set_scope_auto(true).unwrap();
    assert!(samu.scope_auto().unwrap());
    assert_eq!(
        samu.selected_scope().unwrap().unwrap().id,
        scope,
        "the schedule's"
    );
    samu.select_scope(None).unwrap();
    assert!(
        samu.selected_scope().unwrap().is_none(),
        "by hand, after the edge"
    );
    assert_eq!(
        samu.choose_scope_by_word("nur it".into()).unwrap(),
        "Nur IT"
    );
    assert_eq!(samu.selected_scope().unwrap().unwrap().id, scope);
    assert!(samu.choose_scope_by_word("Gibt es nicht".into()).is_err());
    assert_eq!(
        samu.selected_scope().unwrap().unwrap().id,
        scope,
        "unchanged"
    );
    assert_eq!(samu.choose_scope_by_word("".into()).unwrap(), "kein Scope");
    assert!(samu.selected_scope().unwrap().is_none());
    assert!(
        samu.choose_scope_by_word("auto".into())
            .unwrap()
            .starts_with("Nur IT")
    );
    assert_eq!(samu.selected_scope().unwrap().unwrap().id, scope);
    samu.set_scope_auto(false).unwrap();
    samu.set_scope_schedule(scope.clone(), String::new())
        .unwrap();
    samu.select_scope(Some(scope.clone())).unwrap();
    assert!(matches!(
        samu.rename_scope(scope.clone(), " ".into()),
        Err(MobileError::Input { .. })
    ));

    // Deleted on another device: none is chosen any more.
    let delete = json!([{"op": "delete", "object": scope,
                         "hlc": {"wall": 0, "counter": 0, "device": "x"}}]);
    other
        .call("POST", "/changes?stamp=server", Some(&delete))
        .unwrap();
    samu.sync().unwrap();
    assert!(samu.selected_scope().unwrap().is_none());
    // The four from the start and the two captured above.
    assert_eq!(shown().len(), 6);
    assert!(samu.scopes().unwrap().is_empty());

    // Deleted here: the choice goes with it.
    let second = samu.create_scope("Garten".into(), true).unwrap();
    samu.select_scope(Some(second.clone())).unwrap();
    samu.delete_scope(second).unwrap();
    assert!(samu.selected_scope().unwrap().is_none());
    assert_eq!(shown().len(), 6);
}

/// ADR 0059, 0060: an icon on a folder and a project and a colour on a
/// scope, set offline through the outbox, shown in the rows and on the
/// server; a wrong value is an input error and writes nothing.
#[test]
fn icons_and_a_scope_colour_go_through_the_outbox() {
    let server = Server::start();
    let other = server.other_client();
    let samu = phone(&server);
    let folder = samu.create_folder("Haus".into(), None).unwrap();
    let project = samu.create_project("Garten".into(), None).unwrap();
    let scope = samu.create_scope("prv".into(), true).unwrap();
    samu.set_place_icon(folder.clone(), Some("🏠".into()))
        .unwrap();
    samu.set_place_icon(project.clone(), Some(" 🌻 ".into()))
        .unwrap();
    assert!(
        samu.set_place_icon(project.clone(), Some("a".repeat(33)))
            .is_err()
    );
    samu.set_scope_color(scope.clone(), Some("green".into()))
        .unwrap();
    assert!(
        samu.set_scope_color(scope.clone(), Some("black".into()))
            .is_err()
    );

    assert_eq!(
        samu.place_detail(folder.clone()).unwrap().icon.as_deref(),
        Some("🏠")
    );
    let tree = samu.project_tree().unwrap();
    let row = tree.iter().find(|p| p.id == project).unwrap();
    assert_eq!(row.icon.as_deref(), Some("🌻"));
    let row = tree.iter().find(|p| p.id == folder).unwrap();
    assert_eq!(row.icon.as_deref(), Some("🏠"), "a folder's emoji too");
    assert_eq!(samu.scopes().unwrap()[0].color.as_deref(), Some("green"));

    samu.sync().unwrap();
    assert_eq!(stored(&other, &folder)["icon"], "🏠");
    assert_eq!(stored(&other, &project)["icon"], "🌻");
    assert_eq!(stored(&other, &scope)["color"], "green");

    samu.set_place_icon(project.clone(), None).unwrap();
    samu.set_scope_color(scope.clone(), None).unwrap();
    samu.sync().unwrap();
    assert!(stored(&other, &project).get("icon").is_none());
    assert!(stored(&other, &scope).get("color").is_none());
    assert_eq!(samu.scopes().unwrap()[0].color, None);
}

/// A tag picker makes a tag by path, offline, and finds it again by whole
/// title (92af4fff); a prefix is no match, a state is refused.
/// ADR 0094 §8, 0072: the choice „Nur im Scope:“ and a scope's inspector
/// layout go through the outbox; the task page folds
/// what the layout leaves out and names what of it has a value.
#[test]
fn own_scope_and_the_inspector_layout_of_a_scope() {
    let server = Server::start();
    let other = server.other_client();
    let project = make_project(&other, "Habits");
    let task = make_task(
        &other,
        "Rauchfrei",
        json!({"project": project, "due": {"date": "2026-11-20"}}),
    );
    let samu = phone(&server);
    samu.sync().unwrap();

    let scope = samu.create_scope("Habits".into(), true).unwrap();
    assert!(
        samu.set_own_scope(project.clone(), Some(project.clone()))
            .is_err(),
        "not a scope"
    );
    samu.set_own_scope(project.clone(), Some(scope.clone()))
        .unwrap();
    let detail = samu.place_detail(project.clone()).unwrap();
    assert!(detail.scope_only);
    assert_eq!(detail.own_scope.as_deref(), Some(scope.as_str()));
    assert_eq!(detail.own_scope_title.as_deref(), Some("Habits"));
    assert!(
        samu.project_tree().unwrap().iter().all(|p| p.id != project),
        "hidden without a scope"
    );
    let members = samu.scope_members(scope.clone()).unwrap();
    assert!(members.iter().any(|m| m.id == project
        && m.scope_only
        && m.own_scope_title.as_deref() == Some("Habits")));
    // Its own scope shows it without a tick (ADR 0094 §3).
    samu.select_scope(Some(scope.clone())).unwrap();
    assert!(samu.project_tree().unwrap().iter().any(|p| p.id == project));
    samu.select_scope(None).unwrap();

    assert_eq!(samu.scope_layout(scope.clone()).unwrap(), None);
    samu.select_scope(Some(scope.clone())).unwrap();
    assert_eq!(
        samu.task_layout(task.clone()).unwrap(),
        None,
        "no layout, all shown"
    );
    // Given out of order; kept in the inspector's.
    let habit = samu_mobile::InspectorLayoutRow {
        shown: vec![
            "reminders".into(),
            "note".into(),
            "planned".into(),
            "repeat".into(),
        ],
        note_rendered: true,
    };
    samu.set_scope_layout(scope.clone(), Some(habit)).unwrap();
    assert!(
        samu.set_scope_layout(
            scope.clone(),
            Some(samu_mobile::InspectorLayoutRow {
                shown: vec!["nix".into()],
                note_rendered: false
            })
        )
        .is_err()
    );
    let layout = samu.scope_layout(scope.clone()).unwrap().unwrap();
    assert_eq!(layout.shown, ["planned", "repeat", "note", "reminders"]);
    assert!(layout.note_rendered);

    let page = samu.task_layout(task.clone()).unwrap().unwrap();
    assert_eq!(
        page.shown.as_deref().unwrap(),
        ["planned", "repeat", "note", "reminders"]
    );
    assert_eq!(
        page.valued,
        ["project", "due"],
        "folded with a value, named"
    );
    assert_eq!(
        page.rest, 14,
        "series and checklist are parts too (ADR 0092 §3, ADR 0141)"
    );
    assert!(page.note_rendered && !page.unfolded);

    samu.set_inspector_unfolded(true).unwrap();
    let open = samu.task_layout(task.clone()).unwrap().unwrap();
    assert!(open.unfolded && open.shown.is_none());
    // Unfolded is this scope's; another one with the same layout folds.
    let second = samu.create_scope("Zweiter".into(), true).unwrap();
    samu.set_scope_layout(second.clone(), samu.scope_layout(scope.clone()).unwrap())
        .unwrap();
    samu.select_scope(Some(second)).unwrap();
    assert!(!samu.task_layout(task.clone()).unwrap().unwrap().unfolded);
    samu.select_scope(None).unwrap();
    assert_eq!(samu.task_layout(task).unwrap(), None, "no scope, no layout");

    samu.sync().unwrap();
    assert_eq!(stored(&other, &project)["own_scope"], json!(scope));
    assert_eq!(
        stored(&other, &project)["scope_only"],
        true,
        "derived for older readers"
    );
    assert_eq!(
        stored(&other, &scope)["layout"],
        json!({"inspector": {"shown": ["planned", "repeat", "note", "reminders"], "note": "rendered"}})
    );
    samu.set_scope_layout(scope.clone(), None).unwrap();
    samu.set_own_scope(project.clone(), None).unwrap();
    samu.sync().unwrap();
    assert_eq!(stored(&other, &scope).get("layout"), None);
    assert_eq!(stored(&other, &project).get("own_scope"), None);
    assert_ne!(stored(&other, &project)["scope_only"], true);
}

/// ADR 0088 §7: the tree marks a project due for review; „Geprüft“ goes
/// through the outbox and reaches the server on the next sync.
#[test]
fn a_project_due_for_review_is_marked_reviewed_offline() {
    let server = Server::start();
    let other = server.other_client();
    let project = make_project(&other, "Keller");
    let changes = json!([
        {"op": "set", "object": project, "field": "review_interval", "value": "P1W",
         "hlc": {"wall": 0, "counter": 0, "device": "ignored"}},
        {"op": "set", "object": project, "field": "next_review", "value": "2020-01-01",
         "hlc": {"wall": 0, "counter": 0, "device": "ignored"}},
    ]);
    other
        .call("POST", "/changes?stamp=server", Some(&changes))
        .unwrap();
    let samu = phone(&server);
    samu.sync().unwrap();
    let due = |samu: &Samu| {
        samu.project_tree()
            .unwrap()
            .iter()
            .find(|p| p.id == project)
            .unwrap()
            .review_due
    };
    assert!(due(&samu));

    samu.mark_reviewed(project.clone()).unwrap();
    assert!(!due(&samu), "at once, before the sync");
    assert!(samu.mark_reviewed("not an id".into()).is_err());
    samu.sync().unwrap();
    assert!(stored(&other, &project)["last_reviewed"].is_string());
    assert!(stored(&other, &project).get("next_review").is_none());
}

#[test]
fn a_picker_makes_a_tag_by_path() {
    let server = Server::start();
    let other = server.other_client();
    let samu = phone(&server);
    let buero = samu.create_tag_path("Ort/Büro".into()).unwrap();
    assert_eq!(samu.create_tag_path(" ort / büro ".into()).unwrap(), buero);
    let ort = samu.create_tag_path("Ort".into()).unwrap();
    let neu = samu.create_tag_path("Bü".into()).unwrap();
    assert_ne!(neu, buero, "a prefix makes a new tag");
    assert!(samu.create_tag_path(" / ".into()).is_err());
    let tags: Vec<(String, u32)> = samu
        .places(false)
        .unwrap()
        .into_iter()
        .filter(|p| p.kind == "tag")
        .map(|p| (p.title, p.depth))
        .collect();
    assert_eq!(
        tags,
        [("Ort".into(), 0), ("Büro".into(), 1), ("Bü".into(), 0)]
    );
    samu.sync().unwrap();
    assert_eq!(stored(&other, &buero)["parent"], ort.as_str());
    assert_eq!(stored(&other, &neu)["title"], "Bü");
}

/// ADR 0036: projects, folders and tags made and renamed offline through
/// the outbox; deleting is online, a preview writes nothing, and the tasks
/// of a deleted project show in the inbox on the phone. An orphan made
/// behind the phone's back shows there too (§6).
#[test]
fn places_are_made_offline_and_deleted_online() {
    let mut server = Server::start();
    let other = server.other_client();
    let samu = phone(&server);
    let folder = samu.create_folder("Haus".into(), None).unwrap();
    let project = samu
        .create_project("Umzug".into(), Some(folder.clone()))
        .unwrap();
    let tag = samu.create_tag("Orte".into(), None).unwrap();
    samu.rename_place(project.clone(), "Umzug 2027".into())
        .unwrap();
    assert!(samu.rename_place(project.clone(), "  ".into()).is_err());
    // A slash in a tag title would make a path mean two tags (ADR 0027 §3);
    // a project may have one.
    assert!(samu.create_tag("Ort/Büro".into(), None).is_err());
    assert!(samu.rename_place(tag.clone(), "Ort/Büro".into()).is_err());
    samu.rename_place(project.clone(), "Umzug 2027/28".into())
        .unwrap();
    samu.rename_place(project.clone(), "Umzug 2027".into())
        .unwrap();
    let places: Vec<(String, String, u32)> = samu
        .places(false)
        .unwrap()
        .into_iter()
        .map(|p| (p.kind, p.title, p.depth))
        .collect();
    assert_eq!(
        places,
        [
            ("folder".into(), "Haus".into(), 0),
            ("project".into(), "Umzug 2027".into(), 1),
            ("tag".into(), "Orte".into(), 0),
        ]
    );
    samu.sync().unwrap();
    assert_eq!(stored(&other, &project)["title"], "Umzug 2027");
    assert_eq!(stored(&other, &project)["folder"], folder.as_str());

    make_task(&other, "Kartons", json!({"project": project}));
    samu.sync().unwrap();
    let inbox = |samu: &Samu| -> Vec<String> {
        samu.inbox(None)
            .unwrap()
            .iter()
            .flat_map(|s| s.rows.iter().map(|r| r.title.clone()))
            .collect()
    };
    assert!(inbox(&samu).is_empty(), "the probe: not yet in the inbox");

    let preview = samu
        .delete_place(project.clone(), Some("inbox".into()), None, true)
        .unwrap();
    assert_eq!((preview.deleted, preview.moved), (1, 1));
    assert_eq!(stored(&other, &project)["title"], "Umzug 2027");
    assert!(
        samu.delete_place(project.clone(), Some("project".into()), None, true)
            .is_err(),
        "another project needs a target"
    );
    samu.delete_place(project.clone(), Some("inbox".into()), None, false)
        .unwrap();
    assert_eq!(inbox(&samu), ["Kartons"]);
    assert!(samu.places(false).unwrap().iter().all(|p| p.id != project));
    let gone = other
        .call("GET", &format!("/objects/{project}"), None)
        .unwrap_err();
    assert!(gone.to_string().contains("no object"), "{gone}");

    // A plain tombstone on a project with tasks is refused (ADR 0076 §2):
    // it would leave the task pointing at nothing. How an orphan from
    // before is shown, samu-core tests (availability).
    let lone = samu.create_project("Steuer".into(), None).unwrap();
    samu.sync().unwrap();
    make_task(&other, "Belege", json!({"project": lone}));
    let tombstone = json!([{"op": "delete", "object": lone,
                            "hlc": {"wall": 0, "counter": 0, "device": "x"}}]);
    let outcome = other
        .call("POST", "/changes?stamp=server", Some(&tombstone))
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(outcome[0]["outcome"], "rejected", "{outcome}");
    assert!(
        outcome[0]["reason"].as_str().unwrap().contains("1 tasks"),
        "{outcome}"
    );
    samu.sync().unwrap();
    assert!(samu.places(false).unwrap().iter().any(|p| p.id == lone));
    assert_eq!(inbox(&samu), ["Kartons"]);

    let refused = samu.delete_place(tag.clone(), Some("inbox".into()), None, false);
    assert!(
        matches!(refused, Err(MobileError::Refused { .. })),
        "{refused:?}"
    );
    server.stop();
    let offline = samu.delete_place(tag, None, None, false);
    assert!(
        matches!(offline, Err(MobileError::Offline { .. })),
        "{offline:?}"
    );
}

/// ADR 0037: a project moves into a folder and becomes a single-action
/// list on hold, offline through the outbox; a folder never goes below
/// itself, and its own subtree is never offered as a target.
#[test]
fn places_move_and_projects_are_set_offline() {
    let server = Server::start();
    let other = server.other_client();
    let samu = phone(&server);
    let gtd = samu.create_folder("GTD".into(), None).unwrap();
    let house = samu.create_folder("Haus".into(), None).unwrap();
    let garden = samu
        .create_folder("Garten".into(), Some(house.clone()))
        .unwrap();
    let project = samu.create_project("Sammelsurium".into(), None).unwrap();
    let status = samu.create_tag("Status".into(), None).unwrap();
    // A workflow is an exclusive group; made so by another client.
    samu.sync().unwrap();
    other
        .call(
            "POST",
            "/changes?stamp=server",
            Some(
                &json!([{"op": "set", "object": status, "field": "exclusive",
                          "value": true, "hlc": {"wall": 0, "counter": 0, "device": "x"}}]),
            ),
        )
        .unwrap();
    samu.sync().unwrap();

    samu.move_place(project.clone(), Some(gtd.clone())).unwrap();
    samu.set_project_kind(project.clone(), "single_actions".into())
        .unwrap();
    samu.set_project_status(project.clone(), "on_hold".into())
        .unwrap();
    samu.set_project_review(project.clone(), Some("P2W".into()))
        .unwrap();
    assert_eq!(samu.workflows().unwrap()[0].id, status);
    samu.set_project_workflow(project.clone(), Some(status.clone()))
        .unwrap();
    assert!(
        samu.set_project_kind(project.clone(), "odd".into())
            .is_err()
    );

    let detail = samu.place_detail(project.clone()).unwrap();
    assert_eq!(detail.place.as_deref(), Some(gtd.as_str()));
    assert_eq!(detail.place_title.as_deref(), Some("GTD"));
    assert_eq!(detail.project_kind.as_deref(), Some("single_actions"));
    assert_eq!(detail.status.as_deref(), Some("on_hold"));
    assert_eq!(detail.review.as_deref(), Some("P2W"));
    let line = samu
        .places(false)
        .unwrap()
        .into_iter()
        .find(|p| p.id == project)
        .unwrap();
    assert_eq!(
        (line.depth, line.meta.as_str()),
        (1, "Einzelaufgaben · ruht")
    );

    let targets: Vec<String> = samu
        .place_targets(house.clone())
        .unwrap()
        .into_iter()
        .map(|p| p.title)
        .collect();
    assert_eq!(targets, ["GTD"], "never itself or below it");
    assert!(
        samu.move_place(house.clone(), Some(garden.clone()))
            .is_err()
    );
    samu.move_place(garden.clone(), None).unwrap();

    samu.sync().unwrap();
    let stored = stored(&other, &project);
    assert_eq!(stored["folder"], gtd.as_str());
    assert_eq!(stored["kind"], "single_actions");
    assert_eq!(stored["status"], "on_hold");
    assert_eq!(stored["review_interval"], "P2W");
    assert_eq!(stored["workflow"], status.as_str());
    assert!(
        other_folder_parent(&other, &garden).is_null(),
        "Garten is at the top"
    );

    samu.set_project_status(project.clone(), "completed".into())
        .unwrap();
    samu.sync().unwrap();
    assert!(stored_field(&other, &project, "completed_at").is_string());
    assert!(samu.places(false).unwrap().iter().all(|p| p.id != project));
    assert!(
        samu.places(true)
            .unwrap()
            .iter()
            .any(|p| p.id == project && p.closed)
    );
}

/// The tab „Projekte“ as a tree: rows, folding, stepping (ADR 0085).
#[test]
fn the_project_tree_folds_and_steps_offline() {
    let server = Server::start();
    let other = server.other_client();
    let samu = phone(&server);
    let house = samu.create_folder("Haus".into(), None).unwrap();
    let garden = samu
        .create_folder("Garten".into(), Some(house.clone()))
        .unwrap();
    let work = samu.create_folder("Arbeit".into(), None).unwrap();
    let roof = samu
        .create_project("Dach".into(), Some(house.clone()))
        .unwrap();
    let beds = samu
        .create_project("Beete".into(), Some(garden.clone()))
        .unwrap();
    let a = samu.create_project("A".into(), None).unwrap();
    let b = samu.create_project("B".into(), None).unwrap();
    let c = samu.create_project("C".into(), None).unwrap();
    samu.capture("Ziegel zählen".into(), Some(roof.clone()))
        .unwrap();
    samu.capture("Säen".into(), Some(beds.clone())).unwrap();
    samu.capture("Gießen".into(), Some(beds.clone())).unwrap();

    let line = |rows: &[samu_mobile::TreeRow]| -> Vec<String> {
        rows.iter()
            .map(|r| format!("{}{}", "  ".repeat(r.depth as usize), r.title))
            .collect()
    };
    let tree = samu.project_tree().unwrap();
    assert_eq!(
        line(&tree),
        [
            "Haus",
            "  Garten",
            "    Beete",
            "  Dach",
            "Arbeit",
            "A",
            "B",
            "C"
        ],
        "folders first, then projects, under each parent"
    );
    let row = |rows: &[samu_mobile::TreeRow], id: &str| {
        rows.iter().find(|r| r.id == id).cloned().unwrap()
    };
    assert_eq!(row(&tree, &house).available, 3, "the sum below the folder");
    assert_eq!(row(&tree, &beds).available, 2);
    let first = row(&tree, &a);
    assert!(
        !first.can_up && first.can_down,
        "A is the first project at the top"
    );
    assert!(row(&tree, &house).can_down && !row(&tree, &work).can_down);

    samu.set_folded(house.clone(), true).unwrap();
    let tree = samu.project_tree().unwrap();
    assert_eq!(line(&tree), ["Haus", "Arbeit", "A", "B", "C"]);
    assert!(row(&tree, &house).folded);
    assert_eq!(row(&tree, &house).available, 3, "folded, it still sums up");
    samu.set_folded(house.clone(), false).unwrap();
    assert_eq!(samu.project_tree().unwrap().len(), 8);

    // C passes B, a folder passes a folder; at the edge it says no.
    samu.step_place(c.clone(), true).unwrap();
    samu.step_place(work.clone(), true).unwrap();
    assert!(
        samu.step_place(work.clone(), true).is_err(),
        "already the first"
    );
    assert!(
        samu.step_place(b.clone(), false).is_err(),
        "already the last"
    );
    let tree = samu.project_tree().unwrap();
    assert_eq!(
        line(&tree),
        [
            "Arbeit",
            "Haus",
            "  Garten",
            "    Beete",
            "  Dach",
            "A",
            "C",
            "B"
        ]
    );

    // A scope that hides C: A steps down past B, over the hidden C, and
    // all three are counted anew.
    let scope = samu.create_scope("Ohne C".into(), false).unwrap();
    samu.set_scope_member(scope.clone(), c.clone(), true)
        .unwrap();
    samu.select_scope(Some(scope.clone())).unwrap();
    let tree = samu.project_tree().unwrap();
    assert!(tree.iter().all(|r| r.id != c), "the scope hides C");
    samu.step_place(a.clone(), false).unwrap();
    samu.select_scope(None).unwrap();
    assert_eq!(line(&samu.project_tree().unwrap())[5..], ["C", "B", "A"]);

    // A new project in a folder lands last there.
    let shed = samu
        .create_project("Schuppen".into(), Some(house.clone()))
        .unwrap();
    assert_eq!(
        line(&samu.project_tree().unwrap())[1..6],
        ["Haus", "  Garten", "    Beete", "  Dach", "  Schuppen"]
    );

    samu.sync().unwrap();
    let position = |id: &str| stored_field(&other, id, "position").as_i64().unwrap();
    assert!(position(&c) < position(&b) && position(&b) < position(&a));
    assert!(position(&work) < position(&house));
    assert!(position(&roof) < position(&shed));
    // Folding stays on the phone; nothing about it went out.
    assert!(stored(&other, &house).get("folded").is_none());
}

fn stored_field(client: &Client, id: &str, field: &str) -> Value {
    stored(client, id)[field].clone()
}

fn other_folder_parent(client: &Client, id: &str) -> Value {
    stored_field(client, id, "parent")
}

/// An agent's session, as `samu login --agent` makes it (ADR 0039 §2).
fn agent_client(server: &Server) -> Client {
    let answer = Client::new(&server.url, None)
        .unwrap()
        .call(
            "POST",
            "/login",
            Some(
                &json!({"account": "privat", "password": PASSWORD, "device": "claude",
                         "kind": "agent"}),
            ),
        )
        .unwrap()
        .json()
        .unwrap();
    Client::new(
        &server.url,
        Some(answer["token"].as_str().unwrap().to_string()),
    )
    .unwrap()
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A file the app hands over, as `TakePicture` or `MediaRecorder` leave it.
fn taken(server: &Server, name: &str, bytes: &[u8]) -> String {
    let path = server.dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path.display().to_string()
}

/// ADR 0039 §6: a comment with a photo and a recording on the task, all
/// written before the server sees any of it. The sync sends the files
/// first, so the store takes the attachments that need them.
#[test]
fn a_comment_with_a_photo_and_a_recording_reach_the_server_files_first() {
    let server = Server::start();
    let other = server.other_client();
    let task = make_task(&other, "Zaun streichen", json!({}));
    let samu = phone(&server);
    samu.sync().unwrap();

    let photo = b"\xff\xd8\xff not really a jpeg";
    let voice = b"not really aac";
    let comment = samu
        .add_comment(task.clone(), "  Farbe ist da  ".into())
        .unwrap();
    let on_comment = samu
        .attach_file(
            comment.clone(),
            taken(&server, "photo.jpg", photo),
            "foto.jpg".into(),
            "image/jpeg".into(),
        )
        .unwrap();
    let on_task = samu
        .attach_file(
            task.clone(),
            taken(&server, "voice.m4a", voice),
            "aufnahme.m4a".into(),
            "audio/mp4".into(),
        )
        .unwrap();
    assert!(
        !server.dir.join("photo.jpg").exists(),
        "the file moved into the phone's store"
    );

    // Shown before the server has it.
    let comments = samu.comments(task.clone()).unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].body, "Farbe ist da");
    assert!(comments[0].pending && comments[0].changeable && !comments[0].agent);
    assert_eq!(comments[0].attachments.len(), 1);
    assert!(comments[0].attachments[0].pending);
    assert!(comments[0].attachments[0].local.is_some());
    let files = samu.attachments(task.clone()).unwrap();
    assert_eq!(files.len(), 1, "the comment's photo is not the task's");
    assert_eq!(files[0].name, "aufnahme.m4a");

    let report = samu.sync().unwrap();
    assert_eq!(
        report.rejected,
        0,
        "{:?}",
        samu.status().unwrap().last_error
    );
    assert_eq!(samu.status().unwrap().last_error, None);

    // What the server holds, asked of the server.
    let got = other
        .call("GET", &format!("/blobs/{}", sha256(photo)), None)
        .unwrap();
    assert_eq!(got.body, photo);
    let stored_comment = stored(&other, &comment);
    assert_eq!(stored_comment["body"], "Farbe ist da");
    assert_eq!(stored_comment["owner"], task.as_str());
    assert_eq!(
        stored_comment["created_by"],
        json!({"device": "pixel", "kind": "human"})
    );
    let stored_photo = stored(&other, &on_comment);
    assert_eq!(stored_photo["owner"], comment.as_str());
    assert_eq!(stored_photo["blob"], sha256(photo));
    assert_eq!(stored_photo["size"], photo.len());
    assert_eq!(stored_photo["media_type"], "image/jpeg");
    assert_eq!(stored(&other, &on_task)["blob"], sha256(voice));

    let comments = samu.comments(task.clone()).unwrap();
    assert!(!comments[0].pending && !comments[0].attachments[0].pending);
    assert_eq!(comments[0].author.as_deref(), Some("pixel"));

    samu.edit_comment(comment.clone(), "Farbe reicht nicht".into())
        .unwrap();
    samu.delete_note(on_task.clone()).unwrap();
    samu.sync().unwrap();
    assert_eq!(stored(&other, &comment)["body"], "Farbe reicht nicht");
    assert!(samu.attachments(task.clone()).unwrap().is_empty());
    assert!(
        other
            .call("GET", &format!("/blobs/{}", sha256(voice)), None)
            .is_ok(),
        "deleting an attachment leaves its file"
    );
}

/// ADR 0039 §3: an agent's comment is marked and stays the agent's; its
/// file comes down on demand and is kept.
#[test]
fn an_agents_comment_is_marked_and_not_the_phones_to_change() {
    let server = Server::start();
    let other = server.other_client();
    let task = make_task(&other, "Bericht", json!({}));
    let agent = agent_client(&server);
    let bytes = b"a picture of the board";
    let hash = sha256(bytes);
    agent
        .call_bytes("PUT", &format!("/blobs/{hash}"), "image/png", &[], bytes)
        .unwrap();
    let comment = samu_api::Id::new().to_string();
    let file = samu_api::Id::new().to_string();
    let at = "2026-11-08T08:00:00Z";
    let changes = json!([
        {"op": "create", "object": comment, "kind": "comment", "hlc": {"wall": 0, "counter": 0, "device": "x"},
         "value": {"id": comment, "created": at, "modified": at, "owner": task, "body": "Stand"}},
        {"op": "create", "object": file, "kind": "attachment", "hlc": {"wall": 0, "counter": 0, "device": "x"},
         "value": {"id": file, "created": at, "modified": at, "owner": comment, "blob": hash,
                   "name": "board.png", "media_type": "image/png", "size": bytes.len()}}
    ]);
    let outcomes = agent
        .call("POST", "/changes?stamp=server", Some(&changes))
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(outcomes[1]["outcome"], "applied", "{outcomes}");

    let samu = phone(&server);
    samu.sync().unwrap();
    let comments = samu.comments(task.clone()).unwrap();
    assert_eq!(comments.len(), 1);
    assert!(comments[0].agent && !comments[0].changeable);
    assert_eq!(comments[0].author.as_deref(), Some("claude"));
    assert!(matches!(
        samu.edit_comment(comment.clone(), "anders".into()),
        Err(MobileError::Input { .. })
    ));
    assert!(matches!(
        samu.delete_note(comment.clone()),
        Err(MobileError::Input { .. })
    ));
    assert_eq!(samu.status().unwrap().outbox, 0, "nothing was queued");

    let row = &comments[0].attachments[0];
    assert_eq!(row.local, None, "not fetched yet");
    let path = samu.fetch_blob(row.hash.clone()).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(
        samu.comments(task).unwrap()[0].attachments[0].local,
        Some(path)
    );
}

/// Without a server a file waits with the outbox; nothing is lost.
#[test]
fn a_file_waits_for_the_server() {
    let mut server = Server::start();
    let other = server.other_client();
    let task = make_task(&other, "Quittung", json!({}));
    let samu = phone(&server);
    samu.sync().unwrap();
    server.stop();

    samu.attach_file(
        task.clone(),
        taken(&server, "q.jpg", b"receipt"),
        "q.jpg".into(),
        "image/jpeg".into(),
    )
    .unwrap();
    assert!(matches!(samu.sync(), Err(MobileError::Offline { .. })));
    assert_eq!(samu.status().unwrap().outbox, 1);
    let files = samu.attachments(task).unwrap();
    assert!(files[0].pending);
    assert_eq!(
        std::fs::read(files[0].local.as_ref().unwrap()).unwrap(),
        b"receipt"
    );
}

fn make_project(client: &Client, title: &str) -> String {
    let project = samu_api::Id::new().to_string();
    let change = json!([{"op": "create", "object": project, "kind": "project",
        "value": {"id": project, "created": "2026-11-08T08:00:00Z",
                  "modified": "2026-11-08T08:00:00Z", "title": title,
                  "kind": "parallel", "status": "active", "flagged": false, "position": 0},
        "hlc": {"wall": 0, "counter": 0, "device": "x"}}]);
    let outcomes = client
        .call("POST", "/changes?stamp=server", Some(&change))
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(outcomes[0]["outcome"], "applied", "{outcomes}");
    project
}

/// ADR 0040 §2: every field of the task page, offline through the outbox,
/// as the server then holds it.
#[test]
fn the_task_page_edits_every_field_through_the_outbox() {
    let server = Server::start();
    let other = server.other_client();
    let task = make_task(&other, "Zaun", json!({}));
    let office = make_tag(&other, "Büro", None);
    let samu = phone(&server);
    samu.sync().unwrap();

    samu.set_title(task.clone(), "  Zaun streichen ".into())
        .unwrap();
    assert!(samu.set_title(task.clone(), "  ".into()).is_err());
    samu.set_note(task.clone(), Some("grün".into())).unwrap();
    samu.set_date(task.clone(), "due".into(), Some("2026-12-24".into()))
        .unwrap();
    samu.set_date(task.clone(), "defer".into(), Some("2026-12-01".into()))
        .unwrap();
    samu.set_date(task.clone(), "due".into(), Some("2026-12-24 14:30".into()))
        .unwrap();
    assert!(
        samu.set_date(task.clone(), "due".into(), Some("2026-12-24 lunch".into()))
            .is_err(),
        "words left over"
    );
    assert!(
        samu.set_date(task.clone(), "soon".into(), None).is_err(),
        "no such date"
    );
    samu.set_estimate(task.clone(), Some(90)).unwrap();
    assert_eq!(
        samu.task_edit(task.clone()).unwrap().due_warning_days,
        None,
        "follows the account"
    );
    samu.set_due_warning(task.clone(), Some(7)).unwrap();
    assert!(samu.set_due_warning(task.clone(), Some(-1)).is_err());
    samu.set_priority(task.clone(), Some("B".into())).unwrap();
    assert!(samu.set_priority(task.clone(), Some("D".into())).is_err());
    samu.set_tag(task.clone(), office.clone(), true).unwrap();
    samu.plan_in(task.clone(), 1).unwrap();

    let edit = samu.task_edit(task.clone()).unwrap();
    assert_eq!(edit.title, "Zaun streichen");
    assert_eq!(edit.estimate_minutes, Some(90));
    assert_eq!(edit.due_warning_days, Some(7));
    assert_eq!(edit.due_soon_days, Some(2));
    assert_eq!(edit.priority.as_deref(), Some("B"));
    assert!(edit.tags.iter().any(|t| t.id == office && t.on));

    samu.sync().unwrap();
    let held = stored(&other, &task);
    assert_eq!(held["title"], "Zaun streichen");
    assert_eq!(held["note"], "grün");
    // ADR 0144 §2: in the account's zone, the test server's default.
    let due = held["due"]["at"].as_str().unwrap_or_default();
    assert!(due.starts_with("2026-12-24T14:30:00"), "{held}");
    assert_eq!(held["defer"], json!({"date": "2026-12-01"}));
    assert_eq!(held["planned"], json!({"date": day(&samu, 1)}));
    assert_eq!(held["estimate"], "PT90M");
    assert_eq!(held["due_warning"], "P7D");
    assert_eq!(held["priority"], "B");
    assert_eq!(held["tags"][0]["tag"], office.as_str());

    samu.set_note(task.clone(), Some("".into())).unwrap();
    samu.set_date(task.clone(), "defer".into(), None).unwrap();
    samu.set_estimate(task.clone(), None).unwrap();
    samu.set_priority(task.clone(), None).unwrap();
    samu.set_tag(task.clone(), office.clone(), false).unwrap();
    samu.drop_task(task.clone()).unwrap();
    samu.sync().unwrap();
    let after = stored(&other, &task);
    assert!(after.get("note").is_none(), "{after}");
    assert!(after.get("defer").is_none(), "{after}");
    assert!(after.get("estimate").is_none(), "{after}");
    assert!(after.get("priority").is_none(), "{after}");
    assert!(
        after["tags"].as_array().is_none_or(Vec::is_empty),
        "{after}"
    );
    assert!(after["dropped_at"].is_string());
}

/// 85b3799a: the task page sets a repetition from the form's parts,
/// keeps what the form does not know, takes a typed rule, refuses a bad
/// one, and shows what completing would make.
#[test]
fn the_task_page_sets_a_repetition() {
    let server = Server::start();
    let other = server.other_client();
    let task = make_task(
        &other,
        "Miete",
        json!({"due": {"date": "2099-01-31"},
               "repeat": {"rule": "FREQ=MONTHLY;BYMONTHDAY=-1", "mode": "fixed"}}),
    );
    let plain = make_task(&other, "Brot", json!({}));
    let samu = phone(&server);
    samu.sync().unwrap();

    assert!(samu.task_edit(plain.clone()).unwrap().repeat.is_none());
    let edit = samu.task_edit(task.clone()).unwrap().repeat.unwrap();
    assert_eq!(edit.freq.as_deref(), Some("MONTHLY"));
    assert_eq!(edit.rest, vec!["BYMONTHDAY=-1".to_string()]);
    assert_eq!(edit.next_field.as_deref(), Some("due"));
    assert_eq!(edit.next_date.as_deref(), Some("2099-02-28"));

    // The form: every 3 months, ending after 4, from completion.
    samu.set_repeat(
        task.clone(),
        samu_mobile::RepeatEdit {
            interval: 3,
            count: Some(4),
            mode: "due_after_completion".into(),
            catch_up: true,
            ..edit.clone()
        },
    )
    .unwrap();
    // A weekly one on a task that did not repeat.
    samu.set_repeat(
        plain.clone(),
        samu_mobile::RepeatEdit {
            rule: String::new(),
            mode: "fixed".into(),
            catch_up: false,
            freq: Some("WEEKLY".into()),
            interval: 1,
            byday: vec!["TH".into(), "MO".into()],
            until: Some("2099-12-31".into()),
            count: None,
            rest: vec![],
            next_field: None,
            next_date: None,
            ended: false,
        },
    )
    .unwrap();
    let bad = samu_mobile::RepeatEdit {
        rule: "FREQ=HOURLY".into(),
        freq: None,
        ..edit.clone()
    };
    assert!(
        samu.set_repeat(task.clone(), bad).is_err(),
        "at least daily"
    );
    samu.sync().unwrap();
    assert_eq!(
        stored(&other, &task)["repeat"],
        json!({"rule": "FREQ=MONTHLY;INTERVAL=3;BYMONTHDAY=-1;COUNT=4",
               "mode": "due_after_completion", "catch_up": false})
    );
    assert_eq!(
        stored(&other, &plain)["repeat"]["rule"],
        "FREQ=WEEKLY;BYDAY=MO,TH;UNTIL=20991231T235959Z"
    );

    // A rule typed as it is, with the prefix some calendars write.
    let typed = samu_mobile::RepeatEdit {
        rule: "RRULE:FREQ=YEARLY;BYMONTH=3;BYMONTHDAY=1".into(),
        freq: None,
        ..edit
    };
    samu.set_repeat(task.clone(), typed).unwrap();
    samu.clear_repeat(plain.clone()).unwrap();
    samu.sync().unwrap();
    assert_eq!(
        stored(&other, &task)["repeat"]["rule"],
        "FREQ=YEARLY;BYMONTH=3;BYMONTHDAY=1"
    );
    assert!(stored(&other, &plain).get("repeat").is_none());
}

/// ADR 0040 §3: a group moves with its descendants, which keep their
/// parents; the moved task is at the top level and last in the target.
#[test]
fn a_group_moves_with_its_descendants() {
    let server = Server::start();
    let other = server.other_client();
    let house = make_project(&other, "Haus");
    let garden = make_project(&other, "Garten");
    make_task(
        &other,
        "Schon da",
        json!({"project": garden, "position": 4}),
    );
    let group = make_task(&other, "Zaun", json!({"project": house}));
    let child = make_task(&other, "Farbe", json!({"project": house, "parent": group}));
    let grandchild = make_task(&other, "Pinsel", json!({"project": house, "parent": child}));
    let inner = make_task(&other, "Innen", json!({"project": house}));
    let nested = make_task(&other, "Teil", json!({"project": house, "parent": inner}));
    let samu = phone(&server);
    samu.sync().unwrap();

    assert_eq!(samu.task_edit(group.clone()).unwrap().descendants, 2);
    assert!(
        samu.move_targets()
            .unwrap()
            .iter()
            .any(|p| p.id == garden && p.kind == "project")
    );
    samu.move_task(group.clone(), Some(garden.clone())).unwrap();
    // A task inside a group moves out of it to the top level.
    samu.move_task(nested.clone(), None).unwrap();
    samu.sync().unwrap();

    assert_eq!(stored_field(&other, &group, "project"), garden.as_str());
    assert!(stored_field(&other, &group, "parent").is_null());
    assert_eq!(stored_field(&other, &group, "position"), 5);
    for (id, parent) in [(&child, &group), (&grandchild, &child)] {
        assert_eq!(stored_field(&other, id, "project"), garden.as_str());
        assert_eq!(stored_field(&other, id, "parent"), parent.as_str());
    }
    assert!(stored_field(&other, &nested, "project").is_null());
    assert!(stored_field(&other, &nested, "parent").is_null());
    assert_eq!(stored_field(&other, &inner, "project"), house.as_str());
    assert!(
        samu.move_task(group, Some(inner)).is_err(),
        "a task is no project"
    );
}

/// ADR 0040 §4: deleting takes the descendants, counted first; it needs
/// the server.
#[test]
fn deleting_a_task_takes_its_descendants() {
    let mut server = Server::start();
    let other = server.other_client();
    let group = make_task(&other, "Umzug", json!({}));
    let child = make_task(&other, "Kartons", json!({"parent": group}));
    let keep = make_task(&other, "Bleibt", json!({}));
    let samu = phone(&server);
    samu.sync().unwrap();

    let preview = samu.delete_task(group.clone(), true).unwrap();
    assert_eq!(preview.deleted, 2);
    assert_eq!(
        stored(&other, &group)["title"],
        "Umzug",
        "a preview deletes nothing"
    );
    samu.delete_task(group.clone(), false).unwrap();
    for id in [&group, &child] {
        let gone = other
            .call("GET", &format!("/objects/{id}"), None)
            .unwrap_err();
        assert!(gone.to_string().contains("no object"), "{gone}");
    }
    assert!(samu.task(group).is_err());
    assert_eq!(stored(&other, &keep)["title"], "Bleibt");

    server.stop();
    assert!(matches!(
        samu.delete_task(keep, false),
        Err(MobileError::Offline { .. })
    ));
}

/// ADR 0046 §2, §3, §8: offline the phone keeps one clock running and
/// stops it with its task, as the server does; after the sync the server
/// holds the same.
#[test]
fn clocks_offline_follow_the_servers_rules() {
    let server = Server::start();
    let other = server.other_client();
    let a = make_task(&other, "Steuer", json!({}));
    let b = make_task(&other, "Garten", json!({}));
    let samu = phone(&server);
    samu.sync().unwrap();
    assert_eq!(samu.running_clock().unwrap(), None);

    let first = samu.clock_in(a.clone()).unwrap();
    assert_eq!(samu.running_clock().unwrap().unwrap().title, "Steuer");
    let second = samu.clock_in(b.clone()).unwrap();
    let on = samu.running_clock().unwrap().unwrap();
    assert_eq!(
        (on.id.as_str(), on.title.as_str()),
        (second.as_str(), "Garten")
    );
    let mine = samu.clocks(a.clone()).unwrap();
    assert_eq!(mine.len(), 1);
    assert!(mine[0].end_ms.is_some(), "the first ended offline");

    // Completing Garten stops its clock; nothing runs.
    samu.complete(b.clone()).unwrap();
    assert_eq!(samu.running_clock().unwrap(), None);
    assert!(samu.clocks(b.clone()).unwrap()[0].end_ms.is_some());
    assert!(samu.clocked_seconds(a.clone()).unwrap() >= 0);
    // A clock on something closed is refused here.
    assert!(samu.clock_in(b.clone()).is_err());

    let report = samu.sync().unwrap();
    assert_eq!(report.rejected, 0);
    assert_eq!(samu.outbox_count().unwrap(), 0);
    // Asked of the server: two clocks, both ended.
    for clock in [&first, &second] {
        let object = stored(&other, clock);
        assert!(object["end"].is_string(), "{object}");
    }
    samu.clock_in(a.clone()).unwrap();
    samu.clock_out().unwrap();
    assert_eq!(samu.running_clock().unwrap(), None);
    samu.sync().unwrap();
    assert_eq!(samu.clocks(a).unwrap().len(), 2);
}

/// A calendar served over plain HTTP from a thread: tomorrow at nine in
/// Berlin, and a trip over today and the next two days.
fn ics_server(today: &str, tomorrow: &str, after: &str) -> String {
    let compact = |d: &str| d.replace('-', "");
    let ics = [
        "BEGIN:VCALENDAR".to_string(),
        "BEGIN:VEVENT".into(),
        "UID:dentist@example.org".into(),
        "SUMMARY:Zahnarzt".into(),
        "LOCATION:Hauptstraße 1".into(),
        "URL:https://calendar.example.org/event/dentist".into(),
        format!("DTSTART;TZID=Europe/Berlin:{}T090000", compact(tomorrow)),
        format!("DTEND;TZID=Europe/Berlin:{}T094500", compact(tomorrow)),
        "END:VEVENT".into(),
        "BEGIN:VEVENT".into(),
        "UID:trip@example.org".into(),
        "SUMMARY:Reise".into(),
        format!("DTSTART;VALUE=DATE:{}", compact(today)),
        format!("DTEND;VALUE=DATE:{}", compact(after)),
        "END:VEVENT".into(),
        "END:VCALENDAR".into(),
        String::new(),
    ]
    .join("\r\n");
    serve_ics(ics)
}

/// Serves `ics` over plain HTTP from a thread; the URL.
fn serve_ics(ics: String) -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 2 {
                line.clear();
            }
            let answer = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/calendar\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{ics}",
                ics.len()
            );
            let _ = stream.write_all(answer.as_bytes());
        }
    });
    format!("http://{address}/cal.ics")
}

/// A calendar "Familie" on the server, fetched once from `url`; its id.
fn subscribe(other: &Client, url: &str) -> String {
    let calendar = samu_api::Id::new().to_string();
    let value = json!({"id": calendar, "created": "2026-11-09T08:00:00Z",
                       "modified": "2026-11-09T08:00:00Z", "title": "Familie",
                       "kind": "ics", "color": "#3b82f6", "enabled": true,
                       "refresh": 15, "position": 0});
    other
        .call(
            "POST",
            "/changes?stamp=server",
            Some(
                &json!([{"op": "create", "object": calendar, "kind": "calendar",
                          "value": value, "hlc": {"wall": 0, "counter": 0, "device": "x"}}]),
            ),
        )
        .unwrap();
    other
        .call(
            "PUT",
            &format!("/calendars/{calendar}/source"),
            Some(&json!({"url": url})),
        )
        .unwrap();
    let refreshed = other
        .call("POST", &format!("/calendars/{calendar}/refresh"), None)
        .unwrap()
        .json()
        .unwrap();
    assert!(refreshed.get("error").is_none(), "{refreshed}");
    calendar
}

/// ADR 0101 §6, §7, §9: the phone reminds of appointments from their
/// VALARMs only when switched on here, once per occurrence, with time and
/// place; "later" brings the occurrence back while it is there; cancelled
/// appointments and switched-off calendars stay quiet; the sheet names
/// the alarms whether the switch is on or not.
#[test]
fn appointments_remind_on_the_phone_only_when_switched_on() {
    let server = Server::start();
    let other = server.other_client();
    let alarm = |trigger: &str| {
        format!(
            "BEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:x\r\nTRIGGER:{trigger}\r\nEND:VALARM"
        )
    };
    let samu = phone(&server);
    let (zone, tomorrow, second) = (samu.status().unwrap().zone, day(&samu, 1), day(&samu, 2));
    let compact = |d: &str| d.replace('-', "");
    // An instant at a time of a day in the account's zone.
    let at = |date: &str, time: &str| -> i64 {
        format!("{date}T{time}[{zone}]")
            .parse::<jiff::Zoned>()
            .unwrap()
            .timestamp()
            .as_millisecond()
    };
    let ics = [
        "BEGIN:VCALENDAR".to_string(),
        "BEGIN:VEVENT".into(),
        "UID:dentist@example.org".into(),
        "SUMMARY:Zahnarzt".into(),
        "LOCATION:Hauptstraße 1".into(),
        format!("DTSTART;TZID={zone}:{}T100000", compact(&tomorrow)),
        format!("DTEND;TZID={zone}:{}T104500", compact(&tomorrow)),
        "RRULE:FREQ=DAILY;COUNT=2".into(),
        alarm("-PT15M"),
        "END:VEVENT".into(),
        "BEGIN:VEVENT".into(),
        "UID:cancelled@example.org".into(),
        "SUMMARY:Abgesagt".into(),
        "STATUS:CANCELLED".into(),
        format!("DTSTART;TZID={zone}:{}T110000", compact(&tomorrow)),
        format!("DTEND;TZID={zone}:{}T113000", compact(&tomorrow)),
        alarm("-PT15M"),
        "END:VEVENT".into(),
        "END:VCALENDAR".into(),
        String::new(),
    ]
    .join("\r\n");
    let calendar = subscribe(&other, &serve_ics(ics));
    samu.sync().unwrap();

    // Off unless switched on: nothing to set, nothing to show.
    let t0 = at(&tomorrow, "09:00");
    assert!(!samu.appointment_reminders().unwrap());
    assert_eq!(samu.next_alarm(t0).unwrap(), None);
    assert!(samu.fire(at(&tomorrow, "09:50")).unwrap().is_empty());

    samu.set_appointment_reminders(true, at(&tomorrow, "09:50"))
        .unwrap();
    assert!(samu.appointment_reminders().unwrap());
    assert_eq!(samu.outbox_count().unwrap(), 0, "the switch is not synced");
    // What went off before switching on is not shown afterwards.
    assert!(samu.fire(at(&tomorrow, "09:51")).unwrap().is_empty());
    let first = at(&second, "09:45");
    assert_eq!(
        samu.next_alarm(at(&tomorrow, "09:51")).unwrap(),
        Some(first)
    );

    let fired = samu.fire(first + 9).unwrap();
    assert_eq!(owners(&fired), [("Zahnarzt", first)]);
    let notice = &fired[0];
    assert!(!notice.is_task);
    assert!(notice.key.ends_with(&format!("@{}", at(&second, "10:00"))));
    let row = notice.appointment.as_ref().unwrap();
    assert_eq!(row.start.as_deref(), Some("10:00"));
    assert_eq!(row.location.as_deref(), Some("Hauptstraße 1"));
    assert_eq!(row.date, second);
    let fifteen = samu_mobile::AlarmRow {
        before_minutes: Some(15),
        end: false,
        at_ms: None,
    };
    assert_eq!(row.alarms, std::slice::from_ref(&fifteen));
    assert!(samu.fire(first + 60_000).unwrap().is_empty(), "shown once");
    assert_eq!(
        samu.next_alarm(first + 60_000).unwrap(),
        None,
        "the series is over, the cancelled one quiet"
    );

    // Later: the occurrence an hour on.
    samu.snooze(notice.key.clone(), 60, first + 60_000).unwrap();
    let hour = first + 61 * 60_000;
    assert_eq!(samu.next_alarm(first + 60_000).unwrap(), Some(hour));
    let again = samu.fire(hour).unwrap();
    assert_eq!(owners(&again), [("Zahnarzt", hour)]);
    assert_eq!(again[0].key, notice.key);

    // The sheet, opened from the notice, names the alarm also when off.
    samu.set_appointment_reminders(false, hour).unwrap();
    let sheet = samu.appointment(notice.key.clone()).unwrap().unwrap();
    assert_eq!(sheet.title, "Zahnarzt");
    assert_eq!(sheet.alarms, [fifteen]);

    // A snooze of a calendar switched off in the meantime stays quiet.
    samu.set_appointment_reminders(true, hour).unwrap();
    samu.snooze(notice.key.clone(), 60, hour).unwrap();
    samu.set_calendar(calendar, "enabled".into(), "off".into())
        .unwrap();
    assert!(samu.fire(hour + 61 * 60_000).unwrap().is_empty());
    assert_eq!(samu.appointment(notice.key.clone()).unwrap(), None);
    assert!(samu.appointment("nothing".into()).is_err());
}

/// ADR 0047 §10: after a sync the phone shows the appointments in Forecast
/// on their days, computed offline by samu-core, before the tasks; with
/// the setting off (offline too), none.
#[test]
fn forecast_shows_the_calendars_appointments_offline() {
    let server = Server::start();
    let other = server.other_client();
    let samu = phone(&server);
    let (today, tomorrow, second, third) =
        (day(&samu, 0), day(&samu, 1), day(&samu, 2), day(&samu, 3));
    subscribe(&other, &ics_server(&today, &tomorrow, &third));
    make_task(&other, "heute fällig", json!({"due": {"date": today}}));
    samu.sync().unwrap();

    let days = samu.forecast(None).unwrap();
    let on = |date: &str| -> Vec<String> {
        days.iter()
            .find(|d| d.kind == "day" && d.date.as_deref() == Some(date))
            .map(|d| d.appointments.iter().map(|a| a.title.clone()).collect())
            .unwrap_or_default()
    };
    assert_eq!(on(&today), ["Reise"]);
    assert_eq!(on(&tomorrow), ["Reise", "Zahnarzt"]);
    assert_eq!(on(&second), ["Reise"]);
    assert!(on(&third).is_empty());
    let today_day = days.iter().find(|d| d.today).unwrap();
    assert_eq!(titles(&today_day.rows), ["heute fällig"]);
    let dentist = days
        .iter()
        .flat_map(|d| d.appointments.iter())
        .find(|a| a.title == "Zahnarzt")
        .unwrap();
    assert_eq!(dentist.start.as_deref(), Some("09:00"));
    assert_eq!(dentist.end.as_deref(), Some("09:45"));
    assert!(!dentist.all_day);
    assert_eq!(dentist.calendar, "Familie");
    assert_eq!(dentist.color.as_deref(), Some("#3b82f6"));
    assert_eq!(dentist.location.as_deref(), Some("Hauptstraße 1"));
    assert_eq!(
        dentist.link.as_deref(),
        Some("https://calendar.example.org/event/dentist")
    );
    assert!(samu.forecast_settings().unwrap().events);

    // Off, through the outbox, before any sync.
    samu.set_forecast_setting("events".into(), "off".into())
        .unwrap();
    assert!(!samu.forecast_settings().unwrap().events);
    let quiet = samu.forecast(None).unwrap();
    assert!(quiet.iter().all(|d| d.appointments.is_empty()));
}

/// ADR 0047 §11, ADR 0049: the calendar view on the phone, offline after a
/// sync. A day shows its timed appointments and the tasks planned at a
/// time as blocks, overlapping ones side by side; whole days in the band on
/// each of their days; the week starts on Monday; each appointment carries
/// its UID and instants for the calendar app.
#[test]
fn calendar_view_lays_out_appointments_and_planned_tasks() {
    let server = Server::start();
    let other = server.other_client();
    let samu = phone(&server);
    let (today, tomorrow, third) = (day(&samu, 0), day(&samu, 1), day(&samu, 3));
    subscribe(&other, &ics_server(&today, &tomorrow, &third));
    let zone = samu.status().unwrap().zone;
    // Overlaps the dentist (09:00–09:45): 09:30 for its estimate of an hour.
    make_task(
        &other,
        "Anrufen",
        json!({"planned": {"at": format!("{tomorrow}T09:30:00+01:00[{zone}]")},
               "estimate": "PT1H"}),
    );
    // Later the same day, without any length: 30 minutes.
    make_task(
        &other,
        "Lesen",
        json!({"planned": {"at": format!("{tomorrow}T14:00:00+01:00[{zone}]")}}),
    );
    // Planned for a whole day: no block.
    make_task(&other, "Irgendwann", json!({"planned": {"date": tomorrow}}));
    samu.sync().unwrap();

    let page = samu.calendar(Some(tomorrow.clone()), false).unwrap();
    assert_eq!(page.days.len(), 1);
    assert_eq!(page.today, today);
    assert_eq!(page.previous, today);
    let d = &page.days[0];
    assert_eq!(d.date, tomorrow);
    assert!(!d.today && d.now.is_none());
    assert_eq!(
        d.all_day
            .iter()
            .map(|a| a.title.as_str())
            .collect::<Vec<_>>(),
        ["Reise"]
    );
    let what = |b: &samu_mobile::Block| {
        b.appointment
            .as_ref()
            .map(|a| a.title.clone())
            .or_else(|| b.task.as_ref().map(|t| t.title.clone()))
            .unwrap()
    };
    let blocks: Vec<_> = d
        .blocks
        .iter()
        .map(|b| (what(b), b.top, b.bottom, b.lane, b.lanes, b.label.clone()))
        .collect();
    assert_eq!(
        blocks,
        [
            ("Zahnarzt".into(), 540, 585, 0, 2, "09:00".into()),
            ("Anrufen".into(), 570, 630, 1, 2, "09:30".into()),
            ("Lesen".into(), 840, 870, 0, 1, "14:00".into()),
        ]
    );
    let dentist = d.blocks[0].appointment.as_ref().unwrap();
    assert_eq!(dentist.uid, "dentist@example.org");
    assert_eq!(dentist.date, tomorrow);
    assert_eq!(dentist.end_ms - dentist.begin_ms, 45 * 60 * 1000);
    assert_eq!(
        d.start_ms + 540 * 60 * 1000,
        dentist.begin_ms,
        "a minute of the grid is the day's start plus the minute"
    );
    let trip = &d.all_day[0];
    assert_eq!(trip.uid, "trip@example.org");
    // Whole days at UTC midnight, the end excluded: three days.
    assert_eq!(trip.begin_ms % 86_400_000, 0);
    assert_eq!(trip.end_ms - trip.begin_ms, 3 * 86_400_000);

    // The week: seven days from Monday, the trip on each of its days.
    let week = samu.calendar(Some(tomorrow.clone()), true).unwrap();
    assert_eq!(week.days.len(), 7);
    let monday: jiff::civil::Date = week.days[0].date.parse().unwrap();
    assert_eq!(monday.weekday(), jiff::civil::Weekday::Monday);
    let with_trip = week
        .days
        .iter()
        .filter(|d| d.all_day.iter().any(|a| a.title == "Reise"))
        .count();
    let in_week = [day(&samu, 0), day(&samu, 1), day(&samu, 2)]
        .iter()
        .filter(|d| week.days.iter().any(|w| &&w.date == d))
        .count();
    assert_eq!(with_trip, in_week);
    let shown: jiff::civil::Date = week.next.parse().unwrap();
    assert_eq!(shown, monday.checked_add(jiff::ToSpan::days(7)).unwrap());

    // Today without a day; the line on it.
    let now = samu.calendar(None, false).unwrap();
    assert_eq!(now.days[0].date, today);
    assert!(now.days[0].now.is_some());
    assert!(samu.calendar(Some("morgen".into()), false).is_err());
}

/// A mirror pulled by an app built against an older samu-api lost the kinds
/// that one did not know (the phone of 09.11.2026 had no calendars or
/// appointments after the update). The next sync of a newer app loads
/// everything once, and only once.
#[test]
fn a_mirror_from_another_samu_api_is_loaded_again() {
    let server = Server::start();
    let other = server.other_client();
    let samu = phone(&server);
    make_task(&other, "eins", json!({}));
    samu.sync().unwrap();
    assert_eq!(titles(&samu.inbox(None).unwrap()[0].rows), ["eins"]);

    // What the old app left: the task gone from the mirror, the change
    // number past it, another samu-api.
    let db = server.dir.join("phone").join("samu.db");
    {
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute("DELETE FROM object", []).unwrap();
        conn.execute("UPDATE meta SET value = '0.1.0' WHERE key = 'api'", [])
            .unwrap();
    }
    assert!(samu.inbox(None).unwrap().is_empty());
    let report = samu.sync().unwrap();
    assert!(report.pulled >= 1);
    assert_eq!(titles(&samu.inbox(None).unwrap()[0].rows), ["eins"]);

    // The same samu-api: only what changed.
    let again = samu.sync().unwrap();
    assert_eq!(again.pulled, 0);
}

/// ADR 0052 §6: the bar of the app moves, sets a state by its title and
/// deletes many tasks in one go; a picked child goes with its parent.
#[test]
fn many_tasks_move_change_state_and_go_together() {
    let server = Server::start();
    let other = server.other_client();
    let garden = make_project(&other, "Garten");
    let status = make_tag(&other, "Status", None);
    other
        .call(
            "POST",
            "/changes?stamp=server",
            Some(&json!([
                {"op": "set", "object": status, "field": "exclusive", "value": true,
                 "hlc": {"wall": 0, "counter": 0, "device": "x"}},
                {"op": "set", "object": status, "field": "default_workflow", "value": true,
                 "hlc": {"wall": 0, "counter": 0, "device": "x"}}
            ])),
        )
        .unwrap();
    let running = make_tag(&other, "Läuft", Some(&status));
    let phone_tag = make_tag(&other, "Telefon", None);
    make_task(
        &other,
        "Schon da",
        json!({"project": garden, "position": 3}),
    );
    let group = make_task(&other, "Zaun", json!({}));
    let child = make_task(&other, "Farbe", json!({"parent": group}));
    let alone = make_task(&other, "Hecke", json!(tagged(&phone_tag)));
    let samu = phone(&server);
    samu.sync().unwrap();

    let picked = vec![child.clone(), group.clone(), alone.clone()];
    let edit = samu.bulk_edit(picked.clone()).unwrap();
    assert_eq!(edit.states, ["Läuft"]);
    assert_eq!(edit.deleted, 3, "the child is counted once");
    let telefon = edit.tags.iter().find(|t| t.id == phone_tag).unwrap();
    assert!(!telefon.on, "not every picked task carries it");

    samu.move_tasks(picked.clone(), Some(garden.clone()))
        .unwrap();
    assert_eq!(
        samu.set_states_titled(picked.clone(), Some("Läuft".into()))
            .unwrap(),
        0
    );
    assert_eq!(
        samu.set_states_titled(vec![alone.clone()], Some("Fertig".into()))
            .unwrap(),
        1
    );
    samu.sync().unwrap();

    assert_eq!(stored_field(&other, &group, "project"), garden.as_str());
    assert_eq!(stored_field(&other, &group, "position"), 4);
    assert_eq!(stored_field(&other, &alone, "position"), 5);
    assert_eq!(stored_field(&other, &child, "project"), garden.as_str());
    assert_eq!(
        stored_field(&other, &child, "parent"),
        group.as_str(),
        "it stays under its parent"
    );
    for id in [&group, &child, &alone] {
        let tags = stored_field(&other, id, "tags");
        assert!(
            tags.as_array()
                .unwrap()
                .iter()
                .any(|t| t["tag"] == running.as_str()),
            "{id}: {tags}"
        );
    }

    assert_eq!(samu.delete_tasks(picked.clone(), true).unwrap().deleted, 3);
    assert_eq!(samu.delete_tasks(picked, false).unwrap().deleted, 3);
    for id in [&group, &child, &alone] {
        assert!(other.call("GET", &format!("/objects/{id}"), None).is_err());
    }
}

/// ADR 0047 §8, §13: the calendars in the app's settings. Title, colour and
/// on/off change offline through the outbox and reach the server; an
/// unchecked calendar leaves Forecast at once; fetching now is online and
/// brings the appointments.
#[test]
fn calendars_are_set_offline_and_fetched_online() {
    let server = Server::start();
    let other = server.other_client();
    let samu = phone(&server);
    let (today, tomorrow, third) = (day(&samu, 0), day(&samu, 1), day(&samu, 3));
    let url = ics_server(&today, &tomorrow, &third);
    let calendar = samu_api::Id::new().to_string();
    let value = json!({"id": calendar, "created": "2026-11-09T08:00:00Z",
                       "modified": "2026-11-09T08:00:00Z", "title": "Familie",
                       "kind": "ics", "enabled": true, "refresh": 15, "position": 0});
    other
        .call(
            "POST",
            "/changes?stamp=server",
            Some(
                &json!([{"op": "create", "object": calendar, "kind": "calendar",
                          "value": value, "hlc": {"wall": 0, "counter": 0, "device": "x"}}]),
            ),
        )
        .unwrap();
    // The phone syncs before the calendar has a source: setting one makes
    // the server fetch at once in the background, which raced a sync after
    // it (93d757d6).
    samu.sync().unwrap();
    other
        .call(
            "PUT",
            &format!("/calendars/{calendar}/source"),
            Some(&json!({"url": url})),
        )
        .unwrap();

    let rows = samu.calendars().unwrap();
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!((row.title.as_str(), row.kind.as_str()), ("Familie", "ics"));
    assert!(row.enabled && row.color.is_none() && row.error.is_none());
    assert_eq!(row.appointments, 0, "not fetched yet");

    // Fetching now brings the appointments, and where they come from, to
    // the phone.
    assert_eq!(samu.refresh_calendar(calendar.clone()).unwrap(), None);
    let row = &samu.calendars().unwrap()[0];
    assert!(row.appointments > 0);
    assert_eq!(row.host.as_deref(), Some("127.0.0.1"));
    let shown = |samu: &Samu| -> usize {
        samu.forecast(None)
            .unwrap()
            .iter()
            .map(|d| d.appointments.len())
            .sum()
    };
    assert!(shown(&samu) > 0);

    // Offline: the server is told only on the next sync.
    samu.set_calendar(calendar.clone(), "title".into(), " Zuhause ".into())
        .unwrap();
    samu.set_calendar(calendar.clone(), "color".into(), "#10B981".into())
        .unwrap();
    samu.set_calendar(calendar.clone(), "enabled".into(), "off".into())
        .unwrap();
    let row = &samu.calendars().unwrap()[0];
    assert_eq!(row.title, "Zuhause");
    assert_eq!(row.color.as_deref(), Some("#10b981"));
    assert!(!row.enabled);
    assert_eq!(shown(&samu), 0, "an unchecked calendar leaves Forecast");
    assert_eq!(stored_field(&other, &calendar, "title"), json!("Familie"));

    samu.sync().unwrap();
    assert_eq!(samu.outbox_count().unwrap(), 0);
    assert_eq!(stored_field(&other, &calendar, "title"), json!("Zuhause"));
    assert_eq!(stored_field(&other, &calendar, "color"), json!("#10b981"));
    assert_eq!(stored_field(&other, &calendar, "enabled"), json!(false));

    // No colour again; and what is not a colour, a name or on/off is refused.
    samu.set_calendar(calendar.clone(), "color".into(), "".into())
        .unwrap();
    assert!(samu.calendars().unwrap()[0].color.is_none());
    for (key, value) in [
        ("color", "blue"),
        ("title", "  "),
        ("enabled", "maybe"),
        ("refresh", "5"),
    ] {
        assert!(
            samu.set_calendar(calendar.clone(), key.into(), value.into())
                .is_err(),
            "{key} {value}"
        );
    }
    assert!(
        samu.set_calendar(
            samu_api::Id::new().to_string(),
            "enabled".into(),
            "on".into()
        )
        .is_err()
    );
}

/// 21df91a1, ADR 0026 §6, §7: where a task lives, with the whole folder
/// path: on each row („Projekt · Ordner/Unterordner“, none in the inbox),
/// at the project's heading, and in the project list.
#[test]
fn a_task_says_where_it_lives_with_the_whole_folder_path() {
    let server = Server::start();
    let samu = phone(&server);
    let privat = samu.create_folder("Privat".into(), None).unwrap();
    let wohnung = samu
        .create_folder("Wohnung".into(), Some(privat.clone()))
        .unwrap();
    let bad = samu
        .create_project("Neues Bad".into(), Some(wohnung))
        .unwrap();
    samu.create_project("Lose".into(), None).unwrap();
    samu.capture("Fliesen aussuchen #neues ! d:heute".into(), None)
        .unwrap();
    samu.capture("Etwas #lose".into(), None).unwrap();
    samu.capture("Milch ! d:heute".into(), None).unwrap();
    samu.sync().unwrap();

    let place = |title: &str| -> Option<String> {
        samu.search(title.into(), None)
            .unwrap()
            .into_iter()
            .flat_map(|s| s.rows)
            .find(|r| r.title == title)
            .unwrap()
            .place
    };
    assert_eq!(
        place("Fliesen aussuchen").as_deref(),
        Some("Neues Bad · Privat/Wohnung")
    );
    assert_eq!(place("Etwas").as_deref(), Some("Lose"));
    assert_eq!(place("Milch"), None, "the inbox");
    // Forecast mixes projects, so its rows carry the place too.
    let today: Vec<Option<String>> = samu
        .forecast(None)
        .unwrap()
        .into_iter()
        .find(|d| d.today)
        .unwrap()
        .rows
        .into_iter()
        .map(|r| r.place)
        .collect();
    assert!(today.contains(&Some("Neues Bad · Privat/Wohnung".into())));

    let sections = samu.available(None).unwrap();
    let head = sections
        .iter()
        .find(|s| s.project_id.as_deref() == Some(bad.as_str()))
        .unwrap();
    assert_eq!(head.folder.as_deref(), Some("Privat/Wohnung"));
    assert!(
        sections
            .iter()
            .any(|s| s.project_id.is_none() && s.folder.is_none())
    );
    // The tab „Projekte“ shows the place as the tree (ADR 0085).
    let titles: Vec<(u32, String)> = samu
        .project_tree()
        .unwrap()
        .into_iter()
        .map(|p| (p.depth, p.title))
        .collect();
    let at = titles.iter().position(|(_, t)| t == "Privat").unwrap();
    assert_eq!(
        titles[at..at + 3],
        [
            (0, "Privat".into()),
            (1, "Wohnung".into()),
            (2, "Neues Bad".into())
        ]
    );
}

/// ADR 0053, 0130, 0131: a row carries what its lines need (Start, Geplant, the
/// estimate as written, the note's first line, the repetition, when it
/// was made and changed); the layout is this phone's own and stays across
/// opening the cache again.
#[test]
fn a_row_has_its_fields_and_the_layout_stays_on_the_phone() {
    let server = Server::start();
    let samu = phone(&server);
    samu.capture(
        "Plan zeichnen s:2026-12-01 p:2026-12-02 ~1h30m every:2w // \n  Maße vom Bad  \nzweite Zeile".into(),
        None,
    )
    .unwrap();
    samu.capture("Milch".into(), None).unwrap();
    samu.sync().unwrap();
    let rows: Vec<samu_mobile::TaskRow> = samu
        .inbox(None)
        .unwrap()
        .into_iter()
        .flat_map(|s| s.rows)
        .collect();
    let plan = rows.iter().find(|r| r.title == "Plan zeichnen").unwrap();
    assert_eq!(plan.defer.as_deref(), Some("2026-12-01"));
    assert_eq!(plan.planned.as_deref(), Some("2026-12-02"));
    assert_eq!(plan.estimate.as_deref(), Some("PT1H30M"));
    assert!(plan.has_note);
    assert_eq!(plan.note_line.as_deref(), Some("Maße vom Bad"));
    let repeat = plan.repeat.as_ref().expect("every:2w repeats");
    assert_eq!(
        (repeat.freq.as_deref(), repeat.interval),
        (Some("WEEKLY"), 2)
    );
    assert!(plan.created_ms > 0 && plan.modified_ms >= plan.created_ms);
    let milk = rows.iter().find(|r| r.title == "Milch").unwrap();
    assert!(milk.estimate.is_none() && !milk.has_note && milk.note_line.is_none());
    assert!(milk.repeat.is_none());

    // ADR 0131 §6: the state before the title, the due date beside it.
    let start = samu.row_fields().unwrap();
    assert!(start.short);
    assert_eq!(start.line1, ["state", "title", "due"]);
    assert_eq!(start.line2, ["tags"]);
    assert_eq!(samu.row_fields_default(), start);
    let tags_first = samu_mobile::RowFields {
        line1: vec!["tags".into(), "title".into()],
        line2: vec!["state".into(), "due".into()],
        short: false,
    };
    assert_eq!(samu.set_row_fields(tags_first.clone()).unwrap(), tags_first);
    drop(samu);
    let again = Samu::open(server.dir.join("phone").display().to_string()).unwrap();
    assert_eq!(again.row_fields().unwrap(), tags_first);
}

/// ADR 0054: the phone shows own and inherited properties with where they
/// lead, sets one offline, sets a template, and the server takes both.
#[test]
fn properties_inherit_lead_somewhere_and_are_set_offline() {
    let server = Server::start();
    let other = server.other_client();
    let group = make_task(
        &other,
        "Dachrinne",
        json!({"properties": {"jira": "HAUS-12", "source": "org-import"}}),
    );
    let child = make_task(
        &other,
        "abdichten",
        json!({"parent": group, "properties": {"doc": "https://wiki.example.org/rinne"}}),
    );
    let samu = phone(&server);
    samu.sync().unwrap();

    let rows = samu.properties(child.clone()).unwrap();
    let keys: Vec<(&str, bool)> = rows.iter().map(|r| (r.key.as_str(), r.own)).collect();
    assert_eq!(
        keys,
        [("doc", true), ("jira", false)],
        "source does not inherit"
    );
    assert_eq!(
        rows[0].link.as_deref(),
        Some("https://wiki.example.org/rinne")
    );
    assert_eq!(rows[1].from.as_deref(), Some("Dachrinne"));
    assert_eq!(rows[1].link, None, "no template yet");

    samu.set_property_link(
        "jira".into(),
        Some("https://jira.example.org/browse/{value}".into()),
    )
    .unwrap();
    assert!(matches!(
        samu.set_property_link("jira".into(), Some("ohne Platzhalter".into())),
        Err(MobileError::Input { .. })
    ));
    let rows = samu.properties(child.clone()).unwrap();
    assert_eq!(
        rows[1].link.as_deref(),
        Some("https://jira.example.org/browse/HAUS-12")
    );

    samu.set_property(child.clone(), "Jira".into(), Some("HAUS-13".into()))
        .unwrap();
    assert!(matches!(
        samu.set_property(child.clone(), "a b".into(), Some("x".into())),
        Err(MobileError::Input { .. })
    ));
    let rows = samu.properties(child.clone()).unwrap();
    assert!(
        rows.iter()
            .any(|r| r.key == "jira" && r.own && r.value == "HAUS-13")
    );
    assert_eq!(samu.property_keys().unwrap(), ["doc", "jira"]);

    // Group rows: the system key last.
    let rows = samu.properties(group.clone()).unwrap();
    assert_eq!(
        rows.last().map(|r| (r.key.as_str(), r.system)),
        Some(("source", true))
    );

    samu.sync().unwrap();
    assert_eq!(samu.outbox_count().unwrap(), 0, "the server took it");
    assert_eq!(stored(&other, &child)["properties"]["jira"], "HAUS-13");
    samu.set_property(child.clone(), "doc".into(), None)
        .unwrap();
    samu.sync().unwrap();
    assert!(stored(&other, &child)["properties"].get("doc").is_none());
    assert_eq!(samu.property_links().unwrap().len(), 1);

    assert!(samu.properties_open().unwrap());
    samu.set_properties_open(false).unwrap();
    assert!(!samu.properties_open().unwrap());
}

/// ae5a3206: a blocker set on the phone waits in the outbox, makes the
/// task unavailable at once, reaches the server as a `blocked_by` link and
/// goes away again. The picker leaves out the task itself, its blocker and
/// one that would close a circle.
#[test]
fn a_blocker_set_on_the_phone_reaches_the_server() {
    let server = Server::start();
    let other = server.other_client();
    let dach = make_task(&other, "Dach decken", json!({}));
    let gerüst = make_task(&other, "Gerüst stellen", json!({}));
    let ziegel = make_task(&other, "Ziegel bestellen", json!({}));
    let samu = phone(&server);
    samu.sync().unwrap();

    let offered: Vec<String> = samu
        .blocker_targets(dach.clone(), String::new())
        .unwrap()
        .into_iter()
        .map(|r| r.title)
        .collect();
    assert_eq!(offered, ["Gerüst stellen", "Ziegel bestellen"]);
    let found = samu.blocker_targets(dach.clone(), "zieg".into()).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].task_id, ziegel);

    samu.add_blocker(dach.clone(), gerüst.clone()).unwrap();
    assert!(
        samu.add_blocker(dach.clone(), gerüst.clone()).is_err(),
        "twice"
    );
    assert!(
        samu.add_blocker(gerüst.clone(), dach.clone()).is_err(),
        "a circle"
    );
    assert!(
        samu.add_blocker(dach.clone(), dach.clone()).is_err(),
        "itself"
    );
    let blockers = samu.blockers(dach.clone()).unwrap();
    let held_up = samu.held_up(gerüst.clone()).unwrap();
    assert_eq!(held_up.len(), 1, "Gerüst holds up Dach");
    assert_eq!(held_up[0].task_id, dach);
    assert_eq!(held_up[0].link_id, blockers[0].link_id);
    assert!(samu.held_up(dach.clone()).unwrap().is_empty());
    assert_eq!(blockers.len(), 1);
    assert_eq!(blockers[0].title, "Gerüst stellen");
    assert!(!blockers[0].done && blockers[0].link_id.is_some());
    assert!(
        !samu.task(dach.clone()).unwrap().row.available,
        "at once, offline"
    );
    assert!(samu.task(gerüst.clone()).unwrap().row.blocks, "the 🚧");
    assert!(!samu.task(dach.clone()).unwrap().row.blocks);
    let for_gerüst: Vec<String> = samu
        .blocker_targets(gerüst.clone(), String::new())
        .unwrap()
        .into_iter()
        .map(|r| r.task_id)
        .collect();
    assert_eq!(
        for_gerüst,
        std::slice::from_ref(&ziegel),
        "Dach waits for Gerüst already"
    );

    let report = samu.sync().unwrap();
    assert_eq!((report.pushed, report.rejected), (1, 0));
    let link_id = blockers[0].link_id.clone().unwrap();
    let link = stored(&other, &link_id);
    assert_eq!(link["kind"], "blocked_by");
    assert_eq!(link["owner"], dach.as_str());
    assert_eq!(link["uri"], format!("samu:{gerüst}"));

    samu.remove_blocker(link_id.clone()).unwrap();
    assert!(samu.blockers(dach.clone()).unwrap().is_empty());
    assert!(samu.held_up(gerüst.clone()).unwrap().is_empty());
    assert!(!samu.task(gerüst.clone()).unwrap().row.blocks);
    let report = samu.sync().unwrap();
    assert_eq!((report.pushed, report.rejected), (1, 0));
    let all = other
        .call("GET", "/changes?after=0", None)
        .unwrap()
        .json()
        .unwrap();
    let ids: Vec<&str> = all["objects"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|o| o["object"]["id"].as_str())
        .collect();
    assert!(ids.contains(&dach.as_str()), "the probe finds objects");
    assert!(!ids.contains(&link_id.as_str()), "the link is gone");
    assert!(samu.task(dach).unwrap().row.available);
}

/// Habits on the phone (ADR 0092): an entry offline counts at once, two
/// add up, none of it completes the habit, and the widget's list passes no
/// habit with an own scope over.
#[test]
fn habit_entries_count_offline_and_never_complete_the_habit() {
    let server = Server::start();
    let other = server.other_client();
    let project = make_project(&other, "Guardrails");
    let water = make_task(
        &other,
        "Wasser",
        json!({"project": project, "position": 0,
               "habit": {"kind": "build", "times": 8, "per": "day", "unit": "Glas"}}),
    );
    let smoke = make_task(
        &other,
        "Rauchfrei",
        json!({"project": project, "position": 1, "habit": {"kind": "avoid"}}),
    );
    let sport = make_task(
        &other,
        "Sport",
        json!({"project": project, "position": 2,
               "habit": {"kind": "build", "times": 3, "per": "week"}}),
    );
    let plain = make_task(&other, "Milch kaufen", json!({"project": project}));
    let samu = phone(&server);
    samu.sync().unwrap();
    let habits = samu.create_scope("Habits".into(), true).unwrap();
    samu.set_own_scope(project.clone(), Some(habits)).unwrap();

    let lines = samu.habits().unwrap();
    let names: Vec<&str> = lines.iter().map(|l| l.title.as_str()).collect();
    assert_eq!(
        names,
        ["Wasser", "Rauchfrei", "Sport"],
        "an own scope stays in"
    );
    assert_eq!(lines[0].habit.target, 8);
    assert_eq!(lines[0].habit.unit.as_deref(), Some("Glas"));
    assert_eq!(lines[1].habit.kind, "avoid");
    assert_eq!(lines[2].habit.per, "week");
    assert!(samu.task(plain.clone()).unwrap().row.habit.is_none());
    // The bar's Habits: the same three as rows, under their project,
    // although the scope would hide them.
    let sections = samu.habit_list(None).unwrap();
    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].title.as_deref(), Some("Guardrails"));
    let rows: Vec<&str> = sections[0].rows.iter().map(|r| r.title.as_str()).collect();
    assert_eq!(rows, ["Wasser", "Rauchfrei", "Sport"]);
    assert!(sections[0].rows.iter().all(|r| r.habit.is_some()));
    // A tag none of them carries leaves the list empty.
    let unused = samu_api::Id::new().to_string();
    assert!(samu.habit_list(Some(unused)).unwrap().is_empty());
    let before = samu.outbox_count().unwrap();

    // Two taps offline: both wait in the outbox and count at once.
    samu.enter_habit(water.clone(), "done".into(), None, None)
        .unwrap();
    let second = samu
        .enter_habit(water.clone(), "done".into(), None, None)
        .unwrap();
    assert_eq!(samu.outbox_count().unwrap(), before + 2);
    let row = samu.task(water.clone()).unwrap().row;
    let habit = row.habit.clone().unwrap();
    assert_eq!(
        (habit.amount, habit.today.as_str(), habit.met),
        (2, "partial", false)
    );
    assert!(!row.done, "an entry never completes the habit");

    // What the phone refuses before the server does.
    assert!(
        samu.enter_habit(smoke.clone(), "done".into(), None, None)
            .is_err(),
        "avoid takes only a relapse or a pause"
    );
    assert!(
        samu.enter_habit(water.clone(), "done".into(), Some(day(&samu, 1)), None)
            .is_err(),
        "no day after today"
    );
    assert!(
        samu.enter_habit(water.clone(), "fail".into(), None, Some(2))
            .is_err(),
        "an amount only with done"
    );
    assert!(
        samu.enter_habit(plain, "done".into(), None, None).is_err(),
        "no habit"
    );

    // Entering a day before: the calendar shows it.
    samu.enter_habit(sport.clone(), "fail".into(), Some(day(&samu, -1)), None)
        .unwrap();
    let detail = samu.habit(sport.clone()).unwrap();
    assert_eq!(detail.days.len(), 12 * 7, "twelve whole weeks");
    let first: jiff::civil::Date = detail.days[0].day.parse().unwrap();
    assert_eq!(first.weekday(), jiff::civil::Weekday::Monday);
    let yesterday = day(&samu, -1);
    let marked = detail.days.iter().find(|d| d.day == yesterday).unwrap();
    assert_eq!(marked.state, "fail");
    assert!(
        detail
            .days
            .iter()
            .filter(|d| d.future)
            .all(|d| d.day > detail.today)
    );
    assert_eq!(detail.entries.len(), 1);
    assert!(detail.entries[0].pending);

    let report = samu.sync().unwrap();
    assert_eq!((report.pushed, report.rejected), (before + 3, 0));
    let server_side: Value = other
        .call("GET", &format!("/habits/{water}"), None)
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(server_side["period"]["amount"], 2, "{server_side}");
    assert!(stored(&other, &water).get("completed_at").is_none());

    // Taking a tap back.
    samu.remove_habit_entry(second).unwrap();
    assert_eq!(samu.habit(water.clone()).unwrap().row.amount, 1);
    samu.sync().unwrap();
    let server_side: Value = other
        .call("GET", &format!("/habits/{water}"), None)
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(server_side["period"]["amount"], 1, "{server_side}");
}

/// ADR 0093: a daily reminder on a habit goes off today only while today
/// is open; done today, it is next due tomorrow, and an `avoid` habit never
/// reminds.
#[test]
fn a_daily_reminder_on_a_habit_keeps_quiet_once_the_day_is_done() {
    use jiff::ToSpan;
    let server = Server::start();
    let other = server.other_client();
    let project = make_project(&other, "Guardrails");
    let stretch = make_task(
        &other,
        "Dehnen",
        json!({"project": project, "habit": {"kind": "build", "times": 1, "per": "day"}}),
    );
    let smoke = make_task(
        &other,
        "Rauchfrei",
        json!({"project": project, "habit": {"kind": "avoid"}}),
    );
    let samu = phone(&server);
    samu.sync().unwrap();

    samu.add_daily_reminder(stretch.clone(), 20 * 60).unwrap();
    samu.add_daily_reminder(smoke.clone(), 20 * 60).unwrap();
    assert!(samu.add_daily_reminder(stretch.clone(), 24 * 60).is_err());

    let zone = jiff::tz::TimeZone::get(&samu.status().unwrap().zone).unwrap();
    let now = jiff::Timestamp::now();
    let today = now.to_zoned(zone.clone()).date();
    let at = |date: jiff::civil::Date| {
        date.at(20, 0, 0, 0)
            .to_zoned(zone.clone())
            .unwrap()
            .timestamp()
            .as_millisecond()
    };
    let tomorrow = at(today.checked_add(1.day()).unwrap());
    let first = if at(today) > now.as_millisecond() {
        at(today)
    } else {
        tomorrow
    };

    let rows = samu.reminders(stretch.clone()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].anchor, samu_mobile::Anchor::Daily);
    assert_eq!(rows[0].time_minutes, Some(20 * 60));
    assert_eq!(rows[0].goes_off_ms, Some(first));
    assert_eq!(samu.reminders(smoke.clone()).unwrap()[0].goes_off_ms, None);

    samu.enter_habit(stretch.clone(), "done".into(), None, None)
        .unwrap();
    assert_eq!(
        samu.reminders(stretch.clone()).unwrap()[0].goes_off_ms,
        Some(tomorrow)
    );

    // On the server it is the anchor `daily` with its time.
    samu.sync().unwrap();
    let id = &rows[0].id;
    let made = stored(&other, id);
    assert_eq!(made["anchor"], "daily");
    assert_eq!(made["time"], "20:00:00");
}

/// "Frei ab hier" (4c20d39e): a `done` on a day before the creation of a
/// habit to avoid moves where "frei seit" counts from; a plain `done` stays
/// refused.
#[test]
fn free_from_moves_the_beginning_of_an_avoid_habit() {
    let server = Server::start();
    let other = server.other_client();
    // Made just now, so that "frei seit" starts today.
    let now = jiff::Timestamp::now().to_string();
    let smoke = make_task(
        &other,
        "Rauchfrei",
        json!({"created": now, "modified": now, "habit": {"kind": "avoid"}}),
    );
    let water = make_task(
        &other,
        "Wasser",
        json!({"habit": {"kind": "build", "times": 8, "per": "day"}}),
    );
    let samu = phone(&server);
    samu.sync().unwrap();
    assert_eq!(samu.habit(smoke.clone()).unwrap().row.streak, 0);
    let before = samu.outbox_count().unwrap();

    // Offline: counts at once from that day, whole days up to yesterday.
    let from = day(&samu, -5);
    samu.free_from(smoke.clone(), from.clone()).unwrap();
    assert_eq!(samu.outbox_count().unwrap(), before + 1);
    let detail = samu.habit(smoke.clone()).unwrap();
    assert_eq!(detail.row.streak, 5);
    let entry = detail.entries.iter().find(|e| e.day == from).unwrap();
    assert_eq!((entry.status.as_str(), entry.pending), ("done", true));

    // What stays refused.
    assert!(
        samu.enter_habit(smoke.clone(), "done".into(), Some(from.clone()), None)
            .is_err(),
        "no plain done for avoid"
    );
    assert!(
        samu.free_from(smoke.clone(), day(&samu, 1)).is_err(),
        "no day after today"
    );
    assert!(
        samu.free_from(water.clone(), from.clone()).is_err(),
        "only for a habit to avoid"
    );
    assert_eq!(samu.outbox_count().unwrap(), before + 1);

    let report = samu.sync().unwrap();
    assert_eq!((report.pushed, report.rejected), (before + 1, 0));
    let server_side: Value = other
        .call("GET", &format!("/habits/{smoke}?from={from}"), None)
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(server_side["streak"], 5, "{server_side}");
    let entries = server_side["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "{server_side}");
    assert_eq!(
        (entries[0]["day"].as_str(), entries[0]["status"].as_str()),
        (Some(from.as_str()), Some("done"))
    );
    assert!(stored(&other, &smoke).get("completed_at").is_none());
}

fn make_perspective(client: &Client, title: &str, extra: Value) -> String {
    let id = samu_api::Id::new().to_string();
    let mut value = json!({"id": id, "created": "2026-11-08T08:00:00Z",
        "modified": "2026-11-08T08:00:00Z", "title": title, "position": 0});
    value
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let change = json!([{"op": "create", "object": id, "kind": "perspective", "value": value,
                         "hlc": {"wall": 0, "counter": 0, "device": "x"}}]);
    let outcomes = client
        .call("POST", "/changes?stamp=server", Some(&change))
        .unwrap()
        .json()
        .unwrap();
    assert_eq!(outcomes[0]["outcome"], "applied", "{outcomes}");
    id
}

fn section_titles(sections: &[samu_mobile::Section]) -> Vec<(Option<&str>, Vec<&str>)> {
    sections
        .iter()
        .map(|s| (s.title.as_deref(), titles(&s.rows)))
        .collect()
}

/// ADR 0102 §1: Heute, Markiert, Stuck and Review as the web has them.
#[test]
fn today_flagged_stuck_and_review_as_lists() {
    let server = Server::start();
    let other = server.other_client();
    let garden = make_project(&other, "Garten");
    make_project(&other, "Leer");
    let cellar = make_project(&other, "Keller");
    let changes = json!([
        {"op": "set", "object": cellar, "field": "review_interval", "value": "P1W",
         "hlc": {"wall": 0, "counter": 0, "device": "ignored"}},
        {"op": "set", "object": cellar, "field": "next_review", "value": "2020-01-01",
         "hlc": {"wall": 0, "counter": 0, "device": "ignored"}},
    ]);
    other
        .call("POST", "/changes?stamp=server", Some(&changes))
        .unwrap();
    let samu = phone(&server);
    let today = day(&samu, 0);
    make_task(&other, "anrufen", json!({"planned": {"date": today}}));
    make_task(
        &other,
        "fällig, nicht geplant",
        json!({"due": {"date": today}}),
    );
    make_task(
        &other,
        "jäten",
        json!({"project": garden, "planned": {"date": today}}),
    );
    make_task(
        &other,
        "mähen",
        json!({"project": garden, "flagged": true, "position": 1}),
    );
    make_task(
        &other,
        "gießen",
        json!({"project": garden, "flagged": true, "defer": {"date": "2999-01-01"}}),
    );
    let done = make_task(
        &other,
        "kehren",
        json!({"project": garden, "flagged": true, "planned": {"date": today}, "position": 2}),
    );
    make_task(&other, "aufräumen", json!({"project": cellar}));
    samu.sync().unwrap();

    assert_eq!(
        section_titles(&samu.today_list(None).unwrap()),
        [
            (None, vec!["anrufen"]),
            (Some("Garten"), vec!["jäten", "kehren"])
        ]
    );
    assert_eq!(
        section_titles(&samu.flagged(None).unwrap()),
        [(Some("Garten"), vec!["mähen", "kehren"])],
        "deferred is not available"
    );
    samu.complete(done).unwrap();
    assert_eq!(
        section_titles(&samu.today_list(None).unwrap()),
        [(None, vec!["anrufen"]), (Some("Garten"), vec!["jäten"])],
        "done leaves at once"
    );
    assert_eq!(
        section_titles(&samu.flagged(None).unwrap()),
        [(Some("Garten"), vec!["mähen"])]
    );

    let stuck: Vec<_> = samu.stuck().unwrap().into_iter().map(|p| p.title).collect();
    assert_eq!(stuck, ["Leer"]);

    let review = samu.review().unwrap();
    assert_eq!(review.len(), 1);
    assert_eq!(review[0].title, "Keller");
    assert_eq!(review[0].review.as_deref(), Some("2020-01-01"));
    assert_eq!(review[0].last_reviewed, None);
    assert_eq!(titles(&review[0].rows), ["aufräumen"]);
    samu.mark_reviewed(cellar).unwrap();
    assert!(samu.review().unwrap().is_empty(), "reviewed leaves at once");
}

/// ADR 0102 §4: a perspective's hits as GET /available?perspective= has
/// them, grouped and sorted as it says.
#[test]
fn a_perspective_shows_its_hits_as_a_list() {
    let server = Server::start();
    let other = server.other_client();
    let office = make_tag(&other, "Büro", None);
    let samu = phone(&server);
    let tomorrow = day(&samu, 1);
    let with = |extra: Value| {
        let mut value = tagged(&office);
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        value
    };
    make_task(&other, "Angebot schreiben", with(json!({})));
    make_task(
        &other,
        "Angebot prüfen",
        with(json!({"defer": {"date": "2999-01-01"}})),
    );
    make_task(&other, "Angebot privat", json!({}));
    make_task(&other, "Bericht", with(json!({"due": {"date": tomorrow}})));
    make_task(&other, "Alt", with(json!({"due": {"date": "2000-01-01"}})));
    let chip = json!([{"kind": "tag", "value": office}]);
    let offers = make_perspective(
        &other,
        "Angebote",
        json!({"chips": chip, "text": "angebot", "icon": "💼"}),
    );
    let remaining = make_perspective(
        &other,
        "Alle Angebote",
        json!({"chips": chip, "text": "angebot", "filter": "remaining", "position": 1}),
    );
    let by_due = make_perspective(
        &other,
        "Nach Fälligkeit",
        json!({"chips": chip, "arrange": {"group": "none", "sort": "due"}, "position": 2}),
    );
    let by_date = make_perspective(
        &other,
        "Nach Tag",
        json!({"chips": chip, "arrange": {"group": "date"}, "position": 3}),
    );
    samu.sync().unwrap();

    let list = samu.perspectives().unwrap();
    let names: Vec<_> = list.iter().map(|p| p.title.as_str()).collect();
    assert_eq!(
        names,
        ["Angebote", "Alle Angebote", "Nach Fälligkeit", "Nach Tag"]
    );
    assert_eq!(list[0].icon.as_deref(), Some("💼"));
    assert_eq!(list[0].chips.len(), 1);
    assert_eq!(list[0].chips[0].label, "@Büro");
    assert_eq!(list[0].text, "angebot");
    assert_eq!(
        (
            list[1].filter.as_str(),
            list[2].group.as_str(),
            list[2].sort.as_str()
        ),
        ("remaining", "none", "due")
    );

    assert_eq!(
        section_titles(&samu.perspective(offers, None).unwrap()),
        [(None, vec!["Angebot schreiben"])]
    );
    let mut all: Vec<_> = samu
        .perspective(remaining, None)
        .unwrap()
        .into_iter()
        .flat_map(|s| s.rows)
        .map(|r| r.title)
        .collect();
    all.sort();
    assert_eq!(all, ["Angebot prüfen", "Angebot schreiben"]);
    assert_eq!(
        section_titles(&samu.perspective(by_due, None).unwrap()),
        [(None, vec!["Alt", "Bericht", "Angebot schreiben"])],
        "what has no due date last"
    );
    let days = samu.perspective(by_date, None).unwrap();
    let heads: Vec<_> = days.iter().map(|s| s.title.as_deref().unwrap()).collect();
    assert_eq!(heads.len(), 3);
    assert_eq!((heads[0], heads[2]), ("Überfällig", "Ohne Datum"));
    assert!(heads[1].ends_with(&format!("{}.{}.", &tomorrow[8..10], &tomorrow[5..7])));
    assert!(
        samu.perspective(samu_api::Id::new().to_string(), None)
            .is_err()
    );
}

/// ADR 0102 §4, ADR 0116 §4: a block perspective shows every task of its
/// blocks once, and nothing else; before, the phone ignored the blocks and
/// showed every available task.
#[test]
fn a_block_perspective_shows_the_union_of_its_blocks() {
    let server = Server::start();
    let other = server.other_client();
    let office = make_tag(&other, "Büro", None);
    make_task(&other, "anrufen", tagged(&office));
    let mut both = tagged(&office);
    both["flagged"] = json!(true);
    make_task(&other, "beides", both);
    make_task(&other, "markiert", json!({"flagged": true, "position": 1}));
    make_task(&other, "nichts davon", json!({"position": 2}));
    let calls = make_perspective(
        &other,
        "Büro",
        json!({"chips": [{"kind": "tag", "value": office}]}),
    );
    let blocks = make_perspective(
        &other,
        "Morgens",
        json!({"blocks": [{"perspective": calls}, {"view": "flagged"}],
               "arrange": {"group": "none", "sort": "title"}, "position": 1}),
    );
    let samu = phone(&server);
    samu.sync().unwrap();

    assert_eq!(
        section_titles(&samu.perspective(blocks, None).unwrap()),
        [(None, vec!["anrufen", "beides", "markiert"])]
    );
}

/// ADR 0147 §1, §2: a block perspective block by block: each block with
/// its heading, its tasks in its own arrangement, a task only in the first
/// block that shows it, a note block with the note of its project, which
/// the phone can write.
#[test]
fn a_block_perspective_shows_its_blocks_one_by_one() {
    let server = Server::start();
    let other = server.other_client();
    let office = make_tag(&other, "Büro", None);
    let garden = make_project(&other, "Garten");
    make_task(&other, "anrufen", tagged(&office));
    let mut both = tagged(&office);
    both["flagged"] = json!(true);
    make_task(&other, "beides", both);
    make_task(&other, "markiert", json!({"flagged": true, "position": 1}));
    make_task(&other, "nichts davon", json!({"position": 2}));
    let calls = make_perspective(
        &other,
        "Büro",
        json!({"chips": [{"kind": "tag", "value": office}],
               "arrange": {"group": "none", "sort": "title", "desc": true}}),
    );
    let morning = make_perspective(
        &other,
        "Morgens",
        json!({"blocks": [{"note": garden}, {"perspective": calls},
                          {"view": "flagged", "title": "Wichtig"}],
               "position": 1}),
    );
    let samu = phone(&server);
    samu.sync().unwrap();

    let blocks = samu.blocks(morning.clone(), None, None).unwrap();
    let shown: Vec<_> = blocks
        .iter()
        .map(|b| (b.title.as_str(), b.count, section_titles(&b.sections)))
        .collect();
    assert_eq!(
        shown,
        [
            ("Garten", 0, vec![]),
            ("Büro", 2, vec![(None, vec!["beides", "anrufen"])]),
            ("Wichtig", 1, vec![(None, vec!["markiert"])]),
        ]
    );
    assert_eq!(blocks[0].object.as_deref(), Some(garden.as_str()));
    assert_eq!(blocks[0].note.as_deref(), Some(""));
    let opens = blocks[1].opens.as_ref().unwrap();
    assert_eq!(
        (opens.kind.as_str(), opens.key.as_str()),
        ("perspective", calls.as_str())
    );
    assert_eq!(blocks[2].opens.as_ref().unwrap().key, "flagged");
    assert!(blocks[1].note.is_none() && blocks[1].object.is_none());

    // Only what carries the tag, and the count with it.
    let tagged_only = samu
        .blocks(morning.clone(), Some(office.clone()), None)
        .unwrap();
    assert_eq!(
        tagged_only.iter().map(|b| b.count).collect::<Vec<_>>(),
        [0, 2, 0]
    );

    // The note block's note is written through the phone, to the project.
    samu.set_note(garden.clone(), Some("- [ ] Beete".into()))
        .unwrap();
    assert_eq!(
        samu.blocks(morning, None, None).unwrap()[0].note.as_deref(),
        Some("- [ ] Beete")
    );
    samu.sync().unwrap();
    assert_eq!(stored(&other, &garden)["note"], "- [ ] Beete");
}

/// ADR 0116 §9: a filter chosen for the moment holds for every block, the
/// fixed views included, in place of each one's own; without it each
/// keeps its own.
#[test]
fn a_block_perspective_takes_one_filter_for_the_moment() {
    let server = Server::start();
    let other = server.other_client();
    let office = make_tag(&other, "Büro", None);
    make_task(&other, "anrufen", tagged(&office));
    let mut later = tagged(&office);
    later["defer"] = json!({"date": "2999-01-01"});
    later["position"] = json!(1);
    make_task(&other, "später", later);
    let mut done = tagged(&office);
    done["completed_at"] = json!("2026-11-08T09:00:00Z");
    done["position"] = json!(2);
    make_task(&other, "erledigt", done);
    make_task(
        &other,
        "markiert später",
        json!({"flagged": true, "defer": {"date": "2999-01-01"}, "position": 3}),
    );
    let calls = make_perspective(
        &other,
        "Büro",
        json!({"chips": [{"kind": "tag", "value": office}],
               "arrange": {"group": "none", "sort": "title"}}),
    );
    let morning = make_perspective(
        &other,
        "Morgens",
        json!({"blocks": [{"perspective": calls}, {"view": "flagged"}], "position": 1}),
    );
    let samu = phone(&server);
    samu.sync().unwrap();

    let titles = |filter: Option<&str>| -> Vec<Vec<String>> {
        samu.blocks(morning.clone(), None, filter.map(str::to_owned))
            .unwrap()
            .iter()
            .map(|b| {
                let mut all: Vec<String> = section_titles(&b.sections)
                    .into_iter()
                    .flat_map(|(_, rows)| rows)
                    .map(str::to_owned)
                    .collect();
                all.sort();
                all
            })
            .collect()
    };
    let own: Vec<Vec<String>> = vec![vec!["anrufen".into()], vec![]];
    assert_eq!(titles(None), own, "each block its own filter");
    assert_eq!(titles(Some("available")), own);
    assert_eq!(
        titles(Some("remaining")),
        [vec!["anrufen", "später"], vec!["markiert später"]]
    );
    assert_eq!(
        titles(Some("all")),
        [
            vec!["anrufen", "erledigt", "später"],
            vec!["markiert später"]
        ]
    );
    assert!(samu.blocks(morning, None, Some("nonsense".into())).is_err());
}

/// ADR 0103 §5: a search saved as a perspective offline, its chips and
/// words apart, showing what the search showed, and on the server after
/// the next sync.
#[test]
fn a_search_is_saved_as_a_perspective_offline() {
    let server = Server::start();
    let other = server.other_client();
    let office = make_tag(&other, "Büro", None);
    make_perspective(&other, "Telefonate", json!({"position": 4}));
    make_task(&other, "Angebot schreiben", tagged(&office));
    let mut waiting = tagged(&office);
    waiting["defer"] = json!({"date": "2999-01-01"});
    make_task(&other, "Angebot prüfen", waiting);
    make_task(&other, "Angebot privat", json!({}));
    let samu = phone(&server);
    samu.sync().unwrap();
    let search = "@Büro  angebot ".to_owned();
    let mut found: Vec<_> = samu
        .search(search.clone(), None)
        .unwrap()
        .into_iter()
        .flat_map(|s| s.rows)
        .map(|r| r.title)
        .collect();
    found.sort();

    let refused = |title: &str, search: &str| {
        samu.create_perspective(title.into(), None, search.into())
            .is_err()
    };
    assert!(refused("  ", &search), "no name");
    assert!(
        refused("telefonate", &search),
        "a name taken, ignoring case"
    );
    assert!(refused("Leer", "  "), "neither chip nor word");
    let id = samu
        .create_perspective(" Angebote ".into(), Some("💼".into()), search)
        .unwrap();
    let list = samu.perspectives().unwrap();
    let saved = list.iter().find(|p| p.id == id).unwrap();
    assert_eq!(saved.title, "Angebote");
    assert_eq!(saved.icon.as_deref(), Some("💼"));
    let labels: Vec<_> = saved.chips.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(labels, ["@Büro"]);
    assert_eq!(
        (saved.text.as_str(), saved.filter.as_str()),
        ("angebot", "remaining")
    );
    assert_eq!(list.last().unwrap().id, id, "last in the order");
    let mut shown: Vec<_> = samu
        .perspective(id.clone(), None)
        .unwrap()
        .into_iter()
        .flat_map(|s| s.rows)
        .map(|r| r.title)
        .collect();
    shown.sort();
    assert_eq!(shown, ["Angebot prüfen", "Angebot schreiben"]);
    assert_eq!(shown, found, "what the search showed");

    assert_eq!(samu.status().unwrap().outbox, 1);
    samu.sync().unwrap();
    assert_eq!(samu.status().unwrap().outbox, 0, "the server took it");
    let there = stored(&other, &id);
    assert_eq!(there["title"], "Angebote");
    assert_eq!(there["filter"], "remaining");
    assert_eq!(there["text"], "angebot");
    assert_eq!(there["position"], 5);
    assert_eq!(there["chips"], json!([{"kind": "tag", "value": office}]));
}

/// ADR 0102 §3: the bar per phone, Inbox always in it, what is gone
/// leaves it silently.
#[test]
fn the_bar_is_set_per_phone_and_forgets_what_is_gone() {
    use samu_mobile::BarItem;
    let server = Server::start();
    let other = server.other_client();
    let garden = make_project(&other, "Garten");
    let office = make_tag(&other, "Büro", None);
    let calls = make_perspective(&other, "Telefonate", json!({"icon": "📞"}));
    let samu = phone(&server);
    samu.sync().unwrap();
    let keys = |samu: &Samu| -> Vec<(String, String, bool)> {
        samu.bar()
            .unwrap()
            .into_iter()
            .map(|e| (e.key, e.title, e.hidden))
            .collect()
    };
    let item = |kind: &str, key: &str, hidden: bool| BarItem {
        kind: kind.into(),
        key: key.into(),
        hidden,
    };

    let first: Vec<_> = samu.bar().unwrap().into_iter().map(|e| e.key).collect();
    assert_eq!(
        first,
        ["inbox", "available", "forecast", "projects", "search"]
    );
    assert_eq!(samu.bar_views().len(), 10);

    samu.set_bar(vec![
        item("view", "today", false),
        item("perspective", &calls, false),
        item("view", "inbox", true),
        item("project", &garden, false),
        item("tag", &office, true),
        item("view", "today", false),
    ])
    .unwrap();
    assert_eq!(
        keys(&samu),
        [
            ("today".into(), "Heute".into(), false),
            (calls.clone(), "Telefonate".into(), false),
            ("inbox".into(), "Inbox".into(), false),
            (garden.clone(), "Garten".into(), false),
            (office.clone(), "Büro".into(), true),
        ],
        "inbox stays shown, twice counts once"
    );
    assert_eq!(samu.bar().unwrap()[1].icon.as_deref(), Some("📞"));

    samu.set_bar(vec![item("view", "flagged", false)]).unwrap();
    let keys_only: Vec<_> = keys(&samu).into_iter().map(|k| k.0).collect();
    assert_eq!(keys_only, ["inbox", "flagged"], "inbox cannot leave");

    assert!(samu.set_bar(vec![item("view", "nothing", false)]).is_err());
    assert!(samu.set_bar(vec![item("tag", "not an id", false)]).is_err());
    assert!(
        samu.set_bar(vec![item(
            "project",
            &samu_api::Id::new().to_string(),
            false
        )])
        .is_err()
    );
    assert!(samu.set_bar(vec![item("widget", &garden, false)]).is_err());

    samu.set_bar(vec![
        item("project", &garden, false),
        item("tag", &office, false),
        item("perspective", &calls, false),
    ])
    .unwrap();
    let gone = json!([
        {"op": "set", "object": garden, "field": "status", "value": "dropped",
         "hlc": {"wall": 0, "counter": 0, "device": "ignored"}},
        {"op": "set", "object": office, "field": "status", "value": "dropped",
         "hlc": {"wall": 0, "counter": 0, "device": "ignored"}},
        {"op": "delete", "object": calls,
         "hlc": {"wall": 0, "counter": 0, "device": "ignored"}},
    ]);
    let outcomes: Value = other
        .call("POST", "/changes?stamp=server", Some(&gone))
        .unwrap()
        .json()
        .unwrap();
    assert!(
        outcomes
            .as_array()
            .unwrap()
            .iter()
            .all(|o| o["outcome"] == "applied"),
        "{outcomes}"
    );
    samu.sync().unwrap();
    let left: Vec<_> = keys(&samu).into_iter().map(|k| k.0).collect();
    assert_eq!(left, ["inbox"], "gone and dropped leave silently");
}

/// ADR 0062 §2–3, ADR 0102 §7: „Unteraufgabe anlegen“ in the app makes the
/// last child of the task, in its project, offline; a project the line
/// names wins; a closed task takes nothing under it.
#[test]
fn a_subtask_goes_last_under_its_parent_offline() {
    let server = Server::start();
    let other = server.other_client();
    let garden = make_project(&other, "Garten");
    let home = make_project(&other, "Haus");
    let fence = make_task(&other, "Zaun", json!({"project": garden}));
    make_task(
        &other,
        "Latten holen",
        json!({"project": garden, "parent": fence, "position": 0}),
    );
    let done = make_task(
        &other,
        "Alt",
        json!({"project": garden, "completed_at": "2026-11-08T09:00:00Z"}),
    );
    let samu = phone(&server);
    samu.sync().unwrap();

    let child = samu
        .capture_under("Streichen @farbe".into(), fence.clone())
        .unwrap();
    let elsewhere = samu
        .capture_under("Dach #Haus".into(), fence.clone())
        .unwrap();
    assert!(samu.capture_under("x".into(), done).is_err());
    assert!(samu.capture_under("  ".into(), fence.clone()).is_err());
    assert_eq!(samu.outbox_count().unwrap(), 4, "tag, task, add; task");

    samu.sync().unwrap();
    assert_eq!(samu.outbox_count().unwrap(), 0);
    let object = stored(&other, &child);
    assert_eq!(object["title"], "Streichen");
    assert_eq!(object["parent"], json!(fence));
    assert_eq!(object["project"], json!(garden));
    assert_eq!(object["position"], 1, "after Latten holen");
    assert_eq!(object["tags"].as_array().unwrap().len(), 1);
    let object = stored(&other, &elsewhere);
    assert_eq!(object["project"], json!(home));
    assert!(object.get("parent").is_none_or(Value::is_null), "{object}");
}

/// ADR 0129 in the app: several lines make a task each, offline, an
/// indented one under its line; the preview shows depths; a failing line
/// makes none.
#[test]
fn several_lines_make_a_task_each_offline() {
    let server = Server::start();
    let other = server.other_client();
    let garden = make_project(&other, "Garten");
    let samu = phone(&server);
    samu.sync().unwrap();

    let text = "Beet\n  - Erde @baumarkt\n  - Samen #Garten\nGießen";
    assert!(samu.is_several(text.into()));
    assert!(!samu.is_several("eine\n\n".into()));
    let seen = samu
        .preview_lines(text.into(), Some(garden.clone()))
        .unwrap();
    let depths: Vec<u32> = seen.iter().map(|l| l.depth).collect();
    assert_eq!(depths, [0, 1, 1, 0]);
    assert_eq!(seen[0].preview.project.as_deref(), Some("Garten"));
    assert_eq!(seen[1].preview.project, None, "it goes into its line's");
    assert_eq!(seen[2].preview.project.as_deref(), Some("Garten"));
    assert_eq!(samu.outbox_count().unwrap(), 0, "a preview writes nothing");

    let ids = samu
        .capture_lines(text.into(), Some(garden.clone()), None)
        .unwrap();
    assert_eq!(ids.len(), 4);
    assert!(
        samu.capture_lines(
            "geht\nnicht".into(),
            None,
            Some(samu_api::Id::new().to_string())
        )
        .is_err()
    );
    let queued = samu.outbox_count().unwrap();
    samu.sync().unwrap();
    assert_eq!(samu.outbox_count().unwrap(), 0, "{queued} sent");
    let beet = stored(&other, &ids[0]);
    assert_eq!(beet["title"], "Beet");
    assert_eq!(beet["project"], json!(garden));
    let erde = stored(&other, &ids[1]);
    assert_eq!(erde["parent"], json!(ids[0]));
    assert_eq!(erde["project"], json!(garden));
    assert_eq!(erde["tags"].as_array().unwrap().len(), 1);
    let giessen = stored(&other, &ids[3]);
    assert!(
        giessen.get("parent").is_none_or(Value::is_null),
        "{giessen}"
    );
}

/// ADR 0078 §1–3 in the app (ADR 0102 §7): the chips of a task, what a
/// line would add, free text refused, and saving title, removals and line
/// in one write that the server then holds.
#[test]
fn the_edit_dialog_takes_chips_off_and_applies_the_line_offline() {
    let server = Server::start();
    let other = server.other_client();
    let garden = make_project(&other, "Garten");
    let office = make_tag(&other, "Büro", None);
    let mut extra = tagged(&office);
    extra["project"] = json!(garden);
    extra["flagged"] = json!(true);
    extra["due"] = json!({"date": "2026-11-30"});
    let task = make_task(&other, "Zaun", extra);
    let samu = phone(&server);
    samu.sync().unwrap();

    let keys: Vec<String> = samu
        .edit_chips(task.clone())
        .unwrap()
        .into_iter()
        .map(|c| c.key)
        .collect();
    assert_eq!(
        keys,
        [
            "project".to_owned(),
            format!("tag:{office}"),
            "flagged".into(),
            "due".into()
        ]
    );

    let preview = samu
        .edit_preview(task.clone(), "@farbe d:2026-12-01 // grün".into())
        .unwrap();
    let added: Vec<(&str, &str)> = preview
        .added
        .iter()
        .map(|c| (c.kind.as_str(), c.value.as_str()))
        .collect();
    assert_eq!(
        added,
        [("due", "2026-12-01"), ("new_tag", "farbe"), ("note", "")]
    );
    assert!(preview.warnings.is_empty(), "{:?}", preview.warnings);
    let free = samu
        .edit_preview(task.clone(), "streichen @farbe".into())
        .unwrap();
    assert_eq!(free.warnings[0].code, "free_text");
    assert_eq!(free.warnings[0].text, "streichen");
    assert_eq!(samu.outbox_count().unwrap(), 0, "a preview writes nothing");

    assert!(
        samu.edit_save(task.clone(), "Zaun".into(), vec![], "streichen".into())
            .is_err()
    );
    assert_eq!(samu.outbox_count().unwrap(), 0, "refused writes nothing");
    samu.edit_save(
        task.clone(),
        " Zaun streichen ".into(),
        vec![format!("tag:{office}"), "flagged".into(), "project".into()],
        "@farbe d:2026-12-01".into(),
    )
    .unwrap();
    assert!(samu.outbox_count().unwrap() > 0);

    samu.sync().unwrap();
    assert_eq!(samu.outbox_count().unwrap(), 0);
    let object = stored(&other, &task);
    assert_eq!(object["title"], "Zaun streichen");
    assert_eq!(object["flagged"], false);
    assert!(object.get("project").is_none_or(Value::is_null), "{object}");
    assert_eq!(object["due"], json!({"date": "2026-12-01"}));
    let tags = object["tags"].as_array().unwrap();
    assert_eq!(tags.len(), 1, "{object}");
    assert_eq!(
        stored(&other, tags[0]["tag"].as_str().unwrap())["title"],
        "farbe"
    );
}

/// The tasks on the server that repeat `id`.
fn follow_ups(client: &Client, id: &str) -> usize {
    let page = client
        .call("GET", "/changes?after=0", None)
        .unwrap()
        .json()
        .unwrap();
    page["objects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|o| o["kind"] == "task" && o["object"]["repeat_of"] == json!(id))
        .count()
}

/// ADR 0104 §3, §4: „Rückgängig“ before the sync takes the completion back
/// out of the outbox, so the server never makes the next task; once sent,
/// `withdraw` says no and only a change against it is left.
#[test]
fn withdraw_takes_a_completion_back_before_the_server_sees_it() {
    let server = Server::start();
    let other = server.other_client();
    let repeat = json!({"due": {"date": "2099-01-31"},
                        "repeat": {"rule": "FREQ=MONTHLY;BYMONTHDAY=-1", "mode": "fixed"}});
    let task = make_task(&other, "Miete", repeat.clone());
    let samu = phone(&server);
    samu.sync().unwrap();

    let numbers = samu.complete(task.clone()).unwrap();
    assert_eq!(numbers.len(), 1);
    let flagged = samu.set_flag(task.clone(), true).unwrap();
    assert!(samu.withdraw(numbers.clone()).unwrap());
    assert!(samu.withdraw(flagged).unwrap());
    assert_eq!(samu.outbox_count().unwrap(), 0);
    assert!(!samu.withdraw(numbers).unwrap(), "taken already");
    samu.sync().unwrap();
    let stored_task = stored(&other, &task);
    assert!(stored_task["completed_at"].is_null(), "{stored_task}");
    assert_eq!(stored_task["flagged"], false);
    assert_eq!(follow_ups(&other, &task), 0);

    // The probe that must find something: sent, it is the server's.
    let sent = make_task(&other, "Strom", repeat);
    samu.sync().unwrap();
    let numbers = samu.drop_task(sent.clone()).unwrap();
    let done = samu.complete(sent.clone()).unwrap();
    samu.sync().unwrap();
    assert!(!samu.withdraw(numbers).unwrap());
    assert!(!samu.withdraw(done).unwrap());
    assert_eq!(follow_ups(&other, &sent), 1);
    samu.reopen(sent.clone()).unwrap();
    samu.sync().unwrap();
    assert!(stored(&other, &sent)["dropped_at"].is_null());
}

fn arranged(group: &str, sort: &str, then: Option<&str>, then_desc: bool) -> samu_mobile::Arrange {
    samu_mobile::Arrange {
        group: group.into(),
        sort: sort.into(),
        desc: false,
        then: then.map(Into::into),
        then_desc,
    }
}

/// ADR 0125: every list arranges as the web does, in groups and by two
/// keys; a view on this phone, a perspective through the outbox.
#[test]
fn lists_arrange_in_groups_and_by_two_keys() {
    let server = Server::start();
    let other = server.other_client();
    let status = make_tag(&other, "Status", None);
    other
        .call(
            "POST",
            "/changes?stamp=server",
            Some(&json!([
                {"op": "set", "object": status, "field": "exclusive", "value": true,
                 "hlc": {"wall": 0, "counter": 0, "device": "x"}},
                {"op": "set", "object": status, "field": "default_workflow", "value": true,
                 "hlc": {"wall": 0, "counter": 0, "device": "x"}}
            ])),
        )
        .unwrap();
    let running = make_tag(&other, "Läuft", Some(&status));
    let waiting = make_tag(&other, "Warten", Some(&status));
    other
        .call(
            "POST",
            "/changes?stamp=server",
            Some(
                &json!([{"op": "set", "object": waiting, "field": "position", "value": 1,
                          "hlc": {"wall": 0, "counter": 0, "device": "x"}}]),
            ),
        )
        .unwrap();
    let office = make_tag(&other, "Büro", None);
    let garden = make_project(&other, "Garten");
    let samu = phone(&server);
    let house = samu.create_folder("Haus".into(), None).unwrap();
    let cellar = samu
        .create_project("Keller".into(), Some(house.clone()))
        .unwrap();
    samu.sync().unwrap();
    let (tomorrow, later) = (day(&samu, 1), day(&samu, 3));
    make_task(&other, "Zettel", json!({}));
    make_task(
        &other,
        "jäten",
        json!({"project": garden, "priority": "B", "due": {"date": tomorrow},
               "tags": [{"tag": running, "position": 0}]}),
    );
    make_task(
        &other,
        "gießen",
        json!({"project": garden, "priority": "B", "due": {"date": later}}),
    );
    make_task(&other, "mähen", json!({"project": garden, "priority": "A"}));
    make_task(
        &other,
        "aufräumen",
        json!({"project": cellar, "priority": "A",
               "tags": [{"tag": waiting, "position": 0}, {"tag": office, "position": 1}]}),
    );
    samu.sync().unwrap();

    let default = samu.arrange("available".into()).unwrap();
    assert_eq!(default, arranged("project", "position", None, false));
    let first = samu.available(None).unwrap();
    let by_project = section_titles(&first);
    assert_eq!(
        by_project[0],
        (None, vec!["Zettel"]),
        "the inbox first, as before"
    );

    // Priority, then due late first: the second key decides within B.
    samu.set_arrange(
        "available".into(),
        arranged("priority", "priority", Some("due"), true),
    )
    .unwrap();
    assert_eq!(
        section_titles(&samu.available(None).unwrap()),
        [
            (Some("Priorität A"), vec!["aufräumen", "mähen"]),
            (Some("Priorität B"), vec!["gießen", "jäten"]),
            (Some("Ohne Priorität"), vec!["Zettel"]),
        ]
    );
    samu.set_arrange(
        "available".into(),
        arranged("none", "priority", Some("due"), false),
    )
    .unwrap();
    assert_eq!(
        section_titles(&samu.available(None).unwrap()),
        [(
            None,
            vec!["aufräumen", "mähen", "jäten", "gießen", "Zettel"]
        )],
        "what has no value last, on each level"
    );
    samu.set_arrange("available".into(), arranged("state", "title", None, false))
        .unwrap();
    assert_eq!(
        section_titles(&samu.available(None).unwrap()),
        [
            (Some("Läuft"), vec!["jäten"]),
            (Some("Warten"), vec!["aufräumen"]),
            (Some("Ohne Zustand"), vec!["gießen", "mähen", "Zettel"]),
        ],
        "in the order of the workflow"
    );
    samu.set_arrange("available".into(), arranged("folder", "title", None, false))
        .unwrap();
    assert_eq!(
        section_titles(&samu.available(None).unwrap()),
        [
            (Some("Inbox"), vec!["Zettel"]),
            (Some("Haus"), vec!["aufräumen"]),
            (Some("Ohne Ordner"), vec!["gießen", "jäten", "mähen"]),
        ]
    );
    samu.set_arrange("available".into(), arranged("tag", "title", None, false))
        .unwrap();
    assert_eq!(
        section_titles(&samu.available(None).unwrap()),
        [
            (Some("Büro"), vec!["aufräumen"]),
            (Some("Status/Läuft"), vec!["jäten"]),
            (Some("Status/Warten"), vec!["aufräumen"]),
            (Some("Ohne Tag"), vec!["gießen", "mähen", "Zettel"]),
        ],
        "a task with two tags stands in both"
    );

    // Each view on its own; a project by its id; the tag of the bar too.
    assert_eq!(samu.arrange("inbox".into()).unwrap(), default);
    samu.set_arrange(
        format!("project:{garden}"),
        arranged("none", "due", None, false),
    )
    .unwrap();
    assert_eq!(
        section_titles(&samu.project(garden.clone(), None).unwrap()),
        [(None, vec!["jäten", "gießen", "mähen"])]
    );
    assert_eq!(
        section_titles(&samu.tagged(office.clone()).unwrap()),
        [(Some("Keller"), vec!["aufräumen"])]
    );

    // Zurücksetzen forgets it; the second key goes with Ordnung.
    samu.set_arrange(
        "available".into(),
        arranged("project", "position", Some("due"), true),
    )
    .unwrap();
    assert_eq!(samu.arrange("available".into()).unwrap(), default);
    assert_eq!(section_titles(&samu.available(None).unwrap()), by_project);

    // Refused: what no list is, and what no grouping or key is.
    assert!(
        samu.set_arrange("forecast".into(), default.clone())
            .is_err()
    );
    assert!(
        samu.set_arrange("available".into(), arranged("colour", "title", None, false))
            .is_err()
    );
    assert!(
        samu.set_arrange("available".into(), arranged("none", "size", None, false))
            .is_err()
    );

    // A perspective keeps it on the server, offline first.
    let view = make_perspective(
        &other,
        "Garten",
        json!({"chips": [{"kind": "project", "value": garden}]}),
    );
    samu.sync().unwrap();
    let key = format!("perspective:{view}");
    samu.set_arrange(key.clone(), arranged("none", "title", None, false))
        .unwrap();
    assert_eq!(
        section_titles(&samu.perspective(view.clone(), None).unwrap()),
        [(None, vec!["gießen", "jäten", "mähen"])]
    );
    samu.sync().unwrap();
    assert_eq!(
        stored(&other, &view)["arrange"],
        json!({"group": "none", "sort": "title"})
    );
    assert!(
        samu.set_arrange(format!("perspective:{}", samu_api::Id::new()), default)
            .is_err()
    );
}

/// A group with two open children and a done one, under a task at the top
/// with a sibling after it (ADR 0128).
fn family(other: &Client) -> (String, String, String, String, String) {
    let project = {
        let id = samu_api::Id::new().to_string();
        let value = json!({"id": id, "created": "2026-11-08T08:00:00Z",
            "modified": "2026-11-08T08:00:00Z", "title": "Haus", "kind": "parallel",
            "status": "active", "flagged": false, "position": 0});
        let change = json!([{"op": "create", "object": id, "kind": "project", "value": value,
                             "hlc": {"wall": 0, "counter": 0, "device": "x"}}]);
        let outcomes = other
            .call("POST", "/changes?stamp=server", Some(&change))
            .unwrap()
            .json()
            .unwrap();
        assert_eq!(outcomes[0]["outcome"], "applied", "{outcomes}");
        id
    };
    let group = make_task(other, "Keller", json!({"project": project, "position": 0}));
    let after = make_task(other, "Dach", json!({"project": project, "position": 1}));
    let a = make_task(
        other,
        "Regal",
        json!({"project": project, "parent": group, "position": 0}),
    );
    let b = make_task(
        other,
        "Lampe",
        json!({"project": project, "parent": group, "position": 1}),
    );
    make_task(
        other,
        "Fegen",
        json!({"project": project, "parent": group, "position": 2,
               "completed_at": "2026-11-08T09:00:00Z"}),
    );
    (group, a, b, after, project)
}

#[test]
fn closing_a_group_with_open_children_asks_and_writes_nothing() {
    let server = Server::start();
    let other = server.other_client();
    let (group, a, _, _, _) = family(&other);
    let samu = phone(&server);
    samu.sync().unwrap();

    let asked = samu
        .close_tasks(vec![group.clone()], "completed".into(), "ask".into())
        .unwrap();
    assert_eq!((asked.asking, asked.open), (1, 2));
    assert_eq!(samu.open_below(group.clone()).unwrap(), 2);
    assert_eq!(samu.open_below(a.clone()).unwrap(), 0);
    assert!(asked.numbers.is_empty() && asked.back.is_empty());
    assert_eq!(samu.outbox_count().unwrap(), 0);
    // Picked along, a child is answered: no question for the group.
    let other_pick = samu
        .close_tasks(vec![a.clone()], "completed".into(), "ask".into())
        .unwrap();
    assert_eq!(other_pick.asking, 0, "a leaf closes without a question");
    samu.withdraw(other_pick.numbers).unwrap();
    assert_eq!(samu.outbox_count().unwrap(), 0);
}

#[test]
fn closing_along_offline_gives_the_children_the_same_instant() {
    let server = Server::start();
    let other = server.other_client();
    let (group, a, b, _, _) = family(&other);
    let samu = phone(&server);
    samu.sync().unwrap();
    let clock = samu.clock_in(b.clone()).unwrap();

    let closed = samu
        .close_tasks(vec![group.clone()], "dropped".into(), "close".into())
        .unwrap();
    assert_eq!(closed.asking, 1, "it still says there were some");
    let rows: Vec<_> = samu
        .available(None)
        .unwrap()
        .into_iter()
        .flat_map(|s| s.rows)
        .collect();
    assert_eq!(titles(&rows), ["Dach"], "only the sibling is left");
    let report = samu.sync().unwrap();
    assert_eq!(report.rejected, 0);

    let at = stored(&other, &group)["dropped_at"].clone();
    assert!(at.is_string(), "{}", stored(&other, &group));
    assert_eq!(stored(&other, &a)["dropped_at"], at);
    assert_eq!(stored(&other, &b)["dropped_at"], at);
    assert!(
        stored(&other, &clock)["end"].is_string(),
        "the clock on a child ends"
    );
}

#[test]
fn lifting_out_puts_the_children_after_the_group_and_back() {
    let server = Server::start();
    let other = server.other_client();
    let (group, a, b, after, _) = family(&other);
    let samu = phone(&server);
    samu.sync().unwrap();

    let closed = samu
        .close_tasks(vec![group.clone()], "completed".into(), "lift".into())
        .unwrap();
    samu.sync().unwrap();
    let (sa, sb, safter) = (
        stored(&other, &a),
        stored(&other, &b),
        stored(&other, &after),
    );
    assert!(
        sa.get("parent").is_none() && sb.get("parent").is_none(),
        "{sa} {sb}"
    );
    assert_eq!((&sa["position"], &sb["position"]), (&json!(1), &json!(2)));
    assert_eq!(safter["position"], 3, "the sibling makes room");
    assert!(stored(&other, &group)["completed_at"].is_string());
    assert!(sa.get("completed_at").is_none());

    // Sent already: withdraw is too late, put_back writes it back.
    assert!(!samu.withdraw(closed.numbers.clone()).unwrap());
    samu.put_back(closed.back).unwrap();
    samu.sync().unwrap();
    let (sa, sb, safter) = (
        stored(&other, &a),
        stored(&other, &b),
        stored(&other, &after),
    );
    assert_eq!(sa["parent"], json!(group));
    assert_eq!(sb["parent"], json!(group));
    assert_eq!((&sa["position"], &sb["position"]), (&json!(0), &json!(1)));
    assert_eq!(safter["position"], 1);
    assert!(stored(&other, &group).get("completed_at").is_none());
}

#[test]
fn a_picked_group_closes_after_its_picked_children() {
    let server = Server::start();
    let other = server.other_client();
    let (group, a, b, _, _) = family(&other);
    let samu = phone(&server);
    samu.sync().unwrap();

    let closed = samu
        .close_tasks(
            vec![group.clone(), a.clone(), b.clone()],
            "completed".into(),
            "ask".into(),
        )
        .unwrap();
    assert_eq!(closed.asking, 0);
    // Before it leaves, withdraw takes it all back.
    assert!(samu.withdraw(closed.numbers.clone()).unwrap());
    assert_eq!(samu.outbox_count().unwrap(), 0);
    let closed = samu
        .close_tasks(
            vec![group.clone(), a.clone(), b.clone()],
            "completed".into(),
            "ask".into(),
        )
        .unwrap();
    assert_eq!(closed.numbers.len(), 3);
    samu.sync().unwrap();
    for id in [&group, &a, &b] {
        assert!(stored(&other, id)["completed_at"].is_string());
    }
}

/// A tag's workflow fields from the phone (e076022f): the detail says
/// what the tag is, the changes reach the server, and samu-core's
/// refusals leave nothing in the outbox.
#[test]
fn tag_settings_from_the_phone() {
    let server = Server::start();
    let other = server.other_client();
    let group = make_tag(&other, "Entwicklung", None);
    let state = make_tag(&other, "Wunsch", Some(&group));
    let plain = make_tag(&other, "Büro", None);
    let samu = phone(&server);
    samu.sync().unwrap();

    let detail = samu.place_detail(group.clone()).unwrap();
    assert_eq!(detail.tag_role.as_deref(), Some("tag"), "no workflow yet");
    assert_eq!(detail.states, 1);
    samu.set_tag_fields(group.clone(), Some(true), Some(true), None, None)
        .unwrap();
    assert_eq!(
        samu.place_detail(state.clone())
            .unwrap()
            .tag_role
            .as_deref(),
        Some("state")
    );
    samu.set_tag_key(state.clone(), Some("w".into())).unwrap();
    samu.set_tag_fields(
        state.clone(),
        None,
        None,
        Some("on_hold".into()),
        Some(true),
    )
    .unwrap();
    // Refused: a key on a plain tag, a default that is no workflow.
    assert!(samu.set_tag_key(plain.clone(), Some("b".into())).is_err());
    assert!(
        samu.set_tag_fields(plain.clone(), None, Some(true), None, None)
            .is_err()
    );
    samu.sync().unwrap();

    let g = stored(&other, &group);
    assert_eq!(
        (g["exclusive"].clone(), g["default_workflow"].clone()),
        (json!(true), json!(true))
    );
    let s = stored(&other, &state);
    assert_eq!(s["key"], "w");
    assert_eq!(s["status"], "on_hold");
    assert_eq!(s["ask_note"], true);
    let p = stored(&other, &plain);
    assert!(p.get("key").is_none_or(Value::is_null), "{p}");
    assert_ne!(p["default_workflow"], true);
    let detail = samu.place_detail(group.clone()).unwrap();
    assert_eq!(detail.tag_role.as_deref(), Some("workflow"));
    assert!(detail.default_workflow);
    samu.set_tag_key(state.clone(), None).unwrap();
    samu.sync().unwrap();
    assert!(stored(&other, &state).get("key").is_none_or(Value::is_null));
}

/// A transcript service for one test: answers every request with `text`
/// once it has read the whole request, and counts them.
fn transcript_service(text: &'static str) -> (String, Arc<std::sync::atomic::AtomicUsize>) {
    use std::io::Read;
    use std::sync::atomic::{AtomicUsize, Ordering};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!(
        "http://{}/v1/audio/transcriptions",
        listener.local_addr().unwrap()
    );
    let asked = Arc::new(AtomicUsize::new(0));
    let counted = asked.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            while let Ok(n) = stream.read(&mut buffer) {
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..n]);
                let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") else {
                    continue;
                };
                let head = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                let length: usize = head
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(0);
                if request.len() >= end + 4 + length {
                    break;
                }
            }
            counted.fetch_add(1, Ordering::SeqCst);
            let body = json!({ "text": text }).to_string();
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (url, asked)
}

/// Syncs until `done` holds for the first attachment of `task`: the
/// transcript is made in the background on the server.
fn synced_until(
    samu: &Samu,
    task: &str,
    done: impl Fn(&samu_mobile::AttachmentRow) -> bool,
) -> samu_mobile::AttachmentRow {
    for _ in 0..200 {
        samu.sync().unwrap();
        let files = samu.attachments(task.to_string()).unwrap();
        if let Some(file) = files.into_iter().next()
            && done(&file)
        {
            return file;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    panic!("the recording of {task} never got there");
}

/// ADR 0140 §4, §6: a voice note made offline is a task in the inbox with
/// its recording, which asks for the transcript; after the sync the
/// server's text is its title, its note and the attachment's transcript.
#[test]
fn a_voice_note_waits_offline_and_comes_back_with_its_text() {
    let (url, asked) = transcript_service("Call the plumber about the tap.\nBefore Friday.");
    let server = Server::start_with_env(&[("SAMU_TRANSCRIBE_URL", &url)]);
    let other = server.other_client();
    let samu = phone(&server);
    samu.sync().unwrap();
    assert!(samu.can_transcribe().unwrap());

    let voice = b"not really aac";
    let task = samu
        .voice_note(
            " Voice note 21.11. 10:00 ".into(),
            taken(&server, "voice.m4a", voice),
            "voice-20261121-1000.m4a".into(),
            "audio/mp4".into(),
        )
        .unwrap();
    assert!(!server.dir.join("voice.m4a").exists());
    // On the phone at once, before the server knows of it.
    let detail = samu.task(task.clone()).unwrap();
    assert_eq!(detail.row.title, "Voice note 21.11. 10:00");
    let files = samu.attachments(task.clone()).unwrap();
    assert_eq!(files.len(), 1);
    assert!(files[0].pending && files[0].transcribing);
    assert_eq!(files[0].transcript, None);
    assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 0);

    let file = synced_until(&samu, &task, |f| f.transcript.is_some());
    assert_eq!(
        file.transcript.as_deref(),
        Some("Call the plumber about the tap.\nBefore Friday.")
    );
    assert!(!file.transcribing && !file.pending && file.transcript_error.is_none());
    assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 1);
    let detail = samu.task(task.clone()).unwrap();
    assert_eq!(detail.row.title, "Call the plumber about the tap.");
    // Asked of the server.
    let stored_task = stored(&other, &task);
    assert_eq!(stored_task["title"], "Call the plumber about the tap.");
    assert_eq!(
        stored_task["note"],
        "Call the plumber about the tap.\nBefore Friday."
    );
    assert_eq!(stored_task["properties"], json!({"source": "voice"}));
    assert!(stored_task.get("project").is_none(), "{stored_task}");
    assert_eq!(stored(&other, &file.id)["blob"], sha256(voice));

    // Asked again from the phone.
    samu.transcribe(file.id.clone()).unwrap();
    synced_until(&samu, &task, |f| !f.transcribing);
    assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 2);

    // What is no recording makes no task.
    let before = stored(&other, &task)["modified"].clone();
    for (path, media_type) in [
        (taken(&server, "empty.m4a", b""), "audio/mp4"),
        (taken(&server, "photo.jpg", b"jpeg"), "image/jpeg"),
        (
            server.dir.join("gone.m4a").display().to_string(),
            "audio/mp4",
        ),
    ] {
        assert!(
            samu.voice_note("Voice note".into(), path, "x".into(), media_type.into())
                .is_err()
        );
    }
    assert_eq!(samu.status().unwrap().outbox, 0);
    assert_eq!(stored(&other, &task)["modified"], before);
}

/// ADR 0140 §1: a server without a transcript service keeps the voice
/// note as it was recorded, and the phone knows not to offer transcripts.
#[test]
fn a_voice_note_without_a_service_stays_as_recorded() {
    let server = Server::start();
    let other = server.other_client();
    let samu = phone(&server);
    samu.sync().unwrap();
    assert!(!samu.can_transcribe().unwrap());
    let task = samu
        .voice_note(
            "Voice note 21.11. 10:00".into(),
            taken(&server, "voice.m4a", b"not really aac"),
            "voice.m4a".into(),
            "audio/mp4".into(),
        )
        .unwrap();
    let file = synced_until(&samu, &task, |f| !f.transcribing && !f.pending);
    assert_eq!((file.transcript, file.transcript_error), (None, None));
    assert_eq!(stored(&other, &task)["title"], "Voice note 21.11. 10:00");
    assert!(samu.transcribe(file.id).is_err());
}

/// A checklist built offline: pasted lists split, ticking and moving go
/// through the outbox, the count shows in the row, and an item and a
/// subtask turn into each other (ADR 0141).
#[test]
fn a_checklist_is_made_offline_and_reaches_the_server() {
    let server = Server::start();
    let other = server.other_client();
    let project = make_project(&other, "Reise");
    let task = make_task(&other, "Packen", json!({"project": project}));
    let samu = phone(&server);
    samu.sync().unwrap();

    assert!(samu.checklist_add(task.clone(), "  \n".into()).is_err());
    let ids = samu
        .checklist_add(
            task.clone(),
            "- [ ] Pass\n* [x] Ladekabel\n3. Sonnenbrille".into(),
        )
        .unwrap();
    assert_eq!(ids.len(), 3);
    let rows = samu.checklist(task.clone()).unwrap();
    let texts: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(texts, ["Pass", "Ladekabel", "Sonnenbrille"]);
    assert_eq!(
        rows.iter().map(|r| r.checked).collect::<Vec<_>>(),
        [false, true, false]
    );
    assert!(rows.iter().all(|r| r.pending));

    samu.checklist_check(ids[0].clone(), true).unwrap();
    samu.checklist_edit(ids[2].clone(), " Sonnenbrille! ".into())
        .unwrap();
    assert!(samu.checklist_edit(ids[2].clone(), " ".into()).is_err());
    // One line: a break is refused here as the CLI and the server do.
    assert!(
        samu.checklist_edit(ids[2].clone(), "Brille\nHut".into())
            .is_err()
    );
    samu.checklist_move(ids[2].clone(), 0).unwrap();
    let texts: Vec<String> = samu
        .checklist(task.clone())
        .unwrap()
        .into_iter()
        .map(|r| r.text)
        .collect();
    assert_eq!(texts, ["Sonnenbrille!", "Pass", "Ladekabel"]);

    let row = samu.task(task.clone()).unwrap().row;
    assert_eq!((row.checks_done, row.checks_all), (2, 3));

    // An item becomes a subtask, ticked as it was; and back.
    let sub = samu.checklist_to_subtask(ids[1].clone()).unwrap();
    assert_eq!(samu.checklist(task.clone()).unwrap().len(), 2);
    let sub_row = samu.task(sub.clone()).unwrap().row;
    assert_eq!(sub_row.title, "Ladekabel");
    assert!(sub_row.done);
    let back = samu.checklist_from_subtask(sub).unwrap();
    assert_eq!(samu.checklist(task.clone()).unwrap().len(), 3);
    // The restored item is the last one.
    assert_eq!(
        samu.checklist(task.clone()).unwrap().last().unwrap().id,
        back
    );
    assert!(samu.checklist_from_subtask(task.clone()).is_err());

    samu.checklist_remove(ids[0].clone()).unwrap();
    samu.sync().unwrap();
    assert!(
        samu.checklist(task.clone())
            .unwrap()
            .iter()
            .all(|r| !r.pending)
    );

    // On the server: the right kind, owner, order.
    let on_server = stored(&other, &ids[2]);
    assert_eq!(on_server["owner"], task.as_str());
    assert_eq!(on_server["text"], "Sonnenbrille!");
    assert_eq!(on_server["position"], 0);
    assert_eq!(stored(&other, &back)["owner"], task.as_str());
    let second = phone_again(&server);
    second.sync().unwrap();
    let texts: Vec<String> = second
        .checklist(task.clone())
        .unwrap()
        .into_iter()
        .map(|r| r.text)
        .collect();
    assert_eq!(texts, ["Sonnenbrille!", "Ladekabel"]);

    // The whole list, both ways, in its order.
    second.checklist_all_to_subtasks(task.clone()).unwrap();
    assert!(second.checklist(task.clone()).unwrap().is_empty());
    assert!(second.checklist_all_to_subtasks(task.clone()).is_err());
    assert!(
        second
            .checklist_all_from_subtasks(task.clone())
            .unwrap()
            .is_empty()
    );
    let rows = second.checklist(task.clone()).unwrap();
    let texts: Vec<&str> = rows.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(texts, ["Sonnenbrille!", "Ladekabel"]);

    // A subtask with a comment stays a subtask: the comment would go with
    // the task, and an item cannot keep it.
    let sub = second.checklist_to_subtask(rows[0].id.clone()).unwrap();
    second.add_comment(sub.clone(), "bleibt".into()).unwrap();
    let refused = second.checklist_from_subtask(sub.clone()).unwrap_err();
    assert!(refused.to_string().contains("comments"), "{refused}");
    assert!(second.checklist_all_from_subtasks(task.clone()).is_err());
    assert_eq!(second.task(sub.clone()).unwrap().row.title, "Sonnenbrille!");
    assert_eq!(second.comments(sub).unwrap().len(), 1);
    assert_eq!(second.checklist(task).unwrap().len(), 1);
}

/// A second phone of the same account.
fn phone_again(server: &Server) -> Arc<Samu> {
    let samu = Samu::open(server.dir.join("phone2").display().to_string()).unwrap();
    samu.login(
        server.url.clone(),
        "privat".into(),
        PASSWORD.into(),
        "pixel2".into(),
    )
    .unwrap();
    samu
}

/// Every title a list shows, whatever its section.
fn listed(sections: &[samu_mobile::Section]) -> Vec<&str> {
    sections.iter().flat_map(|s| titles(&s.rows)).collect()
}

fn show(samu: &Samu, key: &str) -> Option<(String, bool)> {
    samu.show(key.into())
        .unwrap()
        .map(|s| (s.value, s.standard))
}

/// ADR 0020 §1: each list chooses available, remaining or all, a view on
/// this phone, a perspective through the outbox.
#[test]
fn a_list_shows_available_remaining_or_all_as_chosen() {
    let server = Server::start();
    let other = server.other_client();
    let garden = make_project(&other, "Garten");
    let office = make_tag(&other, "Büro", None);
    let later = json!({"date": "2999-01-01"});
    make_task(&other, "Zettel", json!({"flagged": true}));
    make_task(&other, "Brief", json!({"defer": later, "flagged": true}));
    let done_note = make_task(&other, "Altpapier", json!({"flagged": true}));
    make_task(
        &other,
        "mähen",
        json!({"project": garden, "tags": [{"tag": office, "position": 0}]}),
    );
    make_task(
        &other,
        "säen",
        json!({"project": garden, "defer": later, "position": 1,
               "tags": [{"tag": office, "position": 0}]}),
    );
    let done = make_task(&other, "jäten", json!({"project": garden, "position": 2}));
    let samu = phone(&server);
    samu.sync().unwrap();
    samu.complete(done.clone()).unwrap();
    samu.complete(done_note.clone()).unwrap();
    samu.sync().unwrap();
    let project = format!("project:{garden}");
    let tag = format!("tag:{office}");

    // Until someone chooses, each list shows what it always did.
    assert_eq!(show(&samu, "inbox"), Some(("remaining".into(), true)));
    assert_eq!(show(&samu, &project), Some(("remaining".into(), true)));
    assert_eq!(show(&samu, "search"), Some(("remaining".into(), true)));
    assert_eq!(show(&samu, "flagged"), Some(("available".into(), true)));
    assert_eq!(show(&samu, &tag), Some(("available".into(), true)));
    assert_eq!(show(&samu, "available"), None);
    assert_eq!(show(&samu, "today"), None);
    assert_eq!(listed(&samu.inbox(None).unwrap()), ["Zettel", "Brief"]);
    assert_eq!(
        listed(&samu.project(garden.clone(), None).unwrap()),
        ["mähen", "säen"]
    );
    assert_eq!(listed(&samu.flagged(None).unwrap()), ["Zettel"]);
    assert_eq!(listed(&samu.tagged(office.clone()).unwrap()), ["mähen"]);

    // A project on what is available, and only that project.
    samu.set_show(project.clone(), "available".into()).unwrap();
    assert_eq!(show(&samu, &project), Some(("available".into(), false)));
    assert_eq!(
        listed(&samu.project(garden.clone(), None).unwrap()),
        ["mähen"]
    );
    assert_eq!(listed(&samu.inbox(None).unwrap()), ["Zettel", "Brief"]);

    // Everything: what is done too.
    samu.set_show(project.clone(), "all".into()).unwrap();
    assert_eq!(
        listed(&samu.project(garden.clone(), None).unwrap()),
        ["mähen", "säen", "jäten"]
    );
    samu.set_show("inbox".into(), "available".into()).unwrap();
    assert_eq!(listed(&samu.inbox(None).unwrap()), ["Zettel"]);
    samu.set_show("flagged".into(), "remaining".into()).unwrap();
    assert_eq!(listed(&samu.flagged(None).unwrap()), ["Zettel", "Brief"]);
    samu.set_show("flagged".into(), "all".into()).unwrap();
    assert_eq!(
        listed(&samu.flagged(None).unwrap()),
        ["Zettel", "Brief", "Altpapier"]
    );
    samu.set_show(tag.clone(), "remaining".into()).unwrap();
    assert_eq!(
        listed(&samu.tagged(office.clone()).unwrap()),
        ["mähen", "säen"]
    );
    samu.set_show("search".into(), "available".into()).unwrap();
    assert_eq!(listed(&samu.search("en".into(), None).unwrap()), ["mähen"]);

    // The choice stays on this phone, and survives opening it again.
    assert_eq!(samu.outbox_count().unwrap(), 0);
    let again = Samu::open(server.dir.join("phone").display().to_string()).unwrap();
    assert_eq!(show(&again, &project), Some(("all".into(), false)));
    // Back on its default it is as if nobody chose.
    samu.set_show(project.clone(), "remaining".into()).unwrap();
    assert_eq!(show(&samu, &project), Some(("remaining".into(), true)));

    // What it does not know, and a view without a choice, are refused.
    assert!(samu.set_show(project.clone(), "some".into()).is_err());
    assert!(samu.set_show(project, "unknown".into()).is_err());
    assert!(samu.set_show("today".into(), "all".into()).is_err());
    assert!(samu.set_show("available".into(), "all".into()).is_err());
    assert!(samu.show("nowhere".into()).is_err());

    // A perspective's filter is its own, for every device.
    let id = samu
        .create_perspective("Garten alles".into(), None, "#Garten".into())
        .unwrap();
    let key = format!("perspective:{id}");
    assert_eq!(show(&samu, &key), Some(("remaining".into(), true)));
    assert_eq!(
        listed(&samu.perspective(id.clone(), None).unwrap()),
        ["mähen", "säen"]
    );
    samu.set_show(key.clone(), "available".into()).unwrap();
    assert_eq!(show(&samu, &key), Some(("available".into(), true)));
    assert_eq!(
        listed(&samu.perspective(id.clone(), None).unwrap()),
        ["mähen"]
    );
    samu.sync().unwrap();
    assert_eq!(stored(&other, &id)["filter"], "available");
}

/// Title, depth, context and fold of every row, whatever its section.
fn outline(sections: &[samu_mobile::Section]) -> Vec<(&str, u32, bool, Option<bool>)> {
    sections
        .iter()
        .flat_map(|s| &s.rows)
        .map(|r| (r.title.as_str(), r.depth, r.context, r.folded))
        .collect()
}

/// ADR 0021: a group stands above its children, as context when the
/// filter leaves it out, and folds on this phone with a summary.
#[test]
fn a_group_stands_above_its_children_and_folds() {
    let server = Server::start();
    let other = server.other_client();
    let garden = make_project(&other, "Garten");
    let office = make_tag(&other, "Büro", None);
    let later = json!({"date": "2999-01-01"});
    // A deferred group: not available, but its children are their own.
    let bed = make_task(
        &other,
        "Beet",
        json!({"project": garden, "sequential": true, "flagged": true}),
    );
    let dig = make_task(
        &other,
        "umgraben",
        json!({"project": garden, "parent": bed, "tags": [{"tag": office, "position": 0}]}),
    );
    make_task(
        &other,
        "säen",
        json!({"project": garden, "parent": bed, "position": 1}),
    );
    let fence = make_task(&other, "Zaun", json!({"project": garden, "position": 1}));
    let posts = make_task(
        &other,
        "Pfosten",
        json!({"project": garden, "parent": fence}),
    );
    make_task(
        &other,
        "setzen",
        json!({"project": garden, "parent": posts}),
    );
    make_task(
        &other,
        "streichen",
        json!({"project": garden, "parent": posts, "position": 1}),
    );
    make_task(
        &other,
        "Tor",
        json!({"project": garden, "parent": fence, "position": 1, "defer": later}),
    );
    make_task(&other, "mähen", json!({"project": garden, "position": 2}));
    let samu = phone(&server);
    samu.sync().unwrap();
    let project = || samu.project(garden.clone(), None).unwrap();

    // Everything open: the tree, no row as context, groups unfolded.
    assert_eq!(
        outline(&project()),
        [
            ("Beet", 0, false, Some(false)),
            ("umgraben", 1, false, None),
            ("säen", 1, false, None),
            ("Zaun", 0, false, Some(false)),
            ("Pfosten", 1, false, Some(false)),
            ("setzen", 2, false, None),
            ("streichen", 2, false, None),
            ("Tor", 1, false, None),
            ("mähen", 0, false, None),
        ]
    );

    // Available: a sequential group gives its first child only, and the
    // groups, not available themselves, stand above as context.
    let available = samu.available(None).unwrap();
    let rows = outline(&available);
    assert_eq!(
        rows,
        [
            ("Beet", 0, true, Some(false)),
            ("umgraben", 1, false, None),
            ("Zaun", 0, true, Some(false)),
            ("Pfosten", 1, true, Some(false)),
            ("setzen", 2, false, None),
            ("streichen", 2, false, None),
            ("mähen", 0, false, None),
        ]
    );

    // A tag narrows the hits, not the group above them.
    assert_eq!(
        outline(&samu.tagged(office.clone()).unwrap()),
        [("Beet", 0, true, Some(false)), ("umgraben", 1, false, None)]
    );
    // A group that is a hit itself, its children not: they stand below
    // it as context, as far as the list's filter shows them (§6).
    samu.set_show("flagged".into(), "remaining".into()).unwrap();
    assert_eq!(
        outline(&samu.flagged(None).unwrap()),
        [
            ("Beet", 0, false, Some(false)),
            ("umgraben", 1, true, None),
            ("säen", 1, true, None),
        ]
    );
    samu.set_show("flagged".into(), "available".into()).unwrap();
    assert_eq!(
        outline(&samu.flagged(None).unwrap()),
        [] as [(&str, u32, bool, Option<bool>); 0]
    );
    samu.set_show("flagged".into(), "remaining".into()).unwrap();

    // Folded, a group stands for its children: the sequential one names
    // its next action, the parallel one counts its own children that are
    // available; a group among them is not (ADR 0006).
    samu.set_task_folded(bed.clone(), true).unwrap();
    samu.set_task_folded(fence.clone(), true).unwrap();
    let folded = project();
    assert_eq!(
        outline(&folded),
        [
            ("Beet", 0, false, Some(true)),
            ("Zaun", 0, false, Some(true)),
            ("mähen", 0, false, None),
        ]
    );
    let summaries: Vec<Option<&str>> = folded[0]
        .rows
        .iter()
        .map(|r| r.summary.as_deref())
        .collect();
    assert_eq!(
        summaries,
        [Some("nächste: umgraben"), Some("0 verfügbar"), None]
    );
    samu.set_lang("en".into());
    assert_eq!(project()[0].rows[1].summary.as_deref(), Some("0 available"));
    samu.set_lang("de".into());

    // A group folded inside an open one hides only what is below it.
    samu.set_task_folded(fence.clone(), false).unwrap();
    samu.set_task_folded(posts.clone(), true).unwrap();
    assert_eq!(
        outline(&project())[1..],
        [
            ("Zaun", 0, false, Some(false)),
            ("Pfosten", 1, false, Some(true)),
            ("Tor", 1, false, None),
            ("mähen", 0, false, None),
        ]
    );
    assert_eq!(project()[0].rows[2].summary.as_deref(), Some("2 verfügbar"));

    // On this phone only, and still there when it opens again.
    assert_eq!(samu.outbox_count().unwrap(), 0);
    let again = Samu::open(server.dir.join("phone").display().to_string()).unwrap();
    assert_eq!(
        outline(&again.project(garden.clone(), None).unwrap())[0],
        ("Beet", 0, false, Some(true))
    );

    // Arranged any other way a list is flat: no context, nothing to fold.
    samu.set_arrange("available".into(), arranged("none", "title", None, false))
        .unwrap();
    assert_eq!(
        outline(&samu.available(None).unwrap()),
        [
            ("mähen", 0, false, None),
            ("setzen", 0, false, None),
            ("streichen", 0, false, None),
            ("umgraben", 0, false, None),
        ]
    );
    let _ = dig;
}

/// ADR 0021 §6: a group that a search, a tag or a perspective finds
/// brings its children that are no hits, as context; the account says
/// whether it starts unfolded or folded, and the arrow turns that round
/// on this phone.
#[test]
fn a_group_that_is_a_hit_brings_its_children() {
    let server = Server::start();
    let other = server.other_client();
    let home = make_project(&other, "Haus");
    let epic = make_tag(&other, "epic", None);
    let tidy = make_task(
        &other,
        "Aufräumaktion",
        json!({"project": home, "tags": [{"tag": epic, "position": 0}]}),
    );
    make_task(&other, "Keller", json!({"project": home, "parent": tidy}));
    let attic = make_task(
        &other,
        "Dachboden",
        json!({"project": home, "parent": tidy, "position": 1}),
    );
    make_task(
        &other,
        "Kisten",
        json!({"project": home, "parent": attic, "defer": {"date": "2999-01-01"}}),
    );
    let done = make_task(
        &other,
        "Garage",
        json!({"project": home, "parent": tidy, "position": 2}),
    );
    make_task(
        &other,
        "Aufräumaktion planen",
        json!({"project": home, "position": 1}),
    );
    let samu = phone(&server);
    samu.sync().unwrap();
    samu.complete(done.clone()).unwrap();
    let search = |text: &str| samu.search(text.into(), None).unwrap();

    // The search finds the group and a task beside it; the children do
    // not hold the word and come along, theirs too, what is done not.
    let whole = [
        ("Aufräumaktion", 0, false, Some(false)),
        ("Keller", 1, true, None),
        ("Dachboden", 1, true, Some(false)),
        ("Kisten", 2, true, None),
        ("Aufräumaktion planen", 0, false, None),
    ];
    assert_eq!(outline(&search("aufräumaktion")), whole);
    // "All" lets the completed child through as well.
    samu.set_show("search".into(), "all".into()).unwrap();
    assert_eq!(
        outline(&search("aufräumaktion"))[4],
        ("Garage", 1, true, None)
    );
    samu.set_show("search".into(), "remaining".into()).unwrap();

    // The tag of the bar: available ones only, so the deferred one and
    // the group above it stay out.
    assert_eq!(
        outline(&samu.tagged(epic.clone()).unwrap()),
        [] as [(&str, u32, bool, Option<bool>); 0]
    );
    samu.set_show(format!("tag:{epic}"), "remaining".into())
        .unwrap();
    assert_eq!(outline(&samu.tagged(epic.clone()).unwrap()), whole[..4]);

    // A child that is a hit itself is no context, and its group no
    // group left alone.
    assert_eq!(
        outline(&search("keller")),
        [
            ("Aufräumaktion", 0, true, Some(false)),
            ("Keller", 1, false, None)
        ]
    );

    // The account says such a group starts folded; the arrow unfolds it,
    // and a group with a hit below it is not meant.
    assert!(!samu.hit_children_folded().unwrap());
    samu.set_hit_children_folded(true).unwrap();
    assert!(samu.hit_children_folded().unwrap());
    let folded = search("aufräumaktion");
    assert_eq!(
        outline(&folded),
        [
            ("Aufräumaktion", 0, false, Some(true)),
            ("Aufräumaktion planen", 0, false, None),
        ]
    );
    assert_eq!(folded[0].rows[0].summary.as_deref(), Some("1 verfügbar"));
    assert_eq!(
        outline(&samu.project(home.clone(), None).unwrap())[0],
        ("Aufräumaktion", 0, false, Some(false))
    );
    samu.set_task_folded(tidy.clone(), false).unwrap();
    assert_eq!(outline(&search("aufräumaktion")), whole);
    samu.set_task_folded(tidy.clone(), true).unwrap();
    assert_eq!(outline(&search("aufräumaktion")).len(), 2);
    assert_eq!(
        outline(&samu.project(home.clone(), None).unwrap())[0],
        ("Aufräumaktion", 0, false, Some(true))
    );

    // The setting is the account's: it reaches the server.
    samu.sync().unwrap();
    let settings: Value = other
        .call("GET", "/changes?after=0", None)
        .unwrap()
        .json()
        .unwrap();
    assert!(settings.to_string().contains(r#""hit_children":"folded""#));
}
