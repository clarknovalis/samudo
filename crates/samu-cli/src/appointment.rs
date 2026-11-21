// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `samu appointment …` (ADR 0068): appointments of a writable calendar
//! made, changed and deleted. A new one is a `create` over `POST /changes`;
//! an edit or a deletion goes to `POST /appointments/{id}/edit`, where the
//! server (or the replica) turns it into changes, a repetition included.
//! The server writes them back to the calendar at once.

use crate::client::Client;
use anyhow::{Result, anyhow, bail};
use clap::{Subcommand, ValueEnum};
use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;
use samu_api::{
    AnyObject, Appointment, AppointmentEdit, AppointmentEditResponse, AppointmentPatch,
    AppointmentStatus, Change, ChangeOutcome, ChangesPage, EditRange, Id, ObjectKind, When,
};
use samu_client::Error as Failure;
use serde_json::{Value, json};

use crate::config::Config;
use crate::output::{Format, emit};
use crate::{placeholder, short, signed_in, write};

#[derive(Debug, Subcommand)]
pub(crate) enum AppointmentCommand {
    /// Make an appointment in a writable calendar (`--calendar`, else the
    /// account's default_calendar). A day alone is a whole day.
    Add {
        title: String,
        /// `2026-11-20` or `2026-11-20 09:00`, in this computer's zone.
        #[arg(long)]
        start: String,
        /// Excluded for days: `--start 2026-11-20 --end 2026-11-22` is two
        /// days. Without it: one hour, or the one day.
        #[arg(long)]
        end: Option<String>,
        #[arg(long)]
        calendar: Option<String>,
        #[arg(long)]
        location: Option<String>,
        #[arg(long)]
        note: Option<String>,
        /// Leaves the time free (TRANSP:TRANSPARENT).
        #[arg(long)]
        free: bool,
        /// A repetition, like `FREQ=WEEKLY;BYDAY=TU`.
        #[arg(long)]
        rrule: Option<String>,
    },
    /// Change an appointment, or one occurrence of a repetition with
    /// `--instance` and `--range` (POST /appointments/{id}/edit).
    Edit {
        appointment: String,
        /// The start of the occurrence, as `samu appointments` shows it.
        #[arg(long)]
        instance: Option<String>,
        #[arg(long, value_enum)]
        range: Option<Range>,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        start: Option<String>,
        #[arg(long)]
        end: Option<String>,
        /// Empty clears it.
        #[arg(long)]
        location: Option<String>,
        /// Empty clears it.
        #[arg(long)]
        note: Option<String>,
        #[arg(long)]
        busy: Option<bool>,
        #[arg(long)]
        rrule: Option<String>,
        /// Move it into another writable calendar.
        #[arg(long)]
        calendar: Option<String>,
        /// Clear `end` or `rrule` (also `location`, `note`).
        #[arg(long, value_name = "FIELD")]
        clear: Vec<String>,
    },
    /// Delete an appointment, or occurrences of a repetition with
    /// `--instance` and `--range`.
    Delete {
        appointment: String,
        #[arg(long)]
        instance: Option<String>,
        #[arg(long, value_enum)]
        range: Option<Range>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Range {
    This,
    Following,
    All,
}

impl Range {
    const fn wire(self) -> EditRange {
        match self {
            Self::This => EditRange::This,
            Self::Following => EditRange::Following,
            Self::All => EditRange::All,
        }
    }
}

/// `2026-11-20` is a day; `2026-11-20 09:00` (or with a `T`) a time in
/// `zone`.
fn parse_when(text: &str, zone: &TimeZone) -> Result<When> {
    let text = text.trim();
    // jiff reads a day out of a day with a time too: only ten characters
    // are a day alone.
    if text.len() == 10
        && let Ok(day) = text.parse::<Date>()
    {
        return Ok(When::Date(day));
    }
    let wall: DateTime = text
        .replacen(' ', "T", 1)
        .parse()
        .map_err(|e| anyhow!("{text:?} is neither a day nor a day with a time: {e}"))?;
    Ok(When::At(wall.to_zoned(zone.clone())?))
}

/// Everything `GET /changes?after=0` holds.
fn everything(client: &Client) -> Result<Vec<AnyObject>> {
    let page: ChangesPage =
        serde_json::from_value(client.call("GET", "/changes?after=0", None)?.json()?)?;
    Ok(page.objects)
}

/// An appointment by its id or the start or end of one.
fn find(objects: &[AnyObject], reference: &str) -> Result<Appointment> {
    let wanted = reference.to_lowercase();
    let found: Vec<&Appointment> = objects
        .iter()
        .filter_map(|o| match o {
            AnyObject::Appointment(a) if crate::id_matches(&a.id, &wanted) => Some(a),
            _ => None,
        })
        .collect();
    match found.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(Failure::NotFound(format!("no appointment {reference:?}")).into()),
        many => bail!(
            "{reference:?} matches {} appointments, name one by more of its id",
            many.len()
        ),
    }
}

/// A calendar by title, id or the start or end of one; without a name the account's
/// default_calendar.
fn calendar_of(objects: &[AnyObject], reference: Option<&str>) -> Result<Id> {
    let calendars = objects.iter().filter_map(|o| match o {
        AnyObject::Calendar(c) => Some(c),
        _ => None,
    });
    let Some(reference) = reference else {
        return objects
            .iter()
            .find_map(|o| match o {
                AnyObject::Settings(s) => s.default_calendar,
                _ => None,
            })
            .ok_or_else(|| anyhow!("no --calendar and no default_calendar in the settings"));
    };
    let wanted = reference.to_lowercase();
    let found: Vec<Id> = calendars
        .filter(|c| c.title.to_lowercase() == wanted || crate::id_matches(&c.id, &wanted))
        .map(|c| c.id)
        .collect();
    match found.as_slice() {
        [one] => Ok(*one),
        [] => Err(Failure::NotFound(format!("no calendar {reference:?}")).into()),
        _ => bail!("{reference:?} matches several calendars, name one by its id"),
    }
}

/// The end when none was given: an hour after a time, the next day after a
/// day.
fn default_end(start: &When) -> Option<When> {
    match start {
        When::Date(d) => d.tomorrow().ok().map(When::Date),
        When::At(at) => at
            .checked_add(jiff::SignedDuration::from_hours(1))
            .ok()
            .map(When::At),
        When::Unknown => None,
    }
}

fn records(response: &AppointmentEditResponse) -> Vec<Value> {
    response
        .results
        .iter()
        .filter_map(|r| {
            let (op, object, field) = match &r.change {
                Change::Create { object, .. } => ("create", *object, None),
                Change::Set { object, field, .. } => ("set", *object, Some(field.clone())),
                Change::Delete { object, .. } => ("delete", *object, None),
                _ => return None,
            };
            Some(json!({
                "op": op,
                "object": object,
                "short": short(&object),
                "field": field,
                "applied": matches!(r.outcome, ChangeOutcome::Applied { .. }),
            }))
        })
        .collect()
}

fn edit(client: &Client, id: Id, request: &AppointmentEdit) -> Result<AppointmentEditResponse> {
    let answer = client.call(
        "POST",
        &format!("/appointments/{id}/edit"),
        Some(&serde_json::to_value(request)?),
    )?;
    Ok(serde_json::from_value(answer.json()?)?)
}

pub(crate) fn run(config: &Config, format: Format, command: AppointmentCommand) -> Result<()> {
    let client = signed_in(config)?;
    let zone = TimeZone::system();
    match command {
        AppointmentCommand::Add {
            title,
            start,
            end,
            calendar,
            location,
            note,
            free,
            rrule,
        } => {
            let objects = everything(&client)?;
            let calendar = calendar_of(&objects, calendar.as_deref())?;
            let start = parse_when(&start, &zone)?;
            let end = match end {
                Some(end) => Some(parse_when(&end, &zone)?),
                None => default_end(&start),
            };
            let now = jiff::Timestamp::now();
            let id = Id::new();
            let appointment = Appointment {
                id,
                created: now,
                modified: now,
                calendar,
                uid: id.to_string(),
                recurrence_id: None,
                title,
                start,
                end,
                location: location.filter(|l| !l.is_empty()),
                note: note.filter(|n| !n.is_empty()),
                url: None,
                status: AppointmentStatus::Confirmed,
                busy: !free,
                rrule,
                rdate: Vec::new(),
                exdate: Vec::new(),
                meeting: false,
                alarms: Vec::new(),
            };
            write(
                &client,
                &[Change::Create {
                    object: id,
                    kind: ObjectKind::Appointment,
                    value: serde_json::to_value(&appointment)?,
                    hlc: placeholder(),
                }],
            )?;
            emit(
                format,
                &[json!({"id": id, "short": short(&id), "title": appointment.title})],
                &["short", "title"],
            )
        }
        AppointmentCommand::Edit {
            appointment,
            instance,
            range,
            title,
            start,
            end,
            location,
            note,
            busy,
            rrule,
            calendar,
            clear,
        } => {
            let objects = everything(&client)?;
            let target = find(&objects, &appointment)?;
            let calendar = match calendar {
                Some(c) => Some(calendar_of(&objects, Some(&c))?),
                None => None,
            };
            let patch = AppointmentPatch {
                title,
                start: start.map(|s| parse_when(&s, &zone)).transpose()?,
                end: end.map(|s| parse_when(&s, &zone)).transpose()?,
                location,
                note,
                busy,
                rrule,
                calendar,
                clear,
            };
            if patch == AppointmentPatch::default() {
                bail!("nothing to change: name a field");
            }
            let request = AppointmentEdit {
                instance: instance.map(|i| parse_when(&i, &zone)).transpose()?,
                range: range.map_or(EditRange::This, Range::wire),
                patch: Some(patch),
                delete: false,
            };
            let response = edit(&client, target.id, &request)?;
            emit(format, &records(&response), &["op", "short", "field"])
        }
        AppointmentCommand::Delete {
            appointment,
            instance,
            range,
        } => {
            let objects = everything(&client)?;
            let target = find(&objects, &appointment)?;
            let request = AppointmentEdit {
                instance: instance.map(|i| parse_when(&i, &zone)).transpose()?,
                range: range.map_or(EditRange::This, Range::wire),
                patch: None,
                delete: true,
            };
            let response = edit(&client, target.id, &request)?;
            emit(format, &records(&response), &["op", "short", "field"])
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::*;

    #[test]
    fn a_day_is_a_date_and_a_time_is_in_the_zone() {
        let berlin = TimeZone::get("Europe/Berlin").unwrap();
        assert_eq!(
            parse_when("2026-11-20", &berlin).unwrap(),
            When::Date(jiff::civil::date(2026, 11, 20))
        );
        let When::At(at) = parse_when("2026-11-20 09:00", &berlin).unwrap() else {
            panic!()
        };
        assert_eq!(at.to_string(), "2026-11-20T09:00:00+01:00[Europe/Berlin]");
        assert!(parse_when("next friday", &berlin).is_err());
    }

    #[test]
    fn without_an_end_an_hour_or_the_one_day() {
        let berlin = TimeZone::get("Europe/Berlin").unwrap();
        let day = parse_when("2026-11-20", &berlin).unwrap();
        assert_eq!(
            default_end(&day),
            Some(When::Date(jiff::civil::date(2026, 11, 21)))
        );
        let time = parse_when("2026-11-20 09:00", &berlin).unwrap();
        assert_eq!(
            default_end(&time),
            Some(parse_when("2026-11-20 10:00", &berlin).unwrap())
        );
    }
}
