// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Restricted sessions: what a session with a grant sees and may write
//! (ADR 0095 §2, §3).
//!
//! The rule lives here and only here; the server asks this module before
//! the store, for every route that filters (ADR 0095 §4).
//!
//! Seen is every granted folder with all folders below it, every project in
//! one of them and every granted project, their tasks with all subtasks, and
//! what hangs on any of these: links, clocks, reminders, comments,
//! attachments, habit entries. All tags are seen as well, read-only, since
//! states, priorities and taggings cannot be read without them (workflows
//! are tags). Nothing else is: not the inbox, scopes, perspectives,
//! templates, settings, calendars or appointments.
//!
//! Written may be, with `rw` only, what is seen and stays seen, except tags.
//! An object made or moved must lie in the grant afterwards.

use std::collections::{HashMap, HashSet};

use crate::api::{Access, AnyObject, Folder, Grant, Id, Project, Task};
use crate::overlay::Objects;

/// A grant read against an account's folders, projects and tasks, ready to
/// ask about objects.
#[derive(Debug, Clone)]
pub struct Reach {
    access: Access,
    /// The ids the grant names, whatever they are.
    granted: HashSet<Id>,
    /// Seen folders, projects and tasks: what something can hang on.
    folders: HashSet<Id>,
    projects: HashSet<Id>,
    tasks: HashSet<Id>,
}

impl Reach {
    /// The grant over the account's folders, projects and tasks as they are.
    #[must_use]
    pub fn new(grant: &Grant, folders: &[Folder], projects: &[Project], tasks: &[Task]) -> Self {
        let granted: HashSet<Id> = grant.objects.iter().copied().collect();
        let parents: HashMap<Id, Option<Id>> = folders.iter().map(|f| (f.id, f.parent)).collect();
        let folders = folders
            .iter()
            .filter(|f| below_granted(f.id, &parents, &granted))
            .map(|f| f.id)
            .collect();
        let mut reach = Self {
            access: grant.access,
            granted,
            folders,
            projects: HashSet::new(),
            tasks: HashSet::new(),
        };
        reach.projects = projects
            .iter()
            .filter(|p| reach.sees_project(p))
            .map(|p| p.id)
            .collect();
        let task_parents: HashMap<Id, &Task> = tasks.iter().map(|t| (t.id, t)).collect();
        reach.tasks = tasks
            .iter()
            .filter(|t| reach.task_in_projects(t, &task_parents))
            .map(|t| t.id)
            .collect();
        reach
    }

    /// Whether the session sees this object (§2). A folder, project or
    /// task is judged by where it lies, so an object that is new or moved
    /// can be asked about too.
    #[must_use]
    pub fn sees(&self, object: &AnyObject) -> bool {
        match object {
            AnyObject::Folder(folder) => self.sees_folder(folder),
            AnyObject::Project(project) => self.sees_project(project),
            AnyObject::Task(task) => self.sees_task(task),
            AnyObject::Tag(_) => true,
            AnyObject::Link(link) => self.sees_id(link.owner),
            AnyObject::Clock(clock) => self.sees_id(clock.owner),
            AnyObject::Reminder(reminder) => self.sees_id(reminder.owner),
            AnyObject::Comment(comment) => self.sees_id(comment.owner),
            AnyObject::Attachment(attachment) => self.sees_id(attachment.owner),
            AnyObject::HabitEntry(entry) => self.tasks.contains(&entry.habit),
            AnyObject::ChecklistItem(item) => self.sees_id(item.owner),
            AnyObject::Scope(_)
            | AnyObject::Settings(_)
            | AnyObject::Calendar(_)
            | AnyObject::Appointment(_)
            | AnyObject::Template(_)
            | AnyObject::Perspective(_)
            | AnyObject::Unknown => false,
        }
    }

    /// Whether a folder, project or task with this id is seen, as known when
    /// the reach was made or admitted since: what a comment, a logbook or
    /// a blob hangs on. A tag is not among them.
    #[must_use]
    pub fn sees_id(&self, id: Id) -> bool {
        self.folders.contains(&id) || self.projects.contains(&id) || self.tasks.contains(&id)
    }

    /// Whether the session may change an object (§3): `before` as it is,
    /// none for a new one, and `after` as it would be. Deleting asks with
    /// the object on both sides.
    #[must_use]
    pub fn may_write(&self, before: Option<&AnyObject>, after: &AnyObject) -> bool {
        if self.access != Access::Rw {
            return false;
        }
        let allowed =
            |object: &AnyObject| !matches!(object, AnyObject::Tag(_)) && self.sees(object);
        before.is_none_or(allowed) && allowed(after) && !self.moves_a_root(before, after)
    }

    /// Whether a granted folder or project would leave its place: it stays
    /// seen, its id being granted, but would lie elsewhere in the account
    /// (decided with step 2 of ADR 0095, 34bb5b0e).
    fn moves_a_root(&self, before: Option<&AnyObject>, after: &AnyObject) -> bool {
        match (before, after) {
            (Some(AnyObject::Folder(old)), AnyObject::Folder(new)) => {
                self.granted.contains(&new.id) && old.parent != new.parent
            }
            (Some(AnyObject::Project(old)), AnyObject::Project(new)) => {
                self.granted.contains(&new.id) && old.folder != new.folder
            }
            _ => false,
        }
    }

    /// The account's objects as the session sees them (§2): what [`sees`]
    /// lets through, all tags, no settings.
    ///
    /// [`sees`]: Self::sees
    #[must_use]
    pub fn restrict(&self, objects: &Objects) -> Objects {
        Objects {
            folders: keep_seen(self, &objects.folders, AnyObject::Folder),
            projects: keep_seen(self, &objects.projects, AnyObject::Project),
            tasks: keep_seen(self, &objects.tasks, AnyObject::Task),
            tags: objects.tags.clone(),
            links: keep_seen(self, &objects.links, AnyObject::Link),
            clocks: keep_seen(self, &objects.clocks, AnyObject::Clock),
            reminders: keep_seen(self, &objects.reminders, AnyObject::Reminder),
            scopes: Vec::new(),
            comments: keep_seen(self, &objects.comments, AnyObject::Comment),
            attachments: keep_seen(self, &objects.attachments, AnyObject::Attachment),
            settings: None,
            calendars: Vec::new(),
            appointments: Vec::new(),
            templates: Vec::new(),
            perspectives: Vec::new(),
            habit_entries: keep_seen(self, &objects.habit_entries, AnyObject::HabitEntry),
            checklist_items: keep_seen(self, &objects.checklist_items, AnyObject::ChecklistItem),
        }
    }

    /// Takes in an object just made or moved, so what hangs on it is seen
    /// too: a task in a project made in the same batch. Something not seen
    /// is left out, and a folder, project or task moved out is dropped.
    pub fn admit(&mut self, object: &AnyObject) {
        let seen = self.sees(object);
        let (set, id) = match object {
            AnyObject::Folder(folder) => (&mut self.folders, folder.id),
            AnyObject::Project(project) => (&mut self.projects, project.id),
            AnyObject::Task(task) => (&mut self.tasks, task.id),
            _ => return,
        };
        if seen {
            set.insert(id);
        } else {
            set.remove(&id);
        }
    }

    fn sees_folder(&self, folder: &Folder) -> bool {
        self.granted.contains(&folder.id)
            || folder.parent.is_some_and(|p| self.folders.contains(&p))
    }

    fn sees_project(&self, project: &Project) -> bool {
        self.granted.contains(&project.id)
            || project.folder.is_some_and(|f| self.folders.contains(&f))
    }

    /// A task by its project, or without one by its action group, as they
    /// are now seen; the inbox never.
    fn sees_task(&self, task: &Task) -> bool {
        match (task.project, task.parent) {
            (Some(project), _) => self.projects.contains(&project),
            (None, Some(parent)) => self.tasks.contains(&parent),
            (None, None) => false,
        }
    }

    /// A task by its project, or the project of the nearest action group
    /// above it that has one; for building the set, before the tasks are
    /// known.
    fn task_in_projects(&self, task: &Task, tasks: &HashMap<Id, &Task>) -> bool {
        let mut current = task;
        let mut seen = HashSet::new();
        loop {
            if let Some(project) = current.project {
                return self.projects.contains(&project);
            }
            let Some(parent) = current.parent else {
                return false;
            };
            if !seen.insert(parent) {
                return false;
            }
            let Some(next) = tasks.get(&parent) else {
                return false;
            };
            current = next;
        }
    }
}

/// The objects of one kind the reach sees.
fn keep_seen<T: Clone>(reach: &Reach, list: &[T], wrap: fn(T) -> AnyObject) -> Vec<T> {
    list.iter()
        .filter(|object| reach.sees(&wrap((*object).clone())))
        .cloned()
        .collect()
}

/// Whether a folder is granted or lies below one that is, at any depth; a
/// cycle in old data ends the walk.
fn below_granted(id: Id, parents: &HashMap<Id, Option<Id>>, granted: &HashSet<Id>) -> bool {
    let mut current = Some(id);
    let mut seen = HashSet::new();
    while let Some(folder) = current {
        if granted.contains(&folder) {
            return true;
        }
        if !seen.insert(folder) {
            return false;
        }
        current = parents.get(&folder).copied().flatten();
    }
    false
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use samu_api::{FolderStatus, ProjectKind, ProjectStatus, Schedule};
    use serde_json::json;

    use super::*;

    fn id(n: u8) -> Id {
        format!("01936f5e-8a00-7c3e-9b1a-0000000000{n:02x}")
            .parse()
            .unwrap()
    }

    fn at() -> jiff::Timestamp {
        "2026-11-13T08:00:00Z".parse().unwrap()
    }

    fn folder(n: u8, parent: Option<u8>) -> Folder {
        Folder {
            scope_only: false,
            own_scope: None,
            icon: None,
            id: id(n),
            created: at(),
            modified: at(),
            title: format!("folder {n}"),
            parent: parent.map(id),
            status: FolderStatus::Active,
            position: 0,
        }
    }

    fn project(n: u8, folder: Option<u8>) -> Project {
        Project {
            scope_only: false,
            own_scope: None,
            icon: None,
            created_by: None,
            id: id(n),
            created: at(),
            modified: at(),
            title: format!("project {n}"),
            note: None,
            folder: folder.map(id),
            workflow: None,
            kind: ProjectKind::Parallel,
            status: ProjectStatus::Active,
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

    fn task(n: u8, project: Option<u8>, parent: Option<u8>) -> Task {
        Task {
            created_by: None,
            habit: None,
            id: id(n),
            created: at(),
            modified: at(),
            title: format!("task {n}"),
            note: None,
            project: project.map(id),
            parent: parent.map(id),
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

    /// Something that hangs on `owner`, of a kind with that field.
    fn on(kind: &str, n: u8, owner: u8) -> AnyObject {
        let mut object = json!({
            "id": id(n), "created": at(), "modified": at(), "owner": id(owner),
        });
        let extra = match kind {
            "comment" => json!({"body": "x"}),
            "checklist_item" => json!({"text": "x", "position": 0}),
            "attachment" => json!({
                "blob": "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
                "name": "a.txt", "media_type": "text/plain", "size": 1
            }),
            "link" => json!({"uri": "https://example.org/"}),
            "clock" => json!({"start": at()}),
            "reminder" => json!({"anchor": "at", "at": at()}),
            other => panic!("no such kind {other}"),
        };
        for (key, value) in extra.as_object().unwrap() {
            object[key] = value.clone();
        }
        serde_json::from_value(json!({"kind": kind, "object": object})).unwrap()
    }

    fn any(kind: &str, object: serde_json::Value) -> AnyObject {
        serde_json::from_value(json!({"kind": kind, "object": object})).unwrap()
    }

    /// The account: folders 1 (tec) > 2 (ai) > 3 (deep), 1 > 4 (sibling);
    /// projects 10 in 2, 11 in 3, 12 in 4, 13 in 4 but granted, 14 at the
    /// root; tasks 20 in 10, 21 under 20 without a project, 22 under 21,
    /// 23 in 12, 24 in the inbox, 25 in 13.
    fn account() -> (Vec<Folder>, Vec<Project>, Vec<Task>) {
        (
            vec![
                folder(1, None),
                folder(2, Some(1)),
                folder(3, Some(2)),
                folder(4, Some(1)),
            ],
            vec![
                project(10, Some(2)),
                project(11, Some(3)),
                project(12, Some(4)),
                project(13, Some(4)),
                project(14, None),
            ],
            vec![
                task(20, Some(10), None),
                task(21, None, Some(20)),
                task(22, Some(10), Some(21)),
                task(23, Some(12), None),
                task(24, None, None),
                task(25, Some(13), None),
            ],
        )
    }

    fn reach(access: Access) -> Reach {
        let (folders, projects, tasks) = account();
        let grant = Grant {
            objects: vec![id(2), id(13)],
            access,
        };
        Reach::new(&grant, &folders, &projects, &tasks)
    }

    /// §2: a granted folder with every folder below it, the projects in
    /// them and the granted project, their tasks and subtasks; not the
    /// folder above, the sibling, a project elsewhere or the inbox.
    #[test]
    fn a_grant_reaches_down_and_no_further() {
        let reach = reach(Access::Ro);
        let (folders, projects, tasks) = account();
        let seen = |object: AnyObject| reach.sees(&object);
        let folder_seen: Vec<bool> = folders
            .into_iter()
            .map(|f| seen(AnyObject::Folder(f)))
            .collect();
        assert_eq!(folder_seen, [false, true, true, false]);
        let project_seen: Vec<bool> = projects
            .into_iter()
            .map(|p| seen(AnyObject::Project(p)))
            .collect();
        assert_eq!(project_seen, [true, true, false, true, false]);
        let task_seen: Vec<bool> = tasks
            .into_iter()
            .map(|t| seen(AnyObject::Task(t)))
            .collect();
        assert_eq!(task_seen, [true, true, true, false, false, true]);
        assert!(reach.sees_id(id(21)));
        assert!(!reach.sees_id(id(24)));
    }

    /// §2: what hangs on something seen is seen, on something else not;
    /// a thing on a tag is not, though the tag is.
    #[test]
    fn what_hangs_on_it_follows_its_owner() {
        let reach = reach(Access::Ro);
        for kind in [
            "comment",
            "attachment",
            "link",
            "clock",
            "reminder",
            "checklist_item",
        ] {
            assert!(reach.sees(&on(kind, 90, 22)), "{kind} on a subtask");
            assert!(reach.sees(&on(kind, 90, 13)), "{kind} on a granted project");
            assert!(reach.sees(&on(kind, 90, 3)), "{kind} on a folder below");
            assert!(!reach.sees(&on(kind, 90, 23)), "{kind} elsewhere");
            assert!(!reach.sees(&on(kind, 90, 24)), "{kind} in the inbox");
            assert!(!reach.sees(&on(kind, 90, 60)), "{kind} on a tag");
        }
        let entry = |habit: u8| {
            any(
                "habit_entry",
                json!({"id": id(91), "created": at(), "modified": at(),
                       "habit": id(habit), "day": "2026-11-13", "status": "done"}),
            )
        };
        assert!(reach.sees(&entry(20)));
        assert!(!reach.sees(&entry(23)));
    }

    /// §2: every tag is seen; scopes, settings, templates, perspectives,
    /// calendars, appointments and what this reader does not know are not.
    #[test]
    fn tags_are_seen_and_the_rest_of_the_account_is_not() {
        let reach = reach(Access::Rw);
        let tag = any(
            "tag",
            json!({"id": id(60), "created": at(), "modified": at(), "title": "Gebaut",
                   "status": "active", "exclusive": false, "position": 0}),
        );
        assert!(reach.sees(&tag));
        let scope = any(
            "scope",
            json!({"id": id(70), "created": at(), "modified": at(), "title": "AI",
                   "mode": "only", "members": [id(2)], "position": 0}),
        );
        assert!(!reach.sees(&scope));
        assert!(!reach.sees(&AnyObject::Unknown));
    }

    /// §3: `rw` writes what is seen and stays seen, but never a tag, not
    /// even a new one; `ro` writes nothing.
    #[test]
    fn rw_writes_inside_and_ro_nothing() {
        let rw = reach(Access::Rw);
        let ro = reach(Access::Ro);
        let inside = AnyObject::Task(task(20, Some(10), None));
        assert!(rw.may_write(Some(&inside), &inside));
        assert!(!ro.may_write(Some(&inside), &inside));
        let comment = on("comment", 90, 20);
        assert!(rw.may_write(None, &comment));
        assert!(!ro.may_write(None, &comment));
        let tag = any(
            "tag",
            json!({"id": id(60), "created": at(), "modified": at(), "title": "neu",
                   "status": "active", "exclusive": false, "position": 0}),
        );
        assert!(!rw.may_write(None, &tag));
        assert!(!rw.may_write(Some(&tag), &tag));
        let outside = AnyObject::Task(task(23, Some(12), None));
        assert!(!rw.may_write(Some(&outside), &outside));
        let inbox = AnyObject::Task(task(30, None, None));
        assert!(!rw.may_write(None, &inbox), "a new task in the inbox");
    }

    /// §3: an object made or moved must lie in the grant afterwards; out
    /// and in are both refused.
    #[test]
    fn moving_across_the_edge_is_refused() {
        let rw = reach(Access::Rw);
        let here = AnyObject::Task(task(20, Some(10), None));
        let there = AnyObject::Task(task(20, Some(12), None));
        assert!(!rw.may_write(Some(&here), &there), "out");
        assert!(!rw.may_write(Some(&there), &here), "in");
        let deeper = AnyObject::Task(task(20, Some(11), None));
        assert!(rw.may_write(Some(&here), &deeper), "within");
        let folder_out = AnyObject::Folder(folder(3, Some(4)));
        assert!(!rw.may_write(Some(&AnyObject::Folder(folder(3, Some(2)))), &folder_out));
        let new_project = AnyObject::Project(project(15, Some(3)));
        assert!(rw.may_write(None, &new_project));
    }

    /// A granted folder or project keeps its place: it would stay seen,
    /// but lie elsewhere in the account. Renaming it is fine, and so is
    /// moving a folder below it.
    #[test]
    fn a_granted_root_stays_where_it_is() {
        let rw = reach(Access::Rw);
        let root = AnyObject::Folder(folder(2, Some(1)));
        let moved = AnyObject::Folder(folder(2, Some(4)));
        assert!(rw.sees(&moved), "still seen by its id");
        assert!(!rw.may_write(Some(&root), &moved));
        let mut renamed = folder(2, Some(1));
        renamed.title = "ki".into();
        assert!(rw.may_write(Some(&root), &AnyObject::Folder(renamed)));
        let granted = AnyObject::Project(project(13, Some(4)));
        let away = AnyObject::Project(project(13, None));
        assert!(!rw.may_write(Some(&granted), &away));
        let below = AnyObject::Folder(folder(3, Some(2)));
        let up = AnyObject::Folder(folder(3, Some(2)));
        assert!(rw.may_write(Some(&below), &up));
        assert!(
            rw.may_write(Some(&root), &root),
            "a delete asks with the object on both sides"
        );
    }

    /// `restrict` keeps what is seen, all tags, and none of the rest.
    #[test]
    fn restrict_keeps_what_is_seen() {
        let (folders, projects, tasks) = account();
        let reach = reach(Access::Ro);
        let mut objects = Objects {
            folders,
            projects,
            tasks,
            ..Objects::default()
        };
        objects.put(on("comment", 90, 20));
        objects.put(on("comment", 91, 23));
        objects.put(any(
            "scope",
            json!({"id": id(70), "created": at(), "modified": at(), "title": "AI",
                   "mode": "only", "members": [id(2)], "position": 0}),
        ));
        objects.put(any(
            "tag",
            json!({"id": id(60), "created": at(), "modified": at(), "title": "Gebaut",
                   "status": "active", "exclusive": false, "position": 0}),
        ));
        let seen = reach.restrict(&objects);
        let ids = |list: Vec<Id>| list;
        assert_eq!(
            ids(seen.folders.iter().map(|f| f.id).collect()),
            [id(2), id(3)]
        );
        assert_eq!(
            ids(seen.projects.iter().map(|p| p.id).collect()),
            [id(10), id(11), id(13)]
        );
        assert_eq!(
            ids(seen.tasks.iter().map(|t| t.id).collect()),
            [id(20), id(21), id(22), id(25)]
        );
        assert_eq!(ids(seen.comments.iter().map(|c| c.id).collect()), [id(90)]);
        assert_eq!(seen.tags.len(), 1);
        assert!(seen.scopes.is_empty());
    }

    /// A project made in a batch takes in its tasks once admitted; a task
    /// moved out is dropped, and what hangs on it with it.
    #[test]
    fn admit_follows_new_and_moved_objects() {
        let mut rw = reach(Access::Rw);
        let new_project = AnyObject::Project(project(15, Some(3)));
        let new_task = AnyObject::Task(task(31, Some(15), None));
        assert!(!rw.sees(&new_task));
        rw.admit(&new_project);
        assert!(rw.sees(&new_task));
        rw.admit(&new_task);
        assert!(rw.sees(&on("comment", 90, 31)));
        rw.admit(&AnyObject::Task(task(31, Some(12), None)));
        assert!(!rw.sees(&on("comment", 90, 31)));
        rw.admit(&on("comment", 90, 20));
        assert!(!rw.sees_id(id(90)), "only folders, projects and tasks");
    }

    /// Old data with a cycle in its folders or tasks ends the walk.
    #[test]
    fn a_cycle_ends_the_walk() {
        let folders = vec![folder(1, Some(2)), folder(2, Some(1))];
        let tasks = vec![task(20, None, Some(21)), task(21, None, Some(20))];
        let grant = Grant {
            objects: vec![id(9)],
            access: Access::Rw,
        };
        let reach = Reach::new(&grant, &folders, &[], &tasks);
        assert!(!reach.sees_id(id(1)));
        assert!(!reach.sees_id(id(20)));
    }
}
