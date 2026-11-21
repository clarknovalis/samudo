// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `samu done`, `samu drop` and their bulk forms through
//! `POST /objects/{id}/close` (ADR 0128): with open tasks below, nothing is
//! written until `--children` says what happens to them.

use std::collections::HashSet;
use std::fmt;

use crate::client::Client;
use anyhow::Result;
use clap::ValueEnum;
use samu_api::{ChildFate, CloseField, CloseRequest, CloseResponse, Id, Task};

use crate::short;

/// What happens to the open tasks below (ADR 0128 §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Children {
    /// Complete (or drop) them along with the task.
    Close,
    /// Move the open direct children one level up, right after the task.
    Lift,
}

impl From<Children> for ChildFate {
    fn from(children: Children) -> Self {
        match children {
            Children::Close => Self::Close,
            Children::Lift => Self::Lift,
        }
    }
}

/// Some task has open tasks below and no `--children` was given; nothing
/// was written. Exit code [`crate::exit::NEEDS_CHOICE`].
#[derive(Debug)]
pub(crate) struct NeedsChoice(pub(crate) usize);

impl fmt::Display for NeedsChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let tasks = if self.0 == 1 {
            "a task has"
        } else {
            "tasks have"
        };
        write!(
            f,
            "{tasks} open tasks below; nothing changed. Give --children close or --children lift"
        )
    }
}

impl std::error::Error for NeedsChoice {}

fn call(client: &Client, id: Id, request: &CloseRequest) -> Result<CloseResponse> {
    let answer = client.call(
        "POST",
        &format!("/objects/{id}/close"),
        Some(&serde_json::to_value(request)?),
    )?;
    Ok(serde_json::from_value(answer.json()?)?)
}

/// Closes each of `ids` as `field` says. Without `children`, nothing at
/// all is written when any has open tasks below; those are listed on
/// stderr. With `tasks`, the account as the caller read it (bulk), an
/// open task below counts only when it is not among `ids` itself, and the
/// deepest go first, so a group closes after its picked children; without,
/// the server is asked. Answers with what each close did.
pub(crate) fn close(
    client: &Client,
    ids: &[Id],
    field: CloseField,
    children: Option<Children>,
    note: Option<String>,
    tasks: Option<&[Task]>,
) -> Result<Vec<(Id, CloseResponse)>> {
    let request = CloseRequest {
        field,
        children: children.map_or(ChildFate::Ask, ChildFate::from),
        note,
        preview: false,
    };
    let mut ordered = ids.to_vec();
    if let Some(tasks) = tasks {
        ordered.sort_by_key(|id| std::cmp::Reverse(depth(tasks, *id)));
    }
    if children.is_none() {
        let picked: HashSet<Id> = ids.iter().copied().collect();
        let mut waiting = Vec::new();
        for &id in &ordered {
            let waits = match tasks {
                Some(tasks) => open_below(tasks, id)
                    .iter()
                    .any(|(_, t)| !picked.contains(&t.id)),
                None => {
                    let preview = CloseRequest {
                        preview: true,
                        ..request.clone()
                    };
                    call(client, id, &preview)?.open > 0
                }
            };
            if waits {
                waiting.push(id);
            }
        }
        if !waiting.is_empty() {
            let fetched;
            let tasks = match tasks {
                Some(tasks) => tasks,
                None => {
                    fetched = crate::tags::snapshot(client)?.tasks;
                    &fetched
                }
            };
            for &id in &waiting {
                let title = tasks.iter().find(|t| t.id == id).map_or("", |t| &t.title);
                eprintln!("{} {title}", short(&id));
                for (depth, t) in open_below(tasks, id) {
                    eprintln!("{}{} {}", "  ".repeat(depth), short(&t.id), t.title);
                }
            }
            return Err(NeedsChoice(waiting.len()).into());
        }
    }
    let mut answers = Vec::new();
    for id in ordered {
        answers.push((id, call(client, id, &request)?));
    }
    Ok(answers)
}

/// How many groups are above `id`.
fn depth(tasks: &[Task], id: Id) -> usize {
    let mut seen = HashSet::from([id]);
    let mut current = id;
    let mut depth = 0;
    while let Some(parent) = tasks
        .iter()
        .find(|t| t.id == current)
        .and_then(|t| t.parent)
    {
        // A cycle in broken data stops here.
        if !seen.insert(parent) {
            break;
        }
        depth += 1;
        current = parent;
    }
    depth
}

/// The open tasks below `id` at any depth, in order, with their depth.
fn open_below(tasks: &[Task], id: Id) -> Vec<(usize, &Task)> {
    let mut found = Vec::new();
    let mut seen: HashSet<Id> = HashSet::from([id]);
    let mut stack: Vec<(Id, usize)> = vec![(id, 0)];
    while let Some((current, depth)) = stack.pop() {
        let mut below: Vec<&Task> = tasks.iter().filter(|t| t.parent == Some(current)).collect();
        below.sort_by_key(|t| std::cmp::Reverse((t.position, t.id)));
        for t in below {
            if seen.insert(t.id) {
                stack.push((t.id, depth + 1));
            }
        }
        if current != id
            && let Some(t) = tasks.iter().find(|t| t.id == current)
            && t.completed_at.is_none()
            && t.dropped_at.is_none()
        {
            found.push((depth, t));
        }
    }
    found
}

/// One line on stderr for what the closes did, when more than the tasks
/// themselves changed.
pub(crate) fn report(answers: &[(Id, CloseResponse)], verb: &str) {
    let (closed, lifted) = answers
        .iter()
        .fold((0, 0), |(c, l), (_, a)| (c + a.closed, l + a.lifted));
    if closed as usize > answers.len() || lifted > 0 {
        eprintln!("{closed} tasks {verb}, {lifted} lifted out");
    }
}
