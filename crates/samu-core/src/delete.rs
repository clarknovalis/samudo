// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What deleting an object takes with it (ADR 0036).
//!
//! The server reads the account, asks [`plan`] for the changes and writes
//! them in one transaction. Every change is an ordinary field change
//! (ADR 0004) with a placeholder stamp; the server stamps them.

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use crate::api::{Change, Hlc, Id, Scope, Task, TaskFate};
use crate::availability::Snapshot;

/// The changes, in the order to apply them, and what they amount to.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    pub changes: Vec<Change>,
    /// Objects that become tombstones, the object itself among them.
    pub deleted: u64,
    /// Tasks, projects and folders that move.
    pub moved: u64,
    /// Tasks, projects and scopes a deleted tag is taken from.
    pub untagged: u64,
}

/// Why nothing is deleted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeleteError {
    #[error("only a project's tasks have somewhere to go")]
    TasksNotForThis,
    #[error("a choice for the tasks this samu does not know")]
    UnknownFate,
    #[error("the tasks cannot move into the project being deleted")]
    TargetIsItself,
    #[error("no project {0} to move the tasks into")]
    NoTarget(Id),
    #[error("the tag has {0} child tags; move or delete them first")]
    TagHasChildren(usize),
    #[error("project {0:?} uses the tag as its workflow; give it another first")]
    TagIsWorkflow(String),
}

/// What deleting `id` takes with it. An id that is no folder, project, task
/// or tag here (a scope, a reminder, something already deleted) gets a
/// plain tombstone.
///
/// # Errors
///
/// When the request does not fit the object, or a tag is still needed.
pub fn plan(
    snapshot: &Snapshot<'_>,
    scopes: &[Scope],
    id: Id,
    tasks: Option<TaskFate>,
) -> Result<Plan, DeleteError> {
    let is_project = snapshot.projects.iter().any(|p| p.id == id);
    if tasks.is_some() && !is_project {
        return Err(DeleteError::TasksNotForThis);
    }
    let mut plan = Plan::default();
    if is_project {
        project(snapshot, id, tasks.unwrap_or(TaskFate::Inbox), &mut plan)?;
        out_of_scopes(scopes, id, &mut plan);
    } else if snapshot.folders.iter().any(|f| f.id == id) {
        folder(snapshot, id, &mut plan);
        out_of_scopes(scopes, id, &mut plan);
    } else if snapshot.tasks.iter().any(|t| t.id == id) {
        task(snapshot, id, &mut plan);
    } else if snapshot.tags.iter().any(|t| t.id == id) {
        tag(snapshot, scopes, id, &mut plan)?;
    }
    tombstone(id, &mut plan);
    Ok(plan)
}

/// ADR 0036 §2: the tasks go to the inbox, into another project, or along.
fn project(
    snapshot: &Snapshot<'_>,
    id: Id,
    fate: TaskFate,
    plan: &mut Plan,
) -> Result<(), DeleteError> {
    let mut own: Vec<&Task> = snapshot
        .tasks
        .iter()
        .filter(|t| t.project == Some(id))
        .collect();
    own.sort_by_key(|t| (t.position, t.id));
    let members: HashSet<Id> = own.iter().map(|t| t.id).collect();
    // Tasks whose parent is not among them come out on top.
    let on_top = |t: &Task| t.parent.is_none_or(|p| !members.contains(&p));
    match fate {
        TaskFate::Inbox => {
            for t in own {
                set(plan, t.id, "project", Value::Null);
                plan.moved += 1;
            }
        }
        TaskFate::Project(target) => {
            if target == id {
                return Err(DeleteError::TargetIsItself);
            }
            if !snapshot.projects.iter().any(|p| p.id == target) {
                return Err(DeleteError::NoTarget(target));
            }
            let mut next = after_last(
                snapshot
                    .tasks
                    .iter()
                    .filter(|t| t.project == Some(target) && t.parent.is_none())
                    .map(|t| t.position),
            );
            for t in own {
                set(plan, t.id, "project", Value::String(target.to_string()));
                if on_top(t) {
                    set(plan, t.id, "position", Value::from(next));
                    next += 1;
                }
                plan.moved += 1;
            }
        }
        TaskFate::Delete => {
            for t in own {
                tombstone(t.id, plan);
            }
        }
        TaskFate::Unknown => return Err(DeleteError::UnknownFate),
    }
    Ok(())
}

/// ADR 0036 §3: what is in the folder moves up a level, after what is there.
fn folder(snapshot: &Snapshot<'_>, id: Id, plan: &mut Plan) {
    let Some(gone) = snapshot.folders.iter().find(|f| f.id == id) else {
        return;
    };
    let up = gone.parent;
    let up_json = up.map_or(Value::Null, |p| Value::String(p.to_string()));

    let next = after_last(
        snapshot
            .folders
            .iter()
            .filter(|f| f.parent == up && f.id != id)
            .map(|f| f.position),
    );
    let mut below: Vec<_> = snapshot
        .folders
        .iter()
        .filter(|f| f.parent == Some(id))
        .collect();
    below.sort_by_key(|f| (f.position, f.id));
    for (position, f) in (next..).zip(below) {
        set(plan, f.id, "parent", up_json.clone());
        set(plan, f.id, "position", Value::from(position));
        plan.moved += 1;
    }

    let next = after_last(
        snapshot
            .projects
            .iter()
            .filter(|p| p.folder == up)
            .map(|p| p.position),
    );
    let mut inside: Vec<_> = snapshot
        .projects
        .iter()
        .filter(|p| p.folder == Some(id))
        .collect();
    inside.sort_by_key(|p| (p.position, p.id));
    for (position, p) in (next..).zip(inside) {
        set(plan, p.id, "folder", up_json.clone());
        set(plan, p.id, "position", Value::from(position));
        plan.moved += 1;
    }
}

/// ADR 0036 §5: a task takes everything below it along, deepest first.
fn task(snapshot: &Snapshot<'_>, id: Id, plan: &mut Plan) {
    let mut children: HashMap<Id, Vec<Id>> = HashMap::new();
    for t in snapshot.tasks {
        if let Some(parent) = t.parent {
            children.entry(parent).or_default().push(t.id);
        }
    }
    let mut seen = HashSet::from([id]);
    let mut order = Vec::new();
    let mut stack = vec![id];
    while let Some(current) = stack.pop() {
        for &child in children.get(&current).into_iter().flatten() {
            // A cycle in broken data stops here instead of running forever.
            if seen.insert(child) {
                order.push(child);
                stack.push(child);
            }
        }
    }
    for child in order.into_iter().rev() {
        tombstone(child, plan);
    }
}

/// ADR 0036 §4: taken off everything first; refused while still needed.
fn tag(
    snapshot: &Snapshot<'_>,
    scopes: &[Scope],
    id: Id,
    plan: &mut Plan,
) -> Result<(), DeleteError> {
    let children = snapshot
        .tags
        .iter()
        .filter(|t| t.parent == Some(id))
        .count();
    if children > 0 {
        return Err(DeleteError::TagHasChildren(children));
    }
    if let Some(project) = snapshot.projects.iter().find(|p| p.workflow == Some(id)) {
        return Err(DeleteError::TagIsWorkflow(project.title.clone()));
    }
    let carriers = snapshot
        .tasks
        .iter()
        .map(|t| (t.id, &t.tags))
        .chain(snapshot.projects.iter().map(|p| (p.id, &p.tags)));
    for (object, tags) in carriers {
        if tags.iter().any(|t| t.tag == id) {
            remove(plan, object, "tags", id);
            plan.untagged += 1;
        }
    }
    for scope in scopes {
        if scope.members.contains(&id) {
            remove(plan, scope.id, "members", id);
            plan.untagged += 1;
        }
    }
    Ok(())
}

/// ADR 0076 §1: a project or folder leaves every scope it is a member of,
/// as a tag does (ADR 0036 §4); else the scope keeps a member that is gone.
fn out_of_scopes(scopes: &[Scope], id: Id, plan: &mut Plan) {
    for scope in scopes {
        if scope.members.contains(&id) {
            remove(plan, scope.id, "members", id);
        }
    }
}

fn after_last(positions: impl Iterator<Item = i64>) -> i64 {
    positions.max().map_or(0, |p| p.saturating_add(1))
}

/// Stamped by the server before it writes.
fn placeholder() -> Hlc {
    Hlc::new(0, 0, "")
}

fn set(plan: &mut Plan, object: Id, field: &str, value: Value) {
    plan.changes.push(Change::Set {
        object,
        field: field.into(),
        value,
        note: None,
        hlc: placeholder(),
    });
}

fn remove(plan: &mut Plan, object: Id, field: &str, element: Id) {
    plan.changes.push(Change::Remove {
        object,
        field: field.into(),
        element,
        note: None,
        hlc: placeholder(),
    });
}

fn tombstone(object: Id, plan: &mut Plan) {
    plan.changes.push(Change::Delete {
        object,
        hlc: placeholder(),
    });
    plan.deleted += 1;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::*;
    use crate::api::{
        Folder, FolderStatus, Project, ProjectKind, ProjectStatus, ScopeMode, Tag, TagStatus,
        Tagging,
    };
    use jiff::Timestamp;

    fn ts() -> Timestamp {
        "2026-11-08T08:00:00Z".parse().unwrap()
    }

    fn id(n: u128) -> Id {
        format!("01936f5e-8a00-7c3e-9b1a-{n:012x}").parse().unwrap()
    }

    fn folder(n: u128, parent: Option<u128>, position: i64) -> Folder {
        Folder {
            scope_only: false,
            own_scope: None,
            icon: None,
            id: id(n),
            created: ts(),
            modified: ts(),
            title: format!("folder {n}"),
            parent: parent.map(id),
            status: FolderStatus::Active,
            position,
        }
    }

    fn project(n: u128, folder: Option<u128>, position: i64) -> Project {
        let json = serde_json::json!({
            "id": id(n), "created": ts(), "modified": ts(),
            "title": format!("project {n}"), "kind": "parallel", "status": "active",
            "position": position,
        });
        let mut p: Project = serde_json::from_value(json).unwrap();
        p.folder = folder.map(id);
        assert_eq!(p.kind, ProjectKind::Parallel);
        assert_eq!(p.status, ProjectStatus::Active);
        p
    }

    fn task(n: u128, project: Option<u128>, parent: Option<u128>, position: i64) -> Task {
        let json = serde_json::json!({
            "id": id(n), "created": ts(), "modified": ts(), "title": format!("task {n}"),
            "flagged": false, "sequential": false, "complete_with_last_child": false,
            "position": position,
        });
        let mut t: Task = serde_json::from_value(json).unwrap();
        t.project = project.map(id);
        t.parent = parent.map(id);
        t
    }

    fn tag(n: u128, parent: Option<u128>) -> Tag {
        let json = serde_json::json!({
            "id": id(n), "created": ts(), "modified": ts(), "title": format!("tag {n}"),
            "status": "active",
        });
        let mut t: Tag = serde_json::from_value(json).unwrap();
        t.parent = parent.map(id);
        assert_eq!(t.status, TagStatus::Active);
        t
    }

    #[derive(Default)]
    struct World {
        folders: Vec<Folder>,
        projects: Vec<Project>,
        tasks: Vec<Task>,
        tags: Vec<Tag>,
        scopes: Vec<Scope>,
    }

    impl World {
        fn plan(&self, n: u128, fate: Option<TaskFate>) -> Result<Plan, DeleteError> {
            let snapshot = Snapshot {
                folders: &self.folders,
                projects: &self.projects,
                tasks: &self.tasks,
                tags: &self.tags,
                links: &[],
                clocks: &[],
                scopes: &[],
            };
            plan(&snapshot, &self.scopes, id(n), fate)
        }
    }

    /// Project 10 holds a group 20 with child 21 and a task 22; project 11
    /// holds task 30 at position 4.
    fn two_projects() -> World {
        World {
            projects: vec![project(10, None, 0), project(11, None, 1)],
            tasks: vec![
                task(20, Some(10), None, 0),
                task(21, Some(10), Some(20), 0),
                task(22, Some(10), None, 1),
                task(30, Some(11), None, 4),
            ],
            ..World::default()
        }
    }

    fn sets(plan: &Plan) -> Vec<(Id, String, Value)> {
        plan.changes
            .iter()
            .filter_map(|c| match c {
                Change::Set {
                    object,
                    field,
                    value,
                    ..
                } => Some((*object, field.clone(), value.clone())),
                _ => None,
            })
            .collect()
    }

    fn deletes(plan: &Plan) -> Vec<Id> {
        plan.changes
            .iter()
            .filter_map(|c| match c {
                Change::Delete { object, .. } => Some(*object),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_project_sends_its_tasks_to_the_inbox_by_default() {
        let plan = two_projects().plan(10, None).unwrap();
        assert_eq!(
            sets(&plan),
            vec![
                (id(20), "project".into(), Value::Null),
                (id(21), "project".into(), Value::Null),
                (id(22), "project".into(), Value::Null),
            ]
        );
        assert_eq!(deletes(&plan), vec![id(10)], "the project goes last");
        assert_eq!((plan.deleted, plan.moved), (1, 3));
    }

    #[test]
    fn into_another_project_the_top_tasks_line_up_after_its_last() {
        let plan = two_projects()
            .plan(10, Some(TaskFate::Project(id(11))))
            .unwrap();
        let to = Value::String(id(11).to_string());
        assert_eq!(
            sets(&plan),
            vec![
                (id(20), "project".into(), to.clone()),
                (id(20), "position".into(), Value::from(5)),
                (id(21), "project".into(), to.clone()),
                (id(22), "project".into(), to),
                (id(22), "position".into(), Value::from(6)),
            ],
            "the child keeps its parent and its place"
        );
        assert_eq!(plan.moved, 3);
    }

    #[test]
    fn deleting_along_tombstones_every_task() {
        let plan = two_projects().plan(10, Some(TaskFate::Delete)).unwrap();
        assert_eq!(deletes(&plan), vec![id(20), id(21), id(22), id(10)]);
        assert!(sets(&plan).is_empty());
        assert_eq!((plan.deleted, plan.moved), (4, 0));
    }

    #[test]
    fn a_target_must_be_another_project_that_is_there() {
        let world = two_projects();
        assert_eq!(
            world.plan(10, Some(TaskFate::Project(id(10)))),
            Err(DeleteError::TargetIsItself)
        );
        assert_eq!(
            world.plan(10, Some(TaskFate::Project(id(99)))),
            Err(DeleteError::NoTarget(id(99)))
        );
        assert_eq!(
            world.plan(10, Some(TaskFate::Unknown)),
            Err(DeleteError::UnknownFate)
        );
        assert_eq!(
            world.plan(20, Some(TaskFate::Inbox)),
            Err(DeleteError::TasksNotForThis)
        );
    }

    #[test]
    fn a_folder_hands_its_content_one_level_up() {
        let world = World {
            folders: vec![
                folder(1, None, 0),
                folder(2, Some(1), 0),
                folder(3, Some(2), 0),
                folder(4, Some(1), 7),
            ],
            projects: vec![project(10, Some(2), 0), project(11, Some(1), 2)],
            ..World::default()
        };
        let plan = world.plan(2, None).unwrap();
        let up = Value::String(id(1).to_string());
        assert_eq!(
            sets(&plan),
            vec![
                (id(3), "parent".into(), up.clone()),
                (id(3), "position".into(), Value::from(8)),
                (id(10), "folder".into(), up),
                (id(10), "position".into(), Value::from(3)),
            ]
        );
        assert_eq!(deletes(&plan), vec![id(2)]);
        assert_eq!(plan.moved, 2);
    }

    #[test]
    fn a_top_folder_hands_its_content_to_the_top() {
        let world = World {
            folders: vec![folder(1, None, 0)],
            projects: vec![project(10, Some(1), 0)],
            ..World::default()
        };
        let plan = world.plan(1, None).unwrap();
        assert_eq!(
            sets(&plan),
            vec![
                (id(10), "folder".into(), Value::Null),
                (id(10), "position".into(), Value::from(0)),
            ]
        );
    }

    #[test]
    fn a_task_takes_its_descendants_along_deepest_first() {
        let mut world = two_projects();
        world.tasks.push(task(23, Some(10), Some(21), 0));
        let plan = world.plan(20, None).unwrap();
        assert_eq!(deletes(&plan), vec![id(23), id(21), id(20)]);
        assert_eq!(plan.deleted, 3);
    }

    #[test]
    fn a_tag_comes_off_tasks_projects_and_scopes_first() {
        let mut world = two_projects();
        world.tags = vec![tag(40, None), tag(41, None)];
        world.tasks[0].tags = vec![Tagging {
            tag: id(40),
            position: 0,
        }];
        world.projects[1].tags = vec![Tagging {
            tag: id(40),
            position: 0,
        }];
        world.tasks[2].tags = vec![Tagging {
            tag: id(41),
            position: 0,
        }];
        world.scopes = vec![Scope {
            layout: None,
            color: None,
            id: id(50),
            created: ts(),
            modified: ts(),
            title: "scope".into(),
            mode: ScopeMode::Only,
            members: vec![id(40), id(10)],
            inbox: false,
            filter: Vec::new(),
            capture: None,
            schedule: Vec::new(),
            calendars: Vec::new(),
            position: 0,
        }];
        let plan = world.plan(40, None).unwrap();
        let removed: Vec<(Id, String)> = plan
            .changes
            .iter()
            .filter_map(|c| match c {
                Change::Remove {
                    object,
                    field,
                    element,
                    ..
                } => {
                    assert_eq!(*element, id(40));
                    Some((*object, field.clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            removed,
            vec![
                (id(20), "tags".into()),
                (id(11), "tags".into()),
                (id(50), "members".into()),
            ]
        );
        assert_eq!(deletes(&plan), vec![id(40)]);
        assert_eq!(plan.untagged, 3);
    }

    /// Regression (01a1caff): the project "coding" stayed a member of a
    /// scope after it was deleted.
    #[test]
    fn a_project_or_folder_leaves_the_scopes_it_is_in() {
        let mut world = two_projects();
        world.scopes = vec![Scope {
            layout: None,
            color: None,
            id: id(50),
            created: ts(),
            modified: ts(),
            title: "scope".into(),
            mode: ScopeMode::Only,
            members: vec![id(10), id(11)],
            inbox: false,
            filter: Vec::new(),
            capture: None,
            schedule: Vec::new(),
            calendars: Vec::new(),
            position: 0,
        }];
        let removed = |plan: &Plan| -> Vec<(Id, Id)> {
            plan.changes
                .iter()
                .filter_map(|c| match c {
                    Change::Remove {
                        object,
                        field,
                        element,
                        ..
                    } if field == "members" => Some((*object, *element)),
                    _ => None,
                })
                .collect()
        };
        let plan = world.plan(10, None).unwrap();
        assert_eq!(removed(&plan), vec![(id(50), id(10))]);
        // The delete comes last, after the scope let go.
        assert!(
            matches!(plan.changes.last(), Some(Change::Delete { object, .. }) if *object == id(10))
        );
        // A folder that is a member goes out the same way.
        world.folders = vec![folder(1, None, 0)];
        world.scopes[0].members.push(id(1));
        assert_eq!(
            removed(&world.plan(1, None).unwrap()),
            vec![(id(50), id(1))]
        );
    }

    #[test]
    fn a_tag_still_needed_is_refused() {
        let mut world = two_projects();
        world.tags = vec![tag(40, None), tag(41, Some(40)), tag(42, None)];
        assert_eq!(world.plan(40, None), Err(DeleteError::TagHasChildren(1)));
        world.projects[0].workflow = Some(id(42));
        assert_eq!(
            world.plan(42, None),
            Err(DeleteError::TagIsWorkflow("project 10".into()))
        );
        assert!(world.plan(41, None).is_ok(), "the probe deletes one");
    }

    #[test]
    fn anything_else_gets_a_plain_tombstone() {
        let plan = two_projects().plan(77, None).unwrap();
        assert_eq!(deletes(&plan), vec![id(77)]);
        assert_eq!(plan.changes.len(), 1);
    }
}
