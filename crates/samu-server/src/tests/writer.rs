// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Appointments written back (ADR 0068): a small CalDAV server in the test
//! keeps resources with ETags and answers `If-Match` and `If-None-Match`
//! as a real one does. A change over `/changes` goes back at once; what
//! the server holds afterwards is asked of it and of the database.
//! Synthetic data only, dates counted from today.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::IntoResponse;
use jiff::civil::Date;
use samu_store::SecretKey;
use samu_store::core::api::Id;

use super::*;

fn key() -> SecretKey {
    SecretKey::new([5; 32])
}

fn tomorrow() -> Date {
    Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::get("Europe/Berlin").unwrap())
        .date()
        .tomorrow()
        .unwrap()
}

const COLLECTION: &str = "/dav/w/";
const DENTIST: &str = "/dav/w/dentist.ics";

/// The dentist tomorrow at nine, with an alarm samu does not read.
fn dentist(title: &str, extra: &str) -> String {
    let day = tomorrow().strftime("%Y%m%d");
    [
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        "PRODID:-//test//EN",
        "BEGIN:VEVENT",
        "UID:dentist@example.org",
        &format!("SUMMARY:{title}"),
        &format!("DTSTART;TZID=Europe/Berlin:{day}T090000"),
        &format!("DTEND;TZID=Europe/Berlin:{day}T094500"),
        extra,
        "BEGIN:VALARM",
        "ACTION:DISPLAY",
        "TRIGGER:-PT30M",
        "END:VALARM",
        "END:VEVENT",
        "END:VCALENDAR",
        "",
    ]
    .join("\r\n")
    .replace("\r\n\r\n", "\r\n")
}

/// What the test server holds and how it behaves.
#[derive(Default)]
struct Dav {
    resources: BTreeMap<String, (String, u32)>,
    next: u32,
    writable: bool,
    forbid: bool,
    /// Each write as `METHOD path if-match`.
    log: Vec<String>,
}

impl Dav {
    fn put(&mut self, href: &str, text: String) -> String {
        self.next += 1;
        self.resources.insert(href.to_string(), (text, self.next));
        etag(self.next)
    }
}

fn etag(n: u32) -> String {
    format!("\"e{n}\"")
}

type Shared = Arc<Mutex<Dav>>;

async fn dav_server(dav: Shared) -> SocketAddr {
    let app = axum::Router::new().fallback(
        move |method: Method, uri: Uri, headers: HeaderMap, body: String| {
            let dav = dav.clone();
            async move { answer(&dav, &method, uri.path(), &headers, body) }
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    address
}

fn answer(
    dav: &Shared,
    method: &Method,
    path: &str,
    headers: &HeaderMap,
    body: String,
) -> axum::response::Response {
    if headers.get("authorization").and_then(|h| h.to_str().ok()) != Some("Basic YWxpY2U6c2VjcmV0")
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|h| h.to_str().ok())
            .map(str::to_string)
    };
    let mut dav = dav.lock().unwrap();
    let multi = StatusCode::from_u16(207).unwrap();
    match (method.as_str(), path) {
        ("PROPFIND", COLLECTION) => {
            let privileges = if dav.writable {
                "<d:privilege><d:read/></d:privilege><d:privilege><d:write-content/></d:privilege>"
            } else {
                "<d:privilege><d:read/></d:privilege>"
            };
            let body = format!(
                r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:cs="http://calendarserver.org/ns/"><d:response><d:href>{COLLECTION}</d:href><d:propstat><d:prop><cs:getctag>"c{}"</cs:getctag><d:current-user-privilege-set>{privileges}</d:current-user-privilege-set></d:prop></d:propstat></d:response></d:multistatus>"#,
                dav.next
            );
            (multi, body).into_response()
        }
        ("REPORT", COLLECTION) => {
            let objects: String = dav
                .resources
                .iter()
                .map(|(href, (text, n))| {
                    let escaped = text.replace('&', "&amp;").replace('<', "&lt;");
                    format!(
                        "<d:response><d:href>{href}</d:href><d:propstat><d:prop><d:getetag>{}</d:getetag><c:calendar-data>{escaped}</c:calendar-data></d:prop></d:propstat></d:response>",
                        etag(*n).replace('"', "&quot;")
                    )
                })
                .collect();
            let body = format!(
                r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">{objects}</d:multistatus>"#
            );
            (multi, body).into_response()
        }
        ("GET", href) => match dav.resources.get(href) {
            Some((text, n)) => {
                ([(axum::http::header::ETAG, etag(*n))], text.clone()).into_response()
            }
            None => StatusCode::NOT_FOUND.into_response(),
        },
        ("PUT", href) => {
            let if_match = header("if-match");
            let none_match = header("if-none-match");
            dav.log.push(format!(
                "PUT {href} {}",
                if_match.clone().unwrap_or_default()
            ));
            if dav.forbid {
                return StatusCode::FORBIDDEN.into_response();
            }
            let current = dav.resources.get(href).map(|(_, n)| etag(*n));
            if none_match.as_deref() == Some("*") && current.is_some() {
                return StatusCode::PRECONDITION_FAILED.into_response();
            }
            if let Some(wanted) = if_match
                && current.as_ref() != Some(&wanted)
            {
                return StatusCode::PRECONDITION_FAILED.into_response();
            }
            let created = current.is_none();
            let tag = dav.put(href, body);
            let status = if created {
                StatusCode::CREATED
            } else {
                StatusCode::NO_CONTENT
            };
            (status, [(axum::http::header::ETAG, tag)]).into_response()
        }
        ("DELETE", href) => {
            let if_match = header("if-match");
            dav.log.push(format!(
                "DELETE {href} {}",
                if_match.clone().unwrap_or_default()
            ));
            let current = dav.resources.get(href).map(|(_, n)| etag(*n));
            match (current, if_match) {
                (None, _) => StatusCode::NOT_FOUND.into_response(),
                (Some(c), Some(w)) if c != w => StatusCode::PRECONDITION_FAILED.into_response(),
                _ => {
                    dav.resources.remove(href);
                    dav.next += 1;
                    StatusCode::NO_CONTENT.into_response()
                }
            }
        }
        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
    }
}

fn ask(dir: &TempDir, sql: &str) -> i64 {
    let conn = rusqlite::Connection::open(dir.0.join("accounts/privat.db")).unwrap();
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

fn ask_text(dir: &TempDir, sql: &str) -> Option<String> {
    let conn = rusqlite::Connection::open(dir.0.join("accounts/privat.db")).unwrap();
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

const QUEUED: &str = "SELECT count(*) FROM appointment_write";

/// Waits until the queue is empty, five seconds at most.
async fn written(dir: &TempDir) {
    for _ in 0..100 {
        if ask(dir, QUEUED) == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("still queued after five seconds");
}

/// A signed-in server with a CalDAV calendar over the test server, read
/// once. Returns the server, its directory, the token, the calendar.
async fn setup(dav: &Shared) -> (SocketAddr, TempDir, String, String) {
    let at = dav_server(dav.clone()).await;
    let (address, dir) = start_with_key(true, Some(key())).await;
    let token = sign_in(address).await;
    let calendar = Id::new().to_string();
    let value = json!({"id": calendar, "created": "2026-11-11T08:00:00Z",
                       "modified": "2026-11-11T08:00:00Z", "title": "Kalender",
                       "kind": "caldav", "enabled": true, "refresh": 15, "position": 0});
    let made = changes(
        address,
        &token,
        json!([{"op": "create", "object": calendar, "kind": "calendar", "value": value}]),
    )
    .await;
    assert_eq!(made[0]["outcome"], "applied");
    let source = json!({"url": format!("http://{at}{COLLECTION}"),
                        "username": "alice", "password": "secret"});
    let path = format!("/calendars/{calendar}/source");
    assert_eq!(
        call(address, "PUT", &path, Some(&token), Some(&source))
            .await
            .status,
        204
    );
    let refreshed = call(
        address,
        "POST",
        &format!("/calendars/{calendar}/refresh"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(refreshed.status, 200, "{}", refreshed.body);
    (address, dir, token, calendar)
}

/// `POST /changes?stamp=server` with each change stamped by the server.
async fn changes(address: SocketAddr, token: &str, list: Value) -> Value {
    let stamped: Vec<Value> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let mut c = c.clone();
            c["hlc"] = json!({"wall": 0, "counter": 0, "device": "ignored"});
            c
        })
        .collect();
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(token),
        Some(&Value::Array(stamped)),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    answer.json()
}

/// The appointment with this UID, as `/changes` gives it.
async fn appointment(address: SocketAddr, token: &str, uid: &str) -> Value {
    let page = call(address, "GET", "/changes?after=0", Some(token), None).await;
    page.json()["objects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o["object"].clone())
        .find(|o| o["uid"] == uid && o.get("recurrence_id").is_none())
        .unwrap_or_else(|| panic!("no {uid} in {}", page.body))
}

fn with_dentist(writable: bool) -> Shared {
    let mut dav = Dav {
        writable,
        ..Dav::default()
    };
    dav.put(DENTIST, dentist("Zahnarzt", ""));
    Arc::new(Mutex::new(dav))
}

#[tokio::test]
async fn a_changed_title_goes_back_with_everything_else_kept() {
    let dav = with_dentist(true);
    let (address, dir, token, _) = setup(&dav).await;
    let a = appointment(address, &token, "dentist@example.org").await;
    assert_eq!(
        ask(&dir, "SELECT writable FROM calendar"),
        1,
        "write-content makes it writable"
    );
    let answer = changes(
        address,
        &token,
        json!([{"op": "set", "object": a["id"], "field": "title", "value": "Zahnarzt, Kontrolle"}]),
    )
    .await;
    assert_eq!(answer[0]["outcome"], "applied", "{answer}");
    written(&dir).await;

    let dav = dav.lock().unwrap();
    let (text, n) = dav.resources.get(DENTIST).unwrap();
    assert!(text.contains("SUMMARY:Zahnarzt\\, Kontrolle\r\n"), "{text}");
    assert!(
        text.contains("BEGIN:VALARM\r\nACTION:DISPLAY"),
        "the alarm stays"
    );
    assert_eq!(
        dav.log,
        vec![format!("PUT {DENTIST} {}", etag(1))],
        "If-Match the ETag read"
    );
    assert_eq!(
        ask_text(&dir, "SELECT etag FROM appointment").as_deref(),
        Some(etag(*n).as_str()),
        "the new ETag is kept"
    );
}

#[tokio::test]
async fn a_new_appointment_is_put_where_nothing_is() {
    let dav = with_dentist(true);
    let (address, dir, token, calendar) = setup(&dav).await;
    let id = Id::new().to_string();
    let day = tomorrow();
    let value = json!({"id": id, "created": "2026-11-11T08:00:00Z",
        "modified": "2026-11-11T08:00:00Z", "calendar": calendar, "uid": id,
        "title": "Lesung", "start": {"date": day.to_string()},
        "end": {"date": day.tomorrow().unwrap().to_string()}, "status": "confirmed", "busy": false});
    let answer = changes(
        address,
        &token,
        json!([{"op": "create", "object": id, "kind": "appointment", "value": value}]),
    )
    .await;
    assert_eq!(answer[0]["outcome"], "applied", "{answer}");
    written(&dir).await;
    let href = format!("{COLLECTION}{id}.ics");
    let dav = dav.lock().unwrap();
    let (text, _) = dav.resources.get(&href).unwrap();
    assert!(text.contains(&format!("UID:{id}\r\n")));
    assert!(text.contains("TRANSP:TRANSPARENT\r\n"));
    assert_eq!(
        dav.log,
        vec![format!("PUT {href} ")],
        "If-None-Match, no If-Match"
    );
    assert_eq!(
        ask_text(
            &dir,
            &format!("SELECT href FROM appointment WHERE uid = '{id}'")
        ),
        Some(href.clone())
    );
}

#[tokio::test]
async fn a_change_elsewhere_in_between_is_settled_field_by_field() {
    let dav = with_dentist(true);
    let (address, dir, token, _) = setup(&dav).await;
    let a = appointment(address, &token, "dentist@example.org").await;
    // Elsewhere, long after samu's change will be: the title.
    dav.lock().unwrap().put(
        DENTIST,
        dentist("Zahnarzt drüben", "LAST-MODIFIED:20990101T000000Z"),
    );
    changes(
        address,
        &token,
        json!([
            {"op": "set", "object": a["id"], "field": "title", "value": "Zahnarzt samu"},
            {"op": "set", "object": a["id"], "field": "location", "value": "Praxis"}
        ]),
    )
    .await;
    written(&dir).await;
    let (text, _) = dav.lock().unwrap().resources.get(DENTIST).unwrap().clone();
    assert!(
        text.contains("SUMMARY:Zahnarzt drüben\r\n"),
        "theirs is later: {text}"
    );
    assert!(
        text.contains("LOCATION:Praxis\r\n"),
        "samu's location stays"
    );
    assert_eq!(
        ask_text(&dir, "SELECT title FROM appointment").as_deref(),
        Some("Zahnarzt drüben")
    );
}

#[tokio::test]
async fn a_server_that_refuses_leaves_an_error_and_nothing_queued() {
    let dav = with_dentist(true);
    let (address, dir, token, calendar) = setup(&dav).await;
    let a = appointment(address, &token, "dentist@example.org").await;
    dav.lock().unwrap().forbid = true;
    changes(
        address,
        &token,
        json!([{"op": "set", "object": a["id"], "field": "title", "value": "x"}]),
    )
    .await;
    written(&dir).await;
    let c = call(
        address,
        "GET",
        &format!("/objects/{calendar}"),
        Some(&token),
        None,
    )
    .await;
    let error = c.json()["object"]["error"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(error.contains("403"), "{}", c.body);
    assert!(
        dav.lock().unwrap().resources[DENTIST]
            .0
            .contains("SUMMARY:Zahnarzt\r\n")
    );
}

#[tokio::test]
async fn a_resource_gone_elsewhere_goes_here_too() {
    let dav = with_dentist(true);
    let (address, dir, token, _) = setup(&dav).await;
    let a = appointment(address, &token, "dentist@example.org").await;
    dav.lock().unwrap().resources.clear();
    changes(
        address,
        &token,
        json!([{"op": "set", "object": a["id"], "field": "title", "value": "x"}]),
    )
    .await;
    written(&dir).await;
    assert_eq!(
        ask(
            &dir,
            "SELECT count(*) FROM object WHERE kind = 'appointment' AND deleted_at IS NULL"
        ),
        0
    );
}

#[tokio::test]
async fn a_deleted_appointment_is_deleted_there_with_its_etag() {
    let dav = with_dentist(true);
    let (address, dir, token, _) = setup(&dav).await;
    let a = appointment(address, &token, "dentist@example.org").await;
    let answer = changes(
        address,
        &token,
        json!([{"op": "delete", "object": a["id"]}]),
    )
    .await;
    assert_eq!(answer[0]["outcome"], "applied", "{answer}");
    written(&dir).await;
    let dav = dav.lock().unwrap();
    assert!(dav.resources.is_empty());
    assert_eq!(dav.log, vec![format!("DELETE {DENTIST} {}", etag(1))]);
}

#[tokio::test]
async fn a_calendar_without_write_takes_no_change() {
    let dav = with_dentist(false);
    let (address, dir, token, _) = setup(&dav).await;
    let a = appointment(address, &token, "dentist@example.org").await;
    assert_eq!(ask(&dir, "SELECT writable FROM calendar"), 0);
    let answer = changes(
        address,
        &token,
        json!([{"op": "set", "object": a["id"], "field": "title", "value": "x"}]),
    )
    .await;
    assert_eq!(answer[0]["outcome"], "rejected", "{answer}");
    assert_eq!(ask(&dir, QUEUED), 0);
    assert!(dav.lock().unwrap().log.is_empty());
}

const CHOIR: &str = "/dav/w/choir.ics";

/// Choir every week from tomorrow, four times.
fn choir() -> String {
    let day = tomorrow().strftime("%Y%m%d");
    [
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        "BEGIN:VEVENT",
        "UID:choir@example.org",
        "SUMMARY:Chor",
        &format!("DTSTART;TZID=Europe/Berlin:{day}T193000"),
        &format!("DTEND;TZID=Europe/Berlin:{day}T210000"),
        "RRULE:FREQ=WEEKLY;COUNT=4",
        "END:VEVENT",
        "END:VCALENDAR",
        "",
    ]
    .join("\r\n")
}

/// The start of the n-th occurrence (0 first), as the API writes it.
fn occurrence(n: i64) -> Value {
    let day = tomorrow()
        .checked_add(jiff::Span::new().days(7 * n))
        .unwrap();
    let at = day
        .at(19, 30, 0, 0)
        .to_zoned(jiff::tz::TimeZone::get("Europe/Berlin").unwrap())
        .unwrap();
    json!({"at": at.to_string()})
}

#[tokio::test]
async fn one_occurrence_and_the_rest_of_a_series_are_edited_over_the_route() {
    let dav = with_dentist(true);
    dav.lock().unwrap().put(CHOIR, choir());
    let (address, dir, token, _) = setup(&dav).await;
    let a = appointment(address, &token, "choir@example.org").await;
    let path = format!("/appointments/{}/edit", a["id"].as_str().unwrap());

    // The second one later and with another title.
    let this = json!({"instance": occurrence(1), "range": "this",
                      "patch": {"title": "Chor (Probe)"}});
    let answer = call(address, "POST", &path, Some(&token), Some(&this)).await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    assert_eq!(answer.json()["results"][0]["change"]["op"], "create");
    written(&dir).await;
    let text = dav.lock().unwrap().resources[CHOIR].0.clone();
    assert!(text.contains("RECURRENCE-ID;TZID=Europe/Berlin:"), "{text}");
    assert!(text.contains("SUMMARY:Chor (Probe)\r\n"));
    assert!(text.contains("SUMMARY:Chor\r\n"), "the series stays");

    // From the third on: gone. COUNT is cut to two.
    let following = json!({"instance": occurrence(2), "range": "following", "delete": true});
    let answer = call(address, "POST", &path, Some(&token), Some(&following)).await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    written(&dir).await;
    let text = dav.lock().unwrap().resources[CHOIR].0.clone();
    assert!(text.contains("RRULE:FREQ=WEEKLY;COUNT=2\r\n"), "{text}");
    assert!(
        text.contains("SUMMARY:Chor (Probe)\r\n"),
        "the second stays changed"
    );

    // Not an occurrence: 422; an id that is none: 404.
    let wrong = json!({"instance": {"at": "2001-01-01T10:00:00+01:00[Europe/Berlin]"},
                       "range": "this", "delete": true});
    assert_eq!(
        call(address, "POST", &path, Some(&token), Some(&wrong))
            .await
            .status,
        422
    );
    let none = format!("/appointments/{}/edit", Id::new());
    assert_eq!(
        call(address, "POST", &none, Some(&token), Some(&wrong))
            .await
            .status,
        404
    );
}

/// A request with a body of any type, like an iCalendar file.
async fn call_text(address: SocketAddr, path: &str, token: &str, body: &[u8]) -> Answer {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\
         Authorization: Bearer {token}\r\nContent-Type: text/calendar\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).await.unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap();
    Answer {
        status: head.split(' ').nth(1).unwrap().parse().unwrap(),
        headers: head.to_string(),
        body: body.to_string(),
    }
}

/// ADR 0100 §4: a file goes in resource by resource, as it came: a UID the
/// calendar holds replaces its resource with `If-Match`, a new one goes
/// with `If-None-Match: *`, what is no appointment is left out; then the
/// calendar is fetched and the import is in its logbook.
#[tokio::test]
async fn an_ics_file_is_imported_resource_by_resource() {
    let dav = with_dentist(true);
    let (address, dir, token, calendar) = setup(&dav).await;
    let day = tomorrow().strftime("%Y%m%d");
    let file = [
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        "PRODID:-//old//EN",
        "METHOD:PUBLISH",
        "BEGIN:VEVENT",
        "UID:dentist@example.org",
        "SUMMARY:Zahnarzt, verlegt",
        &format!("DTSTART;TZID=Europe/Berlin:{day}T110000"),
        &format!("DTEND;TZID=Europe/Berlin:{day}T114500"),
        "END:VEVENT",
        "BEGIN:VEVENT",
        "UID:konzert@example.org",
        "SUMMARY:Konzert",
        &format!("DTSTART;TZID=Europe/Berlin:{day}T200000"),
        "X-KEPT:yes",
        "BEGIN:VALARM",
        "ACTION:DISPLAY",
        "TRIGGER:-PT1H",
        "END:VALARM",
        "END:VEVENT",
        "BEGIN:VTODO",
        "UID:todo@example.org",
        "SUMMARY:Karten holen",
        "END:VTODO",
        "END:VCALENDAR",
        "",
    ]
    .join("\r\n");
    let path = format!("/calendars/{calendar}/import?name=termine.ics");
    let answer = call_text(address, &path, &token, file.as_bytes()).await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    assert_eq!(
        answer.json(),
        json!({"created": 1, "replaced": 1,
               "skipped": [{"uid": "todo@example.org", "reason": "a VTODO is no appointment"}]})
    );
    {
        let dav = dav.lock().unwrap();
        assert_eq!(
            dav.log,
            vec![
                format!("PUT {DENTIST} {}", etag(1)),
                "PUT /dav/w/konzert@example.org.ics ".to_string(),
            ]
        );
        let (konzert, _) = dav.resources.get("/dav/w/konzert@example.org.ics").unwrap();
        assert!(
            konzert.contains("BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT1H"),
            "{konzert}"
        );
        assert!(konzert.contains("X-KEPT:yes"), "{konzert}");
        assert!(!konzert.contains("METHOD"), "{konzert}");
        assert!(konzert.contains("BEGIN:VTIMEZONE"), "{konzert}");
    }
    // Fetched at once: in the database as the CalDAV server has them.
    assert_eq!(
        ask_text(
            &dir,
            "SELECT title FROM appointment WHERE uid = 'dentist@example.org'"
        )
        .as_deref(),
        Some("Zahnarzt, verlegt")
    );
    assert_eq!(
        ask(
            &dir,
            "SELECT count(*) FROM appointment WHERE uid = 'konzert@example.org'"
        ),
        1
    );
    let log = call(
        address,
        "GET",
        &format!("/objects/{calendar}/logbook"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(log.status, 200, "{}", log.body);
    let entry = log
        .json()
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["kind"] == "imported")
        .cloned();
    assert_eq!(
        entry.map(|e| e["note"].clone()),
        Some(json!("termine.ics: 1 new, 1 replaced, 1 skipped, 0 failed")),
        "{}",
        log.body
    );

    // Again: both are known now and replaced; nothing is doubled.
    let again = call_text(address, &path, &token, file.as_bytes()).await;
    assert_eq!(again.json()["replaced"], 2, "{}", again.body);
    assert_eq!(dav.lock().unwrap().resources.len(), 2);

    // Not text, not iCalendar.
    let answer = call_text(address, &path, &token, &[0xff, 0xfe, 0x00]).await;
    assert_eq!(answer.status, 400, "{}", answer.body);
    let answer = call_text(address, &path, &token, b"hello").await;
    assert_eq!(answer.status, 400, "{}", answer.body);
}

/// ADR 0100 §4: only into a calendar samu may write to.
#[tokio::test]
async fn an_import_needs_a_writable_calendar() {
    let dav = with_dentist(false);
    let (address, _dir, token, calendar) = setup(&dav).await;
    let path = format!("/calendars/{calendar}/import");
    let answer = call_text(address, &path, &token, dentist("x", "").as_bytes()).await;
    assert_eq!(answer.status, 400, "{}", answer.body);
    assert!(answer.body.contains("not writable"), "{}", answer.body);
    assert!(dav.lock().unwrap().log.is_empty());
}
