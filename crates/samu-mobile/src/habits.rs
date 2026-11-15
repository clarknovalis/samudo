//! Habits on the phone (ADR 0091, ADR 0092 §1).
//!
//! What a habit is today comes from `samu_core::habit` over the mirror with
//! the outbox laid over it, so a tap offline counts at once. An entry is an
//! ordinary object through the outbox; a wrong one is deleted, never
//! changed (ADR 0091 §2).

use std::collections::HashMap;
use std::collections::HashSet;

use jiff::civil::{Date, Weekday};
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};
use samu_api::{
    Change, HabitDay, HabitEntry, HabitEntryStatus, HabitKind, HabitStatus, Id, ObjectKind,
    ProjectStatus, Task,
};
use samu_core::availability::evaluate;
use samu_core::overlay::Objects;

use crate::{
    MobileError, Names, Samu, Section, has_tag, input, is_open, parse_id, row, tag_filter,
};

/// How many weeks the calendar on the task page reaches back (ADR 0092 §3).
const CALENDAR_WEEKS: i64 = 12;

/// What a list row shows of a habit (ADR 0092 §2).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HabitRow {
    /// `build` or `avoid`; `unknown` for one from a newer samu.
    pub kind: String,
    /// `day`, `week` or `month`.
    pub per: String,
    /// The sum of the `done` amounts in the running period.
    pub amount: u32,
    /// The goal of the period, the habit's `times`.
    pub target: u32,
    pub unit: Option<String>,
    /// Periods met in a row; for `avoid` the days free since the last
    /// relapse.
    pub streak: u32,
    /// What today is: `done`, `partial`, `fail`, `pause`, `none`.
    pub today: String,
    /// The period's goal is reached; for `avoid` no relapse in it.
    pub met: bool,
}

/// A habit for the widget "Habits" (ADR 0092 §4).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HabitLine {
    pub id: String,
    pub title: String,
    pub habit: HabitRow,
}

/// One day of the calendar.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HabitDayRow {
    /// `2026-11-13`.
    pub day: String,
    /// `done`, `partial`, `fail`, `pause`, `none`.
    pub state: String,
    pub amount: u32,
    /// After today: the calendar keeps its place, empty.
    pub future: bool,
}

/// One entry, to take back on the task page.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HabitEntryRow {
    pub id: String,
    pub day: String,
    /// `done`, `fail` or `pause`.
    pub status: String,
    pub amount: Option<u32>,
    pub until: Option<String>,
    pub created_ms: i64,
    /// Waiting in the outbox.
    pub pending: bool,
}

/// The section "Serie" of the task page (ADR 0092 §3).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct HabitDetail {
    pub row: HabitRow,
    pub longest: u32,
    /// Periods met of those counted in the last 4 weeks.
    pub recent_met: u32,
    pub recent_of: u32,
    /// The same for this calendar year.
    pub year_met: u32,
    pub year_of: u32,
    /// The running period, first and last day.
    pub period_from: String,
    pub period_to: String,
    /// Today in the account's zone.
    pub today: String,
    /// For `avoid`: the day of the last relapse.
    pub last_fail: Option<String>,
    /// From a Monday 12 weeks back to the Sunday of this week, by week.
    pub days: Vec<HabitDayRow>,
    /// The entries of these days, oldest first.
    pub entries: Vec<HabitEntryRow>,
}

/// An enum of the wire as its word, `unknown` for one from a newer samu.
fn word(value: Result<serde_json::Value, serde_json::Error>) -> String {
    value
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".into())
}

/// The status of `task` as a habit today, in `zone`; none for a task that
/// is no habit.
pub(crate) fn status_of(
    task: &Task,
    objects: &Objects,
    zone: &TimeZone,
    now: Timestamp,
    from: Option<Date>,
) -> Option<HabitStatus> {
    let today = now.to_zoned(zone.clone()).date();
    let begin = task.created.to_zoned(zone.clone()).date();
    samu_core::habit::status(task, begin, &objects.habit_entries, today, from)
}

pub(crate) fn habit_row(status: &HabitStatus) -> Option<HabitRow> {
    let habit = status.task.habit.as_ref()?;
    Some(HabitRow {
        kind: word(serde_json::to_value(habit.kind)),
        per: word(serde_json::to_value(habit.per)),
        amount: status.period.amount,
        target: status.period.target,
        unit: habit.unit.clone(),
        streak: status.streak,
        today: word(serde_json::to_value(status.day)),
        met: status.period.met,
    })
}

/// The row of `task` if it is a habit.
pub(crate) fn row_of(
    task: &Task,
    objects: &Objects,
    zone: &TimeZone,
    now: Timestamp,
) -> Option<HabitRow> {
    task.habit.as_ref()?;
    habit_row(&status_of(task, objects, zone, now, None)?)
}

fn entry_status(text: &str) -> Result<HabitEntryStatus, MobileError> {
    match text {
        "done" => Ok(HabitEntryStatus::Done),
        "fail" => Ok(HabitEntryStatus::Fail),
        "pause" => Ok(HabitEntryStatus::Pause),
        other => Err(input(format!(
            "no entry status {other:?}: done, fail or pause"
        ))),
    }
}

fn parse_day(text: &str) -> Result<Date, MobileError> {
    text.parse()
        .map_err(|e| input(format!("no day {text:?}: {e}")))
}

/// The open habits in the order of their projects and positions: the
/// inbox first, then the projects as the tree has them.
fn ordered(objects: &Objects) -> Vec<&Task> {
    let mut places = Vec::new();
    crate::folders(objects, None, 0, true, &mut HashSet::new(), &mut places);
    let rank: HashMap<Id, usize> = places
        .iter()
        .filter(|(_, _, kind, _)| *kind == "project")
        .enumerate()
        .map(|(i, (id, _, _, _))| (*id, i + 1))
        .collect();
    let closed: HashSet<Id> = objects
        .projects
        .iter()
        .filter(|p| matches!(p.status, ProjectStatus::Completed | ProjectStatus::Dropped))
        .map(|p| p.id)
        .collect();
    let mut tasks: Vec<&Task> = objects
        .tasks
        .iter()
        .filter(|t| t.habit.is_some() && is_open(t))
        .filter(|t| t.project.is_none_or(|p| !closed.contains(&p)))
        .collect();
    tasks.sort_by_key(|t| {
        let project = t
            .project
            .map_or(0, |p| rank.get(&p).copied().unwrap_or(usize::MAX));
        (project, t.position, t.id)
    });
    tasks
}

#[uniffi::export]
impl Samu {
    /// Every open habit for the widget "Habits", without a scope: habits
    /// are mostly `scope_only`, and the widget is their own place
    /// (ADR 0092 §4).
    pub fn habits(&self) -> Result<Vec<HabitLine>, MobileError> {
        let cache = self.cache()?;
        let (settings, _) = Self::settings(&cache)?;
        let (objects, _) = Self::view(&cache)?;
        let now = Timestamp::now();
        Ok(ordered(&objects)
            .into_iter()
            .filter_map(|t| {
                Some(HabitLine {
                    id: t.id.to_string(),
                    title: t.title.clone(),
                    habit: row_of(t, &objects, &settings.zone, now)?,
                })
            })
            .collect())
    }

    /// Habits in the bar (ADR 0102 §1): every open habit as a list row,
    /// without a scope like [`Samu::habits`], by project in the tree's
    /// order; with a tag only those carrying it.
    pub fn habit_list(&self, tag: Option<String>) -> Result<Vec<Section>, MobileError> {
        let cache = self.cache()?;
        let (settings, _) = Self::settings(&cache)?;
        let (objects, touched) = Self::view(&cache)?;
        let evaluation = evaluate(&objects.snapshot(), Timestamp::now(), &settings);
        let names = Names::new(&objects);
        let wanted = tag_filter(tag.as_deref(), &objects.tags)?;
        let mut sections: Vec<Section> = Vec::new();
        for task in ordered(&objects) {
            if !has_tag(task, wanted.as_ref()) {
                continue;
            }
            let row = row(
                task,
                0,
                &objects,
                &evaluation,
                &names,
                &touched,
                &settings.zone,
            );
            let home = task.project.map(|p| p.to_string());
            match sections.last_mut() {
                Some(section) if section.project_id == home => section.rows.push(row),
                _ => {
                    let project = task
                        .project
                        .and_then(|p| objects.projects.iter().find(|x| x.id == p));
                    sections.push(Section {
                        project_id: home,
                        title: project.map(|p| p.title.clone()),
                        folder: project.and_then(|p| p.folder).map(|f| names.folder_path(f)),
                        rows: vec![row],
                    });
                }
            }
        }
        Ok(sections)
    }

    /// One habit for the section "Serie": the figures, a calendar of 12
    /// weeks from a Monday, and the entries of those days (ADR 0092 §3).
    /// Also for a completed or dropped habit.
    pub fn habit(&self, id: String) -> Result<HabitDetail, MobileError> {
        let id = parse_id(&id)?;
        let cache = self.cache()?;
        let (settings, _) = Self::settings(&cache)?;
        let (objects, touched) = Self::view(&cache)?;
        let now = Timestamp::now();
        let today = now.to_zoned(settings.zone.clone()).date();
        let monday = today
            .checked_sub(i64::from(today.weekday().to_monday_zero_offset()).days())
            .unwrap_or(today);
        let from = monday
            .checked_sub((CALENDAR_WEEKS - 1).weeks())
            .unwrap_or(monday);
        let task = objects
            .tasks
            .iter()
            .find(|t| t.id == id)
            .ok_or_else(|| input(format!("no task {id}")))?;
        let status = status_of(task, &objects, &settings.zone, now, Some(from))
            .ok_or_else(|| input(format!("{id} is no habit")))?;
        let row = habit_row(&status).ok_or_else(|| input(format!("{id} is no habit")))?;
        let mut days: Vec<HabitDayRow> = status
            .history
            .iter()
            .map(|d| HabitDayRow {
                day: d.day.to_string(),
                state: word(serde_json::to_value(d.state)),
                amount: d.amount,
                future: false,
            })
            .collect();
        // The rest of this week, so that every column has seven days.
        let mut day = today;
        while day.weekday() != Weekday::Sunday {
            let Ok(next) = day.tomorrow() else { break };
            day = next;
            days.push(HabitDayRow {
                day: day.to_string(),
                state: word(serde_json::to_value(HabitDay::None)),
                amount: 0,
                future: true,
            });
        }
        let entries = status
            .entries
            .iter()
            .map(|e| HabitEntryRow {
                id: e.id.to_string(),
                day: e.day.to_string(),
                status: word(serde_json::to_value(e.status)),
                amount: e.amount,
                until: e.until.map(|u| u.to_string()),
                created_ms: e.created.as_millisecond(),
                pending: touched.contains(&e.id),
            })
            .collect();
        Ok(HabitDetail {
            row,
            longest: status.longest,
            recent_met: status.recent.met,
            recent_of: status.recent.of,
            year_met: status.year.met,
            year_of: status.year.of,
            period_from: status.period.from.to_string(),
            period_to: status.period.to.to_string(),
            today: today.to_string(),
            last_fail: status.last_fail.map(|d| d.to_string()),
            days,
            entries,
        })
    }

    /// A new entry through the outbox: `done`, `fail` or `pause` for `day`
    /// (today without one, never a later day) with `amount` for `done`.
    /// Never completes the habit (ADR 0092 §2). Returns the entry's id.
    /// A habit to avoid takes no `done` here; that is [`Self::free_from`].
    pub fn enter_habit(
        &self,
        id: String,
        status: String,
        day: Option<String>,
        amount: Option<u32>,
    ) -> Result<String, MobileError> {
        let status = entry_status(&status)?;
        self.enter(&id, status, day.as_deref(), amount, Purpose::Entry)
    }

    /// "Frei ab hier" for a habit to avoid: a `done` entry on `day`, never a
    /// later day. A day before the habit's beginning moves the beginning
    /// back, so "frei seit" counts from there (ADR 0091 §2). Only on the day
    /// sheet, never as a tap in the row or the widget. Returns the entry's
    /// id.
    pub fn free_from(&self, id: String, day: String) -> Result<String, MobileError> {
        self.enter(
            &id,
            HabitEntryStatus::Done,
            Some(&day),
            None,
            Purpose::FreeFrom,
        )
    }

    /// Takes an entry back: it is deleted, as the server does with a wrong
    /// tap (ADR 0091 §2).
    pub fn remove_habit_entry(&self, id: String) -> Result<(), MobileError> {
        let id = parse_id(&id)?;
        self.queue(|objects, stamp| {
            if !objects.habit_entries.iter().any(|e| e.id == id) {
                return Err(input(format!("no habit entry {id}")));
            }
            Ok(vec![Change::Delete {
                object: id,
                hlc: stamp(),
            }])
        })
    }
}

/// What an entry is meant as: a `done` for a habit to avoid is only the
/// explicit "frei ab hier", so that nobody ticks one off by accident.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Entry,
    FreeFrom,
}

impl Samu {
    fn enter(
        &self,
        id: &str,
        status: HabitEntryStatus,
        day: Option<&str>,
        amount: Option<u32>,
        purpose: Purpose,
    ) -> Result<String, MobileError> {
        let habit = parse_id(id)?;
        let today = {
            let cache = self.cache()?;
            let (settings, _) = Self::settings(&cache)?;
            Timestamp::now().to_zoned(settings.zone.clone()).date()
        };
        let day = day.map(parse_day).transpose()?.unwrap_or(today);
        if day > today {
            return Err(input(format!(
                "{day} is after today; an entry is for today or a day before"
            )));
        }
        let amount = match (status, amount) {
            (HabitEntryStatus::Done, Some(0)) => {
                return Err(input("an amount is at least 1"));
            }
            (HabitEntryStatus::Done, a) => a,
            (_, Some(_)) => return Err(input("an amount goes only with done")),
            (_, None) => None,
        };
        let now = Timestamp::now();
        let entry = HabitEntry {
            id: Id::new(),
            created: now,
            modified: now,
            created_by: None,
            habit,
            day,
            status,
            amount,
            until: None,
        };
        let value = serde_json::to_value(&entry).map_err(|e| input(e.to_string()))?;
        self.queue(|objects, stamp| {
            let task = objects
                .tasks
                .iter()
                .find(|t| t.id == habit)
                .ok_or_else(|| input(format!("no task {habit}")))?;
            let Some(h) = task.habit.as_ref() else {
                return Err(input(format!("{habit} is no habit")));
            };
            let avoid = h.kind == HabitKind::Avoid;
            match purpose {
                Purpose::Entry if avoid && status == HabitEntryStatus::Done => {
                    return Err(input(
                        "a habit to avoid takes only a relapse or a pause; \
                         \"frei ab hier\" is free_from",
                    ));
                }
                Purpose::FreeFrom if !avoid => {
                    return Err(input(format!(
                        "{habit} is no habit to avoid; \"frei ab hier\" is only for one"
                    )));
                }
                _ => {}
            }
            Ok(vec![Change::Create {
                object: entry.id,
                kind: ObjectKind::HabitEntry,
                value,
                hlc: stamp(),
            }])
        })?;
        Ok(entry.id.to_string())
    }
}
