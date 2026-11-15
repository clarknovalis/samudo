//! When appointments remind (ADR 0101 §3, §4): the alarms their VALARMs
//! carry, for every occurrence the repetition unfolds into, with the same
//! function as `GET /appointments`.

use std::collections::{BTreeMap, BTreeSet};

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};
use samu_api::{
    AlarmRelated, Alert, Appointment, AppointmentAlarm, AppointmentStatus, Calendar, Id, When,
};

use crate::availability::Settings;
use crate::calendar::occurrences;

/// The longest an alarm may lie before its occurrence and still be found:
/// a rule gone wrong cannot make the window endless.
const LONGEST_DAYS: i64 = 60;

/// Every alarm of an appointment that goes off after `after` and up to
/// `until`, by time. Cancelled appointments and those of a calendar that
/// is switched off say nothing; scopes do not count (§4). A relative alarm
/// counts from the start or the end of each occurrence, over whole days
/// from midnight in the account's zone (§3); an alarm at a fixed instant
/// goes off once, with the appointment's own start.
#[must_use]
pub fn due(
    appointments: &[Appointment],
    calendars: &[Calendar],
    settings: &Settings,
    after: Timestamp,
    until: Timestamp,
) -> Vec<Alert> {
    let zone = &settings.zone;
    let on: BTreeSet<Id> = calendars
        .iter()
        .filter(|c| c.enabled)
        .map(|c| c.id)
        .collect();
    let by_id: BTreeMap<Id, &Appointment> = appointments
        .iter()
        .filter(|a| {
            on.contains(&a.calendar)
                && a.status != AppointmentStatus::Cancelled
                && !a.alarms.is_empty()
        })
        .map(|a| (a.id, a))
        .collect();
    let mut found: Vec<Alert> = Vec::new();
    let mut keep = |a: &Appointment, at: Timestamp, start: &When, end: Option<&When>| {
        if after < at && at <= until {
            found.push(Alert {
                at,
                title: a.title.clone(),
                owner: None,
                appointment: Some(a.id),
                calendar: Some(a.calendar),
                start: Some(start.clone()),
                end: end.cloned(),
                location: a.location.clone(),
            });
        }
    };
    // Fixed instants: once each.
    for a in by_id.values() {
        for alarm in &a.alarms {
            if let Some(at) = alarm.at {
                keep(a, at, &a.start, a.end.as_ref());
            }
        }
    }
    // Relative ones: each occurrence whose alarms can fall in the window.
    let (lead, lag) = reach(by_id.values().flat_map(|a| a.alarms.iter()));
    let from = day(after, zone)
        .checked_sub((lag + 1).days())
        .unwrap_or(Date::MIN);
    let to = day(until, zone)
        .checked_add((lead + 1).days())
        .unwrap_or(Date::MAX);
    let wanted: Vec<Appointment> = by_id.values().map(|a| (*a).clone()).collect();
    // The main appointments have to be there for their changed instances
    // to replace occurrences, also when those have no alarms.
    let mut all: Vec<Appointment> = wanted;
    for a in appointments {
        if !by_id.contains_key(&a.id) && on.contains(&a.calendar) {
            all.push(a.clone());
        }
    }
    for occurrence in occurrences(&all, calendars, &[], from, to, zone) {
        let Some(a) = by_id.get(&occurrence.appointment) else {
            continue;
        };
        for alarm in a.alarms.iter().filter(|x| x.at.is_none()) {
            if let Some(at) = goes_off(alarm, &occurrence.start, occurrence.end.as_ref(), zone) {
                keep(a, at, &occurrence.start, occurrence.end.as_ref());
            }
        }
    }
    found.sort_by(|x, y| x.at.cmp(&y.at).then_with(|| x.title.cmp(&y.title)));
    found.dedup();
    found
}

/// When one relative alarm of an occurrence goes off; none for an alarm at
/// a fixed instant or one this samu does not know.
#[must_use]
pub fn goes_off(
    alarm: &AppointmentAlarm,
    start: &When,
    end: Option<&When>,
    zone: &TimeZone,
) -> Option<Timestamp> {
    let before = alarm.before?;
    let anchor = match alarm.related {
        AlarmRelated::Start => start,
        AlarmRelated::End => end.unwrap_or(start),
        AlarmRelated::Unknown => return None,
    };
    let anchor = match anchor {
        When::At(at) => at.clone(),
        When::Date(date) => date.to_zoned(zone.clone()).ok()?,
        When::Unknown => return None,
    };
    anchor.checked_sub(before.0).ok().map(|z| z.timestamp())
}

/// How many whole days alarms reach before (`lead`) and after (`lag`)
/// their anchor, each at most [`LONGEST_DAYS`].
fn reach<'a>(alarms: impl Iterator<Item = &'a AppointmentAlarm>) -> (i64, i64) {
    let mut lead = 0;
    let mut lag = 0;
    for alarm in alarms {
        let Some(before) = alarm.before else { continue };
        let Ok(length) = before
            .0
            .to_duration(jiff::SpanRelativeTo::days_are_24_hours())
        else {
            // Months or years: as far as it may reach.
            lead = LONGEST_DAYS;
            lag = LONGEST_DAYS;
            continue;
        };
        let days = length.as_secs() / 86_400 + 1;
        if length.is_negative() {
            lag = lag.max(days.abs());
        } else {
            lead = lead.max(days);
        }
    }
    // An alarm before an end reaches as far as the appointment is long;
    // a day more than the longest covers most.
    (lead.min(LONGEST_DAYS), lag.min(LONGEST_DAYS))
}

fn day(at: Timestamp, zone: &TimeZone) -> Date {
    at.to_zoned(zone.clone()).date()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use samu_api::{CalendarKind, Period};

    use super::*;
    use crate::calendar::read_ics;

    const ICS: &str = "BEGIN:VCALENDAR\r
VERSION:2.0\r
PRODID:-//test//test//EN\r
BEGIN:VEVENT\r
UID:dentist\r
DTSTART;TZID=Europe/Berlin:20261120T090000\r
DTEND;TZID=Europe/Berlin:20261120T094500\r
SUMMARY:Zahnarzt\r
LOCATION:Hauptstraße 1\r
BEGIN:VALARM\r
ACTION:DISPLAY\r
TRIGGER:-PT15M\r
DESCRIPTION:Zahnarzt\r
END:VALARM\r
BEGIN:VALARM\r
ACTION:AUDIO\r
TRIGGER;RELATED=END:PT5M\r
END:VALARM\r
BEGIN:VALARM\r
ACTION:EMAIL\r
TRIGGER:-PT1H\r
END:VALARM\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:fixed\r
DTSTART;TZID=Europe/Berlin:20261121T180000\r
SUMMARY:Konzert\r
BEGIN:VALARM\r
ACTION:DISPLAY\r
TRIGGER;VALUE=DATE-TIME:20261121T120000Z\r
END:VALARM\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:birthday\r
DTSTART;VALUE=DATE:20261123\r
DTEND;VALUE=DATE:20261124\r
SUMMARY:Geburtstag\r
BEGIN:VALARM\r
ACTION:DISPLAY\r
TRIGGER:-PT15H\r
END:VALARM\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:weekly\r
DTSTART;TZID=Europe/Berlin:20261116T100000\r
DTEND;TZID=Europe/Berlin:20261116T110000\r
RRULE:FREQ=WEEKLY;COUNT=4\r
EXDATE;TZID=Europe/Berlin:20261123T100000\r
SUMMARY:Runde\r
BEGIN:VALARM\r
ACTION:DISPLAY\r
TRIGGER:-PT10M\r
END:VALARM\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:weekly\r
RECURRENCE-ID;TZID=Europe/Berlin:20261130T100000\r
DTSTART;TZID=Europe/Berlin:20261130T140000\r
DTEND;TZID=Europe/Berlin:20261130T150000\r
SUMMARY:Runde (verlegt)\r
END:VEVENT\r
BEGIN:VEVENT\r
UID:cancelled\r
DTSTART;TZID=Europe/Berlin:20261120T120000\r
STATUS:CANCELLED\r
SUMMARY:Abgesagt\r
BEGIN:VALARM\r
ACTION:DISPLAY\r
TRIGGER:-PT15M\r
END:VALARM\r
END:VEVENT\r
END:VCALENDAR\r
";

    fn id(n: u8) -> Id {
        format!("01936f5e-8a00-7c3e-9b1a-3f2d4c5b6e{n:02x}")
            .parse()
            .unwrap()
    }

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    fn settings() -> Settings {
        Settings::new(TimeZone::get("Europe/Berlin").unwrap())
    }

    fn calendar(enabled: bool) -> Calendar {
        Calendar {
            id: id(1),
            created: ts("2026-11-01T00:00:00Z"),
            modified: ts("2026-11-01T00:00:00Z"),
            title: "Kalender".into(),
            kind: CalendarKind::Caldav,
            color: None,
            enabled,
            refresh: 15,
            link_template: None,
            position: 0,
            fetched_at: None,
            error: None,
            host: None,
            writable: true,
        }
    }

    fn appointments() -> Vec<Appointment> {
        read_ics(ICS, &settings().zone)
            .unwrap()
            .appointments
            .into_iter()
            .enumerate()
            .map(|(i, a)| {
                a.into_appointment(
                    id(10 + i as u8),
                    id(1),
                    ts("2026-11-01T00:00:00Z"),
                    ts("2026-11-01T00:00:00Z"),
                )
            })
            .collect()
    }

    fn said(alerts: &[Alert]) -> Vec<String> {
        alerts
            .iter()
            .map(|a| {
                format!(
                    "{} {}",
                    a.at.to_zoned(settings().zone).strftime("%m-%d %H:%M"),
                    a.title
                )
            })
            .collect()
    }

    #[test]
    fn valarm_is_read_as_alarms() {
        let all = appointments();
        let dentist = all.iter().find(|a| a.uid == "dentist").unwrap();
        // EMAIL is left out; after the end is a negative `before`.
        assert_eq!(
            dentist.alarms,
            vec![
                AppointmentAlarm {
                    before: Some(Period("PT15M".parse().unwrap())),
                    related: AlarmRelated::Start,
                    at: None
                },
                AppointmentAlarm {
                    before: Some(Period("-PT5M".parse().unwrap())),
                    related: AlarmRelated::End,
                    at: None
                },
            ]
        );
        let fixed = all.iter().find(|a| a.uid == "fixed").unwrap();
        assert_eq!(fixed.alarms[0].at, Some(ts("2026-11-21T12:00:00Z")));
        let moved = all.iter().find(|a| a.recurrence_id.is_some()).unwrap();
        assert!(moved.alarms.is_empty(), "an instance has its own alarms");
    }

    #[test]
    fn start_end_fixed_whole_day_series_and_cancelled() {
        let alerts = due(
            &appointments(),
            &[calendar(true)],
            &settings(),
            ts("2026-11-14T00:00:00Z"),
            ts("2026-12-14T00:00:00Z"),
        );
        assert_eq!(
            said(&alerts),
            vec![
                "11-16 09:50 Runde",
                "11-20 08:45 Zahnarzt",
                "11-20 09:50 Zahnarzt",
                "11-21 13:00 Konzert",
                // -PT15H from midnight of a whole day: the day before at 9.
                "11-22 09:00 Geburtstag",
                // 23 November is out (EXDATE); the 30th is moved and its
                // instance has no alarm of its own.
                "12-07 09:50 Runde",
            ]
        );
        let dentist = &alerts[1];
        assert_eq!(dentist.location.as_deref(), Some("Hauptstraße 1"));
        assert_eq!(dentist.calendar, Some(id(1)));
        assert!(dentist.owner.is_none());
    }

    #[test]
    fn only_the_window_counts() {
        let alerts = due(
            &appointments(),
            &[calendar(true)],
            &settings(),
            ts("2026-11-20T07:45:00Z"),
            ts("2026-11-20T08:50:00Z"),
        );
        // After `after`, up to `until` included.
        assert_eq!(said(&alerts), vec!["11-20 09:50 Zahnarzt"]);
    }

    #[test]
    fn a_calendar_switched_off_says_nothing() {
        let alerts = due(
            &appointments(),
            &[calendar(false)],
            &settings(),
            ts("2026-11-14T00:00:00Z"),
            ts("2026-12-14T00:00:00Z"),
        );
        assert!(alerts.is_empty());
    }

    #[test]
    fn far_ahead_alarms_are_found() {
        let mut all = appointments();
        let dentist = all.iter_mut().find(|a| a.uid == "dentist").unwrap();
        dentist.alarms = vec![AppointmentAlarm {
            before: Some(Period("P1W".parse().unwrap())),
            related: AlarmRelated::Start,
            at: None,
        }];
        let alerts = due(
            &all,
            &[calendar(true)],
            &settings(),
            ts("2026-11-13T00:00:00Z"),
            ts("2026-11-13T12:00:00Z"),
        );
        assert_eq!(said(&alerts), vec!["11-13 09:00 Zahnarzt"]);
    }
}
