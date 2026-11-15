//! An iCalendar file cut into one resource per UID for an import
//! (ADR 0100 §4). Line by line like the rest of `write`: what samu does not
//! know (VALARM, ATTENDEE, X- properties) goes on as it came.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    Block, Item, PRODID, WriteError, add_zones, emit, line_name, parse, split_line, unfold,
};

/// One resource to put: the VEVENTs of one UID in their own VCALENDAR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub uid: String,
    /// The VCALENDAR text, CRLF and folded.
    pub text: String,
}

/// What an import leaves out, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Left {
    /// The UID when it had one.
    pub uid: Option<String>,
    pub reason: String,
}

/// A file cut up: the resources in the order their UIDs first came, and
/// what was left out.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Split {
    pub pieces: Vec<Piece>,
    pub left: Vec<Left>,
}

/// Lines of a VCALENDAR that do not go into a resource of a collection:
/// METHOD belongs to a message, not to stored data (RFC 4791 §4.1).
const DROPPED: &[&str] = &["METHOD"];

/// Cuts `text` (one or more VCALENDARs) into one resource per UID: the main
/// VEVENT and its changed instances together, with the VTIMEZONEs they
/// name (from the file, else made from the zone data). VTODO, VJOURNAL and
/// other components, and a VEVENT without UID or DTSTART, are left out and
/// said.
pub fn split_by_uid(text: &str) -> Result<Split, WriteError> {
    let items = parse(&unfold(text))?;
    let calendars: Vec<&Block> = items
        .iter()
        .filter_map(|item| match item {
            Item::Block(b) if b.name == "VCALENDAR" => Some(b),
            _ => None,
        })
        .collect();
    if calendars.is_empty() {
        return Err(WriteError::NoCalendar);
    }
    let mut out = Split::default();
    let mut head: Vec<String> = Vec::new();
    let mut zones: BTreeMap<String, Block> = BTreeMap::new();
    let mut events: Vec<(String, Vec<Block>)> = Vec::new();
    for calendar in calendars {
        for item in &calendar.items {
            match item {
                Item::Line(line) => {
                    let name = line_name(line);
                    if !DROPPED.contains(&name.as_str())
                        && !head.iter().any(|h| line_name(h) == name)
                    {
                        head.push(line.clone());
                    }
                }
                Item::Block(b) if b.name == "VTIMEZONE" => {
                    if let Some(tzid) = value_of(b, "TZID") {
                        zones.entry(tzid).or_insert_with(|| b.clone());
                    }
                }
                Item::Block(b) if b.name == "VEVENT" => {
                    let uid = value_of(b, "UID").filter(|u| !u.is_empty());
                    let Some(uid) = uid else {
                        out.left.push(Left {
                            uid: None,
                            reason: format!("a VEVENT without UID ({})", summary(b)),
                        });
                        continue;
                    };
                    if value_of(b, "DTSTART").is_none() {
                        out.left.push(Left {
                            uid: Some(uid),
                            reason: "a VEVENT without DTSTART".into(),
                        });
                        continue;
                    }
                    match events.iter_mut().find(|(u, _)| *u == uid) {
                        Some((_, list)) => list.push(b.clone()),
                        None => events.push((uid, vec![b.clone()])),
                    }
                }
                Item::Block(b) => out.left.push(Left {
                    uid: value_of(b, "UID"),
                    reason: format!("a {} is no appointment", b.name),
                }),
            }
        }
    }
    if !head.iter().any(|h| line_name(h) == "VERSION") {
        head.insert(0, "VERSION:2.0".into());
    }
    if !head.iter().any(|h| line_name(h) == "PRODID") {
        head.insert(1, format!("PRODID:{PRODID}"));
    }
    for (uid, mut list) in events {
        // The main VEVENT first, then the changed instances.
        list.sort_by_key(|e| value_of(e, "RECURRENCE-ID").is_some());
        let named = zones_named(&list);
        let mut calendar = Block {
            name: "VCALENDAR".into(),
            items: head.iter().cloned().map(Item::Line).collect(),
        };
        for tzid in &named {
            if let Some(zone) = zones.get(tzid) {
                calendar.items.push(Item::Block(zone.clone()));
            }
        }
        calendar.items.extend(list.into_iter().map(Item::Block));
        add_zones(&mut calendar);
        out.pieces.push(Piece {
            uid,
            text: emit(&[Item::Block(calendar)]),
        });
    }
    Ok(out)
}

/// The value of a component's own property, trimmed.
fn value_of(block: &Block, name: &str) -> Option<String> {
    block
        .lines()
        .find(|l| line_name(l) == name)
        .map(|l| split_line(l).2.trim().to_string())
}

fn summary(block: &Block) -> String {
    value_of(block, "SUMMARY").unwrap_or_else(|| "no title".into())
}

/// The TZIDs the VEVENTs name, nested components included.
fn zones_named(events: &[Block]) -> BTreeSet<String> {
    fn walk(block: &Block, out: &mut BTreeSet<String>) {
        for item in &block.items {
            match item {
                Item::Line(line) => {
                    let (_, params, _) = split_line(line);
                    if let Some((_, tzid)) = params.into_iter().find(|(k, _)| k == "TZID") {
                        out.insert(tzid);
                    }
                }
                Item::Block(inner) => walk(inner, out),
            }
        }
    }
    let mut out = BTreeSet::new();
    for event in events {
        walk(event, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::*;

    const FILE: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Example//Export//EN\r\n\
METHOD:PUBLISH\r\nX-WR-CALNAME:Alt\r\n\
BEGIN:VTIMEZONE\r\nTZID:Europe/Berlin\r\nBEGIN:STANDARD\r\nDTSTART:19701025T030000\r\n\
TZOFFSETFROM:+0200\r\nTZOFFSETTO:+0100\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n\
BEGIN:VTIMEZONE\r\nTZID:America/New_York\r\nBEGIN:STANDARD\r\nDTSTART:19701101T020000\r\n\
TZOFFSETFROM:-0400\r\nTZOFFSETTO:-0500\r\nEND:STANDARD\r\nEND:VTIMEZONE\r\n\
BEGIN:VEVENT\r\nUID:serie@example.org\r\nDTSTART;TZID=Europe/Berlin:20261102T090000\r\n\
DTEND;TZID=Europe/Berlin:20261102T100000\r\nRRULE:FREQ=WEEKLY\r\nSUMMARY:Jour fixe\r\n\
ATTENDEE;CN=\"Kim\":mailto:kim@example.org\r\nX-SOMETHING:kept\r\n\
BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:tag@example.org\r\nDTSTART;VALUE=DATE:20261120\r\nSUMMARY:Ganzer Tag\r\nEND:VEVENT\r\n\
BEGIN:VTODO\r\nUID:todo@example.org\r\nSUMMARY:Aufgabe\r\nEND:VTODO\r\n\
BEGIN:VEVENT\r\nSUMMARY:Ohne UID\r\nDTSTART;VALUE=DATE:20261121\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:serie@example.org\r\nRECURRENCE-ID;TZID=Europe/Berlin:20261109T090000\r\n\
DTSTART;TZID=Europe/Berlin:20261109T110000\r\nSUMMARY:Jour fixe, später\r\nEND:VEVENT\r\n\
END:VCALENDAR\r\n";

    #[test]
    fn a_file_is_cut_into_one_resource_per_uid_with_what_it_needs() {
        let split = split_by_uid(FILE).unwrap();
        let uids: Vec<&str> = split.pieces.iter().map(|p| p.uid.as_str()).collect();
        assert_eq!(uids, ["serie@example.org", "tag@example.org"]);
        let serie = &split.pieces[0].text;
        // Main and changed instance together, the main one first.
        assert_eq!(serie.matches("BEGIN:VEVENT").count(), 2);
        assert!(serie.find("RRULE").unwrap() < serie.find("RECURRENCE-ID").unwrap());
        // What samu does not know goes along as it was.
        assert!(serie.contains("BEGIN:VALARM\r\nACTION:DISPLAY\r\nTRIGGER:-PT15M\r\nEND:VALARM"));
        assert!(serie.contains("ATTENDEE;CN=\"Kim\":mailto:kim@example.org"));
        assert!(serie.contains("X-SOMETHING:kept"));
        // The zone it names from the file, not the one it does not.
        assert!(serie.contains("TZID:Europe/Berlin\r\nBEGIN:STANDARD\r\nDTSTART:19701025T030000"));
        assert!(!serie.contains("America/New_York"));
        // The head without METHOD.
        assert!(
            serie
                .starts_with("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Example//Export//EN\r\n")
        );
        assert!(!serie.contains("METHOD"));
        assert!(serie.contains("X-WR-CALNAME:Alt"));
        let tag = &split.pieces[1].text;
        assert_eq!(tag.matches("BEGIN:VEVENT").count(), 1);
        assert!(!tag.contains("VTIMEZONE"));
        // Each piece reads back as what it holds.
        let read = crate::calendar::read_ics(serie, &jiff::tz::TimeZone::UTC).unwrap();
        assert_eq!(read.appointments.len(), 2);

        assert_eq!(
            split.left,
            vec![
                Left {
                    uid: Some("todo@example.org".into()),
                    reason: "a VTODO is no appointment".into()
                },
                Left {
                    uid: None,
                    reason: "a VEVENT without UID (Ohne UID)".into()
                },
            ]
        );
    }

    #[test]
    fn a_zone_the_file_does_not_define_is_made_and_a_bare_file_gets_a_head() {
        let text = "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:a\nDTSTART;TZID=Europe/Berlin:20261102T090000\nSUMMARY:x\nEND:VEVENT\nEND:VCALENDAR\n";
        let split = split_by_uid(text).unwrap();
        let piece = &split.pieces[0].text;
        assert!(piece.starts_with("BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:"));
        assert!(piece.contains("BEGIN:VTIMEZONE\r\nTZID:Europe/Berlin"));
    }

    #[test]
    fn several_calendars_in_one_file_are_one_and_nothing_is_an_error() {
        let one = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nUID:a\r\nDTSTART;VALUE=DATE:20261102\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let two = one.replace("UID:a", "UID:b");
        let split = split_by_uid(&format!("{one}{two}")).unwrap();
        assert_eq!(split.pieces.len(), 2);
        assert_eq!(split_by_uid("hello"), Err(WriteError::NoCalendar));
        assert!(split_by_uid("BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\n").is_err());
        let no_start =
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:c\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let split = split_by_uid(no_start).unwrap();
        assert!(split.pieces.is_empty());
        assert_eq!(split.left[0].uid.as_deref(), Some("c"));
    }
}
