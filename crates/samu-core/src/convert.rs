// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Turning a task into a project (ADR 0126).
//!
//! The task stays with its id, so comments, clocks, links and the identity
//! an outside sync knows it by stay whole. A new project takes its title,
//! note, dates and children; the task goes in last. The server reads the
//! account, asks [`plan`] for the changes and writes them in one
//! transaction, like a delete (ADR 0036).

use std::collections::{HashMap, HashSet};

use jiff::Timestamp;
use serde_json::Value;

use crate::api::{
    Change, Id, ObjectKind, Project, ProjectKind, ProjectStatus, Schedule, Task, ToProjectRequest,
};
use crate::availability::Snapshot;
use crate::delete::{after_last, placeholder};
use crate::workflow;

/// The property that says an outside sync writes the task's title.
pub const SOURCE: &str = "source";

/// The changes, in the order to apply them, and what they amount to.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub changes: Vec<Change>,
    /// The new project.
    pub project: Id,
    /// Tasks that move into it, the task itself among them.
    pub moved: u64,
}

/// Why no project is made.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConvertError {
    #[error("no task {0}")]
    NoTask(Id),
    #[error("the task is completed or dropped")]
    Closed,
    #[error("a habit cannot become a project")]
    Habit,
    #[error("a repeating task cannot become a project")]
    Repeating,
    #[error("no folder {0}")]
    NoFolder(Id),
    #[error("the project does not serialize: {0}")]
    Shape(String),
}

/// What turning task `id` into the project `project` takes. `project` is
/// the new id, `now` its creation time.
///
/// # Errors
///
/// When `id` is no open task, a habit or repeats, or the folder is unknown.
pub fn plan(
    snapshot: &Snapshot<'_>,
    id: Id,
    project: Id,
    request: &ToProjectRequest,
    now: Timestamp,
) -> Result<Plan, ConvertError> {
    let task = snapshot
        .tasks
        .iter()
        .find(|t| t.id == id)
        .ok_or(ConvertError::NoTask(id))?;
    if task.completed_at.is_some() || task.dropped_at.is_some() {
        return Err(ConvertError::Closed);
    }
    if task.habit.is_some() {
        return Err(ConvertError::Habit);
    }
    if task.schedule.repeat.is_some() {
        return Err(ConvertError::Repeating);
    }
    let origin = task
        .project
        .and_then(|p| snapshot.projects.iter().find(|x| x.id == p));
    let folder = match request.folder {
        Some(f) if snapshot.folders.iter().any(|x| x.id == f) => Some(f),
        Some(f) => return Err(ConvertError::NoFolder(f)),
        None => origin.and_then(|p| p.folder),
    };

    // §2: states stay with the task; every other tag is copied.
    let states = workflow::workflow_of(origin, snapshot.tags)
        .map(|group| workflow::states(group, snapshot.tags))
        .unwrap_or_default();
    let tags = task
        .tags
        .iter()
        .filter(|t| !states.contains(&t.tag))
        .copied()
        .collect();

    let new = Project {
        id: project,
        created: now,
        modified: now,
        created_by: None,
        title: task.title.clone(),
        icon: None,
        scope_only: false,
        own_scope: origin.and_then(|p| p.own_scope),
        note: task.note.clone(),
        folder,
        workflow: origin.and_then(|p| p.workflow),
        kind: if task.sequential {
            ProjectKind::Sequential
        } else {
            ProjectKind::Parallel
        },
        status: ProjectStatus::Active,
        schedule: Schedule {
            repeat: None,
            ..task.schedule.clone()
        },
        review_interval: None,
        next_review: None,
        last_reviewed: None,
        flagged: task.flagged,
        estimate: task.estimate,
        completed_at: None,
        dropped_at: None,
        tags,
        properties: Default::default(),
        position: after_last(
            snapshot
                .projects
                .iter()
                .filter(|p| p.folder == folder)
                .map(|p| p.position),
        ),
    };
    let mut changes = vec![Change::Create {
        object: project,
        kind: ObjectKind::Project,
        value: serde_json::to_value(&new).map_err(|e| ConvertError::Shape(e.to_string()))?,
        hlc: placeholder(),
    }];
    let project_json = Value::String(project.to_string());

    // §5: the direct children on top in their order, the rest along.
    let mut children: HashMap<Id, Vec<&Task>> = HashMap::new();
    for t in snapshot.tasks {
        if let Some(parent) = t.parent {
            children.entry(parent).or_default().push(t);
        }
    }
    let mut direct: Vec<&Task> = children.get(&id).cloned().unwrap_or_default();
    direct.sort_by_key(|t| (t.position, t.id));
    let mut moved = 1;
    let mut position = 0;
    for child in &direct {
        set(&mut changes, child.id, "project", project_json.clone());
        set(&mut changes, child.id, "parent", Value::Null);
        set(&mut changes, child.id, "position", Value::from(position));
        position += 1;
        moved += 1;
    }
    let mut seen: HashSet<Id> = direct.iter().map(|t| t.id).chain([id]).collect();
    let mut stack: Vec<Id> = direct.iter().map(|t| t.id).collect();
    while let Some(current) = stack.pop() {
        let mut below = children.get(&current).cloned().unwrap_or_default();
        below.sort_by_key(|t| (t.position, t.id));
        for t in below {
            // A cycle in broken data stops here instead of running forever.
            if seen.insert(t.id) {
                set(&mut changes, t.id, "project", project_json.clone());
                stack.push(t.id);
                moved += 1;
            }
        }
    }

    // §6: the task last, without its group, and with the prefix.
    set(&mut changes, id, "project", project_json);
    if task.parent.is_some() {
        set(&mut changes, id, "parent", Value::Null);
    }
    set(&mut changes, id, "position", Value::from(position));
    if let Some(prefix) = request.prefix.as_deref().filter(|p| !p.is_empty())
        && !task.properties.contains_key(SOURCE)
    {
        set(
            &mut changes,
            id,
            "title",
            Value::String(format!("{prefix}{}", task.title)),
        );
    }
    // §2: what wandered to the project leaves the task.
    let schedule = &task.schedule;
    let wandered = [
        ("note", task.note.is_some()),
        ("defer", schedule.defer.is_some()),
        ("due", schedule.due.is_some()),
        ("planned", schedule.planned.is_some()),
        ("planned_duration", schedule.planned_duration.is_some()),
        ("due_warning", schedule.due_warning.is_some()),
        ("estimate", task.estimate.is_some()),
    ];
    for (field, present) in wandered {
        if present {
            set(&mut changes, id, field, Value::Null);
        }
    }
    if task.flagged {
        set(&mut changes, id, "flagged", Value::Bool(false));
    }
    Ok(Plan {
        changes,
        project,
        moved,
    })
}

fn set(changes: &mut Vec<Change>, object: Id, field: &str, value: Value) {
    changes.push(Change::Set {
        object,
        field: field.into(),
        value,
        note: None,
        hlc: placeholder(),
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::*;
    use crate::api::{Folder, Tag, Tagging};

    fn ts() -> Timestamp {
        "2026-11-18T08:00:00Z".parse().unwrap()
    }

    fn id(n: u128) -> Id {
        format!("01936f5e-8a00-7c3e-9b1a-{n:012x}").parse().unwrap()
    }

    fn project(n: u128, folder: Option<u128>, position: i64) -> Project {
        let json = serde_json::json!({
            "id": id(n), "created": ts(), "modified": ts(),
            "title": format!("project {n}"), "kind": "parallel", "status": "active",
            "position": position,
        });
        let mut p: Project = serde_json::from_value(json).unwrap();
        p.folder = folder.map(id);
        p
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

    fn folder(n: u128) -> Folder {
        serde_json::from_value(serde_json::json!({
            "id": id(n), "created": ts(), "modified": ts(), "title": format!("folder {n}"),
            "status": "active", "position": 0,
        }))
        .unwrap()
    }

    fn tag(n: u128, parent: Option<u128>, exclusive: bool) -> Tag {
        let mut t: Tag = serde_json::from_value(serde_json::json!({
            "id": id(n), "created": ts(), "modified": ts(), "title": format!("tag {n}"),
            "status": "active", "exclusive": exclusive,
        }))
        .unwrap();
        t.parent = parent.map(id);
        t
    }

    #[derive(Default)]
    struct World {
        folders: Vec<Folder>,
        projects: Vec<Project>,
        tasks: Vec<Task>,
        tags: Vec<Tag>,
    }

    impl World {
        fn plan(&self, n: u128, request: &ToProjectRequest) -> Result<Plan, ConvertError> {
            let snapshot = Snapshot {
                folders: &self.folders,
                projects: &self.projects,
                tasks: &self.tasks,
                tags: &self.tags,
                links: &[],
                clocks: &[],
                scopes: &[],
            };
            plan(&snapshot, id(n), id(99), request, ts())
        }
    }

    /// Folder 1 holds project 10 (and project 11 at position 5); 10 holds
    /// group 20 with children 21 (position 3) and 22 (position 1), 21 a
    /// grandchild 23; task 24 stays in 10.
    fn world() -> World {
        World {
            folders: vec![folder(1)],
            projects: vec![project(10, Some(1), 0), project(11, Some(1), 5)],
            tasks: vec![
                task(20, Some(10), None, 0),
                task(21, Some(10), Some(20), 3),
                task(22, Some(10), Some(20), 1),
                task(23, Some(10), Some(21), 0),
                task(24, Some(10), None, 1),
            ],
            ..World::default()
        }
    }

    fn created(plan: &Plan) -> Project {
        match &plan.changes[0] {
            Change::Create { value, kind, .. } => {
                assert_eq!(*kind, ObjectKind::Project);
                serde_json::from_value(value.clone()).unwrap()
            }
            other => panic!("first change is {other:?}"),
        }
    }

    fn sets_on(plan: &Plan, n: u128) -> Vec<(String, Value)> {
        plan.changes
            .iter()
            .filter_map(|c| match c {
                Change::Set {
                    object,
                    field,
                    value,
                    ..
                } if *object == id(n) => Some((field.clone(), value.clone())),
                _ => None,
            })
            .collect()
    }

    fn get(sets: &[(String, Value)], field: &str) -> Option<Value> {
        sets.iter()
            .find(|(f, _)| f == field)
            .map(|(_, v)| v.clone())
    }

    #[test]
    fn children_go_on_top_in_order_and_the_task_goes_last() {
        let plan = world().plan(20, &ToProjectRequest::default()).unwrap();
        let p = created(&plan);
        assert_eq!(p.id, id(99));
        assert_eq!(p.title, "task 20");
        assert_eq!(p.folder, Some(id(1)));
        assert_eq!(p.position, 6, "after the last project in the folder");
        assert_eq!(p.kind, ProjectKind::Parallel);
        assert_eq!(plan.moved, 4);
        let new = Value::String(id(99).to_string());
        // 22 (position 1) before 21 (position 3).
        let c22 = sets_on(&plan, 22);
        assert_eq!(get(&c22, "position"), Some(Value::from(0)));
        assert_eq!(get(&c22, "parent"), Some(Value::Null));
        assert_eq!(get(&c22, "project"), Some(new.clone()));
        assert_eq!(get(&sets_on(&plan, 21), "position"), Some(Value::from(1)));
        // The grandchild keeps its parent.
        let c23 = sets_on(&plan, 23);
        assert_eq!(c23, vec![("project".into(), new.clone())]);
        let me = sets_on(&plan, 20);
        assert_eq!(get(&me, "project"), Some(new));
        assert_eq!(get(&me, "position"), Some(Value::from(2)));
        assert_eq!(get(&me, "title"), None, "no prefix asked for");
        assert!(sets_on(&plan, 24).is_empty());
    }

    #[test]
    fn note_dates_and_flag_wander_tags_are_copied_states_stay() {
        let mut w = world();
        w.tags = vec![
            tag(40, None, true),
            tag(41, Some(40), false),
            tag(50, None, false),
        ];
        w.tags[0].default_workflow = true;
        w.projects[0].workflow = Some(id(40));
        w.projects[0].own_scope = Some(id(70));
        let t = &mut w.tasks[0];
        t.note = Some("why".into());
        t.flagged = true;
        t.sequential = true;
        t.priority = Some(crate::api::Priority::A);
        t.schedule.due = Some(crate::api::When::Date("2026-12-01".parse().unwrap()));
        t.tags = vec![
            Tagging {
                tag: id(41),
                position: 0,
            },
            Tagging {
                tag: id(50),
                position: 1,
            },
        ];
        let plan = w.plan(20, &ToProjectRequest::default()).unwrap();
        let p = created(&plan);
        assert_eq!(p.note.as_deref(), Some("why"));
        assert!(p.flagged);
        assert_eq!(p.kind, ProjectKind::Sequential);
        assert!(p.schedule.due.is_some());
        assert_eq!(p.workflow, Some(id(40)));
        assert_eq!(p.own_scope, Some(id(70)));
        assert_eq!(
            p.tags.iter().map(|t| t.tag).collect::<Vec<_>>(),
            vec![id(50)]
        );
        let me = sets_on(&plan, 20);
        assert_eq!(get(&me, "note"), Some(Value::Null));
        assert_eq!(get(&me, "due"), Some(Value::Null));
        assert_eq!(get(&me, "flagged"), Some(Value::Bool(false)));
        assert_eq!(get(&me, "defer"), None, "only what was set");
        assert_eq!(get(&me, "priority"), None, "priority stays");
        assert_eq!(get(&me, "tags"), None, "tags stay");
    }

    #[test]
    fn the_prefix_is_skipped_when_an_outside_sync_owns_the_title() {
        let with_prefix = ToProjectRequest {
            prefix: Some("Notizen: ".into()),
            ..ToProjectRequest::default()
        };
        let plan = world().plan(20, &with_prefix).unwrap();
        assert_eq!(
            get(&sets_on(&plan, 20), "title"),
            Some(Value::String("Notizen: task 20".into()))
        );
        let mut w = world();
        w.tasks[0]
            .properties
            .insert(SOURCE.into(), "jira-sync".into());
        w.tasks[0].properties.insert("jira".into(), "ABC-1".into());
        let plan = w.plan(20, &with_prefix).unwrap();
        assert_eq!(get(&sets_on(&plan, 20), "title"), None);
        assert!(created(&plan).properties.is_empty(), "no identity twice");
    }

    #[test]
    fn a_subgroup_leaves_its_parent_and_an_inbox_task_goes_to_the_top() {
        let plan = world().plan(21, &ToProjectRequest::default()).unwrap();
        assert_eq!(plan.moved, 2);
        let me = sets_on(&plan, 21);
        assert_eq!(get(&me, "parent"), Some(Value::Null));
        assert_eq!(get(&me, "position"), Some(Value::from(1)));

        let mut w = world();
        w.tasks.push(task(30, None, None, 0));
        let plan = w.plan(30, &ToProjectRequest::default()).unwrap();
        let p = created(&plan);
        assert_eq!(p.folder, None);
        assert_eq!(p.position, 0);
        assert_eq!(plan.moved, 1, "a task without children is fine");
    }

    #[test]
    fn a_folder_can_be_chosen() {
        let mut w = world();
        w.folders.push(folder(2));
        let request = ToProjectRequest {
            folder: Some(id(2)),
            ..ToProjectRequest::default()
        };
        assert_eq!(created(&w.plan(20, &request).unwrap()).folder, Some(id(2)));
        let request = ToProjectRequest {
            folder: Some(id(3)),
            ..ToProjectRequest::default()
        };
        assert_eq!(w.plan(20, &request), Err(ConvertError::NoFolder(id(3))));
    }

    #[test]
    fn refused_for_closed_habits_repeats_and_non_tasks() {
        let request = ToProjectRequest::default();
        assert_eq!(
            world().plan(10, &request),
            Err(ConvertError::NoTask(id(10)))
        );
        let mut w = world();
        w.tasks[0].completed_at = Some(ts());
        assert_eq!(w.plan(20, &request), Err(ConvertError::Closed));
        let mut w = world();
        w.tasks[0].habit = Some(crate::api::Habit::default());
        assert_eq!(w.plan(20, &request), Err(ConvertError::Habit));
        let mut w = world();
        w.tasks[0].schedule.repeat = Some(
            serde_json::from_value(serde_json::json!({"rule": "FREQ=WEEKLY", "mode": "fixed"}))
                .unwrap(),
        );
        assert_eq!(w.plan(20, &request), Err(ConvertError::Repeating));
    }
}
