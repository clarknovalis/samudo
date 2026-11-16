// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Calendars over the API (ADR 0047, 0048): a source is set, the server
//! fetches it from a small ICS server in the test, the appointments come
//! over `/changes` and `/appointments`. Asked of the database, not of the
//! answers. Synthetic data only; the dates are counted from today so the
//! window of the server holds them whenever this runs.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use jiff::ToSpan;
use jiff::civil::Date;
use samu_store::SecretKey;

use super::*;

fn today() -> Date {
    Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::get("Europe/Berlin").unwrap())
        .date()
}

fn ics_date(d: Date) -> String {
    d.strftime("%Y%m%d").to_string()
}

/// Three appointments: tomorrow at nine, a weekly one since a week ago,
/// and one over today and the next two days.
fn ics() -> String {
    let t = today();
    let tomorrow = ics_date(t.tomorrow().unwrap());
    let week_ago = ics_date(t.checked_sub(7.days()).unwrap());
    let start = ics_date(t);
    let end = ics_date(t.checked_add(3.days()).unwrap());
    [
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        "BEGIN:VEVENT",
        "UID:dentist@example.org",
        "SUMMARY:Zahnarzt",
        &format!("DTSTART;TZID=Europe/Berlin:{tomorrow}T090000"),
        &format!("DTEND;TZID=Europe/Berlin:{tomorrow}T094500"),
        "END:VEVENT",
        "BEGIN:VEVENT",
        "UID:choir@example.org",
        "SUMMARY:Chor",
        &format!("DTSTART;TZID=Europe/Berlin:{week_ago}T193000"),
        &format!("DTEND;TZID=Europe/Berlin:{week_ago}T210000"),
        "RRULE:FREQ=WEEKLY",
        "END:VEVENT",
        "BEGIN:VEVENT",
        "UID:trip@example.org",
        "SUMMARY:Reise",
        &format!("DTSTART;VALUE=DATE:{start}"),
        &format!("DTEND;VALUE=DATE:{end}"),
        "END:VEVENT",
        "END:VCALENDAR",
        "",
    ]
    .join("\r\n")
}

/// Two appointments with VALARMs (ADR 0101): tomorrow at nine, 15
/// minutes before, and a weekly one since a week ago, 10 minutes before;
/// an EMAIL alarm is left out.
fn alarm_ics() -> String {
    let t = today();
    let tomorrow = ics_date(t.tomorrow().unwrap());
    let week_ago = ics_date(t.checked_sub(7.days()).unwrap());
    [
        "BEGIN:VCALENDAR",
        "VERSION:2.0",
        "BEGIN:VEVENT",
        "UID:dentist@example.org",
        "SUMMARY:Zahnarzt",
        "LOCATION:Hauptstraße 1",
        &format!("DTSTART;TZID=Europe/Berlin:{tomorrow}T090000"),
        &format!("DTEND;TZID=Europe/Berlin:{tomorrow}T094500"),
        "BEGIN:VALARM",
        "ACTION:DISPLAY",
        "TRIGGER:-PT15M",
        "END:VALARM",
        "BEGIN:VALARM",
        "ACTION:EMAIL",
        "TRIGGER:-PT1H",
        "END:VALARM",
        "END:VEVENT",
        "BEGIN:VEVENT",
        "UID:choir@example.org",
        "SUMMARY:Chor",
        &format!("DTSTART;TZID=Europe/Berlin:{week_ago}T193000"),
        &format!("DTEND;TZID=Europe/Berlin:{week_ago}T210000"),
        "RRULE:FREQ=WEEKLY",
        "BEGIN:VALARM",
        "ACTION:AUDIO",
        "TRIGGER:-PT10M",
        "END:VALARM",
        "END:VEVENT",
        "END:VCALENDAR",
        "",
    ]
    .join("\r\n")
}

/// An ICS server: `/cal.ics` with an ETag and `304`, `/locked.ics` behind
/// Basic auth, `/page.ics` a sign-in page. Counts the full deliveries.
async fn ics_server() -> (SocketAddr, Arc<AtomicUsize>) {
    let served = Arc::new(AtomicUsize::new(0));
    let counter = served.clone();
    let app = axum::Router::new()
        .route(
            "/cal.ics",
            get(move |headers: HeaderMap| {
                let counter = counter.clone();
                async move {
                    if headers.get("if-none-match").and_then(|v| v.to_str().ok()) == Some("\"v1\"")
                    {
                        return StatusCode::NOT_MODIFIED.into_response();
                    }
                    counter.fetch_add(1, Ordering::SeqCst);
                    (
                        [("etag", "\"v1\""), ("content-type", "text/calendar")],
                        ics(),
                    )
                        .into_response()
                }
            }),
        )
        .route(
            "/locked.ics",
            get(|headers: HeaderMap| async move {
                // alice:secret
                if headers.get("authorization").and_then(|v| v.to_str().ok())
                    == Some("Basic YWxpY2U6c2VjcmV0")
                {
                    ics().into_response()
                } else {
                    StatusCode::UNAUTHORIZED.into_response()
                }
            }),
        )
        .route(
            "/alarm.ics",
            get(|| async { ([("content-type", "text/calendar")], alarm_ics()) }),
        )
        .route(
            "/page.ics",
            get(|| async { "<!DOCTYPE html><html><body>Sign in</body></html>" }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (address, served)
}

fn key() -> SecretKey {
    SecretKey::new([3; 32])
}

async fn add_calendar(address: SocketAddr, token: &str) -> String {
    add_calendar_of(address, token, "ics").await
}

async fn add_calendar_of(address: SocketAddr, token: &str, kind: &str) -> String {
    let id = samu_store::core::api::Id::new().to_string();
    let value = json!({"id": id, "created": "2026-11-09T08:00:00Z",
                       "modified": "2026-11-09T08:00:00Z", "title": "Familie",
                       "kind": kind, "enabled": true, "refresh": 15, "position": 0});
    let answer = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(token),
        Some(
            &json!([{"op": "create", "object": id, "kind": "calendar", "value": value,
                      "hlc": {"wall": 0, "counter": 0, "device": "ignored"}}]),
        ),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    id
}

fn ask(dir: &TempDir, sql: &str) -> i64 {
    let conn = rusqlite::Connection::open(dir.0.join("accounts/privat.db")).unwrap();
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

const LIVE: &str = "SELECT count(*) FROM object WHERE kind = 'appointment' AND deleted_at IS NULL";

#[tokio::test]
async fn a_calendar_is_fetched_listed_and_left_alone_when_unchanged() {
    let (ics, served) = ics_server().await;
    let (address, dir) = start_with_key(true, Some(key())).await;
    let token = sign_in(address).await;
    let calendar = add_calendar(address, &token).await;

    let path = format!("/calendars/{calendar}/source");
    let url = format!("http://{ics}/cal.ics");
    let put = call(
        address,
        "PUT",
        &path,
        Some(&token),
        Some(&json!({"url": url})),
    )
    .await;
    assert_eq!(put.status, 204, "{}", put.body);
    let refresh = format!("/calendars/{calendar}/refresh");
    let answer = call(address, "POST", &refresh, Some(&token), None).await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    assert_eq!(answer.json()["host"], "127.0.0.1");
    assert!(answer.json().get("error").is_none(), "{}", answer.body);
    assert_eq!(ask(&dir, LIVE), 3);

    // The secret never comes back: not in the calendar, not in /changes.
    let changes = call(address, "GET", "/changes?after=0", Some(&token), None).await;
    assert!(!changes.body.contains("/cal.ics"), "{}", changes.body);
    let kinds = changes.json()["objects"]
        .as_array()
        .map(|list| list.iter().filter(|o| o["kind"] == "appointment").count())
        .unwrap_or_default();
    assert_eq!(kinds, 3, "{}", changes.body);

    // Today to a week ahead: the trip on its three days counts once, the
    // choir once a week, the dentist tomorrow.
    let from = today();
    let to = from.checked_add(6.days()).unwrap();
    let list = call(
        address,
        "GET",
        &format!("/appointments?from={from}&to={to}"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(list.status, 200, "{}", list.body);
    let titles: Vec<String> = list
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["title"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(titles.iter().filter(|t| *t == "Reise").count(), 1);
    assert_eq!(titles.iter().filter(|t| *t == "Chor").count(), 1);
    assert_eq!(titles.iter().filter(|t| *t == "Zahnarzt").count(), 1);
    assert_eq!(titles[0], "Reise", "a whole day before the timed ones");

    // Fetched again: 304, nothing written.
    let seq = ask(&dir, "SELECT seq FROM account");
    let delivered = served.load(Ordering::SeqCst);
    assert!(delivered >= 1, "the probe must have delivered once");
    let again = call(address, "POST", &refresh, Some(&token), None).await;
    assert_eq!(again.status, 200, "{}", again.body);
    assert_eq!(
        served.load(Ordering::SeqCst),
        delivered,
        "the 304 was not used"
    );
    assert_eq!(ask(&dir, "SELECT seq FROM account"), seq);

    // Deleting the calendar takes the appointments along.
    let delete = call(
        address,
        "POST",
        &format!("/objects/{calendar}/delete"),
        Some(&token),
        Some(&json!({})),
    )
    .await;
    assert_eq!(delete.status, 200, "{}", delete.body);
    assert_eq!(ask(&dir, LIVE), 0);
    assert_eq!(ask(&dir, "SELECT count(*) FROM calendar_source"), 0);
}

#[tokio::test]
async fn a_refused_or_wrong_source_keeps_what_was_there_and_says_why() {
    let (ics, _served) = ics_server().await;
    let (address, dir) = start_with_key(true, Some(key())).await;
    let token = sign_in(address).await;
    let calendar = add_calendar(address, &token).await;
    let path = format!("/calendars/{calendar}/source");
    let refresh = format!("/calendars/{calendar}/refresh");

    let locked = json!({"url": format!("http://{ics}/locked.ics"),
                        "username": "alice", "password": "secret"});
    assert_eq!(
        call(address, "PUT", &path, Some(&token), Some(&locked))
            .await
            .status,
        204
    );
    let answer = call(address, "POST", &refresh, Some(&token), None).await;
    assert!(answer.json().get("error").is_none(), "{}", answer.body);
    assert_eq!(ask(&dir, LIVE), 3);

    let wrong = json!({"url": format!("http://{ics}/locked.ics"),
                       "username": "alice", "password": "wrong"});
    call(address, "PUT", &path, Some(&token), Some(&wrong)).await;
    let answer = call(address, "POST", &refresh, Some(&token), None).await;
    assert_eq!(
        answer.json()["error"],
        "401 Unauthorized",
        "{}",
        answer.body
    );
    assert_eq!(ask(&dir, LIVE), 3, "a failure keeps the appointments");

    let page = json!({"url": format!("http://{ics}/page.ics")});
    call(address, "PUT", &path, Some(&token), Some(&page)).await;
    let answer = call(address, "POST", &refresh, Some(&token), None).await;
    let error = answer.json()["error"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(error.starts_with("not an iCalendar file"), "{error}");
    assert!(
        !error.contains("page.ics"),
        "the address stays out: {error}"
    );
    assert_eq!(ask(&dir, LIVE), 3);

    // Nothing listens here: the error does not name the address either.
    let gone = json!({"url": "http://127.0.0.1:9/very-secret-token.ics"});
    call(address, "PUT", &path, Some(&token), Some(&gone)).await;
    let answer = call(address, "POST", &refresh, Some(&token), None).await;
    let error = answer.json()["error"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(!error.is_empty(), "{}", answer.body);
    assert!(!error.contains("very-secret-token"), "{error}");
}

#[tokio::test]
async fn without_a_key_no_source_is_taken() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    let calendar = add_calendar(address, &token).await;
    let answer = call(
        address,
        "PUT",
        &format!("/calendars/{calendar}/source"),
        Some(&token),
        Some(&json!({"url": "https://calendar.example.org/x.ics"})),
    )
    .await;
    assert_eq!(answer.status, 503, "{}", answer.body);
    assert!(answer.body.contains("SAMU_SECRET_KEY"), "{}", answer.body);
    // Without a source there is nothing to refresh.
    let answer = call(
        address,
        "POST",
        &format!("/calendars/{calendar}/refresh"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(answer.status, 400, "{}", answer.body);
}

#[tokio::test]
async fn appointments_refuses_a_span_backwards_or_too_long() {
    let (address, _dir) = start(true).await;
    let token = sign_in(address).await;
    for query in [
        "from=2026-11-10&to=2026-11-09",
        "from=2026-01-01&to=2027-12-31",
    ] {
        let answer = call(
            address,
            "GET",
            &format!("/appointments?{query}"),
            Some(&token),
            None,
        )
        .await;
        assert_eq!(answer.status, 400, "{query}: {}", answer.body);
    }
    let empty = call(address, "GET", "/appointments", Some(&token), None).await;
    assert_eq!(empty.status, 200);
    assert_eq!(empty.json(), json!([]));
}

/// A CalDAV collection at `/dav/familie/`, behind Basic auth (alice:secret):
/// `PROPFIND` answers the ctag `version`, `REPORT` two calendar objects, the
/// dentist's title carrying the version. Counts the REPORTs.
async fn caldav_server() -> (SocketAddr, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let version = Arc::new(AtomicUsize::new(1));
    let reports = Arc::new(AtomicUsize::new(0));
    let (v, r) = (version.clone(), reports.clone());
    let app = axum::Router::new().route(
        "/dav/familie/",
        axum::routing::any(
            move |method: axum::http::Method, headers: HeaderMap, body: String| {
                let (v, r) = (v.clone(), r.clone());
                async move {
                    if headers.get("authorization").and_then(|h| h.to_str().ok())
                        != Some("Basic YWxpY2U6c2VjcmV0")
                    {
                        return StatusCode::UNAUTHORIZED.into_response();
                    }
                    let n = v.load(Ordering::SeqCst);
                    let multi = StatusCode::from_u16(207).unwrap();
                    match method.as_str() {
                        "PROPFIND" => (
                            multi,
                            format!(
                                r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:cs="http://calendarserver.org/ns/"><d:response><d:href>/dav/familie/</d:href><d:propstat><d:prop><cs:getctag>"ctag-{n}"</cs:getctag></d:prop><d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response></d:multistatus>"#
                            ),
                        )
                            .into_response(),
                        "REPORT" if body.contains("calendar-query") && body.contains("time-range") => {
                            r.fetch_add(1, Ordering::SeqCst);
                            let escaped = ics()
                                .replace('&', "&amp;")
                                .replace('<', "&lt;")
                                .replace("SUMMARY:Zahnarzt", &format!("SUMMARY:Zahnarzt {n} &amp; Co"));
                            // One object per VEVENT, as a CalDAV server keeps them.
                            let objects: Vec<String> = escaped
                                .split("BEGIN:VEVENT")
                                .skip(1)
                                .map(|e| {
                                    let event = e.split("END:VEVENT").next().unwrap_or_default();
                                    format!(
                                        "<d:response><d:href>/dav/familie/x.ics</d:href><d:propstat><d:prop><d:getetag>\"e\"</d:getetag><c:calendar-data>BEGIN:VCALENDAR\r\nBEGIN:VEVENT{event}END:VEVENT\r\nEND:VCALENDAR\r\n</c:calendar-data></d:prop></d:propstat></d:response>"
                                    )
                                })
                                .collect();
                            (
                                multi,
                                format!(
                                    r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">{}</d:multistatus>"#,
                                    objects.join("")
                                ),
                            )
                                .into_response()
                        }
                        _ => StatusCode::METHOD_NOT_ALLOWED.into_response(),
                    }
                }
            },
        ),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (address, version, reports)
}

#[tokio::test]
async fn a_caldav_collection_is_read_and_asked_again_only_when_its_ctag_moves() {
    let (dav, version, reports) = caldav_server().await;
    let (address, dir) = start_with_key(true, Some(key())).await;
    let token = sign_in(address).await;
    let calendar = add_calendar_of(address, &token, "caldav").await;
    let path = format!("/calendars/{calendar}/source");
    let refresh = format!("/calendars/{calendar}/refresh");
    let source = json!({"url": format!("http://{dav}/dav/familie/"),
                        "username": "alice", "password": "secret"});
    assert_eq!(
        call(address, "PUT", &path, Some(&token), Some(&source))
            .await
            .status,
        204
    );
    let answer = call(address, "POST", &refresh, Some(&token), None).await;
    assert!(answer.json().get("error").is_none(), "{}", answer.body);
    assert_eq!(ask(&dir, LIVE), 3);
    assert_eq!(
        ask(
            &dir,
            "SELECT count(*) FROM appointment WHERE title = 'Zahnarzt 1 & Co'"
        ),
        1,
        "the XML was unescaped"
    );

    // The same ctag: no REPORT, nothing written.
    let asked = reports.load(Ordering::SeqCst);
    assert!(asked >= 1, "the probe must have reported once");
    let seq = ask(&dir, "SELECT seq FROM account");
    call(address, "POST", &refresh, Some(&token), None).await;
    assert_eq!(
        reports.load(Ordering::SeqCst),
        asked,
        "asked again without need"
    );
    assert_eq!(ask(&dir, "SELECT seq FROM account"), seq);

    // A new ctag: asked again, the one change written.
    version.store(2, Ordering::SeqCst);
    call(address, "POST", &refresh, Some(&token), None).await;
    assert_eq!(reports.load(Ordering::SeqCst), asked + 1);
    assert_eq!(
        ask(
            &dir,
            "SELECT count(*) FROM appointment a JOIN object o ON o.id = a.object \
             WHERE o.deleted_at IS NULL AND a.title = 'Zahnarzt 2 & Co'"
        ),
        1
    );
    assert_eq!(ask(&dir, LIVE), 3);

    // A wrong password: said, and the appointments stay.
    let wrong = json!({"url": format!("http://{dav}/dav/familie/"),
                       "username": "alice", "password": "wrong"});
    call(address, "PUT", &path, Some(&token), Some(&wrong)).await;
    let answer = call(address, "POST", &refresh, Some(&token), None).await;
    assert_eq!(
        answer.json()["error"],
        "401 Unauthorized",
        "{}",
        answer.body
    );
    assert_eq!(ask(&dir, LIVE), 3);
}

/// ADR 0047 §10: Forecast shows the appointments on their days, whole days
/// first; with forecast_events off, none.
#[tokio::test]
async fn forecast_shows_the_appointments_of_each_day_unless_switched_off() {
    let (ics, _served) = ics_server().await;
    let (address, _dir) = start_with_key(true, Some(key())).await;
    let token = sign_in(address).await;
    let calendar = add_calendar(address, &token).await;
    let path = format!("/calendars/{calendar}/source");
    call(
        address,
        "PUT",
        &path,
        Some(&token),
        Some(&json!({"url": format!("http://{ics}/cal.ics")})),
    )
    .await;
    let refresh = format!("/calendars/{calendar}/refresh");
    let answer = call(address, "POST", &refresh, Some(&token), None).await;
    assert!(answer.json().get("error").is_none(), "{}", answer.body);

    let day_titles = |forecast: &Value, day: Date| -> Vec<String> {
        forecast
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["section"] == "day" && s["date"] == day.to_string())
            .map(|s| {
                s["appointments"]
                    .as_array()
                    .map(|list| {
                        list.iter()
                            .map(|a| a["title"].as_str().unwrap().to_string())
                            .collect()
                    })
                    .unwrap_or_default()
            })
            .unwrap_or_default()
    };
    let forecast = call(address, "GET", "/forecast", Some(&token), None).await;
    assert_eq!(forecast.status, 200, "{}", forecast.body);
    let forecast = forecast.json();
    let t = today();
    // The choir began a week ago on this weekday: today and in a week.
    assert_eq!(
        day_titles(&forecast, t),
        vec!["Reise", "Chor"],
        "{forecast}"
    );
    assert_eq!(
        day_titles(&forecast, t.tomorrow().unwrap()),
        vec!["Reise", "Zahnarzt"]
    );
    assert_eq!(
        day_titles(&forecast, t.checked_add(2.days()).unwrap()),
        vec!["Reise"]
    );
    assert!(day_titles(&forecast, t.checked_add(3.days()).unwrap()).is_empty());
    // A day with nothing but an appointment has a section of its own.
    assert_eq!(
        day_titles(&forecast, t.checked_add(7.days()).unwrap()),
        vec!["Chor"]
    );
    for section in forecast.as_array().unwrap() {
        if section["section"] != "day" {
            assert!(section.get("appointments").is_none(), "{section}");
        }
    }

    // Switched off: no appointments anywhere.
    let changes = call(address, "GET", "/changes?after=0", Some(&token), None).await;
    let settings = changes.json()["objects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["kind"] == "settings")
        .map(|o| o["object"]["id"].as_str().unwrap().to_string())
        .unwrap();
    let off = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(
            &json!([{"op": "set", "object": settings, "field": "forecast_events",
                      "value": false, "hlc": {"wall": 0, "counter": 0, "device": "x"}}]),
        ),
    )
    .await;
    assert_eq!(off.status, 200, "{}", off.body);
    let forecast = call(address, "GET", "/forecast", Some(&token), None).await;
    assert!(
        !forecast.body.contains("\"appointments\""),
        "{}",
        forecast.body
    );
}

/// ADR 0101 §2, §5: the VALARMs come as `alarms` and go off in
/// `GET /alarms`, only as long as the calendar is on, never for a window
/// over seven days.
#[tokio::test]
async fn alarms_of_appointments_are_read_and_listed() {
    let (ics, _served) = ics_server().await;
    let (address, dir) = start_with_key(true, Some(key())).await;
    let token = sign_in(address).await;
    let calendar = add_calendar(address, &token).await;
    call(
        address,
        "PUT",
        &format!("/calendars/{calendar}/source"),
        Some(&token),
        Some(&json!({"url": format!("http://{ics}/alarm.ics")})),
    )
    .await;
    let answer = call(
        address,
        "POST",
        &format!("/calendars/{calendar}/refresh"),
        Some(&token),
        None,
    )
    .await;
    assert!(answer.json().get("error").is_none(), "{}", answer.body);
    assert_eq!(
        ask(
            &dir,
            "SELECT count(*) FROM appointment WHERE alarms = '[{\"before\":\"PT15M\"}]'"
        ),
        1,
        "stored as the wire has it, without EMAIL"
    );

    let zone = jiff::tz::TimeZone::get("Europe/Berlin").unwrap();
    let t = today();
    let from = t.to_zoned(zone.clone()).unwrap().timestamp();
    let to = t
        .checked_add(2.days())
        .unwrap()
        .to_zoned(zone.clone())
        .unwrap()
        .timestamp();
    let path = format!("/alarms?from={from}&to={to}");
    let alarms = call(address, "GET", &path, Some(&token), None).await;
    assert_eq!(alarms.status, 200, "{}", alarms.body);
    let said: Vec<String> = alarms
        .json()
        .as_array()
        .unwrap()
        .iter()
        .map(|a| {
            let at: Timestamp = a["at"].as_str().unwrap().parse().unwrap();
            format!(
                "{} {}",
                at.to_zoned(zone.clone()).strftime("%d %H:%M"),
                a["title"].as_str().unwrap()
            )
        })
        .collect();
    let day = |d: Date| d.strftime("%d").to_string();
    // The choir is on today's weekday: today, not again within two days.
    assert_eq!(
        said,
        vec![
            format!("{} 19:20 Chor", day(t)),
            format!("{} 08:45 Zahnarzt", day(t.tomorrow().unwrap())),
        ],
        "{}",
        alarms.body
    );
    let first = &alarms.json()[1];
    assert_eq!(first["location"], "Hauptstraße 1");
    assert_eq!(first["calendar"], calendar.as_str());
    assert!(first.get("owner").is_none());

    let none = call(
        address,
        "GET",
        &format!("{path}&appointments=false"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(none.json(), json!([]), "{}", none.body);
    let long = format!(
        "/alarms?from={from}&to={}",
        from.checked_add(jiff::SignedDuration::from_hours(8 * 24))
            .unwrap()
    );
    let long = call(address, "GET", &long, Some(&token), None).await;
    assert_eq!(long.status, 400, "{}", long.body);

    let off = call(
        address,
        "POST",
        "/changes?stamp=server",
        Some(&token),
        Some(
            &json!([{"op": "set", "object": calendar, "field": "enabled",
                       "value": false, "hlc": {"wall": 0, "counter": 0, "device": "x"}}]),
        ),
    )
    .await;
    assert_eq!(off.status, 200, "{}", off.body);
    let quiet = call(address, "GET", &path, Some(&token), None).await;
    assert_eq!(quiet.json(), json!([]), "{}", quiet.body);
}

/// A CalDAV server to be found (ADR 0047 §8): `.well-known/caldav` moves
/// to `/dav/`, which names the principal, which names the home, which
/// holds a calendar with name and colour, one without either, and one for
/// tasks only. Anything but a signed-in PROPFIND is refused, so a redirect
/// that turned into a GET fails.
async fn discoverable_server() -> SocketAddr {
    async fn answer(
        uri: axum::http::Uri,
        method: axum::http::Method,
        headers: HeaderMap,
    ) -> axum::response::Response {
        if uri.path() == "/.well-known/caldav" {
            return (
                StatusCode::MOVED_PERMANENTLY,
                [(axum::http::header::LOCATION, "/dav/")],
            )
                .into_response();
        }
        if method.as_str() != "PROPFIND" {
            return StatusCode::METHOD_NOT_ALLOWED.into_response();
        }
        if headers.get("authorization").and_then(|h| h.to_str().ok())
            != Some("Basic YWxpY2U6c2VjcmV0")
        {
            return StatusCode::UNAUTHORIZED.into_response();
        }
        let response = |href: &str, props: &str| {
            format!(
                "<d:response><d:href>{href}</d:href><d:propstat><d:prop>{props}</d:prop>\
                 <d:status>HTTP/1.1 200 OK</d:status></d:propstat></d:response>"
            )
        };
        let calendar = "<d:resourcetype><d:collection/><c:calendar/></d:resourcetype>";
        let body = match uri.path() {
            "/dav/" => response(
                "/dav/",
                "<d:current-user-principal><d:href>/dav/principals/alice/</d:href>\
                 </d:current-user-principal><d:resourcetype><d:collection/></d:resourcetype>",
            ),
            "/dav/principals/alice/" => response(
                "/dav/principals/alice/",
                "<c:calendar-home-set><d:href>/dav/calendars/alice/</d:href></c:calendar-home-set>",
            ),
            "/dav/calendars/alice/" => [
                response(
                    "/dav/calendars/alice/",
                    "<d:resourcetype><d:collection/></d:resourcetype>",
                ),
                response(
                    "/dav/calendars/alice/familie/",
                    &format!(
                        "{calendar}<d:displayname>Familie</d:displayname>\
                         <a:calendar-color>#22AA44FF</a:calendar-color>\
                         <c:supported-calendar-component-set><c:comp name=\"VEVENT\"/>\
                         </c:supported-calendar-component-set>"
                    ),
                ),
                response("/dav/calendars/alice/dienst/", calendar),
                response(
                    "/dav/calendars/alice/aufgaben/",
                    &format!(
                        "{calendar}<d:displayname>Aufgaben</d:displayname>\
                         <c:supported-calendar-component-set><c:comp name=\"VTODO\"/>\
                         </c:supported-calendar-component-set>"
                    ),
                ),
            ]
            .concat(),
            "/dav/calendars/alice/familie/" => response(
                "/dav/calendars/alice/familie/",
                &format!("{calendar}<d:displayname>Familie</d:displayname>"),
            ),
            _ => return StatusCode::NOT_FOUND.into_response(),
        };
        (
            StatusCode::from_u16(207).unwrap(),
            format!(
                r#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:a="http://apple.com/ns/ical/">{body}</d:multistatus>"#
            ),
        )
            .into_response()
    }
    let app = axum::Router::new().fallback(answer);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    address
}

#[tokio::test]
async fn discovery_finds_the_collections_with_appointments_and_makes_nothing() {
    let dav = discoverable_server().await;
    let (address, dir) = start_with_key(true, Some(key())).await;
    let token = sign_in(address).await;
    let discover = |url: String, password: &str| json!({"url": url, "username": "alice", "password": password});

    // From the bare host: well-known, redirected, principal, home.
    let answer = call(
        address,
        "POST",
        "/calendars/discover",
        Some(&token),
        Some(&discover(format!("http://{dav}"), "secret")),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    assert_eq!(
        answer.json(),
        json!([
            {"url": format!("http://{dav}/dav/calendars/alice/familie/"),
             "title": "Familie", "color": "#22aa44"},
            {"url": format!("http://{dav}/dav/calendars/alice/dienst/"), "title": "dienst"},
        ])
    );

    // The address of a collection is that collection.
    let answer = call(
        address,
        "POST",
        "/calendars/discover",
        Some(&token),
        Some(&discover(
            format!("http://{dav}/dav/calendars/alice/familie/"),
            "secret",
        )),
    )
    .await;
    assert_eq!(answer.status, 200, "{}", answer.body);
    assert_eq!(answer.json().as_array().unwrap().len(), 1);
    assert_eq!(answer.json()[0]["title"], "Familie");

    // A wrong password is said, without the password.
    let answer = call(
        address,
        "POST",
        "/calendars/discover",
        Some(&token),
        Some(&discover(format!("http://{dav}/dav/"), "wrong")),
    )
    .await;
    assert_eq!(answer.status, 502, "{}", answer.body);
    assert!(answer.body.contains("401"), "{}", answer.body);
    assert!(!answer.body.contains("wrong"), "{}", answer.body);

    // Nothing was made: no calendar, no source.
    assert_eq!(ask(&dir, "SELECT count(*) FROM calendar"), 0);
    assert_eq!(ask(&dir, "SELECT count(*) FROM calendar_source"), 0);
}

/// ADR 0100 §1–2: more calendars from the server of one already there,
/// with its stored source, the subscribed ones marked; a new one takes
/// user and password over without them leaving the server, only for the
/// same host.
#[tokio::test]
async fn more_calendars_come_from_the_server_of_one_already_there() {
    let dav = discoverable_server().await;
    let (address, dir) = start_with_key(true, Some(key())).await;
    let token = sign_in(address).await;
    let familie = format!("http://{dav}/dav/calendars/alice/familie/");
    let dienst = format!("http://{dav}/dav/calendars/alice/dienst/");

    let first = add_calendar_of(address, &token, "caldav").await;
    let source = json!({"url": familie, "username": "alice", "password": "secret"});
    let set = call(
        address,
        "PUT",
        &format!("/calendars/{first}/source"),
        Some(&token),
        Some(&source),
    )
    .await;
    assert_eq!(set.status, 204, "{}", set.body);

    let found = call(
        address,
        "POST",
        &format!("/calendars/{first}/discover"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(found.status, 200, "{}", found.body);
    assert_eq!(
        found.json(),
        json!([
            {"url": familie, "title": "Familie", "color": "#22aa44", "known": true},
            {"url": dienst, "title": "dienst"},
        ])
    );

    // Taken over: the second finds with its own source, so it has the password.
    let second = add_calendar_of(address, &token, "caldav").await;
    let over = json!({"url": dienst, "credentials_from": first});
    let set = call(
        address,
        "PUT",
        &format!("/calendars/{second}/source"),
        Some(&token),
        Some(&over),
    )
    .await;
    assert_eq!(set.status, 204, "{}", set.body);
    let found = call(
        address,
        "POST",
        &format!("/calendars/{second}/discover"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(found.status, 200, "{}", found.body);
    assert!(
        found
            .json()
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["known"] == true),
        "{}",
        found.body
    );
    assert_eq!(ask(&dir, "SELECT count(*) FROM calendar_source"), 2);

    // Neither together with user or password, nor to another host, nor
    // from a calendar that is not CalDAV or not there.
    let third = add_calendar_of(address, &token, "caldav").await;
    let path = format!("/calendars/{third}/source");
    let both = json!({"url": dienst, "credentials_from": first, "password": "x"});
    let answer = call(address, "PUT", &path, Some(&token), Some(&both)).await;
    assert_eq!(answer.status, 400, "{}", answer.body);
    let elsewhere = json!({"url": "http://127.0.0.2:9/dav/", "credentials_from": first});
    let answer = call(address, "PUT", &path, Some(&token), Some(&elsewhere)).await;
    assert_eq!(answer.status, 400, "{}", answer.body);
    assert!(answer.body.contains("another host"), "{}", answer.body);
    let ics = add_calendar(address, &token).await;
    let from_ics = json!({"url": dienst, "credentials_from": ics});
    let answer = call(address, "PUT", &path, Some(&token), Some(&from_ics)).await;
    assert_eq!(answer.status, 400, "{}", answer.body);
    let nowhere =
        json!({"url": dienst, "credentials_from": "01936f5e-8a00-7c3e-9b1a-000000000009"});
    let answer = call(address, "PUT", &path, Some(&token), Some(&nowhere)).await;
    assert_eq!(answer.status, 404, "{}", answer.body);
    assert_eq!(ask(&dir, "SELECT count(*) FROM calendar_source"), 2);

    // Finding from an ICS calendar or one without a source is refused.
    let answer = call(
        address,
        "POST",
        &format!("/calendars/{ics}/discover"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(answer.status, 400, "{}", answer.body);
    let answer = call(
        address,
        "POST",
        &format!("/calendars/{third}/discover"),
        Some(&token),
        None,
    )
    .await;
    assert_eq!(answer.status, 400, "{}", answer.body);
}

/// ADR 0097 §3: with a list of hosts, an address elsewhere is refused as
/// a source and to discovery, and a redirect there is not followed.
#[tokio::test]
async fn only_the_allowed_hosts_are_fetched_from() {
    let (ics, served) = ics_server().await;
    let elsewhere = format!("http://localhost:{}/cal.ics", ics.port());
    let target = elsewhere.clone();
    let redirecting =
        axum::Router::new().route(
            "/away.ics",
            get(move || {
                let target = target.clone();
                async move {
                    (StatusCode::FOUND, [(axum::http::header::LOCATION, target)]).into_response()
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let away = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, redirecting).await.unwrap();
    });
    let (address, dir) = start_with(true, |state| {
        state
            .with_secret_key(Some(key()))
            .with_calendar_hosts(Some(vec!["127.0.0.1".into()]))
            .unwrap()
    })
    .await;
    let token = sign_in(address).await;
    let calendar = add_calendar(address, &token).await;
    let path = format!("/calendars/{calendar}/source");
    let refresh = format!("/calendars/{calendar}/refresh");
    let sources = "SELECT count(*) FROM calendar_source";

    for url in [elsewhere.clone(), elsewhere.replace("http://", "webcal://")] {
        let answer = call(
            address,
            "PUT",
            &path,
            Some(&token),
            Some(&json!({"url": url})),
        )
        .await;
        assert_eq!(answer.status, 400, "{url}: {}", answer.body);
        assert!(answer.body.contains("from that host"), "{}", answer.body);
    }
    assert_eq!(ask(&dir, sources), 0);
    let answer = call(
        address,
        "POST",
        "/calendars/discover",
        Some(&token),
        Some(&json!({"url": format!("http://localhost:{}/", ics.port())})),
    )
    .await;
    assert_eq!(answer.status, 400, "{}", answer.body);

    // Allowed, but sent on elsewhere: nothing is fetched.
    let redirected = json!({"url": format!("http://{away}/away.ics")});
    let answer = call(address, "PUT", &path, Some(&token), Some(&redirected)).await;
    assert_eq!(answer.status, 204, "{}", answer.body);
    let answer = call(address, "POST", &refresh, Some(&token), None).await;
    assert!(answer.json().get("error").is_some(), "{}", answer.body);
    assert_eq!(served.load(Ordering::SeqCst), 0);
    assert_eq!(ask(&dir, LIVE), 0);

    // On an allowed host it is fetched as ever.
    let direct = json!({"url": format!("http://{ics}/cal.ics")});
    call(address, "PUT", &path, Some(&token), Some(&direct)).await;
    let answer = call(address, "POST", &refresh, Some(&token), None).await;
    assert!(answer.json().get("error").is_none(), "{}", answer.body);
    assert_eq!(ask(&dir, LIVE), 3);
}
