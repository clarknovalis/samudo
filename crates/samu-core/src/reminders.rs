// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! When tasks and projects remind (ADR 0030).
//!
//! An open task or project with a `due` reminds at it; a `reminder` object
//! reminds at its own instant or a length of time before a date. A date
//! without a time stands for the account's `remind_time` that day, in the
//! account's zone. A `daily` reminder goes off every day at its time,
//! for a habit only while there is something to do (ADR 0093). This is the
//! one place that decides; a phone asks it which alarm to set next and what
//! to show when it goes off.

pub mod appointments;

use std::collections::{BTreeSet, HashMap};

use jiff::{Span, Timestamp, Zoned};
use samu_api::{
    Alert, Appointment, Calendar, HabitEntry, Id, Project, ProjectStatus, Reminder, ReminderAnchor,
    Schedule, Task, When,
};

use crate::availability::{Settings, Snapshot};

/// One reminder that goes off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alarm {
    /// The task or project.
    pub owner: Id,
    pub at: Timestamp,
    pub title: String,
}

/// How many days back from `until` a `daily` reminder is looked at, so a
/// window over all time cannot run for ever.
const DAILY_DAYS: i64 = 400;

/// Every reminder that goes off after `after` and up to `until`, by time
/// and then owner. Several on the same owner at the same instant are one.
/// `entries` are the habit entries: a `daily` reminder on a habit keeps
/// quiet on a day that needs nothing more (ADR 0093 §3).
#[must_use]
pub fn due(
    snapshot: &Snapshot<'_>,
    reminders: &[Reminder],
    entries: &[HabitEntry],
    settings: &Settings,
    after: Timestamp,
    until: Timestamp,
) -> Vec<Alarm> {
    let owners = Owners::new(snapshot);
    let mut found: BTreeSet<(Timestamp, Id)> = BTreeSet::new();
    let mut keep = |owner: Id, at: Option<Timestamp>| {
        if let Some(at) = at.filter(|at| after < *at && *at <= until) {
            found.insert((at, owner));
        }
    };
    for (id, owner) in &owners.open {
        keep(
            *id,
            owner
                .schedule()
                .due
                .as_ref()
                .and_then(|d| instant(d, settings)),
        );
    }
    for reminder in reminders {
        let Some(owner) = owners.open.get(&reminder.owner) else {
            continue;
        };
        if reminder.anchor == ReminderAnchor::Daily {
            for at in daily(reminder, *owner, entries, settings, after, until) {
                keep(reminder.owner, Some(at));
            }
            continue;
        }
        keep(
            reminder.owner,
            goes_off(reminder, owner.schedule(), settings),
        );
    }
    found
        .into_iter()
        .filter_map(|(at, id)| {
            owners.open.get(&id).map(|owner| Alarm {
                owner: id,
                at,
                title: owner.title().to_owned(),
            })
        })
        .collect()
}

/// What [`alerts`] is asked for (ADR 0101 §4): the kinds wanted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wanted {
    pub tasks: bool,
    pub appointments: bool,
}

/// The one list of reminders that go off after `after` and up to `until`
/// (ADR 0101 §4, §5): those of tasks and projects as [`due`] finds them,
/// and those of appointments as [`appointments::due`] does, by time and
/// then title.
#[must_use]
#[allow(clippy::too_many_arguments, reason = "the parts of one account")]
pub fn alerts(
    snapshot: &Snapshot<'_>,
    reminders: &[Reminder],
    entries: &[HabitEntry],
    appointments: &[Appointment],
    calendars: &[Calendar],
    settings: &Settings,
    (after, until): (Timestamp, Timestamp),
    wanted: Wanted,
) -> Vec<Alert> {
    let mut out: Vec<Alert> = Vec::new();
    if wanted.tasks {
        out.extend(
            due(snapshot, reminders, entries, settings, after, until)
                .into_iter()
                .map(|alarm| Alert {
                    at: alarm.at,
                    title: alarm.title,
                    owner: Some(alarm.owner),
                    appointment: None,
                    calendar: None,
                    start: None,
                    end: None,
                    location: None,
                }),
        );
    }
    if wanted.appointments {
        out.extend(appointments::due(
            appointments,
            calendars,
            settings,
            after,
            until,
        ));
    }
    out.sort_by(|x, y| x.at.cmp(&y.at).then_with(|| x.title.cmp(&y.title)));
    out
}

/// The first reminder after `after`, looking at most `horizon` ahead.
#[must_use]
pub fn next(
    snapshot: &Snapshot<'_>,
    reminders: &[Reminder],
    entries: &[HabitEntry],
    settings: &Settings,
    after: Timestamp,
    horizon: Span,
) -> Option<Alarm> {
    let until = after
        .to_zoned(settings.zone.clone())
        .checked_add(horizon)
        .map_or(Timestamp::MAX, |z| z.timestamp());
    due(snapshot, reminders, entries, settings, after, until)
        .into_iter()
        .next()
}

/// The instants a `daily` reminder goes off at after `after` and up to
/// `until`: its time on every day, but on a habit only the days with
/// something to do (ADR 0093 §3).
fn daily(
    reminder: &Reminder,
    owner: Owner<'_>,
    entries: &[HabitEntry],
    settings: &Settings,
    after: Timestamp,
    until: Timestamp,
) -> Vec<Timestamp> {
    let Some(time) = reminder.time else {
        return Vec::new();
    };
    let zone = &settings.zone;
    let last = until.to_zoned(zone.clone()).date();
    let first = after.to_zoned(zone.clone()).date().max(
        last.checked_sub(Span::new().days(DAILY_DAYS))
            .unwrap_or(jiff::civil::Date::MIN),
    );
    let mut found = Vec::new();
    let mut day = first;
    while day <= last {
        let at = day
            .to_datetime(time)
            .to_zoned(zone.clone())
            .ok()
            .map(|z| z.timestamp());
        if let Some(at) = at.filter(|at| after < *at && *at <= until) {
            let quiet = match owner {
                Owner::Task(task) => {
                    let begin = task.created.to_zoned(zone.clone()).date();
                    !crate::habit::to_do(task, begin, entries, day)
                }
                Owner::Project(_) => false,
            };
            if !quiet {
                found.push(at);
            }
        }
        match day.tomorrow() {
            Ok(next) => day = next,
            Err(_) => break,
        }
    }
    found
}

/// When one reminder goes off; none when the date it is relative to is not
/// set, its anchor is unknown, or it is `daily` (it has no one instant;
/// `due` lists each day).
#[must_use]
pub fn goes_off(
    reminder: &Reminder,
    schedule: &Schedule,
    settings: &Settings,
) -> Option<Timestamp> {
    let anchor = match reminder.anchor {
        ReminderAnchor::At => reminder.at?.to_zoned(settings.zone.clone()),
        ReminderAnchor::Due => zoned(schedule.due.as_ref()?, settings)?,
        ReminderAnchor::Planned => zoned(schedule.planned.as_ref()?, settings)?,
        ReminderAnchor::Defer => zoned(schedule.defer.as_ref()?, settings)?,
        ReminderAnchor::Daily | ReminderAnchor::Unknown => return None,
    };
    match reminder.before {
        Some(before) => anchor.checked_sub(before.0).ok().map(|z| z.timestamp()),
        None => Some(anchor.timestamp()),
    }
}

/// The instant a date stands for: its own, or `remind_time` on that day.
#[must_use]
pub fn instant(when: &When, settings: &Settings) -> Option<Timestamp> {
    zoned(when, settings).map(|z| z.timestamp())
}

fn zoned(when: &When, settings: &Settings) -> Option<Zoned> {
    match when {
        When::Date(date) => date
            .to_datetime(settings.remind_time)
            .to_zoned(settings.zone.clone())
            .ok(),
        When::At(at) => Some(at.clone()),
        When::Unknown => None,
    }
}

/// What can remind: a task or a project.
#[derive(Clone, Copy)]
enum Owner<'a> {
    Task(&'a Task),
    Project(&'a Project),
}

impl Owner<'_> {
    fn schedule(&self) -> &Schedule {
        match self {
            Self::Task(t) => &t.schedule,
            Self::Project(p) => &p.schedule,
        }
    }

    fn title(&self) -> &str {
        match self {
            Self::Task(t) => &t.title,
            Self::Project(p) => &p.title,
        }
    }
}

/// The open tasks and projects by id. A task is open when neither it, an
/// action group above it, nor its project is completed or dropped.
struct Owners<'a> {
    open: HashMap<Id, Owner<'a>>,
}

impl<'a> Owners<'a> {
    fn new(snapshot: &Snapshot<'a>) -> Self {
        let projects: HashMap<Id, &Project> = snapshot.projects.iter().map(|p| (p.id, p)).collect();
        let tasks: HashMap<Id, &Task> = snapshot.tasks.iter().map(|t| (t.id, t)).collect();
        let mut open = HashMap::new();
        for project in snapshot.projects {
            if project_open(project) {
                open.insert(project.id, Owner::Project(project));
            }
        }
        for task in snapshot.tasks {
            if task_open(task, &tasks, &projects) {
                open.insert(task.id, Owner::Task(task));
            }
        }
        Self { open }
    }
}

fn project_open(project: &Project) -> bool {
    project.completed_at.is_none()
        && project.dropped_at.is_none()
        && !matches!(
            project.status,
            ProjectStatus::Completed | ProjectStatus::Dropped
        )
}

fn task_open(task: &Task, tasks: &HashMap<Id, &Task>, projects: &HashMap<Id, &Project>) -> bool {
    let mut current = task;
    // At most as many steps as there are tasks, so a cycle cannot hang it.
    for _ in 0..=tasks.len() {
        if current.completed_at.is_some() || current.dropped_at.is_some() {
            return false;
        }
        if current
            .project
            .and_then(|p| projects.get(&p))
            .is_some_and(|p| !project_open(p))
        {
            return false;
        }
        match current.parent.and_then(|p| tasks.get(&p)) {
            Some(parent) => current = parent,
            None => return true,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use jiff::ToSpan;
    use jiff::civil::date;
    use jiff::tz::TimeZone;
    use samu_api::{Folder, Link, Period, ProjectKind, Tag};

    use super::*;

    fn id(n: u8) -> Id {
        format!("01936f5e-8a00-7c3e-9b1a-3f2d4c5b6c{n:02x}")
            .parse()
            .unwrap()
    }

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    fn settings() -> Settings {
        Settings::new(TimeZone::get("Europe/Berlin").unwrap())
    }

    fn task(n: u8, title: &str) -> Task {
        Task {
            created_by: None,
            habit: None,
            id: id(n),
            created: ts("2026-11-01T08:00:00Z"),
            modified: ts("2026-11-01T08:00:00Z"),
            title: title.into(),
            note: None,
            project: None,
            parent: None,
            schedule: Schedule::default(),
            flagged: false,
            estimate: None,
            priority: None,
            completed_at: None,
            dropped_at: None,
            repeat_of: None,
            sequential: false,
            complete_with_last_child: false,
            tags: Vec::new(),
            properties: Default::default(),
            position: 0,
        }
    }

    fn project(n: u8, title: &str, status: ProjectStatus) -> Project {
        Project {
            scope_only: false,
            own_scope: None,
            icon: None,
            created_by: None,
            id: id(n),
            created: ts("2026-11-01T08:00:00Z"),
            modified: ts("2026-11-01T08:00:00Z"),
            title: title.into(),
            note: None,
            folder: None,
            workflow: None,
            kind: ProjectKind::Parallel,
            status,
            schedule: Schedule::default(),
            review_interval: None,
            next_review: None,
            last_reviewed: None,
            flagged: false,
            estimate: None,
            completed_at: None,
            dropped_at: None,
            tags: Vec::new(),
            properties: Default::default(),
            position: 0,
        }
    }

    fn reminder(n: u8, owner: u8, anchor: ReminderAnchor, before: Option<Span>) -> Reminder {
        Reminder {
            id: id(n),
            created: ts("2026-11-01T08:00:00Z"),
            modified: ts("2026-11-01T08:00:00Z"),
            owner: id(owner),
            anchor,
            at: None,
            before: before.map(Period),
            time: None,
        }
    }

    fn at(s: &str) -> When {
        When::At(s.parse().unwrap())
    }

    fn run(
        projects: &[Project],
        tasks: &[Task],
        reminders: &[Reminder],
        after: &str,
        until: &str,
    ) -> Vec<(Id, Timestamp)> {
        let folders: [Folder; 0] = [];
        let tags: [Tag; 0] = [];
        let links: [Link; 0] = [];
        let snapshot = Snapshot {
            folders: &folders,
            projects,
            tasks,
            tags: &tags,
            links: &links,
            clocks: &[],
            scopes: &[],
        };
        due(&snapshot, reminders, &[], &settings(), ts(after), ts(until))
            .into_iter()
            .map(|a| (a.owner, a.at))
            .collect()
    }

    #[test]
    fn a_due_date_reminds_by_itself_and_a_day_at_remind_time() {
        let mut timed = task(1, "Anruf");
        timed.schedule.due = Some(at("2026-11-20T14:30:00+01:00[Europe/Berlin]"));
        let mut day = task(2, "Steuer");
        day.schedule.due = Some(When::Date(date(2026, 11, 20)));
        let found = run(
            &[],
            &[timed, day],
            &[],
            "2026-11-19T00:00:00Z",
            "2026-11-21T00:00:00Z",
        );
        // 09:00 in Berlin is 08:00 UTC in November.
        assert_eq!(
            found,
            vec![
                (id(2), ts("2026-11-20T08:00:00Z")),
                (id(1), ts("2026-11-20T13:30:00Z")),
            ]
        );
    }

    #[test]
    fn a_day_in_summer_time_is_nine_o_clock_there_too() {
        let mut day = task(1, "Sommer");
        day.schedule.due = Some(When::Date(date(2026, 7, 1)));
        let found = run(
            &[],
            &[day],
            &[],
            "2026-06-30T00:00:00Z",
            "2026-07-02T00:00:00Z",
        );
        assert_eq!(found, vec![(id(1), ts("2026-07-01T07:00:00Z"))]);
    }

    #[test]
    fn reminders_relative_to_each_date_and_at_an_instant() {
        let mut t = task(1, "Zug");
        t.schedule.due = Some(at("2026-11-20T18:00:00+01:00[Europe/Berlin]"));
        t.schedule.planned = Some(When::Date(date(2026, 11, 19)));
        t.schedule.defer = Some(When::Date(date(2026, 11, 18)));
        let mut instant = reminder(10, 1, ReminderAnchor::At, None);
        instant.at = Some(ts("2026-11-17T06:00:00Z"));
        let reminders = [
            reminder(11, 1, ReminderAnchor::Due, Some(2.hours())),
            reminder(12, 1, ReminderAnchor::Planned, None),
            reminder(13, 1, ReminderAnchor::Defer, Some(1.day())),
            instant,
        ];
        let found = run(
            &[],
            &[t],
            &reminders,
            "2026-11-16T00:00:00Z",
            "2026-11-30T00:00:00Z",
        );
        assert_eq!(
            found,
            vec![
                (id(1), ts("2026-11-17T06:00:00Z")),
                // A day before 09:00 on the 18th.
                (id(1), ts("2026-11-17T08:00:00Z")),
                (id(1), ts("2026-11-19T08:00:00Z")),
                (id(1), ts("2026-11-20T15:00:00Z")),
                // The due date by itself.
                (id(1), ts("2026-11-20T17:00:00Z")),
            ]
        );
    }

    #[test]
    fn the_same_instant_twice_is_one_alarm() {
        let mut t = task(1, "Einmal");
        t.schedule.due = Some(When::Date(date(2026, 11, 20)));
        let reminders = [reminder(10, 1, ReminderAnchor::Due, None)];
        let found = run(
            &[],
            &[t],
            &reminders,
            "2026-11-19T00:00:00Z",
            "2026-11-21T00:00:00Z",
        );
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn what_is_closed_stays_quiet() {
        let due = Some(When::Date(date(2026, 11, 20)));
        let mut done = task(1, "erledigt");
        done.schedule.due = due.clone();
        done.completed_at = Some(ts("2026-11-10T00:00:00Z"));
        let mut dropped = task(2, "verworfen");
        dropped.schedule.due = due.clone();
        dropped.dropped_at = Some(ts("2026-11-10T00:00:00Z"));
        let closed = project(3, "fertig", ProjectStatus::Completed);
        let mut in_closed = task(4, "im fertigen Projekt");
        in_closed.schedule.due = due.clone();
        in_closed.project = Some(closed.id);
        let mut group = task(5, "Gruppe");
        group.completed_at = Some(ts("2026-11-10T00:00:00Z"));
        let mut child = task(6, "Kind");
        child.schedule.due = due.clone();
        child.parent = Some(group.id);
        let mut on_hold = project(7, "ruht", ProjectStatus::OnHold);
        on_hold.schedule.due = due.clone();
        // A probe that must be found, so the empty rest means something.
        let mut open = task(8, "offen");
        open.schedule.due = due;
        let tasks = [done, dropped, in_closed, group, child, open];
        let reminders = [reminder(10, 1, ReminderAnchor::Due, Some(1.hour()))];
        let found = run(
            &[closed, on_hold],
            &tasks,
            &reminders,
            "2026-11-19T00:00:00Z",
            "2026-11-21T00:00:00Z",
        );
        let owners: Vec<Id> = found.iter().map(|(o, _)| *o).collect();
        assert_eq!(owners, vec![id(7), id(8)]);
    }

    #[test]
    fn missing_dates_and_unknown_anchors_remind_of_nothing() {
        let t = task(1, "ohne Datum");
        let reminders = [
            reminder(10, 1, ReminderAnchor::Due, None),
            reminder(11, 1, ReminderAnchor::Unknown, None),
            reminder(12, 1, ReminderAnchor::At, None),
            reminder(13, 99, ReminderAnchor::Due, None),
        ];
        let found = run(
            &[],
            &[t],
            &reminders,
            "2000-01-01T00:00:00Z",
            "2100-01-01T00:00:00Z",
        );
        assert!(found.is_empty());
    }

    #[test]
    fn the_window_is_open_at_the_start_and_closed_at_the_end() {
        let mut t = task(1, "Grenze");
        t.schedule.due = Some(at("2026-11-20T09:00:00+01:00[Europe/Berlin]"));
        let at_due = "2026-11-20T08:00:00Z";
        assert!(run(&[], &[t.clone()], &[], at_due, "2026-11-21T00:00:00Z").is_empty());
        assert_eq!(run(&[], &[t], &[], "2026-11-19T00:00:00Z", at_due).len(), 1);
    }

    #[test]
    fn next_finds_the_first_within_the_horizon() {
        let mut a = task(1, "später");
        a.schedule.due = Some(When::Date(date(2026, 11, 25)));
        let mut b = task(2, "früher");
        b.schedule.due = Some(When::Date(date(2026, 11, 21)));
        let tasks = [a, b];
        let snapshot = Snapshot {
            folders: &[],
            projects: &[],
            tasks: &tasks,
            tags: &[],
            links: &[],
            clocks: &[],
            scopes: &[],
        };
        let after = ts("2026-11-20T00:00:00Z");
        let first = next(&snapshot, &[], &[], &settings(), after, 7.days()).unwrap();
        assert_eq!((first.owner, first.title.as_str()), (id(2), "früher"));
        assert!(next(&snapshot, &[], &[], &settings(), after, 12.hours()).is_none());
    }

    fn daily_at(n: u8, owner: u8, time: &str) -> Reminder {
        Reminder {
            anchor: ReminderAnchor::Daily,
            time: Some(time.parse().unwrap()),
            ..reminder(n, owner, ReminderAnchor::Daily, None)
        }
    }

    fn habit_task(n: u8, kind: samu_api::HabitKind, times: u32, per: samu_api::HabitPer) -> Task {
        Task {
            habit: Some(samu_api::Habit {
                kind,
                times,
                per,
                unit: None,
            }),
            ..task(n, "Habit")
        }
    }

    fn entry(
        task: &Task,
        day: jiff::civil::Date,
        status: samu_api::HabitEntryStatus,
    ) -> HabitEntry {
        HabitEntry {
            id: Id::new(),
            created: ts("2026-11-01T08:00:00Z"),
            modified: ts("2026-11-01T08:00:00Z"),
            created_by: None,
            habit: task.id,
            day,
            status,
            amount: None,
            until: None,
        }
    }

    fn run_with(
        tasks: &[Task],
        reminders: &[Reminder],
        entries: &[HabitEntry],
        after: &str,
        until: &str,
    ) -> Vec<Timestamp> {
        let snapshot = Snapshot {
            folders: &[],
            projects: &[],
            tasks,
            tags: &[],
            links: &[],
            clocks: &[],
            scopes: &[],
        };
        due(
            &snapshot,
            reminders,
            entries,
            &settings(),
            ts(after),
            ts(until),
        )
        .into_iter()
        .map(|a| a.at)
        .collect()
    }

    /// ADR 0093 §1, §2: every day at its time in the account's zone, for a
    /// task that is no habit; nothing once it is done.
    #[test]
    fn daily_goes_off_every_day_at_its_time() {
        let tasks = [task(1, "Gießen")];
        let r = [daily_at(9, 1, "20:00")];
        assert_eq!(
            run_with(
                &tasks,
                &r,
                &[],
                "2026-11-10T00:00:00Z",
                "2026-11-12T23:00:00Z"
            ),
            vec![
                ts("2026-11-10T19:00:00Z"),
                ts("2026-11-11T19:00:00Z"),
                ts("2026-11-12T19:00:00Z"),
            ]
        );
        let mut done = task(1, "Gießen");
        done.completed_at = Some(ts("2026-11-09T10:00:00Z"));
        assert!(
            run_with(
                &[done],
                &r,
                &[],
                "2026-11-10T00:00:00Z",
                "2026-11-12T23:00:00Z"
            )
            .is_empty()
        );
        // Without its time it reminds of nothing.
        let bare = [Reminder {
            time: None,
            ..daily_at(9, 1, "20:00")
        }];
        assert!(
            run_with(
                &tasks,
                &bare,
                &[],
                "2026-11-10T00:00:00Z",
                "2026-11-12T23:00:00Z"
            )
            .is_empty()
        );
    }

    /// ADR 0093 §3: a habit keeps quiet on a day that is done, failed or
    /// paused, and an `avoid` habit never reminds.
    #[test]
    fn daily_on_a_habit_only_while_there_is_something_to_do() {
        use samu_api::{HabitEntryStatus as S, HabitKind, HabitPer};
        let h = habit_task(1, HabitKind::Build, 1, HabitPer::Day);
        let r = [daily_at(9, 1, "20:00")];
        let entries = [
            entry(&h, date(2026, 11, 10), S::Done),
            entry(&h, date(2026, 11, 11), S::Fail),
            HabitEntry {
                until: Some(date(2026, 11, 13)),
                ..entry(&h, date(2026, 11, 12), S::Pause)
            },
        ];
        assert_eq!(
            run_with(
                std::slice::from_ref(&h),
                &r,
                &entries,
                "2026-11-10T00:00:00Z",
                "2026-11-14T23:00:00Z"
            ),
            vec![ts("2026-11-14T19:00:00Z")]
        );
        let avoid = habit_task(1, HabitKind::Avoid, 1, HabitPer::Day);
        assert!(
            run_with(
                &[avoid],
                &r,
                &[],
                "2026-11-10T00:00:00Z",
                "2026-11-14T23:00:00Z"
            )
            .is_empty()
        );
    }

    /// ADR 0093 §3: with an amount below the day's goal it still reminds;
    /// with a goal per week, not once the week's goal is reached.
    #[test]
    fn daily_on_a_habit_follows_the_goal() {
        use samu_api::{HabitEntryStatus as S, HabitKind, HabitPer};
        let r = [daily_at(9, 1, "20:00")];
        let glasses = habit_task(1, HabitKind::Build, 2, HabitPer::Day);
        let one = [entry(&glasses, date(2026, 11, 10), S::Done)];
        assert_eq!(
            run_with(
                std::slice::from_ref(&glasses),
                &r,
                &one,
                "2026-11-10T00:00:00Z",
                "2026-11-10T23:00:00Z"
            ),
            vec![ts("2026-11-10T19:00:00Z")]
        );
        // Monday 9 to Sunday 15 November 2026: Monday is open, Tuesday and
        // Wednesday are done, and that is the week's goal.
        let weekly = habit_task(1, HabitKind::Build, 2, HabitPer::Week);
        let twice = [
            entry(&weekly, date(2026, 11, 10), S::Done),
            entry(&weekly, date(2026, 11, 11), S::Done),
        ];
        assert_eq!(
            run_with(
                &[weekly],
                &r,
                &twice,
                "2026-11-09T00:00:00Z",
                "2026-11-16T23:00:00Z"
            ),
            vec![ts("2026-11-09T19:00:00Z"), ts("2026-11-16T19:00:00Z")]
        );
    }
}
