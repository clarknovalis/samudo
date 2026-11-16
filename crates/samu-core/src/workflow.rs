// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Workflows (ADR 0024): an exclusive tag group whose children, in order,
//! are the states. Which workflow a task follows, its states, the current
//! one and the next one when cycling. Pure functions for every client; the
//! server decides the exclusion itself (samu-store).

use crate::api::{Id, Project, Tag, TagStatus, Tagging};

/// The workflow of a project (ADR 0024 §3): its own, else the default group
/// with the smallest position, then the smallest id. A task in the inbox
/// passes `None` and gets the default.
#[must_use]
pub fn workflow_of(project: Option<&Project>, tags: &[Tag]) -> Option<Id> {
    if let Some(own) = project.and_then(|p| p.workflow) {
        return Some(own);
    }
    tags.iter()
        .filter(|t| t.exclusive && t.default_workflow && t.status != TagStatus::Dropped)
        .min_by_key(|t| (t.position, t.id))
        .map(|t| t.id)
}

/// The states of a workflow, in order: the group's children by position,
/// then id; dropped ones left out.
#[must_use]
pub fn states(group: Id, tags: &[Tag]) -> Vec<Id> {
    let mut children: Vec<&Tag> = tags
        .iter()
        .filter(|t| t.parent == Some(group) && t.status != TagStatus::Dropped)
        .collect();
    children.sort_by_key(|t| (t.position, t.id));
    children.into_iter().map(|t| t.id).collect()
}

/// The state a task is in: the first of its tags that is a state. Old data
/// may carry two; the first by the task's own order counts.
#[must_use]
pub fn current_state(taggings: &[Tagging], states: &[Id]) -> Option<Id> {
    let mut mine: Vec<&Tagging> = taggings
        .iter()
        .filter(|t| states.contains(&t.tag))
        .collect();
    mine.sort_by_key(|t| t.position);
    mine.first().map(|t| t.tag)
}

/// Cycling (ADR 0024 §4): no state → first → … → last → no state. A state
/// that is not in the list starts over at the first.
#[must_use]
pub fn next_state(states: &[Id], current: Option<Id>) -> Option<Id> {
    match current.and_then(|c| states.iter().position(|s| *s == c)) {
        None => states.first().copied(),
        Some(i) => states.get(i + 1).copied(),
    }
}

/// The state picked by its key (ADR 0024 §4).
#[must_use]
pub fn state_by_key(states: &[Id], tags: &[Tag], key: &str) -> Option<Id> {
    states.iter().copied().find(|id| {
        tags.iter()
            .any(|t| t.id == *id && t.key.as_deref() == Some(key))
    })
}

/// A task in its workflow (ADR 0024, ADR 0025 §3).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Placement {
    pub workflow: Option<Id>,
    pub state: Option<Id>,
    /// Where cycling takes it; none is out of every state.
    pub next: Option<Id>,
}

/// The workflow of a task's project, its state there and the state cycling
/// takes it to. All none without a workflow.
#[must_use]
pub fn placement(taggings: &[Tagging], project: Option<&Project>, tags: &[Tag]) -> Placement {
    let Some(group) = workflow_of(project, tags) else {
        return Placement::default();
    };
    let states = states(group, tags);
    let state = current_state(taggings, &states);
    Placement {
        workflow: Some(group),
        state,
        next: next_state(&states, state),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::*;
    use crate::api::{ProjectKind, ProjectStatus, Schedule};

    fn ts() -> jiff::Timestamp {
        "2026-11-07T08:00:00Z".parse().unwrap()
    }

    fn tag(title: &str, parent: Option<Id>, position: i64) -> Tag {
        Tag {
            scope_only: false,
            own_scope: None,
            id: Id::new(),
            created: ts(),
            modified: ts(),
            title: title.into(),
            parent,
            status: TagStatus::Active,
            exclusive: false,
            key: None,
            ask_note: false,
            default_workflow: false,
            position,
        }
    }

    fn project(workflow: Option<Id>) -> Project {
        Project {
            scope_only: false,
            own_scope: None,
            icon: None,
            created_by: None,
            id: Id::new(),
            created: ts(),
            modified: ts(),
            title: "p".into(),
            note: None,
            folder: None,
            workflow,
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

    /// `Status` (default) with `In Arbeit`, `Wartet`, `Irgendwann`, and a
    /// second group `Bugs`.
    fn setup() -> (Vec<Tag>, Id, Id, Vec<Id>) {
        let mut status = tag("Status", None, 1);
        status.exclusive = true;
        status.default_workflow = true;
        let mut bugs = tag("Bugs", None, 2);
        bugs.exclusive = true;
        // Out of order on purpose: position decides.
        let mut waiting = tag("Wartet", Some(status.id), 1);
        waiting.key = Some("w".into());
        let working = tag("In Arbeit", Some(status.id), 0);
        let someday = tag("Irgendwann", Some(status.id), 2);
        let mut gone = tag("Alt", Some(status.id), 3);
        gone.status = TagStatus::Dropped;
        let order = vec![working.id, waiting.id, someday.id];
        let (s, b) = (status.id, bugs.id);
        (
            vec![status, bugs, waiting, working, someday, gone],
            s,
            b,
            order,
        )
    }

    #[test]
    fn a_project_names_its_workflow_or_takes_the_default() {
        let (tags, status, bugs, _) = setup();
        assert_eq!(workflow_of(Some(&project(Some(bugs))), &tags), Some(bugs));
        assert_eq!(workflow_of(Some(&project(None)), &tags), Some(status));
        assert_eq!(workflow_of(None, &tags), Some(status));
        assert_eq!(workflow_of(None, &[]), None);
    }

    #[test]
    fn two_defaults_are_decided_by_position() {
        let (mut tags, status, bugs, _) = setup();
        tags.iter_mut()
            .find(|t| t.id == bugs)
            .unwrap()
            .default_workflow = true;
        assert_eq!(workflow_of(None, &tags), Some(status));
        tags.iter_mut().find(|t| t.id == bugs).unwrap().position = 0;
        assert_eq!(workflow_of(None, &tags), Some(bugs));
    }

    #[test]
    fn states_are_ordered_and_dropped_ones_left_out() {
        let (tags, status, _, order) = setup();
        assert_eq!(states(status, &tags), order);
    }

    #[test]
    fn cycling_runs_round_to_no_state() {
        let (tags, status, _, order) = setup();
        let states = states(status, &tags);
        let mut seen = Vec::new();
        let mut current = None;
        for _ in 0..4 {
            current = next_state(&states, current);
            seen.push(current);
        }
        assert_eq!(
            seen,
            vec![Some(order[0]), Some(order[1]), Some(order[2]), None]
        );
        assert_eq!(next_state(&states, Some(Id::new())), Some(order[0]));
        assert_eq!(next_state(&[], None), None);
    }

    #[test]
    fn a_placement_names_workflow_state_and_next() {
        let (tags, status, _, order) = setup();
        let taggings = vec![Tagging {
            tag: order[2],
            position: 0,
        }];
        assert_eq!(
            placement(&taggings, None, &tags),
            Placement {
                workflow: Some(status),
                state: Some(order[2]),
                next: None
            }
        );
        assert_eq!(placement(&[], None, &tags).next, Some(order[0]));
        assert_eq!(placement(&[], None, &[]), Placement::default());
    }

    #[test]
    fn the_current_state_and_a_key() {
        let (tags, status, _, order) = setup();
        let states = states(status, &tags);
        let other = Id::new();
        let taggings = vec![
            Tagging {
                tag: other,
                position: 0,
            },
            Tagging {
                tag: order[1],
                position: 1,
            },
        ];
        assert_eq!(current_state(&taggings, &states), Some(order[1]));
        assert_eq!(current_state(&[], &states), None);
        assert_eq!(state_by_key(&states, &tags, "w"), Some(order[1]));
        assert_eq!(state_by_key(&states, &tags, "x"), None);
    }
}
