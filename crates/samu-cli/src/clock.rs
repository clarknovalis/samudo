// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `samu clock …` (ADR 0046): clocks are objects of their own, started,
//! stopped and added over `POST /changes`. That one clock runs per account
//! and that a clock stops with its task, the server sees to; the report is
//! `GET /clocks/report`, computed by samu-core.

use crate::client::Client;
use anyhow::{Result, bail};
use clap::{Subcommand, ValueEnum};
use samu_api::{
    AnyObject, CaptureResponse, Change, ChangesPage, Clock, ClockReport, ClockReportBy, Id,
    ObjectKind,
};
use samu_client::Error as Failure;
use serde_json::{Value, json};

use crate::config::Config;
use crate::output::{Format, emit};
use crate::{placeholder, resolve, scopes, short, signed_in, write};

#[derive(Debug, Subcommand)]
pub(crate) enum ClockCommand {
    /// Start a clock on a task or project; a running one ends where this
    /// one begins.
    In {
        id: String,
        /// When it began: RFC 3339, or a local date and time
        /// (`2026-11-09T08:30`) in this computer's zone; now by default.
        #[arg(long)]
        at: Option<String>,
        #[arg(long)]
        note: Option<String>,
    },
    /// Stop the running clock. Exit 3 when none runs.
    Out {
        /// When it ended; now by default.
        #[arg(long)]
        at: Option<String>,
    },
    /// The running clock, if any; nothing when none runs.
    Status,
    /// The clocks, of every task or of one, oldest first, with their length.
    List { id: Option<String> },
    /// Add a clock that has ended: time spent and not clocked. Either
    /// `--start` and `--end`, or the time as after `z:` in quick capture
    /// (ADR 0081 §5): `"gestern 2h"`, `"14-15:30"`, `1h30`.
    Add {
        id: String,
        /// As after `z:`: a length that ends now, a past day and a length
        /// (after that day's last clock, else from 09:00), or from-to.
        #[arg(conflicts_with_all = ["start", "end", "note"], required_unless_present = "start")]
        time: Option<String>,
        #[arg(long, requires = "end")]
        start: Option<String>,
        #[arg(long, requires = "start")]
        end: Option<String>,
        #[arg(long)]
        note: Option<String>,
    },
    /// Change a clock: when it began, when it ended, its note. The server
    /// refuses an end before the start; overlapping other clocks is allowed.
    Set {
        /// The clock, as `samu clock list` shows it.
        id: String,
        /// When it began: RFC 3339, or a local date and time in this
        /// computer's zone.
        #[arg(long)]
        start: Option<String>,
        #[arg(long, conflicts_with = "running")]
        end: Option<String>,
        /// The note; an empty one removes it.
        #[arg(long)]
        note: Option<String>,
        /// Take the end away so it runs again; refused while another clock
        /// runs (ADR 0046 §2).
        #[arg(long)]
        running: bool,
    },
    /// Time clocked over days (GET /clocks/report), in the account's zone.
    Report {
        /// By task, project, folder, tag or day.
        #[arg(long, value_enum, default_value_t = By::Task)]
        by: By,
        /// The first day (YYYY-MM-DD); today by default.
        #[arg(long)]
        from: Option<String>,
        /// The last day, included; `--from` by default.
        #[arg(long)]
        to: Option<String>,
        /// Through this scope instead of the one chosen here (ADR 0035).
        #[arg(long, conflicts_with = "no_scope")]
        scope: Option<String>,
        /// Without a scope, whatever is chosen here.
        #[arg(long)]
        no_scope: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum By {
    Task,
    Project,
    Folder,
    Tag,
    Day,
}

impl By {
    pub(crate) fn wire(self) -> ClockReportBy {
        match self {
            Self::Task => ClockReportBy::Task,
            Self::Project => ClockReportBy::Project,
            Self::Folder => ClockReportBy::Folder,
            Self::Tag => ClockReportBy::Tag,
            Self::Day => ClockReportBy::Day,
        }
    }
}

/// `1:05` for an hour and five minutes; seconds are dropped.
pub(crate) fn length(seconds: i64) -> String {
    let minutes = seconds.max(0) / 60;
    format!("{}:{:02}", minutes / 60, minutes % 60)
}

/// Every clock and the titles of what they are on.
struct Clocks {
    clocks: Vec<Clock>,
    titles: std::collections::HashMap<Id, String>,
}

fn clocks(client: &Client) -> Result<Clocks> {
    let page: ChangesPage =
        serde_json::from_value(client.call("GET", "/changes?after=0", None)?.json()?)?;
    let mut out = Clocks {
        clocks: Vec::new(),
        titles: std::collections::HashMap::new(),
    };
    for object in page.objects {
        match object {
            AnyObject::Clock(c) => out.clocks.push(c),
            AnyObject::Task(t) => {
                out.titles.insert(t.id, t.title);
            }
            AnyObject::Project(p) => {
                out.titles.insert(p.id, p.title);
            }
            _ => {}
        }
    }
    out.clocks.sort_by_key(|c| (c.start, c.id));
    Ok(out)
}

/// The titles of tasks and projects, by id.
pub(crate) fn titles(client: &Client) -> Result<std::collections::HashMap<Id, String>> {
    Ok(clocks(client)?.titles)
}

fn record(clock: &Clock, title: Option<&String>, now: jiff::Timestamp) -> Value {
    let seconds = (clock.end.unwrap_or(now).as_second() - clock.start.as_second()).max(0);
    json!({
        "id": clock.id,
        "short": short(&clock.id),
        "owner": clock.owner,
        "title": title,
        "start": clock.start.to_string(),
        "end": clock.end.map(|t| t.to_string()),
        "running": clock.end.is_none(),
        "seconds": seconds,
        "length": length(seconds),
        "note": clock.note,
    })
}

fn create(clock: &Clock) -> Result<Change> {
    Ok(Change::Create {
        object: clock.id,
        kind: ObjectKind::Clock,
        value: serde_json::to_value(clock)?,
        hlc: placeholder(),
    })
}

fn instant_or_now(text: Option<&str>) -> Result<jiff::Timestamp> {
    text.map_or_else(|| Ok(jiff::Timestamp::now()), crate::remind::instant)
}

const COLUMNS: &[&str] = &["short", "start", "end", "length", "title"];

pub(crate) fn run(config: &Config, format: Format, command: ClockCommand) -> Result<()> {
    let client = signed_in(config)?;
    let now = jiff::Timestamp::now();
    match command {
        ClockCommand::In { id, at, note } => {
            let owner = resolve(&client, &id)?;
            let start = instant_or_now(at.as_deref())?;
            let before = clocks(&client)?;
            let clock = Clock {
                id: Id::new(),
                created: now,
                modified: now,
                owner,
                start,
                end: None,
                note,
            };
            write(&client, &[create(&clock)?])?;
            let after = clocks(&client)?;
            for old in before.clocks.iter().filter(|c| c.end.is_none()) {
                if let Some(ended) = after
                    .clocks
                    .iter()
                    .find(|c| c.id == old.id && c.end.is_some())
                {
                    eprintln!(
                        "stopped: {} ({})",
                        after.titles.get(&ended.owner).map_or("?", String::as_str),
                        length(ended.end.unwrap_or(now).as_second() - ended.start.as_second())
                    );
                }
            }
            let mine = after
                .clocks
                .iter()
                .find(|c| c.id == clock.id)
                .unwrap_or(&clock);
            if mine.end.is_some() {
                eprintln!("a newer clock runs: this one ended where that one began");
            }
            emit(
                format,
                &[record(mine, after.titles.get(&owner), now)],
                COLUMNS,
            )
        }
        ClockCommand::Out { at } => {
            let all = clocks(&client)?;
            let Some(running) = running_of(&all.clocks) else {
                return Err(Failure::NotFound("no clock runs".into()).into());
            };
            let end = instant_or_now(at.as_deref())?;
            if end < running.start {
                bail!("--at is before the clock began ({})", running.start);
            }
            crate::set_field(&client, running.id, "end", json!(end.to_string()))?;
            let stopped = Clock {
                end: Some(end),
                ..running.clone()
            };
            emit(
                format,
                &[record(&stopped, all.titles.get(&stopped.owner), now)],
                COLUMNS,
            )
        }
        ClockCommand::Status => {
            let all = clocks(&client)?;
            let records: Vec<Value> = running_of(&all.clocks)
                .map(|c| record(c, all.titles.get(&c.owner), now))
                .into_iter()
                .collect();
            emit(format, &records, COLUMNS)
        }
        ClockCommand::List { id } => {
            let owner = id.map(|id| resolve(&client, &id)).transpose()?;
            let all = clocks(&client)?;
            let records: Vec<Value> = all
                .clocks
                .iter()
                .filter(|c| owner.is_none_or(|o| o == c.owner))
                .map(|c| record(c, all.titles.get(&c.owner), now))
                .collect();
            emit(format, &records, COLUMNS)
        }
        ClockCommand::Add {
            id,
            time: Some(time),
            ..
        } => {
            let owner = resolve(&client, &id)?;
            let answer: CaptureResponse = serde_json::from_value(
                client
                    .call(
                        "POST",
                        "/capture",
                        Some(&json!({"text": format!("z:{time}"), "task": owner})),
                    )?
                    .json()?,
            )?;
            for warning in &answer.warnings {
                eprintln!("samu: {}", crate::warning_text(warning));
            }
            if answer.clocks.is_empty() {
                bail!("{time:?}: no clock booked");
            }
            let titles = clocks(&client)?.titles;
            let records: Vec<Value> = answer
                .clocks
                .iter()
                .map(|c| record(c, titles.get(&owner), now))
                .collect();
            emit(format, &records, COLUMNS)
        }
        ClockCommand::Add {
            id,
            time: None,
            start,
            end,
            note,
        } => {
            let owner = resolve(&client, &id)?;
            let (Some(start), Some(end)) = (start, end) else {
                bail!("--start and --end, or the time as after z:");
            };
            let (start, end) = (
                crate::remind::instant(&start)?,
                crate::remind::instant(&end)?,
            );
            if end < start {
                bail!("--end is before --start");
            }
            let clock = Clock {
                id: Id::new(),
                created: now,
                modified: now,
                owner,
                start,
                end: Some(end),
                note,
            };
            write(&client, &[create(&clock)?])?;
            let titles = clocks(&client)?.titles;
            emit(format, &[record(&clock, titles.get(&owner), now)], COLUMNS)
        }
        ClockCommand::Set {
            id,
            start,
            end,
            note,
            running,
        } => {
            let id = resolve(&client, &id)?;
            let all = clocks(&client)?;
            let Some(clock) = all.clocks.iter().find(|c| c.id == id) else {
                return Err(Failure::NotFound(format!("no clock {id}")).into());
            };
            if start.is_none() && end.is_none() && note.is_none() && !running {
                bail!("nothing to change: --start, --end, --note or --running");
            }
            let mut after = clock.clone();
            if let Some(start) = &start {
                after.start = crate::remind::instant(start)?;
            }
            if let Some(end) = &end {
                after.end = Some(crate::remind::instant(end)?);
            }
            if running {
                if let Some(other) = running_of(&all.clocks).filter(|c| c.id != id) {
                    bail!(
                        "another clock runs ({}); stop it first",
                        all.titles.get(&other.owner).map_or("?", String::as_str)
                    );
                }
                after.end = None;
            }
            if let Some(note) = &note {
                after.note = (!note.is_empty()).then(|| note.clone());
            }
            if after.end.is_some_and(|end| end < after.start) {
                bail!("the end is before the start");
            }
            let changes = edit_changes(clock, &after);
            if !changes.is_empty() {
                write(&client, &changes)?;
            }
            emit(
                format,
                &[record(&after, all.titles.get(&after.owner), now)],
                COLUMNS,
            )
        }
        ClockCommand::Report {
            by,
            from,
            to,
            scope,
            no_scope,
        } => {
            let mut query = vec![format!("by={}", by_name(by.wire()))];
            for (name, value) in [("from", &from), ("to", &to)] {
                if let Some(value) = value {
                    let day: jiff::civil::Date = value
                        .parse()
                        .map_err(|e| anyhow::anyhow!("--{name} {value:?} is not a day: {e}"))?;
                    query.push(format!("{name}={day}"));
                }
            }
            if let Some(id) = scopes::query_scope(&client, config, scope.as_deref(), no_scope)? {
                query.push(format!("scope={id}"));
            }
            let path = format!("/clocks/report?{}", query.join("&"));
            let report: ClockReport =
                serde_json::from_value(client.call("GET", &path, None)?.json()?)?;
            emit(
                format,
                &report_records(&report),
                &["what", "length", "estimate"],
            )
        }
    }
}

/// The field changes from `before` to `after`, in an order the server takes
/// one by one: a start moved past the old end goes after the new end, so no
/// step in between ends before it starts.
fn edit_changes(before: &Clock, after: &Clock) -> Vec<Change> {
    let set = |field: &str, value: Value| Change::Set {
        object: before.id,
        field: field.into(),
        value,
        note: None,
        hlc: placeholder(),
    };
    let start = (after.start != before.start).then(|| set("start", json!(after.start.to_string())));
    let end = (after.end != before.end).then(|| {
        set(
            "end",
            after.end.map_or(Value::Null, |e| json!(e.to_string())),
        )
    });
    let start_last = before.end.is_some_and(|old_end| after.start > old_end);
    let mut changes: Vec<Change> = if start_last {
        [end, start].into_iter().flatten().collect()
    } else {
        [start, end].into_iter().flatten().collect()
    };
    if after.note != before.note {
        changes.push(set(
            "note",
            after.note.as_ref().map_or(Value::Null, |n| json!(n)),
        ));
    }
    changes
}

/// The running clock the server keeps (ADR 0046 §2): the latest start, the
/// larger id on a tie. The CLI does not link samu-core.
fn running_of(clocks: &[Clock]) -> Option<&Clock> {
    clocks
        .iter()
        .filter(|c| c.end.is_none())
        .max_by_key(|c| (c.start, c.id))
}

pub(crate) fn by_name(by: ClockReportBy) -> &'static str {
    match by {
        ClockReportBy::Task => "task",
        ClockReportBy::Project => "project",
        ClockReportBy::Folder => "folder",
        ClockReportBy::Tag => "tag",
        ClockReportBy::Day | ClockReportBy::Unknown => "day",
    }
}

/// A row per entry and a last one with the total.
pub(crate) fn report_records(report: &ClockReport) -> Vec<Value> {
    let none = match report.by {
        ClockReportBy::Project => "(Inbox)",
        ClockReportBy::Folder => "(ohne Ordner)",
        ClockReportBy::Tag => "(ohne Tag)",
        _ => "?",
    };
    let mut records: Vec<Value> = report
        .rows
        .iter()
        .map(|row| {
            let what = match (row.date, row.id) {
                (Some(date), _) => date.to_string(),
                (None, Some(_)) if !row.title.is_empty() => row.title.clone(),
                (None, Some(id)) => short(&id),
                (None, None) => none.to_owned(),
            };
            json!({
                "id": row.id,
                "date": row.date.map(|d| d.to_string()),
                "what": what,
                "seconds": row.seconds,
                "length": length(row.seconds),
                "estimate": row.estimate.map(|e| e.0.to_string()),
            })
        })
        .collect();
    records.push(json!({
        "what": format!("Summe {} – {}", report.from, report.to),
        "seconds": report.seconds,
        "length": length(report.seconds),
        "total": true,
    }));
    records
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests may unwrap")]

    use super::*;

    #[test]
    fn a_length_is_hours_and_minutes() {
        assert_eq!(length(0), "0:00");
        assert_eq!(length(59), "0:00");
        assert_eq!(length(3_900), "1:05");
        assert_eq!(length(36_000), "10:00");
        assert_eq!(length(-5), "0:00");
    }

    #[test]
    fn a_start_moved_past_the_old_end_goes_last() {
        let at = |s: &str| s.parse::<jiff::Timestamp>().unwrap();
        let before = Clock {
            id: Id::new(),
            created: at("2026-11-09T08:00:00Z"),
            modified: at("2026-11-09T08:00:00Z"),
            owner: Id::new(),
            start: at("2026-11-09T08:00:00Z"),
            end: Some(at("2026-11-09T09:00:00Z")),
            note: None,
        };
        let fields = |changes: Vec<Change>| -> Vec<String> {
            changes
                .into_iter()
                .map(|c| match c {
                    Change::Set { field, .. } => field,
                    _ => String::new(),
                })
                .collect()
        };
        let later = Clock {
            start: at("2026-11-09T10:00:00Z"),
            end: Some(at("2026-11-09T11:00:00Z")),
            note: Some("Telefon".into()),
            ..before.clone()
        };
        assert_eq!(
            fields(edit_changes(&before, &later)),
            ["end", "start", "note"]
        );
        let earlier = Clock {
            start: at("2026-11-09T07:00:00Z"),
            end: Some(at("2026-11-09T07:30:00Z")),
            ..before.clone()
        };
        assert_eq!(fields(edit_changes(&before, &earlier)), ["start", "end"]);
        assert!(edit_changes(&before, &before).is_empty());
    }

    #[test]
    fn a_report_ends_with_its_total() {
        let report: ClockReport = serde_json::from_value(json!({
            "from": "2026-11-09", "to": "2026-11-09", "by": "project", "seconds": 4500,
            "rows": [
                {"id": "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6c02", "title": "Steuer", "seconds": 2700},
                {"title": "", "seconds": 1800}
            ]
        }))
        .unwrap();
        let records = report_records(&report);
        assert_eq!(records.len(), 3);
        assert_eq!(records[0]["what"], "Steuer");
        assert_eq!(records[1]["what"], "(Inbox)");
        assert_eq!(records[2]["length"], "1:15");
    }
}
