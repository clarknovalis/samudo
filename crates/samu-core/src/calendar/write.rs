//! Appointments written back into iCalendar (ADR 0068 §6).
//!
//! Line by line, not through a model: a resource from elsewhere keeps
//! everything samu does not know (VALARM, ATTENDEE, X- properties, a
//! VTIMEZONE) as it came. In a VEVENT only the properties of a field that
//! changed are replaced; SEQUENCE goes up and LAST-MODIFIED and DTSTAMP are
//! new then. A VTIMEZONE samu needs and the resource lacks is made from
//! jiff's time zone data.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use jiff::tz::{Offset, TimeZone};
use jiff::{SignedDuration, Timestamp};
use samu_api::{AlarmRelated, Appointment, AppointmentAlarm, AppointmentStatus, When};

use super::{ReadAppointment, key_of, read_ics, zones};

mod split;
pub use split::{Left, Piece, Split, split_by_uid};

/// Why a resource could not be written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    #[error("no VCALENDAR in the resource")]
    NoCalendar,
    #[error("BEGIN and END do not match: {0}")]
    Unbalanced(String),
    #[error("{0} has a time samu cannot write")]
    Unwritable(String),
}

/// The product samu writes into a resource it makes.
const PRODID: &str = "-//samu//samu//EN";
/// How far ahead a VTIMEZONE made by samu lists the transitions.
const ZONE_YEARS: i64 = 11;

/// A resource from elsewhere with `wanted` laid over it: all VEVENTs of
/// one UID, the main one and its changed instances. A VEVENT in `original`
/// that `wanted` has no more is dropped; one `wanted` has new is added.
/// `zone` is the account's, for floating times.
pub fn patch_resource(
    original: &str,
    wanted: &[Appointment],
    zone: &TimeZone,
    now: Timestamp,
) -> Result<String, WriteError> {
    let mut items = parse(&unfold(original))?;
    let calendar = items
        .iter_mut()
        .find_map(|item| match item {
            Item::Block(b) if b.name == "VCALENDAR" => Some(b),
            _ => None,
        })
        .ok_or(WriteError::NoCalendar)?;
    let mut by_key: BTreeMap<Option<String>, &Appointment> = wanted
        .iter()
        .map(|a| (a.recurrence_id.as_ref().map(key_of), a))
        .collect();
    let mut kept = Vec::new();
    for item in std::mem::take(&mut calendar.items) {
        match item {
            Item::Block(event) if event.name == "VEVENT" => {
                let key = recurrence_key(&event, zone);
                if let Some(appointment) = by_key.remove(&key) {
                    kept.push(Item::Block(patch_event(event, appointment, zone, now)?));
                }
            }
            other => kept.push(other),
        }
    }
    for appointment in by_key.into_values() {
        kept.push(Item::Block(new_event(appointment, now)?));
    }
    calendar.items = kept;
    add_zones(calendar);
    Ok(emit(&items))
}

/// A new resource holding `wanted`: one UID, the main VEVENT first.
pub fn new_resource(wanted: &[Appointment], now: Timestamp) -> Result<String, WriteError> {
    let mut calendar = Block {
        name: "VCALENDAR".into(),
        items: vec![
            Item::Line("VERSION:2.0".into()),
            Item::Line(format!("PRODID:{PRODID}")),
            Item::Line("CALSCALE:GREGORIAN".into()),
        ],
    };
    let mut ordered: Vec<&Appointment> = wanted.iter().collect();
    ordered.sort_by_key(|a| a.recurrence_id.is_some());
    for appointment in ordered {
        calendar
            .items
            .push(Item::Block(new_event(appointment, now)?));
    }
    add_zones(&mut calendar);
    Ok(emit(&[Item::Block(calendar)]))
}

/// One content line or a component.
#[derive(Debug, Clone)]
enum Item {
    Line(String),
    Block(Block),
}

/// A component: BEGIN and END around its items.
#[derive(Debug, Clone)]
struct Block {
    name: String,
    items: Vec<Item>,
}

impl Block {
    fn lines(&self) -> impl Iterator<Item = &String> {
        self.items.iter().filter_map(|i| match i {
            Item::Line(l) => Some(l),
            Item::Block(_) => None,
        })
    }
}

/// The content lines, continuation lines joined (RFC 5545 §3.1).
fn unfold(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(rest) = line.strip_prefix([' ', '\t'])
            && let Some(last) = out.last_mut()
        {
            last.push_str(rest);
            continue;
        }
        if !line.is_empty() {
            out.push(line.to_string());
        }
    }
    out
}

fn parse(lines: &[String]) -> Result<Vec<Item>, WriteError> {
    let mut stack: Vec<Block> = vec![Block {
        name: String::new(),
        items: Vec::new(),
    }];
    for line in lines {
        let (name, _, value) = split_line(line);
        match name.as_str() {
            "BEGIN" => stack.push(Block {
                name: value.trim().to_ascii_uppercase(),
                items: Vec::new(),
            }),
            "END" => {
                let block = stack
                    .pop()
                    .filter(|b| b.name == value.trim().to_ascii_uppercase() && !b.name.is_empty())
                    .ok_or_else(|| WriteError::Unbalanced(line.clone()))?;
                stack
                    .last_mut()
                    .ok_or_else(|| WriteError::Unbalanced(line.clone()))?
                    .items
                    .push(Item::Block(block));
            }
            _ => {
                if let Some(top) = stack.last_mut() {
                    top.items.push(Item::Line(line.clone()));
                }
            }
        }
    }
    match stack.pop() {
        Some(root) if stack.is_empty() => Ok(root.items),
        Some(open) => Err(WriteError::Unbalanced(format!(
            "BEGIN:{} without END",
            open.name
        ))),
        None => Err(WriteError::NoCalendar),
    }
}

/// A line's name (upper case), its parameters and its value. A `:` or `;`
/// inside quotes belongs to the parameter.
fn split_line(line: &str) -> (String, Vec<(String, String)>, &str) {
    let mut quoted = false;
    let mut cut = line.len();
    for (i, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ':' if !quoted => {
                cut = i;
                break;
            }
            _ => {}
        }
    }
    let head = &line[..cut];
    let value = line.get(cut + 1..).unwrap_or("");
    let mut parts = Vec::new();
    let mut start = 0;
    quoted = false;
    for (i, c) in head.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ';' if !quoted => {
                parts.push(&head[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&head[start..]);
    let name = parts.first().copied().unwrap_or("").to_ascii_uppercase();
    let params = parts
        .iter()
        .skip(1)
        .filter_map(|p| p.split_once('='))
        .map(|(k, v)| (k.to_ascii_uppercase(), v.trim_matches('"').to_string()))
        .collect();
    (name, params, value)
}

fn line_name(line: &str) -> String {
    split_line(line).0
}

/// The key of a VEVENT's RECURRENCE-ID, none for the main one.
fn recurrence_key(event: &Block, zone: &TimeZone) -> Option<String> {
    event.lines().find_map(|line| {
        let (name, params, value) = split_line(line);
        (name == "RECURRENCE-ID").then(|| {
            let param = |k: &str| params.iter().find(|(p, _)| p == k).map(|(_, v)| v.as_str());
            super::when_of(
                value,
                param("VALUE"),
                param("TZID"),
                zone,
                &mut BTreeSet::new(),
            )
            .map(|w| key_of(&w))
        })?
    })
}

/// What a VEVENT says, read the way the server reads it.
fn read_event(event: &Block, zone: &TimeZone) -> Option<ReadAppointment> {
    let text = emit(&[Item::Block(Block {
        name: "VCALENDAR".into(),
        items: vec![Item::Block(event.clone())],
    })]);
    read_ics(&text, zone).ok()?.appointments.into_iter().next()
}

/// The VEVENT with the fields that differ from `wanted` replaced.
fn patch_event(
    mut event: Block,
    wanted: &Appointment,
    zone: &TimeZone,
    now: Timestamp,
) -> Result<Block, WriteError> {
    let Some(was) = read_event(&event, zone) else {
        // Nothing samu can read: written anew, with its UID.
        return new_event(wanted, now);
    };
    let mut changed = false;
    if was.title != wanted.title {
        replace(
            &mut event,
            &["SUMMARY"],
            vec![text_line("SUMMARY", &wanted.title)],
        );
        changed = true;
    }
    if !same(&was.start, &wanted.start) {
        replace(
            &mut event,
            &["DTSTART"],
            vec![when_line("DTSTART", &wanted.start)?],
        );
        changed = true;
    }
    if !same_opt(was.end.as_ref(), wanted.end.as_ref()) {
        let lines = match &wanted.end {
            Some(end) => vec![when_line("DTEND", end)?],
            None => Vec::new(),
        };
        replace(&mut event, &["DTEND", "DURATION"], lines);
        changed = true;
    }
    if was.location != wanted.location {
        let lines = wanted
            .location
            .iter()
            .map(|l| text_line("LOCATION", l))
            .collect();
        replace(&mut event, &["LOCATION"], lines);
        changed = true;
    }
    if was.note != wanted.note {
        let lines = wanted
            .note
            .iter()
            .map(|n| text_line("DESCRIPTION", n))
            .collect();
        replace(&mut event, &["DESCRIPTION"], lines);
        changed = true;
    }
    if was.busy != wanted.busy {
        replace(
            &mut event,
            &["TRANSP", "X-MICROSOFT-CDO-BUSYSTATUS"],
            vec![transp_line(wanted.busy)],
        );
        changed = true;
    }
    if was.rrule != wanted.rrule {
        let lines = wanted.rrule.iter().map(|r| format!("RRULE:{r}")).collect();
        replace(&mut event, &["RRULE"], lines);
        changed = true;
    }
    if !same_list(&was.rdate, &wanted.rdate) {
        replace(&mut event, &["RDATE"], list_lines("RDATE", &wanted.rdate)?);
        changed = true;
    }
    if !same_list(&was.exdate, &wanted.exdate) {
        replace(
            &mut event,
            &["EXDATE"],
            list_lines("EXDATE", &wanted.exdate)?,
        );
        changed = true;
    }
    if changed {
        let sequence = event
            .lines()
            .find(|l| line_name(l) == "SEQUENCE")
            .and_then(|l| split_line(l).2.trim().parse::<u32>().ok())
            .map_or(1, |s| s + 1);
        replace(
            &mut event,
            &["SEQUENCE"],
            vec![format!("SEQUENCE:{sequence}")],
        );
        replace(
            &mut event,
            &["LAST-MODIFIED"],
            vec![stamp_line("LAST-MODIFIED", now)],
        );
        replace(&mut event, &["DTSTAMP"], vec![stamp_line("DTSTAMP", now)]);
    }
    Ok(event)
}

/// Removes every line named in `names` and puts `lines` where the first of
/// them stood, else at the end.
fn replace(event: &mut Block, names: &[&str], lines: Vec<String>) {
    let at = event
        .items
        .iter()
        .position(|i| matches!(i, Item::Line(l) if names.contains(&line_name(l).as_str())));
    event
        .items
        .retain(|i| !matches!(i, Item::Line(l) if names.contains(&line_name(l).as_str())));
    let at = at.map(|at| at.min(event.items.len())).unwrap_or_else(|| {
        // Before the first component inside (VALARM), else at the end.
        event
            .items
            .iter()
            .position(|i| matches!(i, Item::Block(_)))
            .unwrap_or(event.items.len())
    });
    for (n, line) in lines.into_iter().enumerate() {
        event.items.insert(at + n, Item::Line(line));
    }
}

fn new_event(appointment: &Appointment, now: Timestamp) -> Result<Block, WriteError> {
    let mut lines = vec![
        format!("UID:{}", escape(&appointment.uid)),
        stamp_line("DTSTAMP", now),
        stamp_line("LAST-MODIFIED", now),
        "SEQUENCE:0".to_string(),
    ];
    if let Some(rid) = &appointment.recurrence_id {
        lines.push(when_line("RECURRENCE-ID", rid)?);
    }
    lines.push(text_line("SUMMARY", &appointment.title));
    lines.push(when_line("DTSTART", &appointment.start)?);
    if let Some(end) = &appointment.end {
        lines.push(when_line("DTEND", end)?);
    }
    if let Some(location) = &appointment.location {
        lines.push(text_line("LOCATION", location));
    }
    if let Some(note) = &appointment.note {
        lines.push(text_line("DESCRIPTION", note));
    }
    lines.push(transp_line(appointment.busy));
    match appointment.status {
        AppointmentStatus::Tentative => lines.push("STATUS:TENTATIVE".into()),
        AppointmentStatus::Cancelled => lines.push("STATUS:CANCELLED".into()),
        AppointmentStatus::Confirmed | AppointmentStatus::Unknown => {}
    }
    if let Some(rule) = &appointment.rrule {
        lines.push(format!("RRULE:{rule}"));
    }
    lines.extend(list_lines("RDATE", &appointment.rdate)?);
    lines.extend(list_lines("EXDATE", &appointment.exdate)?);
    let mut items: Vec<Item> = lines.into_iter().map(Item::Line).collect();
    // An instance or a series made from one that reminds keeps reminding
    // (ADR 0101 §2): its alarms, cloned from the series, as VALARMs.
    items.extend(
        appointment
            .alarms
            .iter()
            .filter_map(|a| alarm_block(a, &appointment.title))
            .map(Item::Block),
    );
    Ok(Block {
        name: "VEVENT".into(),
        items,
    })
}

/// A VALARM that shows `title` for one alarm; none for a kind this samu
/// does not know.
fn alarm_block(alarm: &AppointmentAlarm, title: &str) -> Option<Block> {
    let trigger = match (alarm.at, alarm.before, alarm.related) {
        (Some(at), _, _) => format!("TRIGGER;VALUE=DATE-TIME:{}", at.strftime("%Y%m%dT%H%M%SZ")),
        (None, Some(before), AlarmRelated::Start) => format!("TRIGGER:{}", before.0.negate()),
        (None, Some(before), AlarmRelated::End) => {
            format!("TRIGGER;RELATED=END:{}", before.0.negate())
        }
        _ => return None,
    };
    Some(Block {
        name: "VALARM".into(),
        items: vec![
            Item::Line("ACTION:DISPLAY".into()),
            Item::Line(trigger),
            Item::Line(text_line("DESCRIPTION", title)),
        ],
    })
}

/// Two times alike: the same date, or the same instant in whatever zone.
fn same(a: &When, b: &When) -> bool {
    match (a, b) {
        (When::Date(x), When::Date(y)) => x == y,
        (When::At(x), When::At(y)) => {
            x.timestamp() == y.timestamp() && x.time_zone().iana_name() == y.time_zone().iana_name()
        }
        _ => false,
    }
}

fn same_opt(a: Option<&When>, b: Option<&When>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => same(a, b),
        _ => false,
    }
}

fn same_list(a: &[When], b: &[When]) -> bool {
    let keys = |l: &[When]| l.iter().map(key_of).collect::<BTreeSet<_>>();
    keys(a) == keys(b)
}

/// A DATE as `VALUE=DATE`, an instant with its IANA zone as `TZID`, in UTC
/// with `Z` when it has none.
fn when_line(name: &str, when: &When) -> Result<String, WriteError> {
    match when {
        When::Date(date) => Ok(format!("{name};VALUE=DATE:{}", date.strftime("%Y%m%d"))),
        When::At(at) => Ok(match at.time_zone().iana_name() {
            Some(zone) if !matches!(zone, "UTC" | "Etc/UTC") => {
                format!(
                    "{name};TZID={zone}:{}",
                    at.datetime().strftime("%Y%m%dT%H%M%S")
                )
            }
            _ => stamp_line(name, at.timestamp()),
        }),
        When::Unknown => Err(WriteError::Unwritable(name.to_string())),
    }
}

/// One line per value: simpler than a list, and every reader takes it.
fn list_lines(name: &str, list: &[When]) -> Result<Vec<String>, WriteError> {
    list.iter().map(|w| when_line(name, w)).collect()
}

fn stamp_line(name: &str, at: Timestamp) -> String {
    format!("{name}:{}", at.strftime("%Y%m%dT%H%M%SZ"))
}

fn text_line(name: &str, value: &str) -> String {
    format!("{name}:{}", escape(value))
}

fn transp_line(busy: bool) -> String {
    format!("TRANSP:{}", if busy { "OPAQUE" } else { "TRANSPARENT" })
}

/// A TEXT value escaped (RFC 5545 §3.3.11).
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// Adds a VTIMEZONE for every TZID a VEVENT names and the calendar does not
/// define, before the first VEVENT.
fn add_zones(calendar: &mut Block) {
    let defined: BTreeSet<String> = calendar
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Block(b) if b.name == "VTIMEZONE" => b
                .lines()
                .find(|l| line_name(l) == "TZID")
                .map(|l| split_line(l).2.trim().to_string()),
            _ => None,
        })
        .collect();
    // The earliest instant per zone, for where its transitions begin.
    let mut needed: BTreeMap<String, Timestamp> = BTreeMap::new();
    for item in &calendar.items {
        let Item::Block(event) = item else { continue };
        if event.name != "VEVENT" {
            continue;
        }
        for line in event.lines() {
            let (_, params, value) = split_line(line);
            let Some((_, tzid)) = params.iter().find(|(k, _)| k == "TZID") else {
                continue;
            };
            if defined.contains(tzid) {
                continue;
            }
            let Some(tz) = zones::zone(tzid) else {
                continue;
            };
            let at = jiff::civil::DateTime::strptime("%Y%m%dT%H%M%S", value.trim())
                .ok()
                .and_then(|wall| wall.to_zoned(tz).ok())
                .map_or(Timestamp::UNIX_EPOCH, |z| z.timestamp());
            needed
                .entry(tzid.clone())
                .and_modify(|t| *t = (*t).min(at))
                .or_insert(at);
        }
    }
    let first_event = calendar
        .items
        .iter()
        .position(|i| matches!(i, Item::Block(b) if b.name == "VEVENT"))
        .unwrap_or(calendar.items.len());
    for (n, (tzid, from)) in needed.into_iter().enumerate() {
        if let Some(block) = vtimezone(&tzid, from) {
            calendar.items.insert(first_event + n, Item::Block(block));
        }
    }
}

/// A VTIMEZONE listing the transitions of `tzid` from a year before `from`
/// on for [`ZONE_YEARS`] years, each as its own STANDARD or DAYLIGHT.
fn vtimezone(tzid: &str, from: Timestamp) -> Option<Block> {
    let tz = zones::zone(tzid)?;
    let year = SignedDuration::from_hours(24 * 366);
    let start = from.checked_sub(year).unwrap_or(from);
    let end = from
        .checked_add(SignedDuration::from_hours(24 * 366 * ZONE_YEARS))
        .unwrap_or(from);
    let mut items = vec![Item::Line(format!("TZID:{tzid}"))];
    for transition in tz.following(start) {
        let at = transition.timestamp();
        if at > end {
            break;
        }
        let before = tz.to_offset(at.checked_sub(SignedDuration::from_secs(1)).unwrap_or(at));
        let kind = if transition.dst().is_dst() {
            "DAYLIGHT"
        } else {
            "STANDARD"
        };
        let mut lines = vec![
            format!(
                "DTSTART:{}",
                before.to_datetime(at).strftime("%Y%m%dT%H%M%S")
            ),
            format!("TZOFFSETFROM:{}", offset_text(before)),
            format!("TZOFFSETTO:{}", offset_text(transition.offset())),
        ];
        if !transition.abbreviation().is_empty() {
            lines.push(format!("TZNAME:{}", transition.abbreviation()));
        }
        items.push(Item::Block(Block {
            name: kind.into(),
            items: lines.into_iter().map(Item::Line).collect(),
        }));
    }
    if items.len() == 1 {
        // A zone without transitions: one STANDARD for all time.
        let offset = offset_text(tz.to_offset(from));
        items.push(Item::Block(Block {
            name: "STANDARD".into(),
            items: vec![
                Item::Line("DTSTART:19700101T000000".into()),
                Item::Line(format!("TZOFFSETFROM:{offset}")),
                Item::Line(format!("TZOFFSETTO:{offset}")),
            ],
        }));
    }
    Some(Block {
        name: "VTIMEZONE".into(),
        items,
    })
}

/// An offset as `+0100`.
fn offset_text(offset: Offset) -> String {
    let seconds = offset.seconds();
    let sign = if seconds < 0 { '-' } else { '+' };
    let minutes = seconds.unsigned_abs() / 60;
    format!("{sign}{:02}{:02}", minutes / 60, minutes % 60)
}

/// The items as text, each line folded at 75 octets, with CRLF.
fn emit(items: &[Item]) -> String {
    let mut out = String::new();
    for item in items {
        emit_item(item, &mut out);
    }
    out
}

fn emit_item(item: &Item, out: &mut String) {
    match item {
        Item::Line(line) => fold(line, out),
        Item::Block(block) => {
            fold(&format!("BEGIN:{}", block.name), out);
            for item in &block.items {
                emit_item(item, out);
            }
            fold(&format!("END:{}", block.name), out);
        }
    }
}

/// One content line, folded so that no line is longer than 75 octets and
/// no character is split (RFC 5545 §3.1).
fn fold(line: &str, out: &mut String) {
    let mut width = 0;
    let mut limit = 75;
    for c in line.chars() {
        if width + c.len_utf8() > limit {
            out.push_str("\r\n ");
            width = 0;
            limit = 74;
        }
        out.push(c);
        width += c.len_utf8();
    }
    let _ = write!(out, "\r\n");
}

#[cfg(test)]
mod tests;
