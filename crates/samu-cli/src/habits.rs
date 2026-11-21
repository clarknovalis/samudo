// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `samu habit` (ADR 0091): habits are tasks with the field `habit`; what
//! they are today comes from `GET /habits` and `GET /habits/{id}`, computed
//! in samu-core; entries are objects of their own, made over `/changes`.

use anyhow::{Result, bail};
use clap::Subcommand;
use jiff::ToSpan;
use jiff::civil::Date;
use samu_api::{
    CaptureRequest, CaptureResponse, Change, HabitDay, HabitEntry, HabitEntryStatus, HabitKind,
    HabitPer, HabitStatus, Id, ObjectKind,
};
use serde_json::{Value, json};

use crate::config::Config;
use crate::output::{Format, emit};
use crate::{placeholder, print_capture, resolve, scopes, set_field, short, signed_in, write};
use samu_client::Client;

#[derive(Debug, Subcommand)]
pub(crate) enum HabitCommand {
    /// The open habits of the scope with what today is and their streak.
    List {
        /// Through this scope instead of the one chosen here (ADR 0035).
        #[arg(long, conflicts_with = "no_scope")]
        scope: Option<String>,
        /// Without a scope, whatever is chosen here.
        #[arg(long)]
        no_scope: bool,
    },
    /// One habit: streak, rate and the last days as a row of marks.
    Show {
        id: String,
        /// How many days back the history goes.
        #[arg(long, default_value_t = 28)]
        days: u32,
    },
    /// Enter it as done, for today or a day before.
    Done {
        id: String,
        /// `YYYY-MM-DD`, `heute`, `gestern` or `-N` days back.
        #[arg(long)]
        day: Option<String>,
        /// How much; without, 1.
        #[arg(long)]
        amount: Option<u32>,
    },
    /// Enter it as not done on purpose; for a habit to avoid, a relapse.
    Fail {
        id: String,
        #[arg(long)]
        day: Option<String>,
    },
    /// A pause, so the streak does not break: from a day (today) to a day.
    Pause {
        id: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        until: Option<String>,
    },
    /// The entries of a habit, newest first.
    Entries {
        id: String,
        /// How many days back.
        #[arg(long, default_value_t = 28)]
        days: u32,
    },
    /// Delete an entry: a tap that was wrong.
    Undo { entry: String },
    /// Make a task a habit, or change its goal, in the short form of
    /// `habit:` (`1/d`, `3/w`, `8/d:Glas`, `avoid`, empty for once a day);
    /// `none` makes it a task again.
    Set {
        id: String,
        #[arg(default_value = "")]
        goal: String,
    },
}

pub(crate) fn run(config: &Config, format: Format, command: HabitCommand) -> Result<()> {
    let client = signed_in(config)?;
    match command {
        HabitCommand::List { scope, no_scope } => {
            let path = match scopes::query_scope(&client, config, scope.as_deref(), no_scope)? {
                Some(id) => format!("/habits?scope={id}"),
                None => "/habits".to_owned(),
            };
            let habits: Vec<HabitStatus> =
                serde_json::from_value(client.call("GET", &path, None)?.json()?)?;
            let records: Vec<Value> = habits.iter().map(record).collect();
            emit(
                format,
                &records,
                &["short", "today", "progress", "streak", "title"],
            )?;
        }
        HabitCommand::Show { id, days } => {
            let id = resolve(&client, &id)?;
            let habit = fetch(&client, id, Some(days))?;
            emit(
                format,
                &[shown(&habit)],
                &[
                    "short", "today", "progress", "streak", "longest", "recent", "marks", "title",
                ],
            )?;
        }
        HabitCommand::Done { id, day, amount } => {
            if amount == Some(0) {
                bail!("an amount is at least 1");
            }
            let id = resolve(&client, &id)?;
            let today = fetch(&client, id, None)?.today;
            let day = day_of(day.as_deref(), today)?;
            enter(
                &client,
                format,
                id,
                day,
                HabitEntryStatus::Done,
                amount,
                None,
            )?;
        }
        HabitCommand::Fail { id, day } => {
            let id = resolve(&client, &id)?;
            let today = fetch(&client, id, None)?.today;
            let day = day_of(day.as_deref(), today)?;
            enter(&client, format, id, day, HabitEntryStatus::Fail, None, None)?;
        }
        HabitCommand::Pause { id, from, until } => {
            let id = resolve(&client, &id)?;
            let today = fetch(&client, id, None)?.today;
            let from = day_of(from.as_deref(), today)?;
            let until = match until {
                Some(until) => Some(day_any(&until, today)?),
                None => None,
            };
            enter(
                &client,
                format,
                id,
                from,
                HabitEntryStatus::Pause,
                None,
                until,
            )?;
        }
        HabitCommand::Entries { id, days } => {
            let id = resolve(&client, &id)?;
            let habit = fetch(&client, id, Some(days))?;
            let mut entries = habit.entries.clone();
            entries.reverse();
            let records: Vec<Value> = entries
                .iter()
                .map(|e| {
                    json!({
                        "id": e.id,
                        "short": short(&e.id),
                        "day": e.day.to_string(),
                        "status": word(e.status),
                        "amount": e.amount,
                        "until": e.until.map(|d| d.to_string()),
                        "by": e.created_by.as_ref().map(|a| a.device.clone()),
                        "entry": e,
                    })
                })
                .collect();
            emit(
                format,
                &records,
                &["short", "day", "status", "amount", "until", "by"],
            )?;
        }
        HabitCommand::Undo { entry } => {
            let id = resolve(&client, &entry)?;
            write(
                &client,
                &[Change::Delete {
                    object: id,
                    hlc: placeholder(),
                }],
            )?;
        }
        HabitCommand::Set { id, goal } => {
            let id = resolve(&client, &id)?;
            let goal = goal.trim().trim_start_matches("habit:");
            if goal.eq_ignore_ascii_case("none") {
                set_field(&client, id, "habit", Value::Null)?;
                return Ok(());
            }
            // The short form is read where the quick capture is read, in
            // samu-core, as `samu edit <id> habit:…` (ADR 0091 §5).
            let request = CaptureRequest {
                text: format!("habit:{goal}"),
                project: None,
                parent: None,
                scope: None,
                template: None,
                task: Some(id),
                preview: false,
            };
            let answer = client.call("POST", "/capture", Some(&serde_json::to_value(&request)?))?;
            let edited: CaptureResponse = serde_json::from_value(answer.json()?)?;
            if edited.task.habit.is_none() {
                bail!("{goal:?}: no habit I know (1/d, 3/w, 8/d:Glas, avoid)");
            }
            print_capture(format, &edited, true, &["short", "title"])?;
        }
    }
    Ok(())
}

/// `GET /habits/{id}`, with the history and entries of the last `days`.
pub(crate) fn fetch(client: &Client, id: Id, days: Option<u32>) -> Result<HabitStatus> {
    let mut path = format!("/habits/{id}");
    if let Some(days) = days {
        // The server's history starts where it was asked, so count back
        // from the account's today, not this computer's.
        let today = crate::report::account_today(client)?;
        let from = today - i64::from(days.max(1) - 1).days();
        path = format!("{path}?from={from}");
    }
    Ok(serde_json::from_value(
        client.call("GET", &path, None)?.json()?,
    )?)
}

fn enter(
    client: &Client,
    format: Format,
    habit: Id,
    day: Date,
    status: HabitEntryStatus,
    amount: Option<u32>,
    until: Option<Date>,
) -> Result<()> {
    let now = jiff::Timestamp::now();
    let entry = HabitEntry {
        id: Id::new(),
        created: now,
        modified: now,
        created_by: None,
        habit,
        day,
        status,
        amount,
        until,
    };
    write(
        client,
        &[Change::Create {
            object: entry.id,
            kind: ObjectKind::HabitEntry,
            value: serde_json::to_value(&entry)?,
            hlc: placeholder(),
        }],
    )?;
    let after = fetch(client, habit, None)?;
    let mut r = record(&after);
    r["entry"] = json!(entry.id);
    emit(
        format,
        &[r],
        &["short", "today", "progress", "streak", "title"],
    )
}

/// A day as typed, not after `today`.
fn day_of(text: Option<&str>, today: Date) -> Result<Date> {
    let Some(text) = text else {
        return Ok(today);
    };
    let day = day_any(text, today)?;
    if day > today {
        bail!("{day} is after today; an entry is for today or a day before");
    }
    Ok(day)
}

/// `YYYY-MM-DD`, `heute`/`today`, `gestern`/`yesterday`, `-N`.
fn day_any(text: &str, today: Date) -> Result<Date> {
    let text = text.trim().to_lowercase();
    match text.as_str() {
        "heute" | "today" => return Ok(today),
        "gestern" | "yesterday" => return Ok(today - 1.day()),
        _ => {}
    }
    if let Some(back) = text.strip_prefix('-')
        && let Ok(n) = back.parse::<i64>()
    {
        return Ok(today - n.days());
    }
    text.parse::<Date>()
        .map_err(|_| anyhow::anyhow!("{text:?}: a day is YYYY-MM-DD, heute, gestern or -N"))
}

fn word<T: serde::Serialize>(value: T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".into())
}

/// How far the running period is, like `5/8 Glas today`, `2/3 this week`
/// or `free 12 d`.
fn progress(h: &HabitStatus) -> String {
    let Some(habit) = &h.task.habit else {
        return String::new();
    };
    if habit.kind == HabitKind::Avoid {
        return format!("free {} d", h.streak);
    }
    let per = match habit.per {
        HabitPer::Day => "today",
        HabitPer::Week => "this week",
        HabitPer::Month => "this month",
        HabitPer::Unknown => "?",
    };
    let unit = habit
        .unit
        .as_deref()
        .map(|u| format!(" {u}"))
        .unwrap_or_default();
    format!("{}/{}{unit} {per}", h.period.amount, h.period.target)
}

pub(crate) fn record(h: &HabitStatus) -> Value {
    json!({
        "id": h.task.id,
        "short": short(&h.task.id),
        "title": h.task.title,
        "today": word(h.day),
        "progress": progress(h),
        "streak": h.streak,
        "longest": h.longest,
        "recent": format!("{}/{}", h.recent.met, h.recent.of),
        "year": format!("{}/{}", h.year.met, h.year.of),
        "last_fail": h.last_fail.map(|d| d.to_string()),
        "status": h,
    })
}

/// A habit as `samu habit show` shows it: with its history as marks.
pub(crate) fn shown(h: &HabitStatus) -> Value {
    let mut r = record(h);
    r["marks"] = json!(marks(h));
    r
}

/// The history as one mark per day, oldest first: `#` done, `+` partial,
/// `x` fail, `=` pause, `.` nothing.
fn marks(h: &HabitStatus) -> String {
    h.history
        .iter()
        .map(|d| match d.state {
            HabitDay::Done => '#',
            HabitDay::Partial => '+',
            HabitDay::Fail => 'x',
            HabitDay::Pause => '=',
            HabitDay::None | HabitDay::Unknown => '.',
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests may unwrap")]

    use jiff::civil::date;

    use super::*;

    #[test]
    fn days_as_typed() {
        let today = date(2026, 11, 13);
        assert_eq!(day_of(None, today).unwrap(), today);
        assert_eq!(day_of(Some("gestern"), today).unwrap(), date(2026, 11, 12));
        assert_eq!(day_of(Some("-3"), today).unwrap(), date(2026, 11, 10));
        assert_eq!(
            day_of(Some("2026-11-01"), today).unwrap(),
            date(2026, 11, 1)
        );
        assert!(day_of(Some("morgen"), today).is_err());
        assert!(day_of(Some("2026-11-14"), today).is_err());
        // A pause may end after today.
        assert_eq!(day_any("2026-11-20", today).unwrap(), date(2026, 11, 20));
    }
}
