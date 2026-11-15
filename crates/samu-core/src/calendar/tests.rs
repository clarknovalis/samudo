#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

use jiff::civil::date;
use samu_api::CalendarKind;

use super::*;

const GOOGLE: &str = include_str!("../../../../testdata/calendar/google.ics");
const EXCHANGE: &str = include_str!("../../../../testdata/calendar/exchange.ics");
const NEXTCLOUD: &str = include_str!("../../../../testdata/calendar/nextcloud.ics");
const BROKEN: &str = include_str!("../../../../testdata/calendar/broken.ics");

fn berlin() -> TimeZone {
    TimeZone::get("Europe/Berlin").unwrap()
}

fn id(n: u8) -> Id {
    format!("01936f5e-8a00-7c3e-9b1a-3f2d4c5b6f{n:02x}")
        .parse()
        .unwrap()
}

fn ts() -> Timestamp {
    "2026-11-09T08:00:00Z".parse().unwrap()
}

fn calendar(n: u8) -> Calendar {
    Calendar {
        id: id(n),
        created: ts(),
        modified: ts(),
        title: format!("calendar {n}"),
        kind: CalendarKind::Ics,
        color: None,
        enabled: true,
        refresh: 15,
        link_template: None,
        position: 0,
        fetched_at: None,
        error: None,
        host: None,
        writable: false,
    }
}

fn at(s: &str) -> When {
    When::At(s.parse().unwrap())
}

/// The appointments of a file in calendar `n`, as the store would keep them.
fn stored(text: &str, n: u8) -> Vec<Appointment> {
    read_ics(text, &berlin())
        .unwrap()
        .appointments
        .into_iter()
        .enumerate()
        .map(|(i, a)| a.into_appointment(id(100 + i as u8), id(n), ts(), ts()))
        .collect()
}

fn titles(instances: &[AppointmentInstance]) -> Vec<String> {
    instances
        .iter()
        .map(|i| match &i.start {
            When::Date(d) => format!("{d} {}", i.title),
            When::At(z) => format!(
                "{} {}",
                z.with_time_zone(berlin()).strftime("%Y-%m-%d %H:%M"),
                i.title
            ),
            When::Unknown => "?".into(),
        })
        .collect()
}

fn find<'a>(read: &'a Read, uid: &str) -> &'a ReadAppointment {
    read.appointments
        .iter()
        .find(|a| a.uid == uid && a.recurrence_id.is_none())
        .unwrap()
}

#[test]
fn a_google_export_reads_into_both_forms_of_time() {
    let read = read_ics(GOOGLE, &berlin()).unwrap();
    // Seven VEVENTs with a UID (three of them chor), one without.
    assert_eq!(read.appointments.len(), 7);
    assert_eq!(read.skipped, 1);
    assert!(read.unknown_zones.is_empty());

    let dentist = find(&read, "zahnarzt@example.org");
    assert_eq!(dentist.title, "Zahnarzt, Kontrolle");
    assert_eq!(dentist.note.as_deref(), Some("Karte mitnehmen\nUnd Zettel"));
    assert_eq!(dentist.location.as_deref(), Some("Hauptstraße 1, Stadt"));
    assert_eq!(
        dentist.start,
        at("2026-11-20T09:00:00+01:00[Europe/Berlin]")
    );
    assert_eq!(
        dentist.end,
        Some(at("2026-11-20T09:45:00+01:00[Europe/Berlin]"))
    );
    assert!(dentist.busy);

    let birthday = find(&read, "geburtstag@example.org");
    assert_eq!(birthday.start, When::Date(date(2026, 12, 5)));
    assert_eq!(birthday.end, Some(When::Date(date(2026, 12, 6))));
    assert!(!birthday.busy, "TRANSP:TRANSPARENT is free");

    let call = find(&read, "anruf@example.org");
    assert_eq!(call.start, at("2026-11-21T15:00:00+00:00[UTC]"));
    assert_eq!(
        call.url.as_deref(),
        Some("https://calendar.example.org/event/anruf")
    );

    let choir = find(&read, "chor@example.org");
    assert_eq!(choir.rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=TU"));
    assert_eq!(
        choir.exdate,
        vec![at("2026-11-24T19:30:00+01:00[Europe/Berlin]")]
    );
    let moved = read
        .appointments
        .iter()
        .find(|a| a.uid == "chor@example.org" && a.recurrence_id.is_some())
        .unwrap();
    assert!(matches!(
        moved.status,
        AppointmentStatus::Tentative | AppointmentStatus::Cancelled
    ));
}

#[test]
fn exchange_zones_are_read_and_an_unknown_one_is_named() {
    let read = read_ics(EXCHANGE, &berlin()).unwrap();
    assert_eq!(read.appointments.len(), 3);
    let jour = read
        .appointments
        .iter()
        .find(|a| a.title == "Jour fixe")
        .unwrap();
    assert_eq!(jour.start, at("2026-11-10T10:00:00+01:00[Europe/Berlin]"));
    assert_eq!(jour.location.as_deref(), Some("Raum 3.14"));
    let free = read
        .appointments
        .iter()
        .find(|a| a.title == "Mittag frei halten")
        .unwrap();
    assert!(!free.busy, "X-MICROSOFT-CDO-BUSYSTATUS:FREE is free");
    assert_eq!(
        read.unknown_zones.iter().collect::<Vec<_>>(),
        vec!["Customized Time Zone"]
    );
    // Read in the account's zone.
    let training = read
        .appointments
        .iter()
        .find(|a| a.title == "Schulung")
        .unwrap();
    assert_eq!(
        training.start,
        at("2026-11-13T09:00:00+01:00[Europe/Berlin]")
    );
}

#[test]
fn a_page_instead_of_a_calendar_is_an_error() {
    // A sign-in page, as a server answers for an address gone stale.
    assert!(matches!(
        read_ics(BROKEN, &berlin()),
        Err(CalendarError::Syntax(_))
    ));
    assert!(read_ics("", &berlin()).is_err());
}

#[test]
fn a_line_without_a_value_does_not_cost_the_appointment() {
    let text = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:x\r\nKAPUTT\r\n\
                DTSTART;VALUE=DATE:20261120\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let read = read_ics(text, &berlin()).unwrap();
    assert_eq!(read.appointments.len(), 1);
    assert_eq!(read.appointments[0].start, When::Date(date(2026, 11, 20)));
}

#[test]
fn a_weekly_rule_unfolds_with_its_exdate_its_moved_and_its_cancelled_instance() {
    let appointments = stored(GOOGLE, 1);
    let found = occurrences(
        &appointments,
        &[calendar(1)],
        &[],
        date(2026, 11, 9),
        date(2026, 12, 6),
        &berlin(),
    );
    assert_eq!(
        titles(&found),
        vec![
            "2026-11-10 19:30 Chor",
            // 17 November moved to the 18th.
            "2026-11-18 19:00 Chor (Generalprobe)",
            "2026-11-20 09:00 Zahnarzt, Kontrolle",
            "2026-11-21 16:00 Anruf New York",
            // 24 November is an EXDATE, 1 December cancelled.
            "2026-12-05 Geburtstag Oma",
        ]
    );
}

#[test]
fn a_repetition_keeps_its_wall_time_over_the_end_of_summer_time() {
    let appointments = stored(GOOGLE, 1);
    let found = occurrences(
        &appointments,
        &[calendar(1)],
        &[],
        date(2026, 10, 20),
        date(2026, 11, 3),
        &berlin(),
    );
    let choir: Vec<String> = found
        .iter()
        .filter(|i| i.title == "Chor")
        .map(|i| match &i.start {
            When::At(z) => z.to_string(),
            _ => String::new(),
        })
        .collect();
    assert_eq!(
        choir,
        vec![
            "2026-10-20T19:30:00+02:00[Europe/Berlin]",
            "2026-10-27T19:30:00+01:00[Europe/Berlin]",
            "2026-11-03T19:30:00+01:00[Europe/Berlin]",
        ]
    );
    // The length carries over.
    assert!(found.iter().filter(|i| i.title == "Chor").all(|i| {
        matches!((&i.start, &i.end), (When::At(s), Some(When::At(e))) if s.duration_until(e).as_mins() == 90)
    }));
}

#[test]
fn until_ends_a_rule_and_rdate_adds_to_it() {
    let appointments = stored(NEXTCLOUD, 1);
    let found = occurrences(
        &appointments,
        &[calendar(1)],
        &[],
        date(2026, 11, 9),
        date(2026, 11, 16),
        &berlin(),
    );
    assert_eq!(
        titles(&found),
        vec![
            "2026-11-11 07:00 Yoga",
            "2026-11-12 07:00 Yoga",
            "2026-11-13 07:00 Yoga",
            "2026-11-15 09:00 Yoga",
            "2026-11-16 20:00 Müll rausstellen",
        ]
    );
    // DURATION became the end.
    assert_eq!(
        found[0].end,
        Some(at("2026-11-11T08:00:00+01:00[Europe/Berlin]"))
    );
}

#[test]
fn an_appointment_over_days_touches_each_of_them_and_no_more() {
    let appointments = stored(NEXTCLOUD, 1);
    let on = |d: Date| {
        titles(&occurrences(
            &appointments,
            &[calendar(1)],
            &[],
            d,
            d,
            &berlin(),
        ))
    };
    assert_eq!(on(date(2026, 12, 21)), vec!["2026-12-21 Urlaub"]);
    assert_eq!(on(date(2026, 12, 23)), vec!["2026-12-21 Urlaub"]);
    assert!(
        on(date(2026, 12, 24)).is_empty(),
        "the end date is excluded"
    );
    // Ending at midnight does not touch the next day.
    assert_eq!(
        on(date(2026, 11, 17)),
        vec!["2026-11-17 22:00 Bis Mitternacht"]
    );
    assert!(on(date(2026, 11, 18)).is_empty());
}

#[test]
fn a_disabled_or_unchosen_calendar_shows_nothing() {
    let mut appointments = stored(GOOGLE, 1);
    appointments.extend(stored(EXCHANGE, 2));
    let mut off = calendar(1);
    off.enabled = false;
    let from = date(2026, 11, 9);
    let to = date(2026, 11, 13);
    let all = occurrences(
        &appointments,
        &[calendar(1), calendar(2)],
        &[],
        from,
        to,
        &berlin(),
    );
    assert!(all.iter().any(|i| i.calendar == id(1)));
    assert!(all.iter().any(|i| i.calendar == id(2)));
    let only = occurrences(
        &appointments,
        &[calendar(1), calendar(2)],
        &[id(2)],
        from,
        to,
        &berlin(),
    );
    assert!(!only.is_empty() && only.iter().all(|i| i.calendar == id(2)));
    let disabled = occurrences(&appointments, &[off, calendar(2)], &[], from, to, &berlin());
    assert!(disabled.iter().all(|i| i.calendar == id(2)));
}

#[test]
fn the_window_keeps_what_is_near_and_what_still_repeats() {
    let read = read_ics(GOOGLE, &berlin()).unwrap();
    let today = date(2026, 11, 9);
    assert!(keep(find(&read, "zahnarzt@example.org"), today));
    assert!(!keep(find(&read, "alt@example.org"), today), "long past");
    let mut choir = find(&read, "chor@example.org").clone();
    choir.start = at("2015-01-06T19:30:00+01:00[Europe/Berlin]");
    assert!(keep(&choir, today), "a rule from long ago that still runs");
    choir.rrule = Some("FREQ=WEEKLY;UNTIL=20200101T000000Z".into());
    assert!(!keep(&choir, today), "a rule that ended");
    let mut far = find(&read, "zahnarzt@example.org").clone();
    far.start = at("2028-06-01T09:00:00+02:00[Europe/Berlin]");
    far.end = None;
    assert!(!keep(&far, today), "too far ahead");
}

#[test]
fn the_link_is_the_url_else_the_template_else_none() {
    let appointments = stored(GOOGLE, 1);
    let call = appointments
        .iter()
        .find(|a| a.uid == "anruf@example.org")
        .unwrap();
    let dentist = appointments
        .iter()
        .find(|a| a.uid == "zahnarzt@example.org")
        .unwrap();
    let mut cal = calendar(1);
    assert_eq!(link(dentist, &cal, &dentist.start, &berlin()), None);
    cal.link_template = Some("https://cal.example.org/day/{y}/{m}/{d}?e={uid}&on={date}".into());
    assert_eq!(
        link(dentist, &cal, &dentist.start, &berlin()).as_deref(),
        Some("https://cal.example.org/day/2026/11/20?e=zahnarzt%40example.org&on=2026-11-20")
    );
    assert_eq!(
        link(call, &cal, &call.start, &berlin()).as_deref(),
        Some("https://calendar.example.org/event/anruf")
    );
}

#[test]
fn a_late_utc_time_takes_the_day_of_the_account_zone_for_the_template() {
    let cal = Calendar {
        link_template: Some("{date}".into()),
        ..calendar(1)
    };
    let mut appointment = stored(GOOGLE, 1).remove(0);
    appointment.url = None;
    let late = at("2026-11-20T23:30:00+00:00[UTC]");
    assert_eq!(
        link(&appointment, &cal, &late, &berlin()).as_deref(),
        Some("2026-11-21")
    );
}

#[test]
fn caldav_objects_read_into_one_and_a_broken_one_costs_only_itself() {
    let one = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:a\r\nSUMMARY:Eins\r\n\
               DTSTART;VALUE=DATE:20261120\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let two = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:b\r\nSUMMARY:Zwei\r\n\
               DTSTART;TZID=Nowhere/Land:20261121T100000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let read = read_objects(&[one, "<html>", two], &berlin());
    let titles: Vec<&str> = read.appointments.iter().map(|a| a.title.as_str()).collect();
    assert_eq!(titles, vec!["Eins", "Zwei"]);
    assert_eq!(read.skipped, 1);
    assert!(read.unknown_zones.contains("Nowhere/Land"));
}
