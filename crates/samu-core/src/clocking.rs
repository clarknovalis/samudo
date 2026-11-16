// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Clocking (ADR 0046): one running clock per account, a clock stops with
//! its task, and the report.
//!
//! Pure functions over clocks and a snapshot. The store applies the first
//! two in the transaction of a change; the phone applies them to its outbox
//! offline; the report is the one `GET /clocks/report` answers with.

use std::collections::HashMap;

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};

use crate::api::{Clock, ClockReport, ClockReportBy, ClockReportRow, Id, Period};
use crate::availability::Snapshot;

/// A running clock that has to end, and when.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ending {
    pub clock: Id,
    pub end: Timestamp,
}

/// The newer running clock wins (ADR 0046 §2): of the running clocks, the
/// one with the latest start keeps running, the larger id on a tie; every
/// other one ends where the next newer one begins, so none overlap.
#[must_use]
pub fn one_running(clocks: &[Clock]) -> Vec<Ending> {
    let mut running: Vec<&Clock> = clocks.iter().filter(|c| c.end.is_none()).collect();
    running.sort_by_key(|c| (c.start, c.id));
    running
        .windows(2)
        .map(|pair| Ending {
            clock: pair[0].id,
            end: pair[1].start,
        })
        .collect()
}

/// Completing or dropping `owner` at `at` ends a running clock on it there
/// (ADR 0046 §3); never before the clock began.
#[must_use]
pub fn stop_on_close(clocks: &[Clock], owner: Id, at: Timestamp) -> Vec<Ending> {
    clocks
        .iter()
        .filter(|c| c.owner == owner && c.end.is_none())
        .map(|c| Ending {
            clock: c.id,
            end: at.max(c.start),
        })
        .collect()
}

/// The running clock, if any; with several (before the server has decided),
/// the one [`one_running`] keeps.
#[must_use]
pub fn running(clocks: &[Clock]) -> Option<&Clock> {
    clocks
        .iter()
        .filter(|c| c.end.is_none())
        .max_by_key(|c| (c.start, c.id))
}

/// Seconds clocked on `owner`, a running clock counted up to `now`.
#[must_use]
pub fn total(clocks: &[Clock], owner: Id, now: Timestamp) -> i64 {
    clocks
        .iter()
        .filter(|c| c.owner == owner)
        .map(|c| seconds(c.start, c.end.unwrap_or(now)))
        .sum()
}

fn seconds(from: Timestamp, to: Timestamp) -> i64 {
    (to.as_second() - from.as_second()).max(0)
}

/// Midnight starting `day` in `zone`; a day whose midnight falls in a gap
/// begins at the first instant there is.
fn day_start(day: Date, zone: &TimeZone) -> Option<Timestamp> {
    day.to_zoned(zone.clone()).ok().map(|z| z.timestamp())
}

/// The report over the days `from` to `to` (both included) in `zone`
/// (ADR 0046 §5). A clock counts with its part in the span, split at the
/// zone's midnights; a running one counts up to `now`. `keep` decides which
/// clocks count at all (a scope).
#[must_use]
pub fn report(
    snapshot: &Snapshot<'_>,
    from: Date,
    to: Date,
    by: ClockReportBy,
    zone: &TimeZone,
    now: Timestamp,
    keep: impl Fn(&Clock) -> bool,
) -> ClockReport {
    let mut out = ClockReport {
        from,
        to,
        by,
        seconds: 0,
        rows: Vec::new(),
    };
    let (Some(span_start), Some(span_end)) = (
        day_start(from, zone),
        to.checked_add(1.day())
            .ok()
            .and_then(|d| day_start(d, zone)),
    ) else {
        return out;
    };
    let tasks: HashMap<Id, _> = snapshot.tasks.iter().map(|t| (t.id, t)).collect();
    let projects: HashMap<Id, _> = snapshot.projects.iter().map(|p| (p.id, p)).collect();
    let folders: HashMap<Id, _> = snapshot.folders.iter().map(|f| (f.id, f)).collect();
    let tags: HashMap<Id, _> = snapshot.tags.iter().map(|t| (t.id, t)).collect();

    // Key: the id, or the day; seconds per key, in the order first seen.
    let mut sums: HashMap<(Option<Id>, Option<Date>), i64> = HashMap::new();
    for clock in snapshot.clocks.iter().filter(|c| keep(c)) {
        let start = clock.start.max(span_start);
        let end = clock.end.unwrap_or(now).min(span_end);
        if end <= start {
            continue;
        }
        out.seconds += seconds(start, end);
        if by == ClockReportBy::Day {
            let mut day = start.to_zoned(zone.clone()).date();
            while day <= to {
                let Some(next) = day.checked_add(1.day()).ok() else {
                    break;
                };
                let (Some(a), Some(b)) = (day_start(day, zone), day_start(next, zone)) else {
                    break;
                };
                let part = seconds(start.max(a), end.min(b));
                if part > 0 {
                    *sums.entry((None, Some(day))).or_default() += part;
                }
                if b >= end {
                    break;
                }
                day = next;
            }
            continue;
        }
        let part = seconds(start, end);
        let task = tasks.get(&clock.owner);
        let project = match task {
            Some(task) => crate::availability::home(task, &projects),
            None => projects.contains_key(&clock.owner).then_some(clock.owner),
        };
        let keys: Vec<Option<Id>> = match by {
            ClockReportBy::Task => vec![Some(clock.owner)],
            ClockReportBy::Project => vec![project],
            ClockReportBy::Folder => {
                vec![
                    project
                        .and_then(|p| projects.get(&p))
                        .and_then(|p| p.folder),
                ]
            }
            ClockReportBy::Tag => {
                let on = match task {
                    Some(task) => &task.tags,
                    None => match projects.get(&clock.owner) {
                        Some(p) => &p.tags,
                        None => &Vec::new(),
                    },
                };
                if on.is_empty() {
                    vec![None]
                } else {
                    on.iter().map(|g| Some(g.tag)).collect()
                }
            }
            ClockReportBy::Day | ClockReportBy::Unknown => vec![None],
        };
        for key in keys {
            *sums.entry((key, None)).or_default() += part;
        }
    }

    let title = |id: Id| -> String {
        if let Some(t) = tasks.get(&id) {
            return t.title.clone();
        }
        if let Some(p) = projects.get(&id) {
            return p.title.clone();
        }
        if let Some(f) = folders.get(&id) {
            return f.title.clone();
        }
        tags.get(&id).map(|t| t.title.clone()).unwrap_or_default()
    };
    let estimate = |id: Id| -> Option<Period> {
        if by != ClockReportBy::Task {
            return None;
        }
        tasks
            .get(&id)
            .and_then(|t| t.estimate)
            .or_else(|| projects.get(&id).and_then(|p| p.estimate))
    };
    out.rows = sums
        .into_iter()
        .map(|((id, date), seconds)| ClockReportRow {
            id,
            date,
            title: id.map(title).unwrap_or_default(),
            seconds,
            estimate: id.and_then(estimate),
        })
        .collect();
    if by == ClockReportBy::Day {
        out.rows.sort_by_key(|r| r.date);
    } else {
        // Longest first; the row without an object last among equals.
        out.rows.sort_by(|a, b| {
            b.seconds
                .cmp(&a.seconds)
                .then(a.id.is_none().cmp(&b.id.is_none()))
                .then_with(|| a.title.cmp(&b.title))
                .then(a.id.cmp(&b.id))
        });
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests may unwrap")]
mod tests {
    use super::*;
    use crate::api::{Folder, Project, Tagging, Task};

    fn id(n: u8) -> Id {
        format!("01936f5e-8a00-7c3e-9b1a-3f2d4c5b6c{n:02x}")
            .parse()
            .unwrap()
    }

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn clock(n: u8, owner: u8, start: &str, end: Option<&str>) -> Clock {
        Clock {
            id: id(n),
            created: at(start),
            modified: at(start),
            owner: id(owner),
            start: at(start),
            end: end.map(at),
            note: None,
        }
    }

    fn task(n: u8, fields: serde_json::Value) -> Task {
        let mut json = serde_json::json!({
            "id": id(n), "created": "2026-11-01T00:00:00Z",
            "modified": "2026-11-01T00:00:00Z", "title": format!("task {n}"),
        });
        if let (Some(base), Some(extra)) = (json.as_object_mut(), fields.as_object()) {
            base.extend(extra.clone());
        }
        serde_json::from_value(json).unwrap()
    }

    fn berlin() -> TimeZone {
        TimeZone::get("Europe/Berlin").unwrap()
    }

    fn day(text: &str) -> Date {
        text.parse().unwrap()
    }

    #[test]
    fn the_newer_running_clock_ends_the_older() {
        let old = clock(1, 10, "2026-11-09T08:00:00Z", None);
        let new = clock(2, 11, "2026-11-09T09:00:00Z", None);
        let done = clock(3, 12, "2026-11-09T07:00:00Z", Some("2026-11-09T07:30:00Z"));
        let want = vec![Ending {
            clock: id(1),
            end: at("2026-11-09T09:00:00Z"),
        }];
        // Whichever arrives last: the outcome is the same.
        assert_eq!(one_running(&[old.clone(), new.clone(), done.clone()]), want);
        assert_eq!(one_running(&[new.clone(), done, old.clone()]), want);
        assert_eq!(running(&[old.clone(), new.clone()]), Some(&new));
        // One running clock: nothing ends.
        assert!(one_running(&[old]).is_empty());
    }

    #[test]
    fn on_the_same_start_the_larger_id_wins_and_three_chain() {
        let a = clock(1, 10, "2026-11-09T08:00:00Z", None);
        let b = clock(2, 10, "2026-11-09T08:00:00Z", None);
        assert_eq!(
            one_running(&[b.clone(), a.clone()]),
            vec![Ending {
                clock: id(1),
                end: at("2026-11-09T08:00:00Z")
            }]
        );
        let c = clock(3, 10, "2026-11-09T10:00:00Z", None);
        let endings = one_running(&[c, a, b]);
        assert_eq!(endings.len(), 2);
        assert_eq!(endings[1].clock, id(2));
        assert_eq!(endings[1].end, at("2026-11-09T10:00:00Z"));
    }

    #[test]
    fn closing_a_task_stops_its_clock_and_no_other() {
        let mine = clock(1, 10, "2026-11-09T08:00:00Z", None);
        let other = clock(2, 11, "2026-11-09T08:00:00Z", None);
        let stopped = clock(3, 10, "2026-11-09T06:00:00Z", Some("2026-11-09T07:00:00Z"));
        let endings = stop_on_close(&[mine, other, stopped], id(10), at("2026-11-09T09:00:00Z"));
        assert_eq!(
            endings,
            vec![Ending {
                clock: id(1),
                end: at("2026-11-09T09:00:00Z")
            }]
        );
        // Completed before the clock began (another device's clock): never
        // an end before the start.
        let late = clock(4, 10, "2026-11-09T10:00:00Z", None);
        assert_eq!(
            stop_on_close(&[late], id(10), at("2026-11-09T09:00:00Z"))[0].end,
            at("2026-11-09T10:00:00Z")
        );
    }

    #[test]
    fn a_clock_over_midnight_splits_in_the_zone() {
        // 23:00 to 01:00 in Berlin (UTC+1 in November).
        let clocks = [clock(
            1,
            10,
            "2026-11-09T22:00:00Z",
            Some("2026-11-10T00:00:00Z"),
        )];
        let tasks = [task(10, serde_json::json!({}))];
        let snapshot = Snapshot {
            tasks: &tasks,
            clocks: &clocks,
            ..Snapshot::default()
        };
        let now = at("2026-11-12T00:00:00Z");
        let r = report(
            &snapshot,
            day("2026-11-09"),
            day("2026-11-10"),
            ClockReportBy::Day,
            &berlin(),
            now,
            |_| true,
        );
        assert_eq!(r.seconds, 7200);
        let days: Vec<_> = r
            .rows
            .iter()
            .map(|r| (r.date.unwrap(), r.seconds))
            .collect();
        assert_eq!(
            days,
            vec![(day("2026-11-09"), 3600), (day("2026-11-10"), 3600)]
        );
        // The span cuts: only the 10th counts the hour after midnight.
        let r = report(
            &snapshot,
            day("2026-11-10"),
            day("2026-11-10"),
            ClockReportBy::Task,
            &berlin(),
            now,
            |_| true,
        );
        assert_eq!(r.seconds, 3600);
        assert_eq!(r.rows[0].id, Some(id(10)));
        assert_eq!(r.rows[0].title, "task 10");
    }

    #[test]
    fn the_day_the_clocks_go_back_has_twenty_five_hours() {
        // 25.10.2026: Berlin goes from UTC+2 to UTC+1. A clock over the
        // whole day, midnight to midnight, is 25 hours on that one day.
        let clocks = [clock(
            1,
            10,
            "2026-10-24T22:00:00Z",
            Some("2026-10-25T23:00:00Z"),
        )];
        let snapshot = Snapshot {
            clocks: &clocks,
            ..Snapshot::default()
        };
        let r = report(
            &snapshot,
            day("2026-10-24"),
            day("2026-10-26"),
            ClockReportBy::Day,
            &berlin(),
            at("2026-11-01T00:00:00Z"),
            |_| true,
        );
        let days: Vec<_> = r
            .rows
            .iter()
            .map(|r| (r.date.unwrap(), r.seconds))
            .collect();
        assert_eq!(days, vec![(day("2026-10-25"), 25 * 3600)]);
    }

    #[test]
    fn a_running_clock_counts_up_to_now() {
        let clocks = [clock(1, 10, "2026-11-09T08:00:00Z", None)];
        let snapshot = Snapshot {
            clocks: &clocks,
            ..Snapshot::default()
        };
        let r = report(
            &snapshot,
            day("2026-11-09"),
            day("2026-11-09"),
            ClockReportBy::Task,
            &berlin(),
            at("2026-11-09T08:20:00Z"),
            |_| true,
        );
        assert_eq!(r.seconds, 1200);
        assert_eq!(total(&clocks, id(10), at("2026-11-09T08:20:00Z")), 1200);
    }

    #[test]
    fn projects_folders_and_tags_group_and_the_total_counts_once() {
        let folders: Vec<Folder> = vec![
            serde_json::from_value(serde_json::json!({
                "id": id(30), "created": "2026-11-01T00:00:00Z",
                "modified": "2026-11-01T00:00:00Z", "title": "Arbeit",
                "status": "active",
            }))
            .unwrap(),
        ];
        let projects: Vec<Project> = vec![
            serde_json::from_value(serde_json::json!({
                "id": id(20), "created": "2026-11-01T00:00:00Z",
                "modified": "2026-11-01T00:00:00Z", "title": "Steuer",
                "folder": id(30), "estimate": "PT2H",
                "kind": "parallel", "status": "active",
            }))
            .unwrap(),
        ];
        let tagging = |n: u8| Tagging {
            tag: id(n),
            position: 0,
        };
        let tasks = [
            task(
                10,
                serde_json::json!({"project": id(20), "estimate": "PT30M",
                    "tags": [tagging(40), tagging(41)]}),
            ),
            task(11, serde_json::json!({})),
        ];
        let clocks = [
            clock(1, 10, "2026-11-09T08:00:00Z", Some("2026-11-09T09:00:00Z")),
            clock(2, 11, "2026-11-09T10:00:00Z", Some("2026-11-09T10:30:00Z")),
            clock(3, 20, "2026-11-09T11:00:00Z", Some("2026-11-09T11:15:00Z")),
        ];
        let snapshot = Snapshot {
            folders: &folders,
            projects: &projects,
            tasks: &tasks,
            clocks: &clocks,
            ..Snapshot::default()
        };
        let run = |by| {
            report(
                &snapshot,
                day("2026-11-09"),
                day("2026-11-09"),
                by,
                &berlin(),
                at("2026-11-10T00:00:00Z"),
                |_| true,
            )
        };
        let rows = |r: ClockReport| -> Vec<(Option<Id>, i64)> {
            r.rows.into_iter().map(|r| (r.id, r.seconds)).collect()
        };

        let by_task = run(ClockReportBy::Task);
        assert_eq!(by_task.seconds, 6300);
        assert_eq!(
            by_task.rows[0].estimate,
            serde_json::from_value(serde_json::json!("PT30M")).unwrap()
        );
        assert_eq!(
            by_task.rows[2].estimate,
            serde_json::from_value(serde_json::json!("PT2H")).unwrap()
        );
        assert_eq!(
            rows(by_task),
            vec![
                (Some(id(10)), 3600),
                (Some(id(11)), 1800),
                (Some(id(20)), 900)
            ]
        );
        assert_eq!(
            rows(run(ClockReportBy::Project)),
            vec![(Some(id(20)), 4500), (None, 1800)]
        );
        let by_folder = run(ClockReportBy::Folder);
        assert_eq!(by_folder.rows[0].title, "Arbeit");
        assert_eq!(rows(by_folder), vec![(Some(id(30)), 4500), (None, 1800)]);
        // Two tags: the hour counts under each, the total once.
        let by_tag = run(ClockReportBy::Tag);
        assert_eq!(by_tag.seconds, 6300);
        assert_eq!(
            rows(by_tag),
            vec![(Some(id(40)), 3600), (Some(id(41)), 3600), (None, 2700)]
        );
        // A scope keeps what it keeps.
        let kept = report(
            &snapshot,
            day("2026-11-09"),
            day("2026-11-09"),
            ClockReportBy::Task,
            &berlin(),
            at("2026-11-10T00:00:00Z"),
            |c| c.owner == id(11),
        );
        assert_eq!(rows(kept), vec![(Some(id(11)), 1800)]);
    }
}
