// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Calendars read from elsewhere (ADR 0047, 0048): an iCalendar text read
//! into appointments, the window the server keeps, repetitions unfolded
//! into occurrences, and the link into the calendar itself.
//!
//! Pure: the server fetches, this reads. Repetitions are computed by the
//! `rrule` crate as in [`crate::repeat`]; dates go to it as text and come
//! back into jiff.

pub mod edit;
pub mod write;
pub mod zones;

use std::collections::{BTreeMap, BTreeSet};

use icalendar::parser::{Component, Property, read_calendar, unfold};
use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;
use jiff::{SignedDuration, Span, Timestamp, ToSpan};
use rrule::RRuleSet;
use samu_api::{
    AlarmRelated, Appointment, AppointmentAlarm, AppointmentInstance, AppointmentStatus, Calendar,
    Id, Period, When,
};

/// Why a calendar could not be read at all.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CalendarError {
    #[error("not an iCalendar file: {0}")]
    Syntax(String),
}

/// What an iCalendar text holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Read {
    pub appointments: Vec<ReadAppointment>,
    /// TZIDs that are neither IANA nor Windows names; their times were read
    /// in the account's zone (ADR 0047 §6).
    pub unknown_zones: BTreeSet<String>,
    /// VEVENTs left out, with why: no UID, no DTSTART.
    pub skipped: usize,
}

/// One VEVENT, as [`Appointment`] without what the store gives it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadAppointment {
    pub uid: String,
    pub recurrence_id: Option<When>,
    pub title: String,
    pub start: When,
    pub end: Option<When>,
    pub location: Option<String>,
    pub note: Option<String>,
    pub url: Option<String>,
    pub status: AppointmentStatus,
    pub busy: bool,
    pub rrule: Option<String>,
    pub rdate: Vec<When>,
    pub exdate: Vec<When>,
    /// It has an ATTENDEE (ADR 0068 §2).
    pub meeting: bool,
    /// Its VALARMs that show or sound (ADR 0101 §1, §2).
    pub alarms: Vec<AppointmentAlarm>,
    /// LAST-MODIFIED, for the rule of ADR 0068 §7; not kept.
    pub last_modified: Option<Timestamp>,
    /// Where it lives on a CalDAV server and its ETag there (ADR 0068
    /// §6); none from an ICS file. Kept by the server, never sent.
    pub href: Option<String>,
    pub etag: Option<String>,
}

impl ReadAppointment {
    /// The appointment the store keeps, with its id and times.
    #[must_use]
    pub fn into_appointment(
        self,
        id: Id,
        calendar: Id,
        created: Timestamp,
        modified: Timestamp,
    ) -> Appointment {
        Appointment {
            id,
            created,
            modified,
            calendar,
            uid: self.uid,
            recurrence_id: self.recurrence_id,
            title: self.title,
            start: self.start,
            end: self.end,
            location: self.location,
            note: self.note,
            url: self.url,
            status: self.status,
            busy: self.busy,
            rrule: self.rrule,
            rdate: self.rdate,
            exdate: self.exdate,
            meeting: self.meeting,
            alarms: self.alarms,
        }
    }

    /// What tells two VEVENTs of a calendar apart: the UID and the instance
    /// it replaces.
    #[must_use]
    pub fn key(&self) -> (String, Option<String>) {
        (self.uid.clone(), self.recurrence_id.as_ref().map(key_of))
    }
}

/// The key of [`ReadAppointment::key`] for a stored appointment.
#[must_use]
pub fn key(appointment: &Appointment) -> (String, Option<String>) {
    (
        appointment.uid.clone(),
        appointment.recurrence_id.as_ref().map(key_of),
    )
}

/// A `When` as a key that ignores the zone it is written in: the date, or
/// the instant.
pub fn key_of(when: &When) -> String {
    match when {
        When::Date(date) => date.to_string(),
        When::At(at) => at.timestamp().to_string(),
        When::Unknown => String::new(),
    }
}

/// Reads an iCalendar text; `zone` is the account's, for floating times and
/// unknown TZIDs. Of VALARM only what shows or sounds is read, as
/// [`AppointmentAlarm`] (ADR 0101, ending ADR 0047 §15). Of two VEVENTs with
/// the same UID and RECURRENCE-ID the later one counts.
pub fn read_ics(text: &str, zone: &TimeZone) -> Result<Read, CalendarError> {
    let unfolded = unfold(text);
    let calendar = read_calendar(&unfolded).map_err(|e| {
        CalendarError::Syntax(e.lines().next().unwrap_or_default().trim().to_string())
    })?;
    let mut read = Read::default();
    let mut by_key: BTreeMap<(String, Option<String>), ReadAppointment> = BTreeMap::new();
    let mut events = Vec::new();
    collect_events(&calendar.components, &mut events);
    for event in events {
        match read_event(event, zone, &mut read.unknown_zones) {
            Some(appointment) => {
                by_key.insert(appointment.key(), appointment);
            }
            None => read.skipped += 1,
        }
    }
    read.appointments = by_key.into_values().collect();
    Ok(read)
}

/// Reads the calendar objects of a CalDAV answer, one iCalendar text each
/// (ADR 0047 §7), into one [`Read`]. An object that is no iCalendar counts
/// as skipped and costs only itself; of two with the same UID and
/// RECURRENCE-ID the later one counts.
#[must_use]
pub fn read_objects<S: AsRef<str>>(texts: &[S], zone: &TimeZone) -> Read {
    let mut read = Read::default();
    let mut by_key: BTreeMap<(String, Option<String>), ReadAppointment> = BTreeMap::new();
    for text in texts {
        match read_ics(text.as_ref(), zone) {
            Ok(one) => {
                read.unknown_zones.extend(one.unknown_zones);
                read.skipped += one.skipped;
                for appointment in one.appointments {
                    by_key.insert(appointment.key(), appointment);
                }
            }
            Err(_) => read.skipped += 1,
        }
    }
    read.appointments = by_key.into_values().collect();
    read
}

/// One calendar object of a CalDAV answer: its href, its ETag, its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resource {
    pub href: String,
    pub etag: Option<String>,
    pub text: String,
}

/// [`read_objects`] for CalDAV, each appointment knowing the resource it
/// came from (ADR 0068 §6).
#[must_use]
pub fn read_resources(resources: &[Resource], zone: &TimeZone) -> Read {
    let mut read = Read::default();
    let mut by_key: BTreeMap<(String, Option<String>), ReadAppointment> = BTreeMap::new();
    for resource in resources {
        match read_ics(&resource.text, zone) {
            Ok(one) => {
                read.unknown_zones.extend(one.unknown_zones);
                read.skipped += one.skipped;
                for mut appointment in one.appointments {
                    appointment.href = Some(resource.href.clone());
                    appointment.etag.clone_from(&resource.etag);
                    by_key.insert(appointment.key(), appointment);
                }
            }
            Err(_) => read.skipped += 1,
        }
    }
    read.appointments = by_key.into_values().collect();
    read
}

/// The VEVENTs, wherever they stand: the parser lays VCALENDAR open, but a
/// file with several calendars nests them.
fn collect_events<'a>(components: &'a [Component<'a>], out: &mut Vec<&'a Component<'a>>) {
    for component in components {
        match component.name.as_str() {
            "VEVENT" => out.push(component),
            "VCALENDAR" => collect_events(&component.components, out),
            _ => {}
        }
    }
}

fn read_event(
    event: &Component<'_>,
    zone: &TimeZone,
    unknown: &mut BTreeSet<String>,
) -> Option<ReadAppointment> {
    let uid = text(event, "UID").filter(|uid| !uid.is_empty())?;
    let start = event
        .find_prop("DTSTART")
        .and_then(|p| when(p, p.val.as_str(), zone, unknown))?;
    let end = match event.find_prop("DTEND") {
        Some(p) => when(p, p.val.as_str(), zone, unknown),
        None => event
            .find_prop("DURATION")
            .and_then(|p| p.val.as_str().parse::<Span>().ok())
            .and_then(|span| add(&start, span)),
    };
    let recurrence_id = event
        .find_prop("RECURRENCE-ID")
        .and_then(|p| when(p, p.val.as_str(), zone, unknown));
    let status = match text(event, "STATUS").as_deref() {
        Some("TENTATIVE") => AppointmentStatus::Tentative,
        Some("CANCELLED") => AppointmentStatus::Cancelled,
        _ => AppointmentStatus::Confirmed,
    };
    let free = text(event, "TRANSP").as_deref() == Some("TRANSPARENT")
        || text(event, "X-MICROSOFT-CDO-BUSYSTATUS").as_deref() == Some("FREE");
    let rrule = text(event, "RRULE")
        .filter(|r| !r.is_empty())
        .map(|r| r.trim_start_matches("RRULE:").to_string());
    Some(ReadAppointment {
        uid,
        recurrence_id,
        title: text(event, "SUMMARY").unwrap_or_default(),
        start,
        end,
        location: text(event, "LOCATION").filter(|s| !s.is_empty()),
        note: text(event, "DESCRIPTION").filter(|s| !s.is_empty()),
        url: text(event, "URL").filter(|s| !s.is_empty()),
        status,
        busy: !free,
        rrule,
        rdate: list(event, "RDATE", zone, unknown),
        exdate: list(event, "EXDATE", zone, unknown),
        meeting: event.find_prop("ATTENDEE").is_some(),
        alarms: alarms(event),
        last_modified: event
            .find_prop("LAST-MODIFIED")
            .and_then(|p| when(p, p.val.as_str(), zone, unknown))
            .and_then(|w| match w {
                When::At(at) => Some(at.timestamp()),
                _ => None,
            }),
        href: None,
        etag: None,
    })
}

/// The VALARMs of an event with `ACTION:DISPLAY` or `AUDIO` (ADR 0101
/// §1); `EMAIL`, unknown actions and triggers that cannot be read are left
/// out. `REPEAT`, `DURATION` and `ACKNOWLEDGED` are not read (§2). The same
/// alarm twice is one.
fn alarms(event: &Component<'_>) -> Vec<AppointmentAlarm> {
    let mut out: Vec<AppointmentAlarm> = Vec::new();
    for alarm in event
        .components
        .iter()
        .filter(|c| c.name.as_str() == "VALARM")
    {
        let action = text(alarm, "ACTION")
            .unwrap_or_default()
            .to_ascii_uppercase();
        if action != "DISPLAY" && action != "AUDIO" {
            continue;
        }
        let Some(trigger) = alarm.find_prop("TRIGGER") else {
            continue;
        };
        if let Some(read) = read_trigger(trigger).filter(|a| !out.contains(a)) {
            out.push(read);
        }
    }
    out
}

/// A TRIGGER: an instant in UTC with `VALUE=DATE-TIME`, else a duration
/// relative to the start or, with `RELATED=END`, the end. A negative
/// duration is before; `before` keeps it the other way round (§2).
fn read_trigger(trigger: &Property<'_>) -> Option<AppointmentAlarm> {
    let value = trigger.val.as_str().trim();
    if param(trigger, "VALUE").is_some_and(|v| v.eq_ignore_ascii_case("DATE-TIME")) {
        let wall = DateTime::strptime("%Y%m%dT%H%M%S", value.strip_suffix('Z')?).ok()?;
        let at = wall.to_zoned(TimeZone::UTC).ok()?.timestamp();
        return Some(AppointmentAlarm {
            before: None,
            related: AlarmRelated::Start,
            at: Some(at),
        });
    }
    let span: Span = value.parse().ok()?;
    let related = match param(trigger, "RELATED") {
        Some(r) if r.eq_ignore_ascii_case("END") => AlarmRelated::End,
        _ => AlarmRelated::Start,
    };
    Some(AppointmentAlarm {
        before: Some(Period(span.negate())),
        related,
        at: None,
    })
}

/// A property's value, trimmed.
fn text(event: &Component<'_>, name: &str) -> Option<String> {
    event
        .find_prop(name)
        .map(|p| p.val.as_str().trim().to_string())
}

/// Every value of every line of a list property (EXDATE, RDATE); a period
/// is not read.
fn list(
    event: &Component<'_>,
    name: &str,
    zone: &TimeZone,
    unknown: &mut BTreeSet<String>,
) -> Vec<When> {
    let mut out = Vec::new();
    for property in event.properties.iter().filter(|p| p.name.as_str() == name) {
        if param(property, "VALUE").as_deref() == Some("PERIOD") {
            continue;
        }
        for value in property.val.as_str().split(',') {
            if let Some(w) = when(property, value, zone, unknown) {
                out.push(w);
            }
        }
    }
    out
}

fn param(property: &Property<'_>, key: &str) -> Option<String> {
    property
        .params
        .iter()
        .find(|p| p.key.as_str().eq_ignore_ascii_case(key))
        .and_then(|p| p.val.as_ref().map(|v| v.as_str().to_string()))
}

/// A DATE or DATE-TIME in one of the two forms of ADR 0007: a date stays a
/// date; a time with `Z` is an instant in UTC, with TZID in that zone, and
/// floating in the account's zone.
fn when(
    property: &Property<'_>,
    value: &str,
    zone: &TimeZone,
    unknown: &mut BTreeSet<String>,
) -> Option<When> {
    when_of(
        value,
        param(property, "VALUE").as_deref(),
        param(property, "TZID").as_deref(),
        zone,
        unknown,
    )
}

/// [`when`] from the parts of a line: its value and its VALUE and TZID
/// parameters.
pub(crate) fn when_of(
    value: &str,
    value_type: Option<&str>,
    tzid: Option<&str>,
    zone: &TimeZone,
    unknown: &mut BTreeSet<String>,
) -> Option<When> {
    let value = value.trim();
    if value.len() == 8 || value_type == Some("DATE") {
        return Date::strptime("%Y%m%d", value.get(..8)?)
            .ok()
            .map(When::Date);
    }
    let (wall, utc) = match value.strip_suffix('Z') {
        Some(wall) => (wall, true),
        None => (value, false),
    };
    let wall = DateTime::strptime("%Y%m%dT%H%M%S", wall).ok()?;
    let tz = if utc {
        TimeZone::UTC
    } else if let Some(tzid) = tzid {
        zones::zone(tzid).unwrap_or_else(|| {
            unknown.insert(tzid.to_string());
            zone.clone()
        })
    } else {
        zone.clone()
    };
    wall.to_zoned(tz).ok().map(When::At)
}

/// `start` plus a DURATION.
fn add(start: &When, span: Span) -> Option<When> {
    match start {
        When::Date(date) => date.checked_add(span).ok().map(When::Date),
        When::At(at) => at.checked_add(span).ok().map(When::At),
        When::Unknown => None,
    }
}

/// The days the server keeps around today (ADR 0047 §7).
pub const KEEP_BEFORE: i32 = 30;
pub const KEEP_AFTER: i32 = 400;

/// Whether the server keeps an appointment: it touches the window from
/// [`KEEP_BEFORE`] days before `today` to [`KEEP_AFTER`] after, or it is the
/// main one of a repetition that still runs then, whenever it began.
#[must_use]
pub fn keep(appointment: &ReadAppointment, today: Date) -> bool {
    let first = today.saturating_sub(KEEP_BEFORE.days());
    let last = today.saturating_add(KEEP_AFTER.days());
    let start = day_of(&appointment.start);
    if start > last {
        return false;
    }
    if appointment.rrule.is_some() {
        return appointment
            .rrule
            .as_deref()
            .and_then(until)
            .is_none_or(|until| until >= first);
    }
    let rdates_after = appointment.rdate.iter().any(|d| day_of(d) >= first);
    last_day(&appointment.start, appointment.end.as_ref()) >= first || rdates_after
}

/// The date of `UNTIL` in a rule, if it has one.
fn until(rule: &str) -> Option<Date> {
    rule.split(';').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        key.eq_ignore_ascii_case("UNTIL")
            .then(|| Date::strptime("%Y%m%d", value.get(..8)?).ok())
            .flatten()
    })
}

fn day_of(when: &When) -> Date {
    match when {
        When::Date(date) => *date,
        When::At(at) => at.date(),
        When::Unknown => Date::MIN,
    }
}

/// The last day an appointment touches, in its own zone: the day before an
/// excluded end date, the day of an end time (the day before when it ends at
/// midnight).
fn last_day(start: &When, end: Option<&When>) -> Date {
    match (start, end) {
        (When::Date(s), Some(When::Date(e))) if e > s => e.yesterday().unwrap_or(*e),
        (When::At(s), Some(When::At(e))) if e > s => {
            if e.time() == jiff::civil::Time::midnight() {
                e.date().yesterday().unwrap_or(e.date())
            } else {
                e.date()
            }
        }
        _ => day_of(start),
    }
}

/// The first and the last day an occurrence touches, in `zone` (the
/// account's): a whole day by its dates, a time by where it falls there.
/// An end at midnight does not touch the day it begins.
#[must_use]
pub fn days(start: &When, end: Option<&When>, zone: &TimeZone) -> (Date, Date) {
    let local = |when: &When| match when {
        When::At(at) => When::At(at.with_time_zone(zone.clone())),
        other => other.clone(),
    };
    let start = local(start);
    let end = end.map(local);
    let first = day_of(&start);
    (first, last_day(&start, end.as_ref()).max(first))
}

/// More occurrences than this before the window is a rule gone wrong.
const SEARCH_LIMIT: usize = 100_000;

/// The occurrences of the appointments of enabled calendars that touch the
/// days `from` to `to` (included) in `zone`, by start (ADR 0047 §5).
/// `only` narrows to some calendars (a scope's `calendars`, ADR 0047 §12);
/// empty means all. Cancelled ones are left out; a changed instance
/// replaces the one it names; EXDATE takes one out.
#[must_use]
pub fn occurrences(
    appointments: &[Appointment],
    calendars: &[Calendar],
    only: &[Id],
    from: Date,
    to: Date,
    zone: &TimeZone,
) -> Vec<AppointmentInstance> {
    let shown: BTreeMap<Id, &Calendar> = calendars
        .iter()
        .filter(|c| c.enabled && (only.is_empty() || only.contains(&c.id)))
        .map(|c| (c.id, c))
        .collect();
    let window = Window::new(from, to, zone);
    // Instances replaced by a changed one, per calendar and UID.
    let replaced: BTreeSet<(Id, &str, String)> = appointments
        .iter()
        .filter_map(|a| {
            let rid = a.recurrence_id.as_ref()?;
            Some((a.calendar, a.uid.as_str(), key_of(rid)))
        })
        .collect();
    let mut out = Vec::new();
    for appointment in appointments {
        let Some(calendar) = shown.get(&appointment.calendar) else {
            continue;
        };
        if appointment.status == AppointmentStatus::Cancelled {
            continue;
        }
        let excluded: BTreeSet<String> = appointment.exdate.iter().map(key_of).collect();
        let starts = if appointment.recurrence_id.is_some() {
            vec![appointment.start.clone()]
        } else {
            starts(appointment, &window)
        };
        for start in starts {
            let k = key_of(&start);
            if appointment.recurrence_id.is_none()
                && (excluded.contains(&k)
                    || replaced.contains(&(appointment.calendar, appointment.uid.as_str(), k)))
            {
                continue;
            }
            let end = shifted_end(appointment, &start);
            if !window.touches(&start, end.as_ref()) {
                continue;
            }
            out.push(AppointmentInstance {
                appointment: appointment.id,
                calendar: appointment.calendar,
                title: appointment.title.clone(),
                link: link(appointment, calendar, &start, zone),
                start,
                end,
                location: appointment.location.clone(),
                note: appointment.note.clone(),
                status: appointment.status,
                busy: appointment.busy,
            });
        }
    }
    out.sort_by(|a, b| {
        window
            .instant(&a.start)
            .cmp(&window.instant(&b.start))
            .then_with(|| a.title.cmp(&b.title))
    });
    out
}

/// The days asked for, as instants in the account's zone.
struct Window {
    from: Date,
    to: Date,
    start: Timestamp,
    end: Timestamp,
    zone: TimeZone,
}

impl Window {
    fn new(from: Date, to: Date, zone: &TimeZone) -> Self {
        let start = midnight(from, zone);
        let end = midnight(to.tomorrow().unwrap_or(to), zone);
        Self {
            from,
            to,
            start,
            end,
            zone: zone.clone(),
        }
    }

    /// Where a start sorts: an appointment over whole days at the midnight
    /// of its first day in the account's zone, before the timed ones.
    fn instant(&self, when: &When) -> Timestamp {
        match when {
            When::Date(date) => midnight(*date, &self.zone),
            When::At(at) => at.timestamp(),
            When::Unknown => Timestamp::MIN,
        }
    }

    fn touches(&self, start: &When, end: Option<&When>) -> bool {
        match start {
            When::Date(s) => {
                let last = last_day(start, end);
                *s <= self.to && last >= self.from
            }
            When::At(s) => {
                let s = s.timestamp();
                let e = match end {
                    Some(When::At(e)) if e.timestamp() > s => e.timestamp(),
                    _ => s,
                };
                // A point in time touches the window when it lies in it.
                s < self.end && (e > self.start || (e == s && s >= self.start))
            }
            When::Unknown => false,
        }
    }
}

fn midnight(date: Date, zone: &TimeZone) -> Timestamp {
    date.to_zoned(zone.clone())
        .map(|z| z.timestamp())
        .unwrap_or(Timestamp::MIN)
}

/// The starts of a main appointment up to the end of the window: its own,
/// those of its rule, and its RDATEs. A rule samu cannot compute gives the
/// first start alone.
fn starts(appointment: &Appointment, window: &Window) -> Vec<When> {
    let mut out = vec![appointment.start.clone()];
    if let Some(rule) = &appointment.rrule
        && let Some(more) = unfold_rule(rule, &appointment.start, window)
    {
        out = more;
    }
    out.extend(appointment.rdate.iter().cloned());
    out
}

/// Whether samu can compute a rule from this start (ADR 0068 §3).
pub(crate) fn rule_reads(rule: &str, start: &When) -> bool {
    rule_set(rule, start).is_some()
}

fn rule_set(rule: &str, start: &When) -> Option<(RRuleSet, Option<TimeZone>)> {
    let (dtstart, tz) = match start {
        When::Date(date) => (format!("DTSTART:{}T000000Z", date.strftime("%Y%m%d")), None),
        When::At(at) => {
            let tz = at.time_zone().clone();
            let wall = at.datetime().strftime("%Y%m%dT%H%M%S");
            let line = match tz.iana_name() {
                Some("UTC" | "Etc/UTC") => format!("DTSTART:{wall}Z"),
                Some(name) => format!("DTSTART;TZID={name}:{wall}"),
                None => return None,
            };
            (line, Some(tz))
        }
        When::Unknown => return None,
    };
    let rule = utc_until(rule);
    let set: RRuleSet = format!("{dtstart}\nRRULE:{rule}").parse().ok()?;
    Some((set, tz))
}

fn unfold_rule(rule: &str, start: &When, window: &Window) -> Option<Vec<When>> {
    let (set, tz) = rule_set(rule, start)?;
    let mut out = Vec::new();
    for occurrence in set.into_iter().take(SEARCH_LIMIT) {
        let when = match &tz {
            None => When::Date(occurrence.format("%Y-%m-%d").to_string().parse().ok()?),
            Some(tz) => {
                let wall: DateTime = occurrence
                    .naive_local()
                    .format("%Y-%m-%dT%H:%M:%S")
                    .to_string()
                    .parse()
                    .ok()?;
                When::At(wall.to_zoned(tz.clone()).ok()?)
            }
        };
        if window.instant(&when) >= window.end {
            break;
        }
        out.push(when);
    }
    Some(out)
}

/// `rrule` wants UNTIL in UTC; calendars also write a bare date or a
/// floating time there. Either becomes the end of that day or that time, as
/// UTC: a little generous, never short.
fn utc_until(rule: &str) -> String {
    rule.split(';')
        .map(|part| match part.split_once('=') {
            Some((key, value)) if key.eq_ignore_ascii_case("UNTIL") => {
                if value.len() == 8 {
                    format!("{key}={value}T235959Z")
                } else if !value.ends_with('Z') {
                    format!("{key}={value}Z")
                } else {
                    part.to_string()
                }
            }
            _ => part.to_string(),
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// The end of an occurrence: the appointment's length after its start.
pub(crate) fn shifted_end(appointment: &Appointment, start: &When) -> Option<When> {
    match (&appointment.start, appointment.end.as_ref(), start) {
        (When::Date(s), Some(When::Date(e)), When::Date(new)) => {
            let days = s.until(*e).ok()?;
            new.checked_add(days).ok().map(When::Date)
        }
        (When::At(s), Some(When::At(e)), When::At(new)) => {
            let length: SignedDuration = s.duration_until(e);
            new.checked_add(length).ok().map(When::At)
        }
        _ => None,
    }
}

/// Where „open in the calendar“ leads for an occurrence (ADR 0047 §9): the
/// appointment's own URL, else the calendar's template filled in for the
/// day of `start` in `zone`, else nothing.
#[must_use]
pub fn link(
    appointment: &Appointment,
    calendar: &Calendar,
    start: &When,
    zone: &TimeZone,
) -> Option<String> {
    if let Some(url) = appointment
        .url
        .as_ref()
        .filter(|u| u.starts_with("https://") || u.starts_with("http://"))
    {
        return Some(url.clone());
    }
    let template = calendar.link_template.as_ref()?;
    let date = match start {
        When::Date(date) => *date,
        When::At(at) => at.with_time_zone(zone.clone()).date(),
        When::Unknown => return None,
    };
    Some(
        template
            .replace("{uid}", &percent(&appointment.uid))
            .replace("{date}", &date.to_string())
            .replace("{y}", &date.strftime("%Y").to_string())
            .replace("{m}", &date.strftime("%m").to_string())
            .replace("{d}", &date.strftime("%d").to_string()),
    )
}

/// A UID as it may stand in a URL.
fn percent(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests;
