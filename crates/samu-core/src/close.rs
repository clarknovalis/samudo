// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Completing or dropping a task with open tasks below it (ADR 0128).
//!
//! An open task under a closed one is unavailable and leaves every view
//! (`container_unavailable`), though nobody decided it was done. So the
//! clients ask first: close the open ones along, lift them one level up,
//! or leave it. The server reads the account, asks [`plan`] for the
//! changes and writes them in one transaction, like a delete (ADR 0036).

use std::collections::{HashMap, HashSet};

use jiff::Timestamp;
use serde_json::Value;

use crate::api::{Change, ChildFate, CloseField, CloseRequest, Id, Task};
use crate::availability::Snapshot;
use crate::delete::placeholder;

/// The changes, in the order to apply them, and what they amount to.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    /// None when it waits for a choice or the task was closed already.
    pub changes: Vec<Change>,
    /// Open tasks below, at any depth.
    pub open: u64,
    /// Tasks closed, the task itself among them.
    pub closed: u64,
    /// Open direct children moved one level up.
    pub lifted: u64,
}

/// Why nothing is closed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CloseError {
    #[error("no task {0}")]
    NoTask(Id),
    #[error("a field this samu does not know")]
    UnknownField,
    #[error("a choice for the open tasks below this samu does not know")]
    UnknownFate,
}

fn is_open(task: &Task) -> bool {
    task.completed_at.is_none() && task.dropped_at.is_none()
}

/// What closing task `id` at `now` takes, as `request` says.
///
/// A task closed already, either way, gives no changes, like a second
/// delete. With [`ChildFate::Ask`] and open tasks below, no changes
/// either: [`Plan::open`] tells the client to ask.
///
/// # Errors
///
/// When `id` is no task, or the request names a field or a choice from a
/// newer samu.
pub fn plan(
    snapshot: &Snapshot<'_>,
    id: Id,
    request: &CloseRequest,
    now: Timestamp,
) -> Result<Plan, CloseError> {
    let field = match request.field {
        CloseField::Completed => "completed_at",
        CloseField::Dropped => "dropped_at",
        CloseField::Unknown => return Err(CloseError::UnknownField),
    };
    if request.children == ChildFate::Unknown {
        return Err(CloseError::UnknownFate);
    }
    let task = snapshot
        .tasks
        .iter()
        .find(|t| t.id == id)
        .ok_or(CloseError::NoTask(id))?;
    if !is_open(task) {
        return Ok(Plan::default());
    }

    let mut children: HashMap<Id, Vec<&Task>> = HashMap::new();
    for t in snapshot.tasks {
        if let Some(parent) = t.parent {
            children.entry(parent).or_default().push(t);
        }
    }
    for list in children.values_mut() {
        list.sort_by_key(|t| (t.position, t.id));
    }

    // Every open one below, through closed ones too: they are hidden all
    // the same. Depth first, in order, so the logbook reads top down.
    let mut below: Vec<&Task> = Vec::new();
    let mut seen: HashSet<Id> = HashSet::from([id]);
    let mut stack: Vec<Id> = vec![id];
    while let Some(current) = stack.pop() {
        let list = children.get(&current).cloned().unwrap_or_default();
        for t in list.iter().rev() {
            // A cycle in broken data stops here instead of running forever.
            if seen.insert(t.id) {
                stack.push(t.id);
            }
        }
        if current != id
            && let Some(t) = snapshot.tasks.iter().find(|t| t.id == current)
            && is_open(t)
        {
            below.push(t);
        }
    }
    let open = below.len() as u64;
    let mut plan = Plan {
        open,
        ..Plan::default()
    };
    if open > 0 && request.children == ChildFate::Ask {
        return Ok(plan);
    }

    let at = Value::String(now.to_string());
    if open > 0 && request.children == ChildFate::Close {
        for t in &below {
            set(&mut plan.changes, t.id, field, at.clone(), None);
            plan.closed += 1;
        }
    }
    if open > 0 && request.children == ChildFate::Lift {
        lift(snapshot, task, &children, &mut plan);
    }
    set(&mut plan.changes, id, field, at, request.note.clone());
    plan.closed += 1;
    Ok(plan)
}

/// §2: the open direct children right after the task, in their order, at
/// the task's level; the siblings after it move down to make room. After
/// means in drawn order, position then id: siblings often share a
/// position (quick capture puts each at 0), and one sharing the task's
/// but drawn after it must still come after the children. So those after
/// are numbered on from the last child.
fn lift(snapshot: &Snapshot<'_>, task: &Task, children: &HashMap<Id, Vec<&Task>>, plan: &mut Plan) {
    let lifted: Vec<&Task> = children
        .get(&task.id)
        .map(|list| list.iter().copied().filter(|t| is_open(t)).collect())
        .unwrap_or_default();
    let count = lifted.len() as i64;
    let parent = task
        .parent
        .map_or(Value::Null, |p| Value::String(p.to_string()));
    let mut siblings: Vec<&Task> = snapshot
        .tasks
        .iter()
        .filter(|t| {
            t.id != task.id
                && t.parent == task.parent
                && t.project == task.project
                && (t.position, t.id) > (task.position, task.id)
        })
        .collect();
    siblings.sort_by_key(|t| (t.position, t.id));
    let mut next = task.position.saturating_add(count);
    for sibling in siblings {
        next = next.saturating_add(1).max(sibling.position);
        if next != sibling.position {
            set(
                &mut plan.changes,
                sibling.id,
                "position",
                Value::from(next),
                None,
            );
        }
    }
    let mut position = task.position;
    for child in lifted {
        position = position.saturating_add(1);
        set(&mut plan.changes, child.id, "parent", parent.clone(), None);
        set(
            &mut plan.changes,
            child.id,
            "position",
            Value::from(position),
            None,
        );
        plan.lifted += 1;
    }
}

fn set(changes: &mut Vec<Change>, object: Id, field: &str, value: Value, note: Option<String>) {
    changes.push(Change::Set {
        object,
        field: field.into(),
        value,
        note,
        hlc: placeholder(),
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::*;

    fn ts() -> Timestamp {
        "2026-11-18T08:00:00Z".parse().unwrap()
    }

    fn id(n: u128) -> Id {
        format!("01936f5e-8a00-7c3e-9b1a-{n:012x}").parse().unwrap()
    }

    fn task(n: u128, project: Option<u128>, parent: Option<u128>, position: i64) -> Task {
        let json = serde_json::json!({
            "id": id(n), "created": ts(), "modified": ts(), "title": format!("task {n}"),
            "position": position,
        });
        let mut t: Task = serde_json::from_value(json).unwrap();
        t.project = project.map(id);
        t.parent = parent.map(id);
        t
    }

    fn done(mut t: Task) -> Task {
        t.completed_at = Some(ts());
        t
    }

    /// In project 10: task 1 (position 0), group 2 (position 1) with open
    /// children 21 (position 5) and 22 (position 2), a completed child 23
    /// with an open child 231, and 21 with an open child 211; task 3
    /// (position 2) after the group.
    fn world() -> Vec<Task> {
        vec![
            task(1, Some(10), None, 0),
            task(2, Some(10), None, 1),
            task(21, Some(10), Some(2), 5),
            task(22, Some(10), Some(2), 2),
            done(task(23, Some(10), Some(2), 3)),
            task(211, Some(10), Some(21), 0),
            task(231, Some(10), Some(23), 0),
            task(3, Some(10), None, 2),
        ]
    }

    fn plan_for(tasks: &[Task], n: u128, children: ChildFate) -> Result<Plan, CloseError> {
        let snapshot = Snapshot {
            folders: &[],
            projects: &[],
            tasks,
            tags: &[],
            links: &[],
            clocks: &[],
            scopes: &[],
        };
        let request = CloseRequest {
            field: CloseField::Completed,
            children,
            note: Some("why".into()),
            preview: false,
        };
        plan(&snapshot, id(n), &request, ts())
    }

    /// The changes as (object, field, value) for comparing.
    fn sets(plan: &Plan) -> Vec<(Id, String, Value)> {
        plan.changes
            .iter()
            .map(|c| match c {
                Change::Set {
                    object,
                    field,
                    value,
                    ..
                } => (*object, field.clone(), value.clone()),
                other => panic!("not a set: {other:?}"),
            })
            .collect()
    }

    fn at() -> Value {
        Value::String(ts().to_string())
    }

    #[test]
    fn a_task_without_open_children_is_closed_without_asking() {
        let plan = plan_for(&world(), 1, ChildFate::Ask).unwrap();
        assert_eq!((plan.open, plan.closed, plan.lifted), (0, 1, 0));
        assert_eq!(sets(&plan), vec![(id(1), "completed_at".into(), at())]);
        match &plan.changes[0] {
            Change::Set { note, .. } => assert_eq!(note.as_deref(), Some("why")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn ask_writes_nothing_and_counts_every_open_one_below() {
        // 21, 211, 22 and 231 under the completed 23; 23 itself is closed.
        let plan = plan_for(&world(), 2, ChildFate::Ask).unwrap();
        assert_eq!((plan.open, plan.closed, plan.lifted), (4, 0, 0));
        assert!(plan.changes.is_empty());
    }

    #[test]
    fn close_closes_the_open_ones_below_first_then_the_task() {
        let plan = plan_for(&world(), 2, ChildFate::Close).unwrap();
        assert_eq!((plan.open, plan.closed, plan.lifted), (4, 5, 0));
        let objects: Vec<Id> = sets(&plan).into_iter().map(|(o, _, _)| o).collect();
        // Depth first in order of position: 22, 23's 231, 21, 211.
        assert_eq!(objects, vec![id(22), id(231), id(21), id(211), id(2)]);
        assert!(
            sets(&plan)
                .iter()
                .all(|(_, f, v)| f == "completed_at" && *v == at())
        );
    }

    #[test]
    fn lift_puts_the_open_children_right_after_the_task() {
        let plan = plan_for(&world(), 2, ChildFate::Lift).unwrap();
        assert_eq!((plan.open, plan.closed, plan.lifted), (4, 1, 2));
        assert_eq!(
            sets(&plan),
            vec![
                // Task 3 makes room for two.
                (id(3), "position".into(), Value::from(4)),
                (id(22), "parent".into(), Value::Null),
                (id(22), "position".into(), Value::from(2)),
                (id(21), "parent".into(), Value::Null),
                (id(21), "position".into(), Value::from(3)),
                (id(2), "completed_at".into(), at()),
            ]
        );
    }

    #[test]
    fn lift_inside_a_group_goes_to_the_group_above() {
        let tasks = vec![
            task(1, None, None, 0),
            task(2, None, Some(1), 0),
            task(3, None, Some(2), 0),
            task(4, None, Some(1), 1),
        ];
        let plan = plan_for(&tasks, 2, ChildFate::Lift).unwrap();
        assert_eq!(
            sets(&plan),
            vec![
                (id(4), "position".into(), Value::from(2)),
                (id(3), "parent".into(), Value::String(id(1).to_string())),
                (id(3), "position".into(), Value::from(1)),
                (id(2), "completed_at".into(), at()),
            ]
        );
    }

    #[test]
    fn lift_comes_before_siblings_that_share_the_position() {
        // As quick capture leaves the inbox: every task at 0, drawn by id.
        let tasks = vec![
            task(1, None, None, 0),
            task(11, None, Some(1), 0),
            task(12, None, Some(1), 0),
            task(2, None, None, 0),
            task(3, None, None, 0),
        ];
        let plan = plan_for(&tasks, 1, ChildFate::Lift).unwrap();
        assert_eq!(
            sets(&plan),
            vec![
                (id(2), "position".into(), Value::from(3)),
                (id(3), "position".into(), Value::from(4)),
                (id(11), "parent".into(), Value::Null),
                (id(11), "position".into(), Value::from(1)),
                (id(12), "parent".into(), Value::Null),
                (id(12), "position".into(), Value::from(2)),
                (id(1), "completed_at".into(), at()),
            ]
        );
    }

    #[test]
    fn a_closed_task_gives_no_changes() {
        let tasks = vec![done(task(1, None, None, 0)), task(2, None, Some(1), 0)];
        let plan = plan_for(&tasks, 1, ChildFate::Ask).unwrap();
        assert_eq!(plan, Plan::default());
    }

    #[test]
    fn dropping_sets_dropped_at() {
        let tasks = vec![task(1, None, None, 0), task(2, None, Some(1), 0)];
        let snapshot = Snapshot {
            folders: &[],
            projects: &[],
            tasks: &tasks,
            tags: &[],
            links: &[],
            clocks: &[],
            scopes: &[],
        };
        let request = CloseRequest {
            field: CloseField::Dropped,
            children: ChildFate::Close,
            ..CloseRequest::default()
        };
        let plan = plan(&snapshot, id(1), &request, ts()).unwrap();
        let fields: Vec<String> = sets(&plan).into_iter().map(|(_, f, _)| f).collect();
        assert_eq!(fields, vec!["dropped_at", "dropped_at"]);
    }

    #[test]
    fn unknown_task_field_or_fate_is_refused() {
        assert_eq!(
            plan_for(&world(), 99, ChildFate::Ask),
            Err(CloseError::NoTask(id(99)))
        );
        assert_eq!(
            plan_for(&world(), 1, ChildFate::Unknown),
            Err(CloseError::UnknownFate)
        );
    }

    #[test]
    fn a_cycle_in_broken_data_ends() {
        let tasks = vec![task(1, None, Some(2), 0), task(2, None, Some(1), 0)];
        let plan = plan_for(&tasks, 1, ChildFate::Ask).unwrap();
        assert_eq!(plan.open, 1);
    }
}
