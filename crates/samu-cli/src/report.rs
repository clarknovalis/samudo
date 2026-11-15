//! `samu report` (ADR 0080 §6): what was done, what was worked on and the
//! time, as `GET /report` computes it. TSV is the way into a spreadsheet.

use anyhow::{Result, anyhow, bail};
use clap::{Args, ValueEnum};
use jiff::ToSpan;
use jiff::civil::Date;
use samu_api::{Report, ReportList, ReportReason};
use serde_json::{Value, json};

use crate::clock::{By, by_name, length, report_records, titles};
use crate::config::Config;
use crate::output::{Format, emit};
use crate::{scopes, short, signed_in};

#[derive(Debug, Args)]
pub(crate) struct ReportArgs {
    /// The first day (YYYY-MM-DD); today by default.
    #[arg(long, conflicts_with = "days")]
    from: Option<String>,
    /// The last day, included; `--from` by default, today with `--days`.
    #[arg(long)]
    to: Option<String>,
    /// The last N days up to `--to` (today), today included.
    #[arg(long)]
    days: Option<u32>,
    /// A filter in the search language (`#project @tag prio:A …`), with
    /// `autor:<account>|agent|alle` (or `author:<account>|agent|all`); a
    /// person's work alone without one.
    #[arg(long)]
    q: Option<String>,
    /// Groups by task (none), project, folder, tag or day.
    #[arg(long, value_enum, default_value_t = By::Task)]
    by: By,
    /// Only these parts; all three without it.
    #[arg(long, value_enum)]
    part: Vec<Part>,
    /// Dropped tasks among the done ones, marked.
    #[arg(long)]
    dropped: bool,
    /// Through this scope instead of the one chosen here (ADR 0035).
    #[arg(long, conflicts_with = "no_scope")]
    scope: Option<String>,
    /// Without a scope, whatever is chosen here.
    #[arg(long)]
    no_scope: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Part {
    Done,
    Worked,
    Time,
}

impl Part {
    fn name(self) -> &'static str {
        match self {
            Self::Done => "done",
            Self::Worked => "worked",
            Self::Time => "time",
        }
    }
}

fn day(name: &str, text: &str) -> Result<Date> {
    text.parse()
        .map_err(|e| anyhow!("--{name} {text:?} is not a day: {e}"))
}

/// The query string: `from` and `to` always, the rest when given.
fn query(args: &ReportArgs, today: Date, scope: Option<samu_api::Id>) -> Result<String> {
    let to = args.to.as_deref().map(|t| day("to", t)).transpose()?;
    let (from, to) = match (args.days, args.from.as_deref()) {
        (Some(0), _) => bail!("--days is at least 1"),
        (Some(n), _) => {
            let to = to.unwrap_or(today);
            (to.checked_sub((i64::from(n) - 1).days())?, to)
        }
        (None, Some(from)) => {
            let from = day("from", from)?;
            (from, to.unwrap_or(from))
        }
        (None, None) => (today, to.unwrap_or(today)),
    };
    let mut parts = vec![
        format!("from={from}"),
        format!("to={to}"),
        format!("by={}", by_name(args.by.wire())),
    ];
    if let Some(q) = args.q.as_deref().filter(|q| !q.trim().is_empty()) {
        parts.push(format!("q={}", urlencoding(q)));
    }
    if !args.part.is_empty() {
        let names: Vec<&str> = args.part.iter().map(|p| p.name()).collect();
        parts.push(format!("parts={}", names.join(",")));
    }
    if args.dropped {
        parts.push("dropped=true".into());
    }
    if let Some(id) = scope {
        parts.push(format!("scope={id}"));
    }
    Ok(parts.join("&"))
}

/// Percent-encodes a query value; what is not unreserved goes as `%XX`.
fn urlencoding(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn reason_word(reason: ReportReason) -> &'static str {
    match reason {
        ReportReason::Clocked => "clocked",
        ReportReason::State => "state",
        ReportReason::Completed => "completed",
        ReportReason::Reopened => "reopened",
        ReportReason::Dropped => "dropped",
        ReportReason::Commented => "commented",
        ReportReason::ChildCompleted => "child_completed",
        ReportReason::Changed => "changed",
        ReportReason::Unknown => "unknown",
    }
}

/// One record per row; with groups one per group and row, the group
/// named.
fn list_records(
    part: &str,
    list: &ReportList,
    names: &std::collections::HashMap<samu_api::Id, String>,
) -> Vec<Value> {
    let record = |row: &samu_api::ReportRow, group: Option<String>| {
        let reasons: Vec<String> = row
            .reasons
            .iter()
            .map(|r| format!("{} {}", reason_word(r.reason), r.count))
            .collect();
        json!({
            "part": part,
            "group": group,
            "id": row.id,
            "short": short(&row.id),
            "at": row.at.to_string(),
            "what": row.title,
            "project": row.project.and_then(|p| names.get(&p)).cloned(),
            "reasons": reasons.join(", "),
            "seconds": row.seconds,
            "length": length(row.seconds),
            "dropped": row.dropped,
        })
    };
    if list.groups.is_empty() {
        return list.rows.iter().map(|r| record(r, None)).collect();
    }
    let mut out = Vec::new();
    for group in &list.groups {
        let name = match (group.date, group.id) {
            (Some(date), _) => date.to_string(),
            (None, Some(_)) if !group.title.is_empty() => group.title.clone(),
            (None, Some(id)) => short(&id),
            (None, None) => "–".into(),
        };
        for id in &group.rows {
            if let Some(row) = list.rows.iter().find(|r| r.id == *id) {
                out.push(record(row, Some(name.clone())));
            }
        }
    }
    out
}

pub(crate) fn records(
    report: &Report,
    names: &std::collections::HashMap<samu_api::Id, String>,
) -> Vec<Value> {
    let mut out = Vec::new();
    if let Some(done) = &report.done {
        out.extend(list_records("done", done, names));
    }
    if let Some(worked) = &report.worked {
        out.extend(list_records("worked", worked, names));
    }
    if let Some(time) = &report.time {
        out.extend(report_records(time).into_iter().map(|mut r| {
            if let Some(map) = r.as_object_mut() {
                map.insert("part".into(), json!("time"));
            }
            r
        }));
    }
    out
}

pub(crate) fn run(config: &Config, format: Format, args: &ReportArgs) -> Result<()> {
    let client = signed_in(config)?;
    let scope = scopes::query_scope(&client, config, args.scope.as_deref(), args.no_scope)?;
    let today = jiff::Zoned::now().date();
    let path = format!("/report?{}", query(args, today, scope)?);
    let report: Report = serde_json::from_value(client.call("GET", &path, None)?.json()?)?;
    let names = titles(&client)?;
    emit(
        format,
        &records(&report, &names),
        &["part", "group", "what", "project", "reasons", "length"],
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests may unwrap")]

    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Wrap {
        #[command(flatten)]
        args: ReportArgs,
    }

    fn args(line: &[&str]) -> ReportArgs {
        let mut all = vec!["report"];
        all.extend(line);
        Wrap::try_parse_from(all).unwrap().args
    }

    #[test]
    fn days_count_back_from_today_included() {
        let today: Date = "2026-11-12".parse().unwrap();
        assert_eq!(
            query(&args(&["--days", "7"]), today, None).unwrap(),
            "from=2026-11-06&to=2026-11-12&by=task"
        );
        assert_eq!(
            query(&args(&[]), today, None).unwrap(),
            "from=2026-11-12&to=2026-11-12&by=task"
        );
        assert!(query(&args(&["--days", "0"]), today, None).is_err());
        assert_eq!(
            query(
                &args(&[
                    "--from",
                    "2026-11-01",
                    "--to",
                    "2026-11-30",
                    "--q",
                    "#samudo autor:alle",
                    "--part",
                    "done",
                    "--part",
                    "time",
                    "--by",
                    "day",
                    "--dropped"
                ]),
                today,
                None
            )
            .unwrap(),
            "from=2026-11-01&to=2026-11-30&by=day&q=%23samudo%20autor%3Aalle&parts=done,time&dropped=true"
        );
    }

    #[test]
    fn rows_name_their_group_and_reasons() {
        let report: Report = serde_json::from_value(json!({
            "from": "2026-11-09", "to": "2026-11-15", "by": "project",
            "worked": {
                "rows": [{"id": "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6c02", "kind": "task",
                          "title": "Steuer", "at": "2026-11-12T15:30:00Z",
                          "reasons": [{"reason": "clocked", "count": 2},
                                      {"reason": "commented", "count": 1}],
                          "seconds": 3600}],
                "groups": [{"id": "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6b01", "title": "Haus",
                            "seconds": 3600, "rows": ["01936f5e-8a00-7c3e-9b1a-3f2d4c5b6c02"]}]
            }
        }))
        .unwrap();
        let records = records(&report, &std::collections::HashMap::new());
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["group"], "Haus");
        assert_eq!(records[0]["reasons"], "clocked 2, commented 1");
        assert_eq!(records[0]["length"], "1:00");
    }
}
