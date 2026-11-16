// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What Forecast shows (ADR 0045), here and only here: the web UI asks the
//! server, the phone asks this module offline.
//!
//! Sections in order: „Überfällig“, today, „Markiert“, the following days up
//! to the horizon, „Später“. Each row says why it stands there. A task
//! stands at most once in a section; across sections it may stand more than
//! once, each time with its own reason (09.11.2026).

use std::collections::{HashMap, HashSet};

use jiff::Timestamp;
use jiff::ToSpan;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use samu_api::{AccountSettings, AppointmentInstance, AppointmentStatus, Id, Task, When};

use crate::availability::{DueState, Evaluation, Reason, Snapshot, TaskState};

/// The parts of the settings Forecast reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Days after today; the rest is „Später“.
    pub days: u32,
    pub flagged: bool,
    pub tag: Option<Id>,
    pub defer: bool,
    pub done: bool,
    /// Each day shows the appointments of the calendars (ADR 0047 §10).
    pub events: bool,
}

impl Default for Options {
    /// As a fresh account has them (ADR 0045 §4).
    fn default() -> Self {
        Self {
            days: 7,
            flagged: true,
            tag: None,
            defer: true,
            done: false,
            events: true,
        }
    }
}

impl From<&AccountSettings> for Options {
    fn from(s: &AccountSettings) -> Self {
        Self {
            days: s.forecast_days,
            flagged: s.forecast_flagged,
            tag: s.forecast_tag,
            defer: s.forecast_defer,
            done: s.forecast_done,
            events: s.forecast_events,
        }
    }
}

/// A section of Forecast.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Section {
    Overdue,
    Day(Date),
    Flagged,
    Later,
}

/// Why a task stands where it stands, in the order that decides when more
/// than one holds in a section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Why {
    Due,
    Planned,
    Defer,
    Tag,
    Done,
    Flagged,
}

/// One row: the task and why it is there; `date` is the date the reason
/// names (for „Später“ and „Überfällig“ the day it was or will be).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row<'a> {
    pub task: &'a Task,
    pub why: Why,
    pub date: Option<Date>,
}

/// The day a time falls on in `zone`.
fn day_of(when: Option<&When>, zone: &TimeZone) -> Option<Date> {
    match when? {
        When::Date(date) => Some(*date),
        When::At(at) => Some(at.timestamp().to_zoned(zone.clone()).date()),
        When::Unknown => None,
    }
}

fn open(task: &Task) -> bool {
    task.completed_at.is_none() && task.dropped_at.is_none()
}

/// Open, but at rest: its project is on hold, completed or dropped, a
/// folder above is dropped, or an action group above is closed or itself at
/// rest. Forecast leaves it out; nothing in it is due now (99476b72).
fn resting(task: Id, evaluation: &Evaluation) -> bool {
    let mut id = task;
    // Each step goes one group up; more steps than tasks is a cycle.
    for _ in 0..=evaluation.tasks.len() {
        let Some(state) = evaluation.tasks.get(&id) else {
            return false;
        };
        let mut above = None;
        for reason in &state.reasons {
            match reason {
                Reason::ProjectNotActive { .. } | Reason::FolderDropped { .. } => return true,
                Reason::Completed | Reason::Dropped if id != task => return true,
                Reason::ContainerUnavailable { id: group } => above = Some(*group),
                _ => {}
            }
        }
        match above {
            Some(group) => id = group,
            None => return false,
        }
    }
    false
}

/// `root` and every tag below it, stopping at a cycle.
fn tag_tree(root: Id, snapshot: &Snapshot<'_>) -> HashSet<Id> {
    let mut found = HashSet::from([root]);
    loop {
        let before = found.len();
        for tag in snapshot.tags {
            if tag.parent.is_some_and(|p| found.contains(&p)) {
                found.insert(tag.id);
            }
        }
        if found.len() == before {
            return found;
        }
    }
}

/// Forecast for `now` in `zone`, over the tasks `shown` lets through (the
/// scope, ADR 0035). Sections without rows are left out, except today.
#[must_use]
pub fn forecast<'a>(
    snapshot: &Snapshot<'a>,
    evaluation: &Evaluation,
    options: &Options,
    now: Timestamp,
    zone: &TimeZone,
    shown: impl Fn(&Task, &TaskState) -> bool,
) -> Vec<(Section, Vec<Row<'a>>)> {
    let today = now.to_zoned(zone.clone()).date();
    let horizon = horizon(today, options);
    let tagged = options.tag.map(|t| tag_tree(t, snapshot));
    let mut sections: HashMap<Section, Vec<Row<'a>>> = HashMap::new();
    let mut put = |section: Section, task: &'a Task, why: Why, date: Option<Date>| {
        let rows = sections.entry(section).or_default();
        match rows.iter_mut().find(|r| r.task.id == task.id) {
            Some(row) if why < row.why => *row = Row { task, why, date },
            Some(_) => {}
            None => rows.push(Row { task, why, date }),
        }
    };
    let place = |day: Date| {
        if day <= today {
            Section::Day(today)
        } else if day <= horizon {
            Section::Day(day)
        } else {
            Section::Later
        }
    };
    for task in snapshot.tasks {
        let Some(state) = evaluation.tasks.get(&task.id) else {
            continue;
        };
        if !shown(task, state) {
            continue;
        }
        if !open(task) {
            // Completed today, under today, when asked for.
            let done_today = task
                .completed_at
                .is_some_and(|at| at.to_zoned(zone.clone()).date() == today);
            if options.done && done_today {
                put(Section::Day(today), task, Why::Done, Some(today));
            }
            continue;
        }
        if resting(task.id, evaluation) {
            continue;
        }
        let schedule = &task.schedule;
        let due = day_of(schedule.due.as_ref(), zone);
        if let Some(day) = due {
            if state.due == DueState::Overdue || day < today {
                put(Section::Overdue, task, Why::Due, Some(day));
            } else {
                put(place(day), task, Why::Due, Some(day));
            }
        }
        if let Some(day) = day_of(schedule.planned.as_ref(), zone) {
            put(place(day), task, Why::Planned, Some(day));
        }
        if options.defer
            && let Some(day) = day_of(schedule.defer.as_ref(), zone)
            && day >= today
            && day <= horizon
        {
            put(Section::Day(day), task, Why::Defer, Some(day));
        }
        if let Some(tags) = &tagged
            && task.tags.iter().any(|t| tags.contains(&t.tag))
        {
            put(Section::Day(today), task, Why::Tag, None);
        }
        if options.flagged && task.flagged && state.available {
            put(Section::Flagged, task, Why::Flagged, None);
        }
    }
    sections.entry(Section::Day(today)).or_default();
    let mut out: Vec<(Section, Vec<Row<'a>>)> = sections.into_iter().collect();
    out.sort_by_key(|(s, _)| rank(s, today));
    for (_, rows) in &mut out {
        rows.sort_by_key(|r| (r.date, r.why, r.task.position, r.task.id));
    }
    out
}

/// The last day that is a day of its own; after it comes „Später“.
#[must_use]
pub fn horizon(today: Date, options: &Options) -> Date {
    today
        .checked_add(i64::from(options.days).days())
        .unwrap_or(today)
}

/// Overdue, today, flagged, the next days, later.
fn rank(section: &Section, today: Date) -> (u8, Option<Date>) {
    match section {
        Section::Overdue => (0, None),
        Section::Day(d) if *d == today => (1, Some(*d)),
        Section::Flagged => (2, None),
        Section::Day(d) => (3, Some(*d)),
        Section::Later => (4, None),
    }
}

/// A section of Forecast with the appointments of its day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part<'a> {
    pub section: Section,
    pub rows: Vec<Row<'a>>,
    /// Only in a day: whole days first, then by start (ADR 0047 §10).
    pub appointments: Vec<AppointmentInstance>,
}

/// Lays the occurrences over the sections (ADR 0047 §10): each on every
/// day from today to the horizon it touches in `zone`, never in
/// „Überfällig“, „Markiert“ or „Später“; a cancelled one nowhere. A day
/// with appointments and no task gets a section of its own.
#[must_use]
pub fn with_appointments<'a>(
    sections: Vec<(Section, Vec<Row<'a>>)>,
    instances: &[AppointmentInstance],
    today: Date,
    horizon: Date,
    zone: &TimeZone,
) -> Vec<Part<'a>> {
    let mut by_day: HashMap<Date, Vec<AppointmentInstance>> = HashMap::new();
    for instance in instances {
        if instance.status == AppointmentStatus::Cancelled {
            continue;
        }
        let (first, last) = crate::calendar::days(&instance.start, instance.end.as_ref(), zone);
        let mut day = first.max(today);
        while day <= last.min(horizon) {
            by_day.entry(day).or_default().push(instance.clone());
            match day.tomorrow() {
                Ok(next) => day = next,
                Err(_) => break,
            }
        }
    }
    let mut parts: Vec<Part<'a>> = sections
        .into_iter()
        .map(|(section, rows)| {
            let appointments = match section {
                Section::Day(d) => by_day.remove(&d).unwrap_or_default(),
                _ => Vec::new(),
            };
            Part {
                section,
                rows,
                appointments,
            }
        })
        .collect();
    parts.extend(by_day.into_iter().map(|(day, appointments)| Part {
        section: Section::Day(day),
        rows: Vec::new(),
        appointments,
    }));
    parts.sort_by_key(|p| rank(&p.section, today));
    for part in &mut parts {
        // Whole days first, then by start; the title breaks a tie.
        part.appointments.sort_by(|a, b| {
            let key = |i: &AppointmentInstance| match &i.start {
                When::Date(d) => (0, d.to_zoned(zone.clone()).map(|z| z.timestamp()).ok()),
                When::At(at) => (1, Some(at.timestamp())),
                When::Unknown => (2, None),
            };
            key(a).cmp(&key(b)).then_with(|| a.title.cmp(&b.title))
        });
    }
    parts
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests may unwrap")]
mod tests {
    use super::*;
    use crate::availability::{Settings, evaluate};

    fn id(n: u8) -> Id {
        format!("01936f5e-8a00-7c3e-9b1a-3f2d4c5b6f{n:02x}")
            .parse()
            .unwrap()
    }

    fn task(n: u8, fields: serde_json::Value) -> Task {
        let mut value = serde_json::json!({
            "id": id(n),
            "created": "2026-11-01T10:00:00Z",
            "modified": "2026-11-01T10:00:00Z",
            "title": format!("t{n}"),
            "flagged": false,
            "sequential": false,
            "complete_with_last_child": false,
            "position": i64::from(n),
        });
        for (k, v) in fields.as_object().unwrap() {
            value[k] = v.clone();
        }
        serde_json::from_value(value).unwrap()
    }

    fn berlin() -> TimeZone {
        TimeZone::get("Europe/Berlin").unwrap()
    }

    // Monday 09.11.2026, noon in Berlin.
    fn now() -> Timestamp {
        "2026-11-09T11:00:00Z".parse().unwrap()
    }

    fn day(d: i8) -> Date {
        jiff::civil::date(2026, 11, d)
    }

    fn run(tasks: &[Task], options: &Options) -> Vec<(Section, Vec<(u8, Why)>)> {
        run_in(&[], &[], tasks, options)
    }

    fn run_in(
        folders: &[samu_api::Folder],
        projects: &[samu_api::Project],
        tasks: &[Task],
        options: &Options,
    ) -> Vec<(Section, Vec<(u8, Why)>)> {
        let tags = vec![
            serde_json::from_value(serde_json::json!({
                "id": id(0xe0), "created": "2026-11-01T10:00:00Z",
                "modified": "2026-11-01T10:00:00Z", "title": "Fokus",
                "status": "active", "exclusive": false, "position": 0,
            }))
            .unwrap(),
            serde_json::from_value(serde_json::json!({
                "id": id(0xe1), "created": "2026-11-01T10:00:00Z",
                "modified": "2026-11-01T10:00:00Z", "title": "Tief", "parent": id(0xe0),
                "status": "active", "exclusive": false, "position": 0,
            }))
            .unwrap(),
        ];
        let snapshot = Snapshot {
            folders,
            projects,
            tasks,
            tags: &tags,
            ..Snapshot::default()
        };
        let evaluation = evaluate(&snapshot, now(), &Settings::new(berlin()));
        forecast(&snapshot, &evaluation, options, now(), &berlin(), |_, _| {
            true
        })
        .into_iter()
        .map(|(s, rows)| {
            let ids = rows
                .iter()
                .map(|r| (r.task.id.uuid().as_bytes()[15], r.why))
                .collect();
            (s, ids)
        })
        .collect()
    }

    #[test]
    fn sections_come_in_order_and_each_row_says_why() {
        let tasks = vec![
            task(1, serde_json::json!({"due": {"date": "2026-11-01"}})),
            task(2, serde_json::json!({"due": {"date": "2026-11-09"}})),
            // Planned tomorrow, due Friday: on both days.
            task(
                3,
                serde_json::json!({"planned": {"date": "2026-11-10"}, "due": {"date": "2026-11-13"}}),
            ),
            // Flagged and due tomorrow: under Markiert and tomorrow.
            task(
                4,
                serde_json::json!({"flagged": true, "due": {"date": "2026-11-10"}}),
            ),
            // Becomes available Wednesday.
            task(5, serde_json::json!({"defer": {"date": "2026-11-11"}})),
            // Far away.
            task(6, serde_json::json!({"due": {"date": "2026-12-24"}})),
            // Planned in the past and not done: today.
            task(7, serde_json::json!({"planned": {"date": "2026-11-05"}})),
            // Nothing to do with Forecast.
            task(8, serde_json::json!({})),
        ];
        let got = run(&tasks, &Options::default());
        assert_eq!(
            got,
            vec![
                (Section::Overdue, vec![(1, Why::Due)]),
                (Section::Day(day(9)), vec![(7, Why::Planned), (2, Why::Due)]),
                (Section::Flagged, vec![(4, Why::Flagged)]),
                (
                    Section::Day(day(10)),
                    vec![(4, Why::Due), (3, Why::Planned)]
                ),
                (Section::Day(day(11)), vec![(5, Why::Defer)]),
                (Section::Day(day(13)), vec![(3, Why::Due)]),
                (Section::Later, vec![(6, Why::Due)]),
            ]
        );
    }

    #[test]
    fn the_options_switch_sections_and_reasons() {
        let tasks = vec![
            task(1, serde_json::json!({"flagged": true})),
            task(2, serde_json::json!({"defer": {"date": "2026-11-11"}})),
            task(
                3,
                serde_json::json!({"completed_at": "2026-11-09T08:00:00Z"}),
            ),
            task(
                4,
                serde_json::json!({"completed_at": "2026-11-08T08:00:00Z"}),
            ),
            task(
                5,
                serde_json::json!({"tags": [{"tag": id(0xe1), "position": 0}]}),
            ),
            task(6, serde_json::json!({"due": {"date": "2026-11-20"}})),
        ];
        // Off: flagged, defer; no tag; done off.
        let quiet = Options {
            days: 14,
            flagged: false,
            tag: None,
            defer: false,
            done: false,
            events: true,
        };
        assert_eq!(
            run(&tasks, &quiet),
            vec![
                (Section::Day(day(9)), vec![]),
                (Section::Day(day(20)), vec![(6, Why::Due)])
            ]
        );
        // On: the tag (a child counts), done today, flagged, defer; 7 days.
        let loud = Options {
            days: 7,
            flagged: true,
            tag: Some(id(0xe0)),
            defer: true,
            done: true,
            events: true,
        };
        assert_eq!(
            run(&tasks, &loud),
            vec![
                (Section::Day(day(9)), vec![(5, Why::Tag), (3, Why::Done)]),
                (Section::Flagged, vec![(1, Why::Flagged)]),
                (Section::Day(day(11)), vec![(2, Why::Defer)]),
                (Section::Later, vec![(6, Why::Due)]),
            ]
        );
    }

    fn project(n: u8, status: &str, folder: Option<Id>) -> samu_api::Project {
        serde_json::from_value(serde_json::json!({
            "id": id(n), "created": "2026-11-01T10:00:00Z",
            "modified": "2026-11-01T10:00:00Z", "title": format!("p{n}"),
            "folder": folder, "kind": "parallel", "status": status,
            "flagged": false, "position": i64::from(n),
        }))
        .unwrap()
    }

    // 99476b72: a task in a project on hold, completed or dropped, below a
    // dropped folder or in a dropped action group is not overdue, nor on a
    // day; a deferred one in an active project still is.
    #[test]
    fn tasks_at_rest_stay_out() {
        let folder: samu_api::Folder = serde_json::from_value(serde_json::json!({
            "id": id(0xf0), "created": "2026-11-01T10:00:00Z",
            "modified": "2026-11-01T10:00:00Z", "title": "f",
            "status": "dropped", "position": 0,
        }))
        .unwrap();
        let projects = vec![
            project(0xa0, "active", None),
            project(0xa1, "on_hold", None),
            project(0xa2, "dropped", None),
            project(0xa3, "completed", None),
            project(0xa4, "active", Some(id(0xf0))),
        ];
        let due = |n: u8, project: u8, more: serde_json::Value| {
            let mut fields = serde_json::json!({
                "project": id(project),
                "due": {"date": "2026-11-01"},
                "planned": {"date": "2026-11-10"},
                "flagged": true,
            });
            for (k, v) in more.as_object().unwrap() {
                fields[k] = v.clone();
            }
            task(n, fields)
        };
        let tasks = vec![
            due(1, 0xa0, serde_json::json!({})),
            due(2, 0xa1, serde_json::json!({})),
            due(3, 0xa2, serde_json::json!({})),
            due(4, 0xa3, serde_json::json!({})),
            due(5, 0xa4, serde_json::json!({})),
            // A dropped group with an open child, in the active project.
            task(
                6,
                serde_json::json!({"project": id(0xa0), "dropped_at": "2026-11-02T10:00:00Z"}),
            ),
            due(7, 0xa0, serde_json::json!({"parent": id(6)})),
            // Deferred, in the active project: still there.
            due(
                8,
                0xa0,
                serde_json::json!({"defer": {"date": "2026-11-20"}}),
            ),
        ];
        assert_eq!(
            run_in(&[folder], &projects, &tasks, &Options::default()),
            vec![
                (Section::Overdue, vec![(1, Why::Due), (8, Why::Due)]),
                (Section::Day(day(9)), vec![]),
                (Section::Flagged, vec![(1, Why::Flagged)]),
                (
                    Section::Day(day(10)),
                    vec![(1, Why::Planned), (8, Why::Planned)]
                ),
            ]
        );
    }

    fn instance(
        title: &str,
        start: &str,
        end: Option<&str>,
        status: AppointmentStatus,
    ) -> AppointmentInstance {
        let when = |s: &str| -> When {
            if s.len() == 10 {
                When::Date(s.parse().unwrap())
            } else {
                When::At(s.parse().unwrap())
            }
        };
        AppointmentInstance {
            appointment: id(0xa0),
            calendar: id(0xc0),
            title: title.into(),
            start: when(start),
            end: end.map(when),
            location: None,
            note: None,
            link: None,
            status,
            busy: true,
        }
    }

    fn titles(parts: &[Part<'_>]) -> Vec<(Section, Vec<String>)> {
        parts
            .iter()
            .map(|p| {
                (
                    p.section,
                    p.appointments.iter().map(|a| a.title.clone()).collect(),
                )
            })
            .collect()
    }

    /// ADR 0047 §10: each day from today to the horizon it touches, whole
    /// days first, a day of its own when it has no task, nothing in
    /// Überfällig, Markiert or Später, nothing cancelled.
    #[test]
    fn appointments_stand_on_each_of_their_days_up_to_the_horizon() {
        let options = Options {
            days: 3,
            ..Options::default()
        };
        let today = day(9);
        let horizon = horizon(today, &options);
        let sections = vec![
            (Section::Overdue, vec![]),
            (Section::Day(day(9)), vec![]),
            (Section::Flagged, vec![]),
            (Section::Later, vec![]),
        ];
        let instances = vec![
            // Began yesterday, ends after the horizon: today to the horizon.
            instance(
                "Reise",
                "2026-11-08",
                Some("2026-11-20"),
                AppointmentStatus::Confirmed,
            ),
            instance(
                "Zahnarzt",
                "2026-11-09T09:00:00+01:00[Europe/Berlin]",
                Some("2026-11-09T09:45:00+01:00[Europe/Berlin]"),
                AppointmentStatus::Confirmed,
            ),
            // 23:30 UTC is the next day in Berlin, and ends at its midnight.
            instance(
                "Anruf",
                "2026-11-10T23:30:00+00:00[UTC]",
                Some("2026-11-10T23:59:00+00:00[UTC]"),
                AppointmentStatus::Confirmed,
            ),
            instance("Abgesagt", "2026-11-10", None, AppointmentStatus::Cancelled),
            instance("Zu weit", "2026-11-30", None, AppointmentStatus::Confirmed),
            instance(
                "Vorbei",
                "2026-11-01",
                Some("2026-11-02"),
                AppointmentStatus::Confirmed,
            ),
        ];
        let parts = with_appointments(sections, &instances, today, horizon, &berlin());
        assert_eq!(
            titles(&parts),
            vec![
                (Section::Overdue, vec![]),
                (
                    Section::Day(day(9)),
                    vec!["Reise".into(), "Zahnarzt".into()]
                ),
                (Section::Flagged, vec![]),
                (Section::Day(day(10)), vec!["Reise".into()]),
                (Section::Day(day(11)), vec!["Reise".into(), "Anruf".into()]),
                (Section::Day(day(12)), vec!["Reise".into()]),
                (Section::Later, vec![]),
            ]
        );
    }

    #[test]
    fn days_are_counted_in_the_accounts_zone() {
        let late = When::At("2026-11-10T23:30:00+00:00[UTC]".parse().unwrap());
        assert_eq!(
            crate::calendar::days(&late, None, &berlin()),
            (day(11), day(11))
        );
        let night = When::At("2026-11-17T22:00:00+01:00[Europe/Berlin]".parse().unwrap());
        let midnight = When::At("2026-11-18T00:00:00+01:00[Europe/Berlin]".parse().unwrap());
        assert_eq!(
            crate::calendar::days(&night, Some(&midnight), &berlin()),
            (day(17), day(17))
        );
        let trip = (When::Date(day(21)), When::Date(day(24)));
        assert_eq!(
            crate::calendar::days(&trip.0, Some(&trip.1), &berlin()),
            (day(21), day(23))
        );
    }
}
