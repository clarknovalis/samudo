//! Restricted sessions over the API (ADR 0095 §4–6). The account:
//! folders tec > ai and tec > other; project P in ai, Q in other; task a in
//! P, task b in Q, task i in the inbox; a tag. The grant is the folder ai.

use std::net::SocketAddr;

use serde_json::{Value, json};

use super::{PASSWORD, call, call_bytes, sha256, sign_in, start};

const AT: &str = "2026-11-13T08:00:00Z";

fn hlc() -> Value {
    json!({"wall": 0, "counter": 0, "device": "ignored"})
}

fn new_id() -> String {
    samu_store::core::api::Id::new().to_string()
}

fn made(kind: &str, id: &str, mut value: Value) -> Value {
    let base = json!({"id": id, "created": AT, "modified": AT, "position": 0});
    value
        .as_object_mut()
        .unwrap()
        .extend(base.as_object().unwrap().clone());
    json!({"op": "create", "object": id, "kind": kind, "value": value, "hlc": hlc()})
}

fn folder(id: &str, title: &str, parent: Option<&str>) -> Value {
    made(
        "folder",
        id,
        json!({"title": title, "parent": parent, "status": "active"}),
    )
}

fn project(id: &str, title: &str, folder: &str) -> Value {
    made(
        "project",
        id,
        json!({"title": title, "folder": folder, "kind": "parallel",
               "status": "active", "flagged": false}),
    )
}

fn task(id: &str, title: &str, project: Option<&str>) -> Value {
    made(
        "task",
        id,
        json!({"title": title, "project": project, "flagged": false,
               "sequential": false, "complete_with_last_child": false}),
    )
}

fn set(id: &str, field: &str, value: Value) -> Value {
    json!({"op": "set", "object": id, "field": field, "value": value, "hlc": hlc()})
}

struct Account {
    ai: String,
    other: String,
    p: String,
    q: String,
    a: String,
    b: String,
    i: String,
    tag: String,
}

/// The account above, written through a session of the whole account.
async fn account(address: SocketAddr, whole: &str) -> Account {
    let account = Account {
        ai: new_id(),
        other: new_id(),
        p: new_id(),
        q: new_id(),
        a: new_id(),
        b: new_id(),
        i: new_id(),
        tag: new_id(),
    };
    let tec = new_id();
    let changes = json!([
        folder(&tec, "tec", None),
        folder(&account.ai, "ai", Some(&tec)),
        folder(&account.other, "other", Some(&tec)),
        project(&account.p, "samudo", &account.ai),
        project(&account.q, "geheim", &account.other),
        task(&account.a, "drinnen", Some(&account.p)),
        task(&account.b, "draussen", Some(&account.q)),
        task(&account.i, "inbox", None),
        made(
            "tag",
            &account.tag,
            json!({"title": "Gebaut", "status": "active", "exclusive": false})
        ),
    ]);
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(whole),
        Some(&changes),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    for outcome in answer.json().as_array().unwrap() {
        assert_eq!(outcome["outcome"], "applied", "{}", answer.body);
    }
    account
}

async fn sign_in_only(address: SocketAddr, objects: &[&str], access: &str) -> super::Answer {
    call(
        address,
        "POST",
        "/login",
        None,
        Some(
            &json!({"account": "privat", "password": PASSWORD, "device": "claude",
                     "kind": "agent", "only": {"objects": objects, "access": access}}),
        ),
    )
    .await
}

async fn restricted(address: SocketAddr, objects: &[&str], access: &str) -> String {
    let answer = sign_in_only(address, objects, access).await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    answer.json()["token"].as_str().unwrap().to_string()
}

fn ids(list: &Value) -> Vec<String> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|o| {
            o.get("object")
                .and_then(|o| o.get("id"))
                .unwrap_or(o)
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect()
}

fn outcomes(answer: &super::Answer) -> Vec<String> {
    assert_eq!(answer.status, 200, "{}", answer.body);
    answer
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["outcome"].as_str().unwrap().to_string())
        .collect()
}

/// §1: a grant names live folders or projects of the account; a wrong one
/// is refused and leaves no session behind. The session shows its grant.
#[tokio::test]
async fn a_grant_is_checked_at_sign_in() {
    let (address, dir) = start(true).await;
    let whole = sign_in(address).await;
    let account = account(address, &whole).await;
    for (objects, access) in [
        (vec![account.a.clone()], "rw"),
        (vec![new_id()], "rw"),
        (vec![account.ai.clone()], "rx"),
    ] {
        let objects: Vec<&str> = objects.iter().map(String::as_str).collect();
        let answer = sign_in_only(address, &objects, access).await;
        assert!(
            matches!(answer.status, 400 | 422),
            "{objects:?} {access}: {} {}",
            answer.status,
            answer.body
        );
    }
    let wrong = call(
        address,
        "POST",
        "/login",
        None,
        Some(
            &json!({"account": "privat", "password": "falsch", "device": "claude",
                     "only": {"objects": [account.ai], "access": "rw"}}),
        ),
    )
    .await;
    assert_eq!(wrong.status, 401);
    let conn = rusqlite::Connection::open(dir.0.join("server.db")).unwrap();
    let count = |conn: &rusqlite::Connection| -> i64 {
        conn.query_row("SELECT count(*) FROM session", [], |r| r.get(0))
            .unwrap()
    };
    assert_eq!(count(&conn), 1, "only the whole session");

    let token = restricted(address, &[&account.ai], "rw").await;
    assert_eq!(count(&conn), 2, "a probe that must find something");
    let session = call(address, "GET", "/session", Some(&token), None).await;
    assert_eq!(
        session.json()["grant"],
        json!({"objects": [account.ai], "access": "rw"})
    );
}

/// §2, §5: the session reads only what its grant reaches, and the rest is
/// not there: 404, not 403.
#[tokio::test]
async fn a_restricted_session_reads_only_its_grant() {
    let (address, _dir) = start(true).await;
    let whole = sign_in(address).await;
    let account = account(address, &whole).await;
    let token = restricted(address, &[&account.ai], "ro").await;

    let page = call(address, "GET", "/changes?after=0", Some(&token), None)
        .await
        .json();
    let seen = ids(&page["objects"]);
    for id in [&account.ai, &account.p, &account.a, &account.tag] {
        assert!(seen.contains(id), "{id} in {seen:?}");
    }
    for id in [&account.other, &account.q, &account.b, &account.i] {
        assert!(!seen.contains(id), "{id} in {seen:?}");
    }
    assert!(
        page["objects"]
            .as_array()
            .unwrap()
            .iter()
            .all(|o| o["kind"] != "settings"),
        "no settings: {page}"
    );

    for (id, status) in [(&account.a, 200), (&account.b, 404), (&account.i, 404)] {
        let object = call(
            address,
            "GET",
            &format!("/objects/{id}"),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(object.status, status, "{id}: {}", object.body);
        let log = call(
            address,
            "GET",
            &format!("/objects/{id}/logbook"),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(log.status, status, "logbook {id}: {}", log.body);
    }

    let available = call(address, "GET", "/available?all=true", Some(&token), None).await;
    assert_eq!(available.status, 200, "{}", available.body);
    let titles: Vec<String> = available
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["task"]["title"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(titles, ["drinnen"]);
    let whole_list = call(address, "GET", "/available?all=true", Some(&whole), None).await;
    assert_eq!(whole_list.json().as_array().unwrap().len(), 3, "the probe");

    let projects = call(address, "GET", "/available/projects", Some(&token), None).await;
    let titles: Vec<Value> = projects
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["project"]["title"].clone())
        .collect();
    assert_eq!(titles, [json!("samudo")], "{}", projects.body);
}

/// §3: `rw` writes inside, not across the edge, no tag, not the root's
/// place; `ro` writes nothing. Asked of the objects afterwards.
#[tokio::test]
async fn a_restricted_session_writes_only_inside() {
    let (address, _dir) = start(true).await;
    let whole = sign_in(address).await;
    let account = account(address, &whole).await;
    let rw = restricted(address, &[&account.ai], "rw").await;
    let ro = restricted(address, &[&account.ai], "ro").await;
    let new_task = new_id();
    let changes = json!([
        set(&account.a, "title", json!("neu")),
        set(&account.a, "project", json!(account.q)),
        set(&account.b, "title", json!("fremd")),
        task(&new_task, "in die inbox", None),
        made(
            "tag",
            &new_id(),
            json!({"title": "neu", "status": "active", "exclusive": false})
        ),
        set(&account.ai, "parent", json!(account.other)),
        {"op": "add", "object": account.a, "field": "tags", "element": account.tag,
         "hlc": hlc()},
    ]);
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&rw),
        Some(&changes),
    )
    .await;
    assert_eq!(
        outcomes(&answer),
        [
            "applied", "rejected", "rejected", "rejected", "rejected", "rejected", "applied"
        ],
        "{}",
        answer.body
    );
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&ro),
        Some(&json!([set(&account.a, "title", json!("ro"))])),
    )
    .await;
    assert_eq!(outcomes(&answer), ["rejected"]);

    let object = |id: String| {
        let whole = whole.clone();
        async move {
            call(
                address,
                "GET",
                &format!("/objects/{id}"),
                Some(&whole),
                None,
            )
            .await
            .json()["object"]
                .clone()
        }
    };
    let a = object(account.a.clone()).await;
    assert_eq!(a["title"], "neu");
    assert_eq!(a["project"], json!(account.p));
    assert_eq!(a["tags"][0]["tag"], json!(account.tag), "{a}");
    assert_eq!(object(account.b.clone()).await["title"], "draussen");
    assert_ne!(
        object(account.ai.clone()).await["parent"],
        json!(account.other)
    );
    let missing = call(
        address,
        "GET",
        &format!("/objects/{new_task}"),
        Some(&whole),
        None,
    )
    .await;
    assert_eq!(missing.status, 404);

    // A new project in the grant takes a task in the same batch.
    let (project_id, task_id) = (new_id(), new_id());
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&rw),
        Some(&json!([
            project(&project_id, "neu", &account.ai),
            task(&task_id, "darin", Some(&project_id)),
        ])),
    )
    .await;
    assert_eq!(outcomes(&answer), ["applied", "applied"]);
}

/// §5: what is moved out of the grant is gone for the session, in
/// `GET /changes` and on `/events`.
#[tokio::test]
async fn moved_out_is_gone() {
    let (address, _dir) = start(true).await;
    let whole = sign_in(address).await;
    let account = account(address, &whole).await;
    let token = restricted(address, &[&account.ai], "ro").await;
    let seq = call(address, "GET", "/changes?after=0", Some(&token), None)
        .await
        .json()["seq"]
        .as_u64()
        .unwrap();
    let listener = {
        let token = token.clone();
        tokio::spawn(async move {
            super::stream_until(address, "/events", &token, "", "event: changed").await
        })
    };
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&whole),
        Some(&json!([
            set(&account.p, "folder", json!(account.other)),
            set(&account.b, "title", json!("still outside")),
        ])),
    )
    .await;
    assert_eq!(outcomes(&answer), ["applied", "applied"]);
    let events = listener.await.unwrap();
    let notice = events
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .map(|data| serde_json::from_str::<Value>(data).unwrap())
        .unwrap_or_else(|| panic!("no notice: {events}"));
    assert_eq!(notice["changed"], json!([]), "{notice}");
    let gone = ids(&notice["gone"]);
    assert!(gone.contains(&account.p), "{notice}");
    let page = call(
        address,
        "GET",
        &format!("/changes?after={seq}"),
        Some(&token),
        None,
    )
    .await
    .json();
    assert_eq!(page["objects"], json!([]), "{page}");
    let gone = ids(&page["gone"]);
    assert!(gone.contains(&account.p), "{page}");
    let object = call(
        address,
        "GET",
        &format!("/objects/{}", account.a),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(object.status, 404, "its task went with it");
}

/// §4: the forbidden routes answer 403, the free ones as always.
#[tokio::test]
async fn forbidden_routes_are_closed() {
    let (address, _dir) = start(true).await;
    let whole = sign_in(address).await;
    let account = account(address, &whole).await;
    let token = restricted(address, &[&account.ai], "rw").await;
    for (method, path) in [
        ("GET", "/export"),
        ("GET", "/hooks"),
        ("GET", "/scopes/scheduled"),
        ("GET", "/appointments"),
        // ADR 0101 §5.
        ("GET", "/alarms"),
        ("POST", "/calendars/discover"),
        ("POST", "/demo"),
    ] {
        let answer = call(address, method, path, Some(&token), Some(&json!({}))).await;
        assert_eq!(answer.status, 403, "{method} {path}: {}", answer.body);
        assert_eq!(answer.json()["error"], "forbidden");
    }
    let export = call(address, "GET", "/export", Some(&whole), None).await;
    assert_eq!(export.status, 200, "the probe");
    let render = call(
        address,
        "POST",
        "/render",
        Some(&token),
        Some(&json!({"texts": [{"text": "*x*"}]})),
    )
    .await;
    assert_eq!(render.status, 200, "{}", render.body);
}

/// §4: a capture goes only into the grant; the inbox and a new tag are
/// forbidden, also as a preview.
#[tokio::test]
async fn capture_only_into_the_grant() {
    let (address, _dir) = start(true).await;
    let whole = sign_in(address).await;
    let account = account(address, &whole).await;
    let token = restricted(address, &[&account.ai], "rw").await;
    for text in [
        "in die inbox",
        "nach draussen #geheim",
        "neuer tag #samudo @gibtsnicht",
    ] {
        for preview in [true, false] {
            let answer = call(
                address,
                "POST",
                "/capture",
                Some(&token),
                Some(&json!({"text": text, "preview": preview})),
            )
            .await;
            assert_eq!(answer.status, 403, "{text}: {}", answer.body);
        }
    }
    let answer = call(
        address,
        "POST",
        "/capture",
        Some(&token),
        Some(&json!({"text": "drinnen auch #samudo @Gebaut"})),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    let made = answer.json()["task"]["id"].as_str().unwrap().to_string();
    let stored = call(
        address,
        "GET",
        &format!("/objects/{made}"),
        Some(&whole),
        None,
    )
    .await
    .json();
    assert_eq!(stored["object"]["project"], json!(account.p), "{stored}");
    // Nothing of the refused ones was written.
    let all = call(address, "GET", "/available?all=true", Some(&whole), None).await;
    assert_eq!(all.json().as_array().unwrap().len(), 4, "{}", all.body);
}

/// §4: a delete that would carry something out of the grant is forbidden;
/// one inside goes through. A delete of what is not seen is 404.
#[tokio::test]
async fn delete_stays_inside() {
    let (address, _dir) = start(true).await;
    let whole = sign_in(address).await;
    let account = account(address, &whole).await;
    let token = restricted(address, &[&account.ai], "rw").await;
    let delete = |id: String, body: Value| {
        let token = token.clone();
        async move {
            call(
                address,
                "POST",
                &format!("/objects/{id}/delete"),
                Some(&token),
                Some(&body),
            )
            .await
        }
    };
    // The project's task would go to the inbox.
    let answer = delete(account.p.clone(), json!({"preview": true})).await;
    assert_eq!(answer.status, 403, "{}", answer.body);
    let answer = delete(account.b.clone(), json!({})).await;
    assert_eq!(answer.status, 404, "{}", answer.body);
    let answer = delete(account.a.clone(), json!({})).await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    let gone = call(
        address,
        "GET",
        &format!("/objects/{}", account.a),
        Some(&whole),
        None,
    )
    .await;
    assert_eq!(gone.status, 404);
}

/// §4: a file only through an attachment the session sees; `ro` puts none.
#[tokio::test]
async fn blobs_follow_their_attachments() {
    let (address, _dir) = start(true).await;
    let whole = sign_in(address).await;
    let account = account(address, &whole).await;
    let auth =
        |token: &str| format!("Authorization: Bearer {token}\r\nContent-Type: text/plain\r\n");
    let (inside, outside) = (b"inside".to_vec(), b"outside".to_vec());
    let mut changes = Vec::new();
    for (file, owner) in [(&inside, &account.a), (&outside, &account.b)] {
        let hash = sha256(file);
        let (status, _, _) = call_bytes(
            address,
            "PUT",
            &format!("/blobs/{hash}"),
            &auth(&whole),
            file,
        )
        .await;
        assert_eq!(status, 200);
        changes.push(made(
            "attachment",
            &new_id(),
            json!({"owner": owner, "blob": hash, "name": "f.txt",
                   "media_type": "text/plain", "size": file.len()}),
        ));
    }
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&whole),
        Some(&Value::Array(changes)),
    )
    .await;
    assert_eq!(outcomes(&answer), ["applied", "applied"]);

    let ro = restricted(address, &[&account.ai], "ro").await;
    let (status, _, body) = call_bytes(
        address,
        "GET",
        &format!("/blobs/{}", sha256(&inside)),
        &auth(&ro),
        &[],
    )
    .await;
    assert_eq!((status, body), (200, inside.clone()));
    let (status, _, _) = call_bytes(
        address,
        "GET",
        &format!("/blobs/{}", sha256(&outside)),
        &auth(&ro),
        &[],
    )
    .await;
    assert_eq!(status, 404);
    let new = b"new".to_vec();
    let (status, _, _) = call_bytes(
        address,
        "PUT",
        &format!("/blobs/{}", sha256(&new)),
        &auth(&ro),
        &new,
    )
    .await;
    assert_eq!(status, 403);
    let rw = restricted(address, &[&account.ai], "rw").await;
    let (status, _, _) = call_bytes(
        address,
        "PUT",
        &format!("/blobs/{}", sha256(&new)),
        &auth(&rw),
        &new,
    )
    .await;
    assert_eq!(status, 200);
}

/// §7: a person's session of the whole account lists the sessions without
/// tokens and ends one by its id; a restricted or an agent's session may
/// not. An ended session is signed out.
#[tokio::test]
async fn sessions_are_seen_and_ended_by_a_person() {
    let (address, _dir) = start(true).await;
    let whole = sign_in(address).await;
    let account = account(address, &whole).await;
    let agent = restricted(address, &[&account.ai], "rw").await;
    let whole_agent = call(
        address,
        "POST",
        "/login",
        None,
        Some(
            &json!({"account": "privat", "password": PASSWORD, "device": "bot",
                     "kind": "agent"}),
        ),
    )
    .await
    .json()["token"]
        .as_str()
        .unwrap()
        .to_string();
    for token in [&agent, &whole_agent] {
        let answer = call(address, "GET", "/sessions", Some(token), None).await;
        assert_eq!(answer.status, 403, "{}", answer.body);
        let answer = call(address, "POST", "/sessions/00/end", Some(token), None).await;
        assert_eq!(answer.status, 403, "{}", answer.body);
    }

    let answer = call(address, "GET", "/sessions", Some(&whole), None).await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    assert!(!answer.body.contains(&agent) && !answer.body.contains(&whole));
    let sessions = answer.json();
    let sessions = sessions.as_array().unwrap();
    assert_eq!(sessions.len(), 3, "{}", answer.body);
    let claude = sessions.iter().find(|s| s["device"] == "claude").unwrap();
    assert_eq!(claude["grant"]["access"], "rw");
    assert_eq!(claude["current"], false);
    let id = claude["id"].as_str().unwrap();

    let unknown = call(
        address,
        "POST",
        "/sessions/ffffffffffff/end",
        Some(&whole),
        None,
    )
    .await;
    assert_eq!(unknown.status, 404, "{}", unknown.body);
    let malformed = call(address, "POST", "/sessions/xyz/end", Some(&whole), None).await;
    assert_eq!(malformed.status, 400, "{}", malformed.body);
    let before = call(address, "GET", "/session", Some(&agent), None).await;
    assert_eq!(before.status, 200, "the probe");
    let ended = call(
        address,
        "POST",
        &format!("/sessions/{id}/end"),
        Some(&whole),
        None,
    )
    .await;
    assert_eq!(ended.status, 204, "{}", ended.body);
    let after = call(address, "GET", "/session", Some(&agent), None).await;
    assert_eq!(after.status, 401);
    let left = call(address, "GET", "/sessions", Some(&whole), None).await;
    assert_eq!(left.json().as_array().unwrap().len(), 2);
}
