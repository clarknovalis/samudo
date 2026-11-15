//! Editing tasks on the phone (ADR 0040): every field the task page edits,
//! the targets and the move with the descendants, and deleting.
//!
//! Everything but deleting is an ordinary change through the outbox, so it
//! works offline. Deleting goes through `POST /objects/{id}/delete` (ADR
//! 0036 §5), which takes the descendants with it.

use std::collections::HashSet;

use jiff::{Span, Timestamp, ToSpan};
use samu_api::{Change, Id, Period, TagStatus, Task};
use samu_core::overlay::Objects;
use samu_core::repeat::{FREQS, Parts, WEEKDAYS, check, preview};
use samu_core::workflow::{current_state, states, workflow_of};
use serde_json::{Value, json};

use crate::{DeleteCount, MobileError, PlaceRow, Samu, input, parse_id, place_tree, project_meta};

/// A tag the task page can switch on or off.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TagOption {
    pub id: String,
    /// `Ort/Büro`.
    pub path: String,
    /// The task carries it.
    pub on: bool,
}

/// What the task page edits beyond the row: the tags to choose from, the
/// states of the workflow and the estimate in minutes.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TaskEdit {
    pub title: String,
    pub note: Option<String>,
    pub project_id: Option<String>,
    /// Ordinary tags: neither a workflow group nor one of its states.
    pub tags: Vec<TagOption>,
    /// The states of its workflow in order, `on` for the one it is in;
    /// empty without a workflow.
    pub states: Vec<TagOption>,
    pub estimate_minutes: Option<i64>,
    /// How long before `due` it counts as due soon, its own (cbda7d51):
    /// as written, `P3D`; none follows the account.
    pub due_warning: Option<String>,
    /// The same in whole days, when it is whole days.
    pub due_warning_days: Option<i64>,
    /// The account's default (ADR 0017 §2) in whole days, for the label.
    pub due_soon_days: Option<i64>,
    /// `A`, `B` or `C` (ADR 0056); none without one.
    pub priority: Option<String>,
    /// It has descendants that a move or a delete takes along.
    pub descendants: u32,
    /// Its repetition; none when it does not repeat.
    pub repeat: Option<RepeatEdit>,
}

/// A repetition as the task page edits it (ADR 0007 §3, 85b3799a): the
/// rule taken apart by `samu_core::repeat::Parts`, the mode, and what
/// completing it now would make.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RepeatEdit {
    /// The whole rule, for the RRULE field.
    pub rule: String,
    /// `fixed`, `due_after_completion` or `defer_after_completion`.
    pub mode: String,
    pub catch_up: bool,
    /// `DAILY`, `WEEKLY`, `MONTHLY` or `YEARLY`; none when the form cannot
    /// show the rule, and then `set_repeat` writes `rule` as it is.
    pub freq: Option<String>,
    pub interval: u32,
    /// `MO` … `SU` of a weekly rule.
    pub byday: Vec<String>,
    /// The last day, `2026-12-31`.
    pub until: Option<String>,
    pub count: Option<u32>,
    /// The parts the form does not edit, as they are: `BYMONTHDAY=-1`.
    pub rest: Vec<String>,
    /// The date that leads the next task if completed now: `due`, `defer`
    /// or `planned`; none when there is none or it ends.
    pub next_field: Option<String>,
    /// That date, `2026-11-20` or `2026-11-20 09:00`.
    pub next_date: Option<String>,
    /// The rule ends with this task: completing makes no next one.
    pub ended: bool,
}

const MODES: [&str; 3] = ["fixed", "due_after_completion", "defer_after_completion"];

fn repeat_edit(repeat: &samu_api::Repeat, next: Option<samu_api::NextRepeat>) -> RepeatEdit {
    let parts = Parts::parse(&repeat.rule);
    let fits = parts.fits();
    let mode = serde_json::to_value(repeat.mode)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    let next = next.unwrap_or_default();
    let lead = [
        ("due", &next.due),
        ("defer", &next.defer),
        ("planned", &next.planned),
    ]
    .into_iter()
    .find_map(|(field, when)| when.as_ref().map(|w| (field, crate::when_text(w))));
    RepeatEdit {
        rule: repeat.rule.clone(),
        mode,
        catch_up: repeat.catch_up,
        freq: if fits { parts.freq } else { None },
        interval: parts.interval,
        byday: parts.byday,
        until: parts.until.map(|d| d.to_string()),
        count: parts.count,
        rest: parts.rest,
        next_field: lead.as_ref().map(|(field, _)| (*field).to_owned()),
        next_date: lead.map(|(_, date)| date),
        ended: next.ended,
    }
}

/// Every task below `id`, at any depth.
pub(crate) fn descendants(objects: &Objects, id: Id) -> Vec<Id> {
    let mut found = Vec::new();
    let mut seen = HashSet::from([id]);
    let mut frontier = vec![id];
    while let Some(parent) = frontier.pop() {
        for task in objects.tasks.iter().filter(|t| t.parent == Some(parent)) {
            if seen.insert(task.id) {
                found.push(task.id);
                frontier.push(task.id);
            }
        }
    }
    found
}

pub(crate) fn find(objects: &Objects, id: Id) -> Result<&Task, MobileError> {
    objects
        .tasks
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| input(format!("no task {id}")))
}

/// The move of `id` into `project` or the inbox, at the top level and last
/// there, its descendants along (ADR 0040 §3).
pub(crate) fn move_changes(
    objects: &Objects,
    id: Id,
    project: Option<Id>,
    stamp: &mut dyn FnMut() -> samu_api::Hlc,
) -> Vec<Change> {
    let position = objects
        .tasks
        .iter()
        .filter(|t| t.id != id && t.project == project && t.parent.is_none())
        .map(|t| t.position + 1)
        .max()
        .unwrap_or(0);
    let target = project.map_or(Value::Null, |p| json!(p));
    let mut changes = vec![
        set(id, "project", target.clone(), stamp()),
        set(id, "parent", Value::Null, stamp()),
        set(id, "position", json!(position), stamp()),
    ];
    for child in descendants(objects, id) {
        changes.push(set(child, "project", target.clone(), stamp()));
    }
    changes
}

pub(crate) fn set(object: Id, field: &str, value: Value, hlc: samu_api::Hlc) -> Change {
    Change::Set {
        object,
        field: field.into(),
        value,
        note: None,
        hlc,
    }
}

/// A length of whole days, weeks counted as seven; none when it has
/// hours, minutes or months in it.
fn days_of(span: Span) -> Option<i64> {
    let finer = span.get_hours() != 0
        || span.get_minutes() != 0
        || span.get_seconds() != 0
        || span.get_milliseconds() != 0
        || span.get_microseconds() != 0
        || span.get_nanoseconds() != 0
        || span.get_months() != 0
        || span.get_years() != 0;
    (!finer).then(|| i64::from(span.get_weeks()) * 7 + i64::from(span.get_days()))
}

fn minutes_of(period: &Period) -> Option<i64> {
    period
        .0
        .total(jiff::Unit::Minute)
        .ok()
        .map(|m| m.round() as i64)
}

impl Samu {
    /// The tags under exclusive groups are states (ADR 0024); they are
    /// chosen apart from the ordinary ones.
    pub(crate) fn ordinary_tags(objects: &Objects) -> HashSet<Id> {
        let groups: HashSet<Id> = objects
            .tags
            .iter()
            .filter(|t| t.exclusive)
            .map(|t| t.id)
            .collect();
        objects
            .tags
            .iter()
            .filter(|t| {
                t.status != TagStatus::Dropped
                    && !groups.contains(&t.id)
                    && !t.parent.is_some_and(|p| groups.contains(&p))
            })
            .map(|t| t.id)
            .collect()
    }
}

#[uniffi::export]
impl Samu {
    /// What the task page edits (ADR 0040 §2).
    pub fn task_edit(&self, id: String) -> Result<TaskEdit, MobileError> {
        let id = parse_id(&id)?;
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let task = find(&objects, id)?;
        let settings = Self::settings(&cache)?.0;
        let zone = settings.zone;
        let names = crate::Names::new(&objects);
        let ordinary = Self::ordinary_tags(&objects);
        let carried: HashSet<Id> = task.tags.iter().map(|t| t.tag).collect();
        let mut tags: Vec<TagOption> = objects
            .tags
            .iter()
            .filter(|t| ordinary.contains(&t.id))
            .map(|t| TagOption {
                id: t.id.to_string(),
                path: names.tag_path(t.id),
                on: carried.contains(&t.id),
            })
            .collect();
        tags.sort_by_key(|t| t.path.to_lowercase());
        let project = task
            .project
            .and_then(|p| objects.projects.iter().find(|x| x.id == p));
        let states = workflow_of(project, &objects.tags)
            .map(|group| {
                let all = states(group, &objects.tags);
                let now = current_state(&task.tags, &all);
                all.into_iter()
                    .map(|s| TagOption {
                        id: s.to_string(),
                        path: names.tag_path(s).rsplit('/').next().unwrap_or("").into(),
                        on: Some(s) == now,
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(TaskEdit {
            title: task.title.clone(),
            note: task.note.clone(),
            project_id: task.project.map(|p| p.to_string()),
            tags,
            states,
            estimate_minutes: task.estimate.as_ref().and_then(minutes_of),
            due_warning: task.schedule.due_warning.as_ref().map(|p| p.0.to_string()),
            due_warning_days: task
                .schedule
                .due_warning
                .as_ref()
                .and_then(|p| days_of(p.0)),
            due_soon_days: days_of(settings.due_soon),
            priority: task.priority.and_then(crate::priority_text),
            descendants: u32::try_from(descendants(&objects, id).len()).unwrap_or(u32::MAX),
            repeat: task
                .schedule
                .repeat
                .as_ref()
                .map(|r| repeat_edit(r, preview(&objects.tasks, task, Timestamp::now(), &zone))),
        })
    }

    /// Sets the repetition: from the form's parts when `freq` is set,
    /// else `rule` as typed (an `RRULE:` before it is dropped). The rule
    /// is checked here as the server checks it (ADR 0033 §4); the next
    /// date fields are ignored.
    pub fn set_repeat(&self, id: String, edit: RepeatEdit) -> Result<(), MobileError> {
        let rule = match &edit.freq {
            Some(freq) => {
                if !FREQS.contains(&freq.as_str()) {
                    return Err(input(format!("{freq:?} is no frequency")));
                }
                if let Some(day) = edit.byday.iter().find(|d| !WEEKDAYS.contains(&d.as_str())) {
                    return Err(input(format!("{day:?} is no weekday")));
                }
                let until = edit
                    .until
                    .as_deref()
                    .map(|d| d.trim().parse::<jiff::civil::Date>())
                    .transpose()
                    .map_err(|_| input(format!("{:?} is no date", edit.until)))?;
                Parts {
                    freq: Some(freq.clone()),
                    interval: edit.interval.max(1),
                    byday: edit.byday.clone(),
                    until,
                    count: edit.count.filter(|n| *n > 0),
                    rest: edit.rest.clone(),
                }
                .rule()
            }
            None => {
                let typed = edit.rule.trim();
                let typed = typed
                    .get(..6)
                    .filter(|p| p.eq_ignore_ascii_case("RRULE:"))
                    .map_or(typed, |_| &typed[6..]);
                typed.to_owned()
            }
        };
        check(&rule).map_err(|e| input(e.to_string()))?;
        if !MODES.contains(&edit.mode.as_str()) {
            return Err(input(format!("{:?} is no repetition mode", edit.mode)));
        }
        let catch_up = edit.catch_up && edit.mode == "fixed";
        self.set_field(
            &id,
            "repeat",
            json!({"rule": rule, "mode": edit.mode, "catch_up": catch_up}),
        )
    }

    /// The task no longer repeats.
    pub fn clear_repeat(&self, id: String) -> Result<(), MobileError> {
        self.set_field(&id, "repeat", Value::Null)
    }

    pub fn set_title(&self, id: String, title: String) -> Result<(), MobileError> {
        let title = title.trim().to_string();
        if title.is_empty() {
            return Err(input("a task needs a title"));
        }
        self.set_field(&id, "title", json!(title))
    }

    /// An empty note takes the note away.
    pub fn set_note(&self, id: String, note: Option<String>) -> Result<(), MobileError> {
        let note = note.filter(|n| !n.trim().is_empty());
        self.set_field(&id, "note", note.map_or(Value::Null, Value::String))
    }

    /// `which` is `defer`, `due` or `planned`; the date is `2026-11-20`,
    /// whole-day (ADR 0007), none takes it away.
    pub fn set_date(
        &self,
        id: String,
        which: String,
        date: Option<String>,
    ) -> Result<(), MobileError> {
        if !matches!(which.as_str(), "defer" | "due" | "planned") {
            return Err(input(format!("no date {which:?}")));
        }
        let value = match date {
            None => Value::Null,
            Some(text) => {
                let date: jiff::civil::Date = text
                    .trim()
                    .parse()
                    .map_err(|_| input(format!("{text:?} is no date")))?;
                json!({"date": date.to_string()})
            }
        };
        self.set_field(&id, &which, value)
    }

    /// Planned for today (0) or tomorrow (1) in the account's zone.
    pub fn plan_in(&self, id: String, days: i64) -> Result<(), MobileError> {
        let zone = {
            let cache = self.cache()?;
            Self::settings(&cache)?.0.zone
        };
        let date = Timestamp::now()
            .to_zoned(zone)
            .date()
            .checked_add(days.days())
            .map_err(|e| input(e.to_string()))?;
        self.set_date(id, "planned".into(), Some(date.to_string()))
    }

    pub fn set_estimate(&self, id: String, minutes: Option<i64>) -> Result<(), MobileError> {
        let value = match minutes.filter(|m| *m > 0) {
            None => Value::Null,
            Some(m) => {
                let span: Span = m.minutes();
                serde_json::to_value(Period(span)).map_err(|e| input(e.to_string()))?
            }
        };
        self.set_field(&id, "estimate", value)
    }

    /// The task's own warning before `due` in days (cbda7d51): `0` on
    /// the day itself; none follows the account's default again.
    pub fn set_due_warning(&self, id: String, days: Option<i64>) -> Result<(), MobileError> {
        let value = match days {
            None => Value::Null,
            Some(d) if d < 0 => return Err(input(format!("{d} days is no warning"))),
            Some(d) => serde_json::to_value(Period(d.days())).map_err(|e| input(e.to_string()))?,
        };
        self.set_field(&id, "due_warning", value)
    }

    /// `A`, `B`, `C`, or none to take it away (ADR 0056).
    pub fn set_priority(&self, id: String, priority: Option<String>) -> Result<(), MobileError> {
        let value = match priority.as_deref() {
            None | Some("") => Value::Null,
            Some(p @ ("A" | "B" | "C")) => Value::String(p.to_owned()),
            Some(other) => return Err(input(format!("{other:?} is no priority: A, B or C"))),
        };
        self.set_field(&id, "priority", value)
    }

    /// Dropped (ADR 0036 keeps it, unlike delete); `reopen` undoes it.
    /// The numbers it got in the outbox, for `withdraw` (ADR 0104 §3).
    pub fn drop_task(&self, id: String) -> Result<Vec<i64>, MobileError> {
        self.close_task(&id, "dropped_at")
    }

    /// A tag on or off. A state of a workflow replaces the one before
    /// (ADR 0024 §1), here as on the server.
    pub fn set_tag(&self, id: String, tag: String, on: bool) -> Result<(), MobileError> {
        let id = parse_id(&id)?;
        let tag = parse_id(&tag)?;
        self.queue(|objects, stamp| {
            let task = find(objects, id)?;
            if !objects.tags.iter().any(|t| t.id == tag) {
                return Err(input(format!("no tag {tag}")));
            }
            let has = task.tags.iter().any(|t| t.tag == tag);
            Ok(match (on, has) {
                (true, false) => vec![Change::Add {
                    object: id,
                    field: "tags".into(),
                    element: tag,
                    position: task.tags.iter().map(|t| t.position + 1).max().unwrap_or(0),
                    note: None,
                    hlc: stamp(),
                }],
                (false, true) => vec![Change::Remove {
                    object: id,
                    field: "tags".into(),
                    element: tag,
                    note: None,
                    hlc: stamp(),
                }],
                _ => Vec::new(),
            })
        })
    }

    /// Into a state of its workflow, or out of every state.
    pub fn set_state(&self, id: String, state: Option<String>) -> Result<(), MobileError> {
        let task_id = parse_id(&id)?;
        match state {
            Some(state) => self.set_tag(id, state, true),
            None => {
                let current = {
                    let cache = self.cache()?;
                    let (objects, _) = Self::view(&cache)?;
                    let task = find(&objects, task_id)?;
                    let project = task
                        .project
                        .and_then(|p| objects.projects.iter().find(|x| x.id == p));
                    workflow_of(project, &objects.tags)
                        .and_then(|group| current_state(&task.tags, &states(group, &objects.tags)))
                };
                match current {
                    Some(current) => self.set_tag(id, current.to_string(), false),
                    None => Ok(()),
                }
            }
        }
    }

    /// Where a task can move (ADR 0040 §3): open folders and projects as a
    /// tree; the app puts the inbox on top.
    pub fn move_targets(&self) -> Result<Vec<PlaceRow>, MobileError> {
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        Ok(place_tree(&objects, false)
            .into_iter()
            .filter(|(_, _, kind, _)| matches!(*kind, "folder" | "project"))
            .map(|(id, title, kind, depth)| PlaceRow {
                id: id.to_string(),
                title,
                kind: kind.into(),
                depth,
                meta: objects
                    .projects
                    .iter()
                    .find(|p| p.id == id)
                    .map(|p| project_meta(p, self.ui_lang()))
                    .unwrap_or_default(),
                closed: false,
                scope_only: crate::is_scope_only(&objects, id),
                own_scope_title: crate::own_scope_title(&objects, id),
            })
            .collect())
    }

    /// Into a project or the inbox (none), at the top level and last there.
    /// Its descendants go along and keep their parents (ADR 0040 §3).
    pub fn move_task(&self, id: String, project: Option<String>) -> Result<(), MobileError> {
        let id = parse_id(&id)?;
        let project = project.as_deref().map(parse_id).transpose()?;
        self.queue(|objects, stamp| {
            find(objects, id)?;
            if let Some(p) = project
                && !objects.projects.iter().any(|x| x.id == p)
            {
                return Err(input(format!("no project {p}")));
            }
            Ok(move_changes(objects, id, project, stamp))
        })
    }

    /// Deletes a task with its descendants through the server (ADR 0036
    /// §5); with `preview` only counts. Needs the server.
    pub fn delete_task(&self, id: String, preview: bool) -> Result<DeleteCount, MobileError> {
        {
            let task = parse_id(&id)?;
            let cache = self.cache()?;
            let (objects, _) = Self::view(&cache)?;
            find(&objects, task)?;
        }
        self.delete_place(id, None, None, preview)
    }
}
