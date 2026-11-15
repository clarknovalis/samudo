//! `samu bulk …`: one action on many tasks (ADR 0052 §7). Every action
//! but `delete` is one `POST /changes`; ids come as arguments or, with
//! `-`, one per line on stdin (the first word, so `samu list --format tsv`
//! pipes straight in).

use std::collections::HashSet;
use std::io::BufRead;

use anyhow::{Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use clap_complete::engine::ArgValueCompleter;
use samu_api::{Change, DeleteRequest, Id, Tag, TagStatus, Task};
use serde_json::{Value, json};

use crate::config::Config;
use crate::output::{Format, emit};
use crate::tags::{self, Snapshot, Target};
use crate::{complete, delete_object, placeholder, resolve_project, short, signed_in, write};
use samu_client::Error as Failure;

#[derive(Debug, Subcommand)]
pub(crate) enum BulkCommand {
    /// Mark the tasks completed.
    Done {
        #[command(flatten)]
        tasks: Tasks,
        /// Why, for the logbook.
        #[arg(long)]
        note: Option<String>,
    },
    /// Mark the tasks dropped.
    Drop {
        #[command(flatten)]
        tasks: Tasks,
        /// Why, for the logbook.
        #[arg(long)]
        note: Option<String>,
    },
    /// Open completed or dropped tasks again.
    Reopen(Tasks),
    /// Flag the tasks.
    Flag(Tasks),
    /// Take the flag off the tasks.
    Unflag(Tasks),
    /// Move the tasks into the inbox, a project or under an action group,
    /// each last there; their children go along. A task whose ancestor is
    /// moved as well stays under it.
    Move {
        /// `inbox`, or a project by title, the start of one, or its id.
        #[arg(long, required_unless_present = "group", add = ArgValueCompleter::new(complete::project))]
        to: Option<String>,
        /// Under this action group; its project is the target.
        #[arg(long, add = ArgValueCompleter::new(complete::task))]
        group: Option<String>,
        #[command(flatten)]
        tasks: Tasks,
    },
    /// Put tags on the tasks or take them off; not the states of a workflow.
    Tag {
        /// A tag to put on, by title or the end of its id; repeatable.
        #[arg(long, add = ArgValueCompleter::new(complete::tag_title))]
        add: Vec<String>,
        /// A tag to take off; repeatable.
        #[arg(long, add = ArgValueCompleter::new(complete::tag_title))]
        remove: Vec<String>,
        #[command(flatten)]
        tasks: Tasks,
    },
    /// Put each task into a state of its own workflow, the next one, or
    /// none. A task whose workflow lacks the state stays as it is.
    State {
        /// The state, by title or key.
        #[arg(long, required_unless_present_any = ["next", "none"], add = ArgValueCompleter::new(complete::state))]
        to: Option<String>,
        /// The next state, round to none.
        #[arg(long, conflicts_with_all = ["to", "none"])]
        next: bool,
        /// Out of every state.
        #[arg(long, conflicts_with = "to")]
        none: bool,
        /// Why, for the logbook; a state that asks for a note needs it.
        #[arg(long)]
        note: Option<String>,
        #[command(flatten)]
        tasks: Tasks,
    },
    /// Set a date of the tasks, all day (ADR 0007), or clear it.
    Date {
        field: DateField,
        /// `today`, `tomorrow`, `YYYY-MM-DD` or `none`.
        value: String,
        #[command(flatten)]
        tasks: Tasks,
    },
    /// Delete the tasks with their children (POST /objects/{id}/delete,
    /// one after another).
    Delete {
        /// Count what would go; write nothing.
        #[arg(long)]
        dry_run: bool,
        #[command(flatten)]
        tasks: Tasks,
    },
}

#[derive(Debug, Args)]
pub(crate) struct Tasks {
    /// Tasks by id, the end of one or URI; `-` reads them from stdin, the
    /// first word of each line.
    #[arg(required = true, add = ArgValueCompleter::new(complete::task))]
    ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum DateField {
    Due,
    Planned,
    Defer,
}

impl DateField {
    fn name(self) -> &'static str {
        match self {
            Self::Due => "due",
            Self::Planned => "planned",
            Self::Defer => "defer",
        }
    }
}

pub(crate) fn run(config: &Config, format: Format, command: BulkCommand) -> Result<()> {
    let client = signed_in(config)?;
    let snapshot = tags::snapshot(&client)?;
    let (verb, picked, changes) = match command {
        BulkCommand::Done { tasks, note } => {
            let picked = pick(&snapshot, &tasks)?;
            let now = json!(jiff::Timestamp::now().to_string());
            let changes = picked
                .iter()
                .filter(|t| t.completed_at.is_none())
                .map(|t| noted(t.id, "completed_at", now.clone(), note.clone()))
                .collect();
            ("completed", picked, changes)
        }
        BulkCommand::Drop { tasks, note } => {
            let picked = pick(&snapshot, &tasks)?;
            let now = json!(jiff::Timestamp::now().to_string());
            let changes = picked
                .iter()
                .filter(|t| t.dropped_at.is_none())
                .map(|t| noted(t.id, "dropped_at", now.clone(), note.clone()))
                .collect();
            ("dropped", picked, changes)
        }
        BulkCommand::Reopen(tasks) => {
            let picked = pick(&snapshot, &tasks)?;
            let mut changes = Vec::new();
            for task in &picked {
                if task.completed_at.is_some() {
                    changes.push(set(task.id, "completed_at", Value::Null));
                }
                if task.dropped_at.is_some() {
                    changes.push(set(task.id, "dropped_at", Value::Null));
                }
            }
            ("reopened", picked, changes)
        }
        BulkCommand::Flag(tasks) => flag(&snapshot, &tasks, true)?,
        BulkCommand::Unflag(tasks) => flag(&snapshot, &tasks, false)?,
        BulkCommand::Move { to, group, tasks } => {
            let picked = roots(pick(&snapshot, &tasks)?, &snapshot.tasks);
            let group = group.map(|g| find_task(&snapshot, &g)).transpose()?;
            let project = match (&to, group) {
                (_, Some(g)) => {
                    let project = g.project;
                    if let Some(to) = &to
                        && target_project(&client, to)? != project
                    {
                        bail!("the group {:?} is not in {to:?}", g.title);
                    }
                    project
                }
                (Some(to), None) => target_project(&client, to)?,
                (None, None) => bail!("give --to or --group"),
            };
            let parent = group.map(|g| g.id);
            if let Some(parent) = parent
                && picked
                    .iter()
                    .any(|t| t.id == parent || below(&snapshot.tasks, parent, t.id))
            {
                bail!("a task cannot go under itself or a task below it");
            }
            let changes = move_tasks(&snapshot.tasks, &picked, project, parent);
            ("moved", picked, changes)
        }
        BulkCommand::Tag { add, remove, tasks } => {
            if add.is_empty() && remove.is_empty() {
                bail!("give --add or --remove");
            }
            let picked = pick(&snapshot, &tasks)?;
            let adds = plain_tags(&snapshot.tags, &add)?;
            let removes = plain_tags(&snapshot.tags, &remove)?;
            let mut changes = Vec::new();
            for task in &picked {
                let has = |tag: Id| task.tags.iter().any(|t| t.tag == tag);
                for &tag in &adds {
                    if !has(tag) {
                        changes.push(Change::Add {
                            object: task.id,
                            field: "tags".into(),
                            element: tag,
                            position: 0,
                            note: None,
                            hlc: placeholder(),
                        });
                    }
                }
                for &tag in &removes {
                    if has(tag) {
                        changes.push(Change::Remove {
                            object: task.id,
                            field: "tags".into(),
                            element: tag,
                            note: None,
                            hlc: placeholder(),
                        });
                    }
                }
            }
            ("tagged", picked, changes)
        }
        BulkCommand::State {
            to,
            next,
            none,
            note,
            tasks,
        } => {
            let picked = pick(&snapshot, &tasks)?;
            let target = if next {
                Target::Next
            } else if none {
                Target::None
            } else if let Some(to) = to {
                Target::Named(to)
            } else {
                bail!("give --to, --next or --none");
            };
            let mut changes = Vec::new();
            let mut missed = 0;
            for task in &picked {
                match state_change(&snapshot, task, &target, note.as_deref())? {
                    StateChange::To(Some(change)) => changes.push(change),
                    StateChange::To(None) => {}
                    StateChange::Unknown => missed += 1,
                }
            }
            if missed > 0 {
                eprintln!("{missed} without that state in their workflow, left as they are");
            }
            ("put into the state", picked, changes)
        }
        BulkCommand::Date {
            field,
            value,
            tasks,
        } => {
            let picked = pick(&snapshot, &tasks)?;
            let value = day(&value)?;
            let changes = picked
                .iter()
                .map(|t| set(t.id, field.name(), value.clone()))
                .collect();
            ("dated", picked, changes)
        }
        BulkCommand::Delete { dry_run, tasks } => {
            let picked = roots(pick(&snapshot, &tasks)?, &snapshot.tasks);
            let mut records = Vec::new();
            let mut deleted = 0;
            for task in &picked {
                let answer = delete_object(
                    &client,
                    task.id,
                    &DeleteRequest {
                        tasks: None,
                        preview: dry_run,
                    },
                )?;
                deleted += answer.deleted;
                records.push(json!({
                    "id": task.id.to_string(),
                    "short": short(&task.id),
                    "title": task.title,
                    "deleted": answer.deleted,
                }));
            }
            let would = if dry_run { "would go" } else { "deleted" };
            eprintln!("{deleted} tasks {would}, children included");
            return emit(format, &records, &["short", "deleted", "title"]);
        }
    };
    if !changes.is_empty() {
        write(&client, &changes)?;
    }
    let touched: HashSet<Id> = changes.iter().filter_map(object_of).collect();
    eprintln!(
        "{} of {} tasks {verb}",
        picked.iter().filter(|t| touched.contains(&t.id)).count(),
        picked.len()
    );
    let records: Vec<Value> = picked
        .iter()
        .map(|t| {
            json!({
                "id": t.id.to_string(),
                "short": short(&t.id),
                "title": t.title,
                "changed": touched.contains(&t.id),
            })
        })
        .collect();
    emit(format, &records, &["short", "title"])
}

type Plan<'a> = (&'static str, Vec<&'a Task>, Vec<Change>);

fn flag<'a>(snapshot: &'a Snapshot, tasks: &Tasks, on: bool) -> Result<Plan<'a>> {
    let picked = pick(snapshot, tasks)?;
    let changes = picked
        .iter()
        .filter(|t| t.flagged != on)
        .map(|t| set(t.id, "flagged", json!(on)))
        .collect();
    Ok((if on { "flagged" } else { "unflagged" }, picked, changes))
}

fn set(object: Id, field: &str, value: Value) -> Change {
    noted(object, field, value, None)
}

fn noted(object: Id, field: &str, value: Value, note: Option<String>) -> Change {
    Change::Set {
        object,
        field: field.into(),
        value,
        note,
        hlc: placeholder(),
    }
}

fn object_of(change: &Change) -> Option<Id> {
    match change {
        Change::Set { object, .. } | Change::Add { object, .. } | Change::Remove { object, .. } => {
            Some(*object)
        }
        _ => None,
    }
}

/// The references, `-` read from stdin, as tasks of the snapshot; each once,
/// in the order given.
fn pick<'a>(snapshot: &'a Snapshot, tasks: &Tasks) -> Result<Vec<&'a Task>> {
    let mut references = Vec::new();
    for reference in &tasks.ids {
        if reference == "-" {
            for line in std::io::stdin().lock().lines() {
                if let Some(word) = line?.split_whitespace().next() {
                    references.push(word.to_string());
                }
            }
        } else {
            references.push(reference.clone());
        }
    }
    if references.is_empty() {
        bail!("no tasks given");
    }
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for reference in &references {
        let task = find_task(snapshot, reference)?;
        if seen.insert(task.id) {
            out.push(task);
        }
    }
    Ok(out)
}

/// A task by its id, URI or the end of its id.
fn find_task<'a>(snapshot: &'a Snapshot, reference: &str) -> Result<&'a Task> {
    let exact = reference
        .parse::<Id>()
        .ok()
        .or_else(|| Id::from_uri(reference));
    let wanted = reference.to_ascii_lowercase();
    let found: Vec<&Task> = snapshot
        .tasks
        .iter()
        .filter(|t| match exact {
            Some(id) => t.id == id,
            None => t.id.to_string().ends_with(&wanted),
        })
        .collect();
    match found.as_slice() {
        [task] => Ok(task),
        [] => Err(Failure::NotFound(format!("no task {reference:?}")).into()),
        many => bail!(
            "{reference:?} matches {} tasks: {}",
            many.len(),
            many.iter()
                .map(|t| t.id.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn target_project(client: &samu_client::Client, to: &str) -> Result<Option<Id>> {
    if to.eq_ignore_ascii_case("inbox") {
        Ok(None)
    } else {
        resolve_project(client, to).map(Some)
    }
}

/// Whether `id` lies somewhere below `ancestor`.
fn below(tasks: &[Task], id: Id, ancestor: Id) -> bool {
    let mut seen = HashSet::new();
    let mut at = tasks.iter().find(|t| t.id == id).and_then(|t| t.parent);
    while let Some(parent) = at {
        if parent == ancestor {
            return true;
        }
        if !seen.insert(parent) {
            return false;
        }
        at = tasks.iter().find(|t| t.id == parent).and_then(|t| t.parent);
    }
    false
}

/// The picked tasks without those whose ancestor is picked too: the
/// ancestor takes them along (ADR 0052 §3).
fn roots<'a>(picked: Vec<&'a Task>, tasks: &[Task]) -> Vec<&'a Task> {
    let ids: Vec<Id> = picked.iter().map(|t| t.id).collect();
    picked
        .into_iter()
        .filter(|t| !ids.iter().any(|&a| a != t.id && below(tasks, t.id, a)))
        .collect()
}

/// Each task last under `project` and `parent`, in the order given; the
/// descendants go along into `project` and keep their parents (as `moveTo`
/// in the web UI, ADR 0040 §3).
fn move_tasks(
    tasks: &[Task],
    picked: &[&Task],
    project: Option<Id>,
    parent: Option<Id>,
) -> Vec<Change> {
    let moving: HashSet<Id> = picked.iter().map(|t| t.id).collect();
    let first = tasks
        .iter()
        .filter(|t| !moving.contains(&t.id) && t.project == project && t.parent == parent)
        .map(|t| t.position + 1)
        .max()
        .unwrap_or(0);
    let mut changes = Vec::new();
    for (task, position) in picked.iter().zip(first..) {
        changes.push(set(
            task.id,
            "project",
            json!(project.map(|p| p.to_string())),
        ));
        changes.push(set(task.id, "parent", json!(parent.map(|p| p.to_string()))));
        changes.push(set(task.id, "position", json!(position)));
        for child in tasks.iter().filter(|t| below(tasks, t.id, task.id)) {
            if child.project != project {
                changes.push(set(
                    child.id,
                    "project",
                    json!(project.map(|p| p.to_string())),
                ));
            }
        }
    }
    changes
}

/// Tags by title or the end of their id; a state of a workflow (or the
/// workflow itself) is refused, it goes through `bulk state`.
fn plain_tags(tags: &[Tag], names: &[String]) -> Result<Vec<Id>> {
    names
        .iter()
        .map(|name| {
            let tag = tags::find_tag(tags, name)?;
            let in_workflow = tag.exclusive
                || tag
                    .parent
                    .and_then(|p| tags.iter().find(|t| t.id == p))
                    .is_some_and(|p| p.exclusive);
            if in_workflow {
                bail!(
                    "{:?} belongs to a workflow; use `samu bulk state`",
                    tag.title
                );
            }
            if tag.status == TagStatus::Dropped {
                bail!("{:?} is dropped", tag.title);
            }
            Ok(tag.id)
        })
        .collect()
}

enum StateChange {
    To(Option<Change>),
    Unknown,
}

/// As `tags::set_state`, for one task of many: a task without a workflow
/// or without the named state is `Unknown`.
fn state_change(
    snapshot: &Snapshot,
    task: &Task,
    target: &Target,
    note: Option<&str>,
) -> Result<StateChange> {
    let project = task
        .project
        .and_then(|p| snapshot.projects.iter().find(|q| q.id == p));
    let Some(group) = tags::workflow_of(project, &snapshot.tags) else {
        return Ok(StateChange::Unknown);
    };
    let states = tags::states(group, &snapshot.tags);
    let mut mine: Vec<_> = task
        .tags
        .iter()
        .filter(|t| states.iter().any(|s| s.id == t.tag))
        .collect();
    mine.sort_by_key(|t| t.position);
    let current = mine.first().map(|t| t.tag);
    let wanted = match target {
        Target::Next => tags::next_state(&states, current),
        Target::None => None,
        Target::Named(name) => {
            let found = states.iter().find(|s| {
                s.title.eq_ignore_ascii_case(name) || s.key.as_deref() == Some(name.as_str())
            });
            match found {
                Some(state) => Some(state.id),
                None => return Ok(StateChange::Unknown),
            }
        }
    };
    if let Some(w) = wanted
        && note.is_none()
        && states.iter().any(|s| s.id == w && s.ask_note)
    {
        let title = states
            .iter()
            .find(|s| s.id == w)
            .map_or("", |s| s.title.as_str());
        bail!("{title:?} asks for a note: give --note");
    }
    let note = note.map(str::to_string);
    Ok(StateChange::To(match (wanted, current) {
        (Some(new), Some(old)) if new == old => None,
        (Some(new), _) => Some(Change::Add {
            object: task.id,
            field: "tags".into(),
            element: new,
            position: 0,
            note,
            hlc: placeholder(),
        }),
        (None, Some(old)) => Some(Change::Remove {
            object: task.id,
            field: "tags".into(),
            element: old,
            note,
            hlc: placeholder(),
        }),
        (None, None) => None,
    }))
}

/// `today`, `tomorrow`, a date or `none` as the value of a date field.
fn day(value: &str) -> Result<Value> {
    let today = jiff::Zoned::now().date();
    let date = match value.to_ascii_lowercase().as_str() {
        "none" => return Ok(Value::Null),
        "today" => today,
        "tomorrow" => today.tomorrow()?,
        other => match other.parse::<jiff::civil::Date>() {
            Ok(date) => date,
            Err(_) => bail!("{value:?} is no date: today, tomorrow, YYYY-MM-DD or none"),
        },
    };
    Ok(json!({ "date": date.to_string() }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests may unwrap")]

    use super::*;

    fn task(n: u8, parent: Option<Id>) -> Task {
        let mut value = json!({
            "id": format!("01a1c000-0000-7000-8000-0000000000{n:02}"),
            "created": "2026-11-09T00:00:00Z",
            "modified": "2026-11-09T00:00:00Z",
            "title": format!("t{n}"),
            "position": n,
        });
        if let Some(parent) = parent {
            value["parent"] = json!(parent.to_string());
        }
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn an_ancestor_takes_its_picked_descendants_along() {
        let a = task(1, None);
        let b = task(2, Some(a.id));
        let c = task(3, Some(b.id));
        let d = task(4, None);
        let all = vec![a.clone(), b.clone(), c.clone(), d.clone()];
        let picked = roots(vec![&all[2], &all[0], &all[3]], &all);
        let ids: Vec<Id> = picked.iter().map(|t| t.id).collect();
        assert_eq!(ids, [a.id, d.id]);
    }

    #[test]
    fn moved_tasks_go_last_in_order_and_children_follow() {
        let a = task(1, None);
        let b = task(2, Some(a.id));
        let d = task(4, None);
        let all = vec![a.clone(), b.clone(), d.clone()];
        let project: Id = "01a1c000-0000-7000-8000-0000000000ff".parse().unwrap();
        let changes = move_tasks(&all, &[&all[2], &all[0]], Some(project), None);
        let positions: Vec<(Id, &Value)> = changes
            .iter()
            .filter_map(|c| match c {
                Change::Set {
                    object,
                    field,
                    value,
                    ..
                } if field == "position" => Some((*object, value)),
                _ => None,
            })
            .collect();
        assert_eq!(positions, [(d.id, &json!(0)), (a.id, &json!(1))]);
        let child_moves = changes
            .iter()
            .filter(|c| matches!(c, Change::Set { object, field, .. } if *object == b.id && field == "project"))
            .count();
        assert_eq!(child_moves, 1);
    }

    #[test]
    fn days_are_words_dates_or_none() {
        assert_eq!(day("none").unwrap(), Value::Null);
        assert_eq!(day("2026-11-20").unwrap(), json!({"date": "2026-11-20"}));
        assert!(day("today").unwrap()["date"].is_string());
        assert!(day("morgen").is_err());
    }
}
