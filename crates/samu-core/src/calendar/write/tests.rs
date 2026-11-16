#![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

use jiff::civil::date;
use samu_api::{Id, Period};

use super::*;
use crate::calendar::read_ics;

const GOOGLE: &str = include_str!("../../../../../testdata/calendar/google.ics");

fn berlin() -> TimeZone {
    TimeZone::get("Europe/Berlin").unwrap()
}

fn now() -> Timestamp {
    "2026-11-11T09:00:00Z".parse().unwrap()
}

fn at(s: &str) -> When {
    When::At(s.parse().unwrap())
}

/// The appointments of one UID in a text, as the store keeps them.
fn of_uid(text: &str, uid: &str) -> Vec<Appointment> {
    read_ics(text, &berlin())
        .unwrap()
        .appointments
        .into_iter()
        .filter(|a| a.uid == uid)
        .map(|a| a.into_appointment(Id::new(), Id::new(), now(), now()))
        .collect()
}

/// The VEVENTs of one UID cut out of the Google file, as a CalDAV
/// resource holds them: the calendar, its zone and those events.
fn resource(uid: &str) -> String {
    let lines = unfold(GOOGLE);
    let items = parse(&lines).unwrap();
    let Item::Block(mut calendar) = items.into_iter().next().unwrap() else {
        panic!()
    };
    calendar.items.retain(|i| match i {
        Item::Block(b) if b.name == "VEVENT" => b.lines().any(|l| l == &format!("UID:{uid}")),
        _ => true,
    });
    emit(&[Item::Block(calendar)])
}

#[test]
fn an_unchanged_appointment_writes_the_resource_back_as_it_was() {
    let original = resource("zahnarzt@example.org");
    let wanted = of_uid(&original, "zahnarzt@example.org");
    let written = patch_resource(&original, &wanted, &berlin(), now()).unwrap();
    assert_eq!(written, original);
}

#[test]
fn a_changed_title_replaces_summary_alone_and_keeps_the_alarm() {
    let original = resource("zahnarzt@example.org");
    let mut wanted = of_uid(&original, "zahnarzt@example.org");
    wanted[0].title = "Zahnarzt; Kontrolle, bitte".into();
    let written = patch_resource(&original, &wanted, &berlin(), now()).unwrap();
    assert!(written.contains("SUMMARY:Zahnarzt\\; Kontrolle\\, bitte\r\n"));
    assert!(written.contains("BEGIN:VALARM\r\nACTION:DISPLAY"));
    assert!(written.contains("LOCATION:Hauptstraße 1\\, Stadt\r\n"));
    assert!(written.contains("SEQUENCE:1\r\n"));
    assert!(written.contains("LAST-MODIFIED:20261111T090000Z\r\n"));
    assert!(written.contains("DTSTAMP:20261111T090000Z\r\n"));
    assert!(
        written.contains("X-LIC-LOCATION:Europe/Berlin"),
        "the zone stays"
    );
    // Read back, it is what was wanted.
    let back = of_uid(&written, "zahnarzt@example.org");
    assert_eq!(back[0].title, "Zahnarzt; Kontrolle, bitte");
    assert_eq!(back[0].note, wanted[0].note);
    assert_eq!(back[0].start, wanted[0].start);
}

#[test]
fn a_moved_series_drops_an_instance_and_adds_a_new_one() {
    let original = resource("chor@example.org");
    let mut wanted = of_uid(&original, "chor@example.org");
    assert_eq!(wanted.len(), 3);
    // The cancelled instance of 1 December goes, a new one on 8 December
    // comes, the main one moves to 20:00.
    wanted.retain(|a| {
        a.recurrence_id.as_ref().map(key_of)
            != Some(key_of(&at("2026-12-01T19:30:00+01:00[Europe/Berlin]")))
    });
    let main = wanted
        .iter_mut()
        .find(|a| a.recurrence_id.is_none())
        .unwrap();
    main.start = at("2026-10-20T20:00:00+02:00[Europe/Berlin]");
    main.end = Some(at("2026-10-20T21:30:00+02:00[Europe/Berlin]"));
    let mut extra = main.clone();
    extra.recurrence_id = Some(at("2026-12-08T20:00:00+01:00[Europe/Berlin]"));
    extra.start = at("2026-12-08T18:00:00+01:00[Europe/Berlin]");
    extra.end = Some(at("2026-12-08T19:00:00+01:00[Europe/Berlin]"));
    extra.rrule = None;
    extra.exdate.clear();
    extra.title = "Chor früher".into();
    wanted.push(extra);

    let written = patch_resource(&original, &wanted, &berlin(), now()).unwrap();
    assert!(!written.contains("STATUS:CANCELLED"));
    assert!(written.contains("DTSTART;TZID=Europe/Berlin:20261020T200000\r\n"));
    assert!(written.contains("RECURRENCE-ID;TZID=Europe/Berlin:20261208T200000\r\n"));
    assert!(
        written.contains("SUMMARY:Chor (Generalprobe)\r\n"),
        "the other stays"
    );
    let back = of_uid(&written, "chor@example.org");
    assert_eq!(back.len(), 3);
    assert!(back.iter().any(|a| a.title == "Chor früher"));
}

#[test]
fn a_new_resource_reads_back_and_brings_its_zone() {
    let appointment = Appointment {
        id: Id::new(),
        created: now(),
        modified: now(),
        calendar: Id::new(),
        uid: "neu@samu".into(),
        recurrence_id: None,
        title: "Lesung".into(),
        start: at("2026-11-20T19:00:00+01:00[Europe/Berlin]"),
        end: Some(at("2026-11-20T21:00:00+01:00[Europe/Berlin]")),
        location: Some("Stadtbücherei".into()),
        note: Some("Erste Zeile\nZweite, mit Komma".into()),
        url: None,
        status: AppointmentStatus::Confirmed,
        busy: true,
        rrule: Some("FREQ=MONTHLY;COUNT=3".into()),
        rdate: Vec::new(),
        exdate: vec![at("2026-12-20T19:00:00+01:00[Europe/Berlin]")],
        meeting: false,
        // Cloned from a series by an edit: written as VALARMs (ADR 0101 §2).
        alarms: vec![
            AppointmentAlarm {
                before: Some(Period("PT15M".parse().unwrap())),
                related: AlarmRelated::Start,
                at: None,
            },
            AppointmentAlarm {
                before: Some(Period("-PT5M".parse().unwrap())),
                related: AlarmRelated::End,
                at: None,
            },
            AppointmentAlarm {
                before: None,
                related: AlarmRelated::Start,
                at: Some("2026-11-20T12:00:00Z".parse().unwrap()),
            },
        ],
    };
    let text = new_resource(std::slice::from_ref(&appointment), now()).unwrap();
    assert!(
        text.contains("BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\n"),
        "{text}"
    );
    assert!(text.contains("TRIGGER;RELATED=END:PT5M\r\n"), "{text}");
    assert!(
        text.contains("TRIGGER;VALUE=DATE-TIME:20261120T120000Z\r\n"),
        "{text}"
    );
    assert!(text.starts_with("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n"));
    assert!(text.contains("BEGIN:VTIMEZONE\r\nTZID:Europe/Berlin\r\n"));
    assert!(text.contains("TZOFFSETFROM:+0100\r\nTZOFFSETTO:+0200\r\nTZNAME:CEST"));
    let back = of_uid(&text, "neu@samu");
    assert_eq!(back.len(), 1);
    let read = &back[0];
    assert_eq!(read.title, appointment.title);
    assert_eq!(read.start, appointment.start);
    assert_eq!(read.end, appointment.end);
    assert_eq!(read.note, appointment.note);
    assert_eq!(read.rrule, appointment.rrule);
    assert_eq!(read.exdate, appointment.exdate);
    assert_eq!(read.alarms, appointment.alarms);
}

#[test]
fn an_all_day_appointment_is_written_as_dates_without_a_zone() {
    let original = resource("geburtstag@example.org");
    let mut wanted = of_uid(&original, "geburtstag@example.org");
    wanted[0].start = When::Date(date(2026, 12, 6));
    wanted[0].end = Some(When::Date(date(2026, 12, 7)));
    wanted[0].busy = true;
    let written = patch_resource(&original, &wanted, &berlin(), now()).unwrap();
    assert!(written.contains("DTSTART;VALUE=DATE:20261206\r\n"));
    assert!(written.contains("DTEND;VALUE=DATE:20261207\r\n"));
    assert!(written.contains("TRANSP:OPAQUE\r\n"));
    assert!(!written.contains("TRANSPARENT"));
}

#[test]
fn long_lines_fold_at_75_octets_without_splitting_a_character() {
    let mut out = String::new();
    let line = format!("DESCRIPTION:{}", "ä".repeat(60));
    fold(&line, &mut out);
    for part in out.split("\r\n").filter(|p| !p.is_empty()) {
        assert!(part.len() <= 75, "{} octets", part.len());
    }
    assert_eq!(unfold(&out), vec![line]);
}

#[test]
fn a_line_splits_where_quotes_do_not_hold_the_colon() {
    let (name, params, value) =
        split_line("ATTENDEE;CN=\"Doe; Jane: Dr.\";ROLE=CHAIR:mailto:jane@example.org");
    assert_eq!(name, "ATTENDEE");
    assert_eq!(params[0], ("CN".into(), "Doe; Jane: Dr.".into()));
    assert_eq!(params[1], ("ROLE".into(), "CHAIR".into()));
    assert_eq!(value, "mailto:jane@example.org");
}

#[test]
fn a_resource_without_a_calendar_or_with_an_open_component_is_refused() {
    let wanted = Vec::new();
    assert_eq!(
        patch_resource("BEGIN:VEVENT\r\nEND:VEVENT\r\n", &wanted, &berlin(), now()),
        Err(WriteError::NoCalendar)
    );
    assert!(matches!(
        patch_resource(
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\n",
            &wanted,
            &berlin(),
            now()
        ),
        Err(WriteError::Unbalanced(_))
    ));
}
