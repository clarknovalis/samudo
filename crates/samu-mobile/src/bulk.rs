// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! One action on many tasks (ADR 0052 §6): what the bar of the app needs
//! beyond calling the functions for one task in a row. Moving and deleting
//! leave out a task whose ancestor is picked as well (§3); a state is
//! chosen by its title, in each task's own workflow (§2).

use std::collections::HashSet;

use samu_api::{Change, Id};
use samu_core::overlay::Objects;
use samu_core::workflow::{current_state, states, workflow_of};
use serde_json::{Value, json};

use crate::edit::{TagOption, descendants, find, set};
use crate::{DeleteCount, MobileError, Samu, input, parse_id};

/// What the bar offers for the picked tasks.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BulkEdit {
    /// The ordinary tags; `on` when every picked task carries it.
    pub tags: Vec<TagOption>,
    /// The titles of the states of their workflows, each once, in order.
    pub states: Vec<String>,
    /// How many tasks a delete takes, the descendants counted.
    pub deleted: u64,
}

/// The ids without those below another of them, in the order given.
fn roots(objects: &Objects, ids: &[Id]) -> Vec<Id> {
    let below: HashSet<Id> = ids
        .iter()
        .flat_map(|&id| descendants(objects, id))
        .collect();
    ids.iter()
        .copied()
        .filter(|id| !below.contains(id))
        .collect()
}

fn parse_all(ids: &[String]) -> Result<Vec<Id>, MobileError> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for id in ids {
        let id = parse_id(id)?;
        if seen.insert(id) {
            out.push(id);
        }
    }
    Ok(out)
}

#[uniffi::export]
impl Samu {
    pub fn bulk_edit(&self, ids: Vec<String>) -> Result<BulkEdit, MobileError> {
        let ids = parse_all(&ids)?;
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let names = crate::Names::new(&objects);
        let tasks = ids
            .iter()
            .map(|&id| find(&objects, id))
            .collect::<Result<Vec<_>, _>>()?;
        let ordinary = Self::ordinary_tags(&objects);
        let mut tags: Vec<TagOption> = objects
            .tags
            .iter()
            .filter(|t| ordinary.contains(&t.id))
            .map(|t| TagOption {
                id: t.id.to_string(),
                path: names.tag_path(t.id),
                on: !tasks.is_empty()
                    && tasks
                        .iter()
                        .all(|task| task.tags.iter().any(|x| x.tag == t.id)),
            })
            .collect();
        tags.sort_by_key(|t| t.path.to_lowercase());
        let mut titles: Vec<String> = Vec::new();
        for task in &tasks {
            let project = task
                .project
                .and_then(|p| objects.projects.iter().find(|x| x.id == p));
            if let Some(group) = workflow_of(project, &objects.tags) {
                for state in states(group, &objects.tags) {
                    let title = names
                        .tag_path(state)
                        .rsplit('/')
                        .next()
                        .unwrap_or("")
                        .to_string();
                    if !titles.contains(&title) {
                        titles.push(title);
                    }
                }
            }
        }
        let deleted = roots(&objects, &ids)
            .iter()
            .map(|&id| 1 + descendants(&objects, id).len() as u64)
            .sum();
        Ok(BulkEdit {
            tags,
            states: titles,
            deleted,
        })
    }

    /// Into a project or the inbox (none), each last at the top level in
    /// the order given, in one write; descendants go along (ADR 0040 §3).
    pub fn move_tasks(&self, ids: Vec<String>, project: Option<String>) -> Result<(), MobileError> {
        let ids = parse_all(&ids)?;
        let project = project.as_deref().map(parse_id).transpose()?;
        self.queue(|objects, stamp| {
            for &id in &ids {
                find(objects, id)?;
            }
            if let Some(p) = project
                && !objects.projects.iter().any(|x| x.id == p)
            {
                return Err(input(format!("no project {p}")));
            }
            let top = roots(objects, &ids);
            let moving: HashSet<Id> = top.iter().copied().collect();
            let first = objects
                .tasks
                .iter()
                .filter(|t| !moving.contains(&t.id) && t.project == project && t.parent.is_none())
                .map(|t| t.position + 1)
                .max()
                .unwrap_or(0);
            let target = project.map_or(Value::Null, |p| json!(p));
            let mut changes = Vec::new();
            for (id, position) in top.iter().zip(first..) {
                changes.push(set(*id, "project", target.clone(), stamp()));
                changes.push(set(*id, "parent", Value::Null, stamp()));
                changes.push(set(*id, "position", json!(position), stamp()));
                for child in descendants(objects, *id) {
                    changes.push(set(child, "project", target.clone(), stamp()));
                }
            }
            Ok(changes)
        })
    }

    /// Each task into the state titled `title` of its own workflow, or out
    /// of every state with none, in one write. Answers how many have no
    /// such state and stay as they are.
    pub fn set_states_titled(
        &self,
        ids: Vec<String>,
        title: Option<String>,
    ) -> Result<u32, MobileError> {
        let ids = parse_all(&ids)?;
        let mut missed = 0u32;
        self.queue(|objects, stamp| {
            let names = crate::Names::new(objects);
            let mut changes = Vec::new();
            for &id in &ids {
                let task = find(objects, id)?;
                let project = task
                    .project
                    .and_then(|p| objects.projects.iter().find(|x| x.id == p));
                let all = workflow_of(project, &objects.tags)
                    .map(|group| states(group, &objects.tags))
                    .unwrap_or_default();
                let now = current_state(&task.tags, &all);
                let wanted = match &title {
                    None => None,
                    Some(title) => {
                        let found = all.iter().copied().find(|&s| {
                            names.tag_path(s).rsplit('/').next() == Some(title.as_str())
                        });
                        if found.is_none() {
                            missed += 1;
                            continue;
                        }
                        found
                    }
                };
                match (wanted, now) {
                    (Some(new), Some(old)) if new == old => {}
                    (Some(new), _) => changes.push(Change::Add {
                        object: id,
                        field: "tags".into(),
                        element: new,
                        position: 0,
                        note: None,
                        hlc: stamp(),
                    }),
                    (None, Some(old)) => changes.push(Change::Remove {
                        object: id,
                        field: "tags".into(),
                        element: old,
                        note: None,
                        hlc: stamp(),
                    }),
                    (None, None) => {}
                }
            }
            Ok(changes)
        })?;
        Ok(missed)
    }

    /// Deletes the tasks with their descendants through the server, one
    /// after another; with `preview` only counts. Needs the server.
    pub fn delete_tasks(
        &self,
        ids: Vec<String>,
        preview: bool,
    ) -> Result<DeleteCount, MobileError> {
        let ids = parse_all(&ids)?;
        let top = {
            let cache = self.cache()?;
            let (objects, _) = Self::view(&cache)?;
            for &id in &ids {
                find(&objects, id)?;
            }
            roots(&objects, &ids)
        };
        let mut total = DeleteCount {
            deleted: 0,
            moved: 0,
            untagged: 0,
        };
        for id in top {
            let one = self.delete_task(id.to_string(), preview)?;
            total.deleted += one.deleted;
            total.moved += one.moved;
            total.untagged += one.untagged;
        }
        Ok(total)
    }
}
