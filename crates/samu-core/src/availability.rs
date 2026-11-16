// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Availability, computed and never stored (ADR 0006, ADR 0017).
//!
//! One pure function over a snapshot of an account: the server and the phone
//! (through samu-mobile) call the same code, so every interface shows the same
//! answer. It reads nothing itself; "now" and the account's zone come in.

use std::collections::{HashMap, HashSet};

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp, ToSpan, Zoned};

use crate::api::{
    Clock, Folder, FolderStatus, GroupSummary, Id, Link, LinkKind, Project, ProjectKind,
    ProjectStatus, Scope, Tag, TagStatus, Tagging, Task, When,
};

/// Everything of one account the computation looks at. Tombstones are not
/// in here.
#[derive(Debug, Clone, Copy, Default)]
pub struct Snapshot<'a> {
    pub folders: &'a [Folder],
    pub projects: &'a [Project],
    pub tasks: &'a [Task],
    pub tags: &'a [Tag],
    pub links: &'a [Link],
    /// ADR 0046: `ist:geclockt` and the report read them.
    pub clocks: &'a [Clock],
    /// ADR 0094 §5: an `own_scope` counts only when the scope exists.
    pub scopes: &'a [Scope],
}

/// The account's settings the computation needs.
#[derive(Debug, Clone)]
pub struct Settings {
    /// Decides what "today" is and when a date is reached (ADR 0007 §2).
    pub zone: TimeZone,
    /// How long before `due` a task counts as due soon, unless it has its own
    /// `due_warning`.
    pub due_soon: Span,
    /// When a reminder on a date without a time goes off (ADR 0030 §3).
    pub remind_time: jiff::civil::Time,
}

impl Settings {
    /// The default warning: two days, like OmniFocus (ADR 0017 §2).
    #[must_use]
    pub fn new(zone: TimeZone) -> Self {
        Self {
            zone,
            due_soon: 2.days(),
            remind_time: jiff::civil::time(9, 0, 0, 0),
        }
    }
}

/// Why something is not available. Every reason that applies is listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    Completed,
    Dropped,
    /// The defer date is not reached yet, its own or one inherited from the
    /// project.
    Deferred {
        until: When,
    },
    /// The project is on hold, completed or dropped.
    ProjectNotActive {
        project: Id,
    },
    /// A folder above is dropped.
    FolderDropped {
        folder: Id,
    },
    /// A tag on it, or a tag above that one, is on hold or dropped.
    TagNotActive {
        tag: Id,
    },
    /// In a sequential project or group, another action comes first.
    WaitingInSequence {
        first: Id,
    },
    /// It is an action group with open children: they are next, not it.
    HasOpenChildren,
    /// The action group it is in is not available.
    ContainerUnavailable {
        id: Id,
    },
    /// A task it is blocked by is still open.
    BlockedBy {
        task: Id,
    },
    /// A value from a newer samu, or a reference that leads nowhere it may.
    UnknownValue,
    /// Broken data: a chain of parents, folders or tags runs in a circle.
    Cycle,
}

/// How close the due date is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DueState {
    #[default]
    None,
    Soon,
    Overdue,
}

/// The computed state of one task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskState {
    pub available: bool,
    pub reasons: Vec<Reason>,
    pub due: DueState,
    /// Planned for today or earlier, and open.
    pub today: bool,
    /// In the inbox: no project, or one the snapshot does not hold
    /// (ADR 0036 §6).
    pub inbox: bool,
}

/// The project a task is in, in effect: none, the inbox, when `project` is
/// empty or leads to a project that is not there, deleted or never known
/// (ADR 0036 §6). Computed, never written.
#[must_use]
pub fn home<P>(task: &Task, projects: &HashMap<Id, P>) -> Option<Id> {
    task.project
        .filter(|project| projects.contains_key(project))
}

/// The computed state of one project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectState {
    pub available: bool,
    pub reasons: Vec<Reason>,
    /// Available, but with neither an available nor a deferred open action.
    pub stuck: bool,
    /// The date of the next review; none without a review interval or for
    /// a completed or dropped project (ADR 0088 §1).
    pub review: Option<Date>,
    /// Today has reached `review`.
    pub review_due: bool,
}

/// The date a project is next up for review (ADR 0088 §1): `next_review`
/// when set by hand, else the local date of `last_reviewed` (without it,
/// of `created`) plus `review_interval`. None without an interval, and for
/// a project that is neither active nor on hold.
#[must_use]
pub fn review_date(project: &Project, zone: &TimeZone) -> Option<Date> {
    let interval = project.review_interval?;
    if !matches!(
        project.status,
        ProjectStatus::Active | ProjectStatus::OnHold
    ) {
        return None;
    }
    if let Some(date) = project.next_review {
        return Some(date);
    }
    let base = project.last_reviewed.unwrap_or(project.created);
    base.to_zoned(zone.clone())
        .date()
        .checked_add(interval.0)
        .ok()
}

/// The result for a whole snapshot.
#[derive(Debug, Clone, Default)]
pub struct Evaluation {
    pub tasks: HashMap<Id, TaskState>,
    pub projects: HashMap<Id, ProjectState>,
}

/// Computes availability for every task and project in `snapshot`.
#[must_use]
pub fn evaluate(snapshot: &Snapshot<'_>, now: Timestamp, settings: &Settings) -> Evaluation {
    let mut run = Run::new(snapshot, now, settings);

    let project_reasons: HashMap<Id, Vec<Reason>> = snapshot
        .projects
        .iter()
        .map(|project| (project.id, run.project_reasons(project)))
        .collect();
    run.project_reasons = project_reasons;

    let mut evaluation = Evaluation::default();
    for task in snapshot.tasks {
        let reasons = run.task_reasons(task.id);
        let open = is_open(task);
        let state = TaskState {
            available: reasons.is_empty(),
            due: if open {
                run.due_state(task)
            } else {
                DueState::None
            },
            today: open && run.planned_today(task),
            inbox: home(task, &run.projects).is_none(),
            reasons,
        };
        evaluation.tasks.insert(task.id, state);
    }

    for project in snapshot.projects {
        let reasons = run.project_reasons[&project.id].clone();
        let available = reasons.is_empty();
        let stuck = available
            && project.kind != ProjectKind::SingleActions
            && !snapshot.tasks.iter().any(|task| {
                task.project == Some(project.id)
                    && is_open(task)
                    && evaluation.tasks.get(&task.id).is_some_and(|state| {
                        state.available
                            || state
                                .reasons
                                .iter()
                                .any(|reason| matches!(reason, Reason::Deferred { .. }))
                    })
            });
        let review = review_date(project, &run.settings.zone);
        evaluation.projects.insert(
            project.id,
            ProjectState {
                available,
                reasons,
                stuck,
                review,
                review_due: review.is_some_and(|date| date <= run.today),
            },
        );
    }
    evaluation
}

fn is_open(task: &Task) -> bool {
    task.completed_at.is_none() && task.dropped_at.is_none()
}

fn push(reasons: &mut Vec<Reason>, reason: Reason) {
    if !reasons.contains(&reason) {
        reasons.push(reason);
    }
}

/// Indexes and memo for one call of [`evaluate`].
struct Run<'a> {
    now: Timestamp,
    today: Date,
    settings: &'a Settings,
    tasks: HashMap<Id, &'a Task>,
    projects: HashMap<Id, &'a Project>,
    folders: HashMap<Id, &'a Folder>,
    tags: HashMap<Id, &'a Tag>,
    /// Children of an action group, in order.
    children: HashMap<Id, Vec<&'a Task>>,
    /// Tasks of a project without a parent, in order.
    top_level: HashMap<Id, Vec<&'a Task>>,
    /// `blocked_by` targets per owner.
    blockers: HashMap<Id, Vec<Id>>,
    project_reasons: HashMap<Id, Vec<Reason>>,
    memo: HashMap<Id, Vec<Reason>>,
    visiting: HashSet<Id>,
}

impl<'a> Run<'a> {
    fn new(snapshot: &Snapshot<'a>, now: Timestamp, settings: &'a Settings) -> Self {
        let projects: HashMap<Id, &Project> = snapshot.projects.iter().map(|p| (p.id, p)).collect();
        let mut children: HashMap<Id, Vec<&Task>> = HashMap::new();
        let mut top_level: HashMap<Id, Vec<&Task>> = HashMap::new();
        for task in snapshot.tasks {
            match (task.parent, home(task, &projects)) {
                (Some(parent), _) => children.entry(parent).or_default().push(task),
                (None, Some(project)) => top_level.entry(project).or_default().push(task),
                (None, None) => {}
            }
        }
        for list in children.values_mut().chain(top_level.values_mut()) {
            list.sort_by_key(|task| (task.position, task.id));
        }

        let mut blockers: HashMap<Id, Vec<Id>> = HashMap::new();
        for link in snapshot.links {
            if link.kind == LinkKind::BlockedBy
                && let Some(target) = Id::from_uri(&link.uri)
            {
                blockers.entry(link.owner).or_default().push(target);
            }
        }

        Self {
            now,
            today: now.to_zoned(settings.zone.clone()).date(),
            settings,
            tasks: snapshot.tasks.iter().map(|t| (t.id, t)).collect(),
            projects,
            folders: snapshot.folders.iter().map(|f| (f.id, f)).collect(),
            tags: snapshot.tags.iter().map(|t| (t.id, t)).collect(),
            children,
            top_level,
            blockers,
            project_reasons: HashMap::new(),
            memo: HashMap::new(),
            visiting: HashSet::new(),
        }
    }

    /// Whether a defer date is reached; `None` for a form this build does
    /// not know.
    fn reached(&self, when: &When) -> Option<bool> {
        match when {
            When::Date(date) => Some(self.today >= *date),
            When::At(at) => Some(self.now >= at.timestamp()),
            When::Unknown => None,
        }
    }

    fn check_defer(&self, defer: Option<&When>, reasons: &mut Vec<Reason>) {
        if let Some(defer) = defer {
            match self.reached(defer) {
                Some(true) => {}
                Some(false) => push(
                    reasons,
                    Reason::Deferred {
                        until: defer.clone(),
                    },
                ),
                None => push(reasons, Reason::UnknownValue),
            }
        }
    }

    fn check_folders(&self, mut folder: Option<Id>, reasons: &mut Vec<Reason>) {
        let mut seen = HashSet::new();
        while let Some(id) = folder {
            if !seen.insert(id) {
                push(reasons, Reason::Cycle);
                return;
            }
            // A folder missing from the snapshot is treated as the top.
            let Some(current) = self.folders.get(&id) else {
                return;
            };
            match current.status {
                FolderStatus::Active => {}
                FolderStatus::Dropped => push(reasons, Reason::FolderDropped { folder: id }),
                FolderStatus::Unknown => push(reasons, Reason::UnknownValue),
            }
            folder = current.parent;
        }
    }

    /// A tag passes its status down to its children (ADR 0017 §4).
    fn check_tags(&self, tags: &[Tagging], reasons: &mut Vec<Reason>) {
        for tagging in tags {
            let mut seen = HashSet::new();
            let mut tag = Some(tagging.tag);
            while let Some(id) = tag {
                if !seen.insert(id) {
                    push(reasons, Reason::Cycle);
                    break;
                }
                let Some(current) = self.tags.get(&id) else {
                    break;
                };
                match current.status {
                    TagStatus::Active => {}
                    TagStatus::OnHold | TagStatus::Dropped => {
                        push(reasons, Reason::TagNotActive { tag: id });
                    }
                    TagStatus::Unknown => push(reasons, Reason::UnknownValue),
                }
                tag = current.parent;
            }
        }
    }

    fn check_blockers(&self, owner: Id, reasons: &mut Vec<Reason>) {
        for target in self.blockers.get(&owner).into_iter().flatten() {
            // A target outside the snapshot does not block (ADR 0017 §7).
            if self.tasks.get(target).is_some_and(|task| is_open(task)) {
                push(reasons, Reason::BlockedBy { task: *target });
            }
        }
    }

    /// What keeps a project, and so everything in it, from being available.
    fn project_reasons(&self, project: &Project) -> Vec<Reason> {
        let mut reasons = Vec::new();
        match project.status {
            ProjectStatus::Active => {}
            ProjectStatus::OnHold | ProjectStatus::Completed | ProjectStatus::Dropped => {
                push(
                    &mut reasons,
                    Reason::ProjectNotActive {
                        project: project.id,
                    },
                );
            }
            ProjectStatus::Unknown => push(&mut reasons, Reason::UnknownValue),
        }
        self.check_defer(project.schedule.defer.as_ref(), &mut reasons);
        self.check_folders(project.folder, &mut reasons);
        self.check_tags(&project.tags, &mut reasons);
        self.check_blockers(project.id, &mut reasons);
        reasons
    }

    /// The siblings a task is ordered among, and whether that order is
    /// sequential. `None` if the task has no container or a broken one.
    fn sequence_of(&self, task: &Task) -> Option<&[&'a Task]> {
        match task.parent {
            Some(parent) => {
                let group = self.tasks.get(&parent)?;
                group
                    .sequential
                    .then(|| self.children.get(&parent).map(Vec::as_slice))
                    .flatten()
            }
            None => {
                let project = self.projects.get(&task.project?)?;
                (project.kind == ProjectKind::Sequential)
                    .then(|| self.top_level.get(&project.id).map(Vec::as_slice))
                    .flatten()
            }
        }
    }

    fn task_reasons(&mut self, id: Id) -> Vec<Reason> {
        if let Some(reasons) = self.memo.get(&id) {
            return reasons.clone();
        }
        if !self.visiting.insert(id) {
            return vec![Reason::Cycle];
        }
        let reasons = match self.tasks.get(&id).copied() {
            Some(task) => self.compute_task(task),
            None => vec![Reason::UnknownValue],
        };
        self.visiting.remove(&id);
        self.memo.insert(id, reasons.clone());
        reasons
    }

    fn compute_task(&mut self, task: &'a Task) -> Vec<Reason> {
        // Rule 1. A completed or dropped task needs no further reasons.
        if task.completed_at.is_some() {
            return vec![Reason::Completed];
        }
        if task.dropped_at.is_some() {
            return vec![Reason::Dropped];
        }

        let mut reasons = Vec::new();
        // Rule 2.
        self.check_defer(task.schedule.defer.as_ref(), &mut reasons);
        // Rules 3 and 7 for the project: its reasons are the task's too. A
        // project that is not there makes it an inbox task (ADR 0036 §6).
        if let Some(project) = home(task, &self.projects)
            && let Some(project_reasons) = self.project_reasons.get(&project)
        {
            for reason in project_reasons.clone() {
                push(&mut reasons, reason);
            }
        }
        // Rule 4.
        self.check_tags(&task.tags, &mut reasons);
        // Rule 5.
        if let Some(siblings) = self.sequence_of(task)
            && let Some(first) = siblings.iter().find(|sibling| is_open(sibling))
            && first.id != task.id
        {
            push(&mut reasons, Reason::WaitingInSequence { first: first.id });
        }
        // Rule 6.
        if self
            .children
            .get(&task.id)
            .is_some_and(|children| children.iter().any(|child| is_open(child)))
        {
            push(&mut reasons, Reason::HasOpenChildren);
        }
        // Rule 7 for the action group above.
        if let Some(parent) = task.parent {
            match self.tasks.get(&parent).copied() {
                Some(group) if home(group, &self.projects) == home(task, &self.projects) => {
                    let group_reasons = self.task_reasons(parent);
                    if group_reasons.contains(&Reason::Cycle) {
                        push(&mut reasons, Reason::Cycle);
                    } else if group_reasons
                        .iter()
                        .any(|reason| *reason != Reason::HasOpenChildren)
                    {
                        push(&mut reasons, Reason::ContainerUnavailable { id: parent });
                    }
                }
                // A parent in another project, or one that is not there.
                _ => push(&mut reasons, Reason::UnknownValue),
            }
        }
        // Rule 8.
        self.check_blockers(task.id, &mut reasons);
        reasons
    }

    /// The instant a due date falls at: a date is due until the end of its
    /// day in the account's zone (ADR 0017 §2).
    fn due_instant(&self, due: &When) -> Option<Zoned> {
        match due {
            When::Date(date) => date
                .tomorrow()
                .ok()?
                .to_zoned(self.settings.zone.clone())
                .ok(),
            When::At(at) => Some(at.clone()),
            When::Unknown => None,
        }
    }

    fn due_state(&self, task: &Task) -> DueState {
        let Some(due) = task.schedule.due.as_ref().and_then(|d| self.due_instant(d)) else {
            return DueState::None;
        };
        if self.now >= due.timestamp() {
            return DueState::Overdue;
        }
        let warning = task
            .schedule
            .due_warning
            .map_or(self.settings.due_soon, |period| period.0);
        match due.checked_sub(warning) {
            Ok(start) if self.now >= start.timestamp() => DueState::Soon,
            _ => DueState::None,
        }
    }

    /// `planned` inherits down from groups and the project (ADR 0006).
    fn planned_today(&self, task: &Task) -> bool {
        let mut planned = task.schedule.planned.as_ref();
        let mut parent = task.parent;
        let mut seen = HashSet::new();
        while planned.is_none() {
            let Some(id) = parent else { break };
            if !seen.insert(id) {
                break;
            }
            let Some(group) = self.tasks.get(&id) else {
                break;
            };
            planned = group.schedule.planned.as_ref();
            parent = group.parent;
        }
        if planned.is_none() {
            planned = task
                .project
                .and_then(|p| self.projects.get(&p))
                .and_then(|project| project.schedule.planned.as_ref());
        }
        match planned {
            Some(When::Date(date)) => *date <= self.today,
            Some(When::At(at)) => {
                at.timestamp().to_zoned(self.settings.zone.clone()).date() <= self.today
            }
            Some(When::Unknown) | None => false,
        }
    }
}

impl From<&Reason> for crate::api::Reason {
    fn from(reason: &Reason) -> Self {
        match reason.clone() {
            Reason::Completed => Self::Completed,
            Reason::Dropped => Self::Dropped,
            Reason::Deferred { until } => Self::Deferred { until },
            Reason::ProjectNotActive { project } => Self::ProjectNotActive { project },
            Reason::FolderDropped { folder } => Self::FolderDropped { folder },
            Reason::TagNotActive { tag } => Self::TagNotActive { tag },
            Reason::WaitingInSequence { first } => Self::WaitingInSequence { first },
            Reason::HasOpenChildren => Self::HasOpenChildren,
            Reason::ContainerUnavailable { id } => Self::ContainerUnavailable { id },
            Reason::BlockedBy { task } => Self::BlockedBy { task },
            Reason::UnknownValue => Self::UnknownValue,
            Reason::Cycle => Self::Cycle,
        }
    }
}

impl From<DueState> for crate::api::DueState {
    fn from(due: DueState) -> Self {
        match due {
            DueState::None => Self::None,
            DueState::Soon => Self::Soon,
            DueState::Overdue => Self::Overdue,
        }
    }
}

/// What a folded action group shows (ADR 0021 §3): its direct children
/// open and done, how many are available, and for a sequential group the
/// first open child by position. `None` for a task without children.
#[must_use]
pub fn group_summary(
    tasks: &[Task],
    evaluation: &Evaluation,
    group: &Task,
) -> Option<GroupSummary> {
    let mut children: Vec<&Task> = tasks
        .iter()
        .filter(|t| t.parent == Some(group.id))
        .collect();
    if children.is_empty() {
        return None;
    }
    children.sort_by_key(|t| (t.position, t.id));
    let mut summary = GroupSummary::default();
    for child in &children {
        if is_open(child) {
            summary.open += 1;
            if evaluation.tasks.get(&child.id).is_some_and(|s| s.available) {
                summary.available += 1;
            }
        } else if child.completed_at.is_some() {
            summary.done += 1;
        }
    }
    if group.sequential {
        summary.next = children.iter().find(|t| is_open(t)).map(|t| t.id);
    }
    Some(summary)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::*;
    use crate::api::{Period, Schedule};

    fn ts(s: &str) -> Timestamp {
        s.parse().unwrap()
    }

    fn date(s: &str) -> When {
        When::Date(s.parse().unwrap())
    }

    fn at(s: &str) -> When {
        When::At(s.parse().unwrap())
    }

    fn berlin() -> Settings {
        Settings::new(TimeZone::get("Europe/Berlin").unwrap())
    }

    /// 07.11.2026, 10:00 in Berlin.
    const NOW: &str = "2026-11-07T09:00:00Z";

    fn task(position: i64) -> Task {
        Task {
            created_by: None,
            habit: None,
            id: Id::new(),
            created: ts("2026-11-01T00:00:00Z"),
            modified: ts("2026-11-01T00:00:00Z"),
            title: format!("task {position}"),
            note: None,
            project: None,
            parent: None,
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
            position,
        }
    }

    fn project(kind: ProjectKind) -> Project {
        Project {
            scope_only: false,
            own_scope: None,
            icon: None,
            created_by: None,
            id: Id::new(),
            created: ts("2026-11-01T00:00:00Z"),
            modified: ts("2026-11-01T00:00:00Z"),
            title: "project".into(),
            note: None,
            folder: None,
            workflow: None,
            kind,
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

    fn folder(status: FolderStatus, parent: Option<Id>) -> Folder {
        Folder {
            scope_only: false,
            own_scope: None,
            icon: None,
            id: Id::new(),
            created: ts("2026-11-01T00:00:00Z"),
            modified: ts("2026-11-01T00:00:00Z"),
            title: "folder".into(),
            parent,
            status,
            position: 0,
        }
    }

    fn tag(status: TagStatus, parent: Option<Id>) -> Tag {
        Tag {
            scope_only: false,
            own_scope: None,
            id: Id::new(),
            created: ts("2026-11-01T00:00:00Z"),
            modified: ts("2026-11-01T00:00:00Z"),
            title: "tag".into(),
            parent,
            status,
            exclusive: false,
            key: None,
            ask_note: false,
            default_workflow: false,
            position: 0,
        }
    }

    fn blocked_by(owner: Id, target: Id) -> Link {
        Link {
            id: Id::new(),
            created: ts("2026-11-01T00:00:00Z"),
            modified: ts("2026-11-01T00:00:00Z"),
            owner,
            uri: target.uri(),
            label: None,
            kind: LinkKind::BlockedBy,
        }
    }

    fn tagged(id: Id) -> Vec<Tagging> {
        vec![Tagging {
            tag: id,
            position: 0,
        }]
    }

    #[derive(Default)]
    struct World {
        folders: Vec<Folder>,
        projects: Vec<Project>,
        tasks: Vec<Task>,
        tags: Vec<Tag>,
        links: Vec<Link>,
    }

    impl World {
        fn run_at(&self, now: &str) -> Evaluation {
            let snapshot = Snapshot {
                folders: &self.folders,
                projects: &self.projects,
                tasks: &self.tasks,
                tags: &self.tags,
                links: &self.links,
                clocks: &[],
                scopes: &[],
            };
            evaluate(&snapshot, ts(now), &berlin())
        }

        fn run(&self) -> Evaluation {
            self.run_at(NOW)
        }
    }

    fn state(evaluation: &Evaluation, task: &Task) -> TaskState {
        evaluation.tasks[&task.id].clone()
    }

    // Rule 1.
    #[test]
    fn completed_and_dropped_are_not_available() {
        let open = task(0);
        let mut done = task(1);
        done.completed_at = Some(ts("2026-11-06T00:00:00Z"));
        let mut dropped = task(2);
        dropped.dropped_at = Some(ts("2026-11-06T00:00:00Z"));
        let world = World {
            tasks: vec![open.clone(), done.clone(), dropped.clone()],
            ..World::default()
        };
        let result = world.run();
        assert!(
            state(&result, &open).available,
            "an inbox task is available"
        );
        assert_eq!(state(&result, &done).reasons, vec![Reason::Completed]);
        assert_eq!(state(&result, &dropped).reasons, vec![Reason::Dropped]);
    }

    // Rule 2, with a date in the account's zone.
    #[test]
    fn a_defer_date_is_reached_at_midnight_in_the_accounts_zone() {
        let mut deferred = task(0);
        deferred.schedule.defer = Some(date("2026-11-20"));
        let world = World {
            tasks: vec![deferred.clone()],
            ..World::default()
        };
        // 23:30 UTC on the 19th is 00:30 on the 20th in Berlin.
        assert!(state(&world.run_at("2026-11-19T23:30:00Z"), &deferred).available);
        // 22:30 UTC is 23:30 on the 19th in Berlin.
        let before = state(&world.run_at("2026-11-19T22:30:00Z"), &deferred);
        assert_eq!(
            before.reasons,
            vec![Reason::Deferred {
                until: date("2026-11-20")
            }]
        );
    }

    #[test]
    fn a_defer_date_across_the_change_to_summer_time() {
        let mut deferred = task(0);
        deferred.schedule.defer = Some(date("2026-03-29"));
        let world = World {
            tasks: vec![deferred.clone()],
            ..World::default()
        };
        // Still winter time (+01:00) at midnight on the 29th.
        assert!(!state(&world.run_at("2026-03-28T22:59:00Z"), &deferred).available);
        assert!(state(&world.run_at("2026-03-28T23:00:00Z"), &deferred).available);
    }

    #[test]
    fn a_defer_instant_is_reached_at_that_instant() {
        let mut deferred = task(0);
        deferred.schedule.defer = Some(at("2026-11-07T10:30:00+01:00[Europe/Berlin]"));
        let world = World {
            tasks: vec![deferred.clone()],
            ..World::default()
        };
        assert!(!state(&world.run_at("2026-11-07T09:29:00Z"), &deferred).available);
        assert!(state(&world.run_at("2026-11-07T09:30:00Z"), &deferred).available);
    }

    // Rule 3.
    #[test]
    fn a_project_on_hold_holds_its_tasks() {
        let mut held = project(ProjectKind::Parallel);
        held.status = ProjectStatus::OnHold;
        let mut inside = task(0);
        inside.project = Some(held.id);
        let world = World {
            projects: vec![held.clone()],
            tasks: vec![inside.clone()],
            ..World::default()
        };
        let result = world.run();
        assert_eq!(
            state(&result, &inside).reasons,
            vec![Reason::ProjectNotActive { project: held.id }]
        );
        assert!(!result.projects[&held.id].stuck, "on hold is not stuck");
    }

    // ADR 0088 §1: no interval, no review; never reviewed counts from
    // `created`; `last_reviewed` in the account's zone; `next_review` wins.
    #[test]
    fn a_review_is_due_after_the_interval() {
        let plain = project(ProjectKind::Parallel);
        let mut fresh = project(ProjectKind::Parallel);
        fresh.review_interval = Some(Period(1.week()));
        let mut reviewed = fresh.clone();
        reviewed.id = Id::new();
        // 23:30 UTC on the 1st is the 2nd in Berlin.
        reviewed.last_reviewed = Some(ts("2026-11-01T23:30:00Z"));
        let mut put_off = fresh.clone();
        put_off.id = Id::new();
        put_off.next_review = Some(jiff::civil::date(2026, 12, 1));
        let world = World {
            projects: vec![
                plain.clone(),
                fresh.clone(),
                reviewed.clone(),
                put_off.clone(),
            ],
            ..World::default()
        };
        let on = |now: &str, p: &Project| {
            let result = world.run_at(now);
            let state = &result.projects[&p.id];
            (state.review, state.review_due)
        };
        assert_eq!(on("2026-11-07T22:00:00Z", &plain), (None, false));
        let nov8 = Some(jiff::civil::date(2026, 11, 8));
        // created 2026-11-01, one week: due from the 8th in Berlin.
        assert_eq!(on("2026-11-07T22:59:00Z", &fresh), (nov8, false));
        assert_eq!(on("2026-11-07T23:00:00Z", &fresh), (nov8, true));
        let nov9 = Some(jiff::civil::date(2026, 11, 9));
        assert_eq!(on("2026-11-08T12:00:00Z", &reviewed), (nov9, false));
        assert_eq!(on("2026-11-09T12:00:00Z", &reviewed), (nov9, true));
        let dec1 = Some(jiff::civil::date(2026, 12, 1));
        assert_eq!(on("2026-11-20T12:00:00Z", &put_off), (dec1, false));
    }

    // ADR 0088 §1: on hold is reviewed, completed and dropped are not.
    #[test]
    fn only_active_and_held_projects_are_reviewed() {
        let mut held = project(ProjectKind::Parallel);
        held.status = ProjectStatus::OnHold;
        held.review_interval = Some(Period(1.week()));
        let mut done = held.clone();
        done.id = Id::new();
        done.status = ProjectStatus::Completed;
        let mut dropped = held.clone();
        dropped.id = Id::new();
        dropped.status = ProjectStatus::Dropped;
        let world = World {
            projects: vec![held.clone(), done.clone(), dropped.clone()],
            ..World::default()
        };
        let result = world.run_at("2026-11-20T12:00:00Z");
        assert!(result.projects[&held.id].review_due);
        assert_eq!(result.projects[&done.id].review, None);
        assert!(!result.projects[&dropped.id].review_due);
    }

    #[test]
    fn a_dropped_folder_anywhere_above_hides_the_project() {
        let top = folder(FolderStatus::Dropped, None);
        let middle = folder(FolderStatus::Active, Some(top.id));
        let mut inner = project(ProjectKind::Parallel);
        inner.folder = Some(middle.id);
        let mut inside = task(0);
        inside.project = Some(inner.id);
        let world = World {
            folders: vec![top.clone(), middle],
            projects: vec![inner],
            tasks: vec![inside.clone()],
            ..World::default()
        };
        assert_eq!(
            state(&world.run(), &inside).reasons,
            vec![Reason::FolderDropped { folder: top.id }]
        );
    }

    // Rule 4, and ADR 0017 §4.
    #[test]
    fn a_tag_on_hold_holds_its_tasks_and_its_child_tags() {
        let someday = tag(TagStatus::OnHold, None);
        let child = tag(TagStatus::Active, Some(someday.id));
        let active = tag(TagStatus::Active, None);
        let mut direct = task(0);
        direct.tags = tagged(someday.id);
        let mut below = task(1);
        below.tags = tagged(child.id);
        let mut fine = task(2);
        fine.tags = tagged(active.id);
        let world = World {
            tags: vec![someday.clone(), child, active],
            tasks: vec![direct.clone(), below.clone(), fine.clone()],
            ..World::default()
        };
        let result = world.run();
        let held = vec![Reason::TagNotActive { tag: someday.id }];
        assert_eq!(state(&result, &direct).reasons, held);
        assert_eq!(state(&result, &below).reasons, held);
        assert!(state(&result, &fine).available);
    }

    // Rule 5, and ADR 0017 §5.
    #[test]
    fn in_a_sequential_project_only_the_first_open_action_is_next() {
        let seq = project(ProjectKind::Sequential);
        let mut done = task(0);
        done.project = Some(seq.id);
        done.completed_at = Some(ts("2026-11-06T00:00:00Z"));
        let mut first = task(1);
        first.project = Some(seq.id);
        let mut second = task(2);
        second.project = Some(seq.id);
        let world = World {
            projects: vec![seq],
            tasks: vec![second.clone(), done, first.clone()],
            ..World::default()
        };
        let result = world.run();
        assert!(state(&result, &first).available);
        assert_eq!(
            state(&result, &second).reasons,
            vec![Reason::WaitingInSequence { first: first.id }]
        );
    }

    #[test]
    fn a_deferred_first_action_still_holds_the_rest() {
        let seq = project(ProjectKind::Sequential);
        let mut first = task(0);
        first.project = Some(seq.id);
        first.schedule.defer = Some(date("2026-12-01"));
        let mut second = task(1);
        second.project = Some(seq.id);
        let world = World {
            projects: vec![seq.clone()],
            tasks: vec![first.clone(), second.clone()],
            ..World::default()
        };
        let result = world.run();
        assert_eq!(
            state(&result, &second).reasons,
            vec![Reason::WaitingInSequence { first: first.id }]
        );
        assert!(
            !result.projects[&seq.id].stuck,
            "a deferred action is not stuck"
        );
    }

    #[test]
    fn in_a_parallel_project_every_action_is_available() {
        let par = project(ProjectKind::Parallel);
        let mut a = task(0);
        a.project = Some(par.id);
        let mut b = task(1);
        b.project = Some(par.id);
        let world = World {
            projects: vec![par],
            tasks: vec![a.clone(), b.clone()],
            ..World::default()
        };
        let result = world.run();
        assert!(state(&result, &a).available);
        assert!(state(&result, &b).available);
    }

    // Rules 5, 6 and 7 for action groups.
    #[test]
    fn an_action_group_hands_over_to_its_children() {
        let par = project(ProjectKind::Parallel);
        let mut group = task(0);
        group.project = Some(par.id);
        group.sequential = true;
        let mut first = task(0);
        first.project = Some(par.id);
        first.parent = Some(group.id);
        let mut second = task(1);
        second.project = Some(par.id);
        second.parent = Some(group.id);
        let world = World {
            projects: vec![par],
            tasks: vec![group.clone(), first.clone(), second.clone()],
            ..World::default()
        };
        let result = world.run();
        assert_eq!(
            state(&result, &group).reasons,
            vec![Reason::HasOpenChildren]
        );
        assert!(state(&result, &first).available);
        assert_eq!(
            state(&result, &second).reasons,
            vec![Reason::WaitingInSequence { first: first.id }]
        );
    }

    #[test]
    fn a_deferred_group_defers_its_children() {
        let par = project(ProjectKind::Parallel);
        let mut group = task(0);
        group.project = Some(par.id);
        group.schedule.defer = Some(date("2026-12-01"));
        let mut child = task(0);
        child.project = Some(par.id);
        child.parent = Some(group.id);
        let world = World {
            projects: vec![par],
            tasks: vec![group.clone(), child.clone()],
            ..World::default()
        };
        assert_eq!(
            state(&world.run(), &child).reasons,
            vec![Reason::ContainerUnavailable { id: group.id }]
        );
    }

    #[test]
    fn a_deferred_project_defers_its_tasks() {
        let mut later = project(ProjectKind::Parallel);
        later.schedule.defer = Some(date("2026-12-01"));
        let mut inside = task(0);
        inside.project = Some(later.id);
        let world = World {
            projects: vec![later.clone()],
            tasks: vec![inside.clone()],
            ..World::default()
        };
        let result = world.run();
        assert_eq!(
            state(&result, &inside).reasons,
            vec![Reason::Deferred {
                until: date("2026-12-01")
            }]
        );
        assert!(!result.projects[&later.id].stuck);
    }

    // Rule 8, and ADR 0017 §7.
    #[test]
    fn blocked_while_the_blocker_is_open() {
        let blocker = task(0);
        let mut done_blocker = task(1);
        done_blocker.completed_at = Some(ts("2026-11-06T00:00:00Z"));
        let waiting = task(2);
        let free = task(3);
        let dangling = task(4);
        let world = World {
            tasks: vec![
                blocker.clone(),
                done_blocker.clone(),
                waiting.clone(),
                free.clone(),
                dangling.clone(),
            ],
            links: vec![
                blocked_by(waiting.id, blocker.id),
                blocked_by(free.id, done_blocker.id),
                blocked_by(dangling.id, Id::new()),
            ],
            ..World::default()
        };
        let result = world.run();
        assert_eq!(
            state(&result, &waiting).reasons,
            vec![Reason::BlockedBy { task: blocker.id }]
        );
        assert!(state(&result, &free).available);
        assert!(
            state(&result, &dangling).available,
            "a target not there does not block"
        );
    }

    #[test]
    fn two_tasks_blocking_each_other_do_not_hang() {
        let a = task(0);
        let b = task(1);
        let world = World {
            tasks: vec![a.clone(), b.clone()],
            links: vec![blocked_by(a.id, b.id), blocked_by(b.id, a.id)],
            ..World::default()
        };
        let result = world.run();
        assert!(!state(&result, &a).available);
        assert!(!state(&result, &b).available);
    }

    // ADR 0017 §10: all reasons, not the first.
    #[test]
    fn every_reason_is_listed() {
        let mut held = project(ProjectKind::Parallel);
        held.status = ProjectStatus::OnHold;
        let someday = tag(TagStatus::OnHold, None);
        let mut inside = task(0);
        inside.project = Some(held.id);
        inside.schedule.defer = Some(date("2026-12-01"));
        inside.tags = tagged(someday.id);
        let world = World {
            projects: vec![held.clone()],
            tags: vec![someday.clone()],
            tasks: vec![inside.clone()],
            ..World::default()
        };
        assert_eq!(
            state(&world.run(), &inside).reasons,
            vec![
                Reason::Deferred {
                    until: date("2026-12-01")
                },
                Reason::ProjectNotActive { project: held.id },
                Reason::TagNotActive { tag: someday.id },
            ]
        );
    }

    // ADR 0017 §2 and §3: unknown values.
    #[test]
    fn unknown_values_are_not_active() {
        let mut odd = project(ProjectKind::Parallel);
        odd.status = ProjectStatus::Unknown;
        let mut inside = task(0);
        inside.project = Some(odd.id);
        let mut odd_defer = task(1);
        odd_defer.schedule.defer = Some(When::Unknown);
        let mut odd_due = task(2);
        odd_due.schedule.due = Some(When::Unknown);
        odd_due.schedule.planned = Some(When::Unknown);
        let world = World {
            projects: vec![odd],
            tasks: vec![inside.clone(), odd_defer.clone(), odd_due.clone()],
            ..World::default()
        };
        let result = world.run();
        assert_eq!(state(&result, &inside).reasons, vec![Reason::UnknownValue]);
        assert_eq!(
            state(&result, &odd_defer).reasons,
            vec![Reason::UnknownValue]
        );
        let due = state(&result, &odd_due);
        assert!(due.available, "an unknown due or planned is passed over");
        assert_eq!(due.due, DueState::None);
        assert!(!due.today);
    }

    // ADR 0017 §9.
    #[test]
    fn a_circle_of_parents_is_a_reason_not_a_hang() {
        let mut a = task(0);
        let mut b = task(1);
        a.parent = Some(b.id);
        b.parent = Some(a.id);
        let world = World {
            tasks: vec![a.clone(), b.clone()],
            ..World::default()
        };
        let result = world.run();
        assert!(state(&result, &a).reasons.contains(&Reason::Cycle));
        assert!(state(&result, &b).reasons.contains(&Reason::Cycle));
    }

    #[test]
    fn circles_of_folders_and_tags_are_reasons_too() {
        let mut f1 = folder(FolderStatus::Active, None);
        let f2 = folder(FolderStatus::Active, Some(f1.id));
        f1.parent = Some(f2.id);
        let mut inner = project(ProjectKind::Parallel);
        inner.folder = Some(f1.id);
        let mut t1 = tag(TagStatus::Active, None);
        let t2 = tag(TagStatus::Active, Some(t1.id));
        t1.parent = Some(t2.id);
        let mut in_folder = task(0);
        in_folder.project = Some(inner.id);
        let mut with_tag = task(1);
        with_tag.tags = tagged(t1.id);
        let world = World {
            folders: vec![f1, f2],
            projects: vec![inner],
            tags: vec![t1, t2],
            tasks: vec![in_folder.clone(), with_tag.clone()],
            ..World::default()
        };
        let result = world.run();
        assert_eq!(state(&result, &in_folder).reasons, vec![Reason::Cycle]);
        assert_eq!(state(&result, &with_tag).reasons, vec![Reason::Cycle]);
    }

    #[test]
    fn a_parent_in_another_project_is_broken_data() {
        let p1 = project(ProjectKind::Parallel);
        let p2 = project(ProjectKind::Parallel);
        let mut group = task(0);
        group.project = Some(p1.id);
        let mut child = task(0);
        child.project = Some(p2.id);
        child.parent = Some(group.id);
        let world = World {
            projects: vec![p1, p2],
            tasks: vec![group, child.clone()],
            ..World::default()
        };
        assert_eq!(
            state(&world.run(), &child).reasons,
            vec![Reason::UnknownValue]
        );
    }

    // ADR 0036 §6.
    #[test]
    fn a_task_whose_project_is_gone_is_an_available_inbox_task() {
        let gone = project(ProjectKind::Sequential);
        let held = Project {
            status: ProjectStatus::OnHold,
            ..project(ProjectKind::Parallel)
        };
        let mut orphan = task(0);
        orphan.project = Some(gone.id);
        let mut inside = task(0);
        inside.project = Some(held.id);
        let world = World {
            projects: vec![held],
            tasks: vec![orphan.clone(), inside.clone()],
            ..World::default()
        };
        let result = world.run();
        let orphan = state(&result, &orphan);
        assert!(orphan.available, "{:?}", orphan.reasons);
        assert!(orphan.inbox);
        let inside = state(&result, &inside);
        assert!(!inside.available, "the probe finds a project reason");
        assert!(!inside.inbox);
    }

    #[test]
    fn an_orphaned_group_keeps_its_children_whole() {
        let gone = project(ProjectKind::Parallel);
        let mut group = task(0);
        group.project = Some(gone.id);
        let mut child = task(0);
        child.project = Some(gone.id);
        child.parent = Some(group.id);
        let world = World {
            tasks: vec![group.clone(), child.clone()],
            ..World::default()
        };
        let result = world.run();
        assert!(state(&result, &child).available);
        assert!(state(&result, &child).inbox);
        assert_eq!(
            state(&result, &group).reasons,
            vec![Reason::HasOpenChildren]
        );
    }

    // ADR 0017 §8.
    #[test]
    fn a_project_without_a_next_action_is_stuck() {
        let empty = project(ProjectKind::Parallel);
        let list = project(ProjectKind::SingleActions);
        let busy = project(ProjectKind::Parallel);
        let mut action = task(0);
        action.project = Some(busy.id);
        let world = World {
            projects: vec![empty.clone(), list.clone(), busy.clone()],
            tasks: vec![action],
            ..World::default()
        };
        let result = world.run();
        assert!(result.projects[&empty.id].stuck);
        assert!(
            !result.projects[&list.id].stuck,
            "single action lists are never stuck"
        );
        assert!(!result.projects[&busy.id].stuck);
    }

    #[test]
    fn a_project_whose_only_action_is_blocked_is_stuck() {
        let par = project(ProjectKind::Parallel);
        let elsewhere = task(5);
        let mut blocked = task(0);
        blocked.project = Some(par.id);
        let world = World {
            projects: vec![par.clone()],
            tasks: vec![blocked.clone(), elsewhere.clone()],
            links: vec![blocked_by(blocked.id, elsewhere.id)],
            ..World::default()
        };
        assert!(world.run().projects[&par.id].stuck);
    }

    // ADR 0017 §2: due dates.
    #[test]
    fn a_due_date_is_overdue_from_the_next_day() {
        let mut today = task(0);
        today.schedule.due = Some(date("2026-11-07"));
        let mut yesterday = task(1);
        yesterday.schedule.due = Some(date("2026-11-06"));
        let mut in_two_days = task(2);
        in_two_days.schedule.due = Some(date("2026-11-08"));
        let mut next_week = task(3);
        next_week.schedule.due = Some(date("2026-11-14"));
        let world = World {
            tasks: vec![
                today.clone(),
                yesterday.clone(),
                in_two_days.clone(),
                next_week.clone(),
            ],
            ..World::default()
        };
        let result = world.run();
        assert_eq!(state(&result, &today).due, DueState::Soon);
        assert_eq!(state(&result, &yesterday).due, DueState::Overdue);
        assert_eq!(state(&result, &in_two_days).due, DueState::Soon);
        assert_eq!(state(&result, &next_week).due, DueState::None);
    }

    #[test]
    fn a_due_instant_is_overdue_from_that_instant() {
        let mut call = task(0);
        call.schedule.due = Some(at("2026-11-07T10:30:00+01:00[Europe/Berlin]"));
        let world = World {
            tasks: vec![call.clone()],
            ..World::default()
        };
        assert_eq!(
            state(&world.run_at("2026-11-07T09:29:00Z"), &call).due,
            DueState::Soon
        );
        assert_eq!(
            state(&world.run_at("2026-11-07T09:30:00Z"), &call).due,
            DueState::Overdue
        );
    }

    #[test]
    fn a_task_can_bring_its_own_warning() {
        let mut deadline = task(0);
        deadline.schedule.due = Some(date("2026-11-12"));
        deadline.schedule.due_warning = Some(Period(7.days()));
        let world = World {
            tasks: vec![deadline.clone()],
            ..World::default()
        };
        assert_eq!(state(&world.run(), &deadline).due, DueState::Soon);
    }

    #[test]
    fn planned_for_today_inherits_from_the_project() {
        let mut planned = project(ProjectKind::Parallel);
        planned.schedule.planned = Some(date("2026-11-07"));
        let mut inside = task(0);
        inside.project = Some(planned.id);
        let mut tomorrow = task(1);
        tomorrow.schedule.planned = Some(date("2026-11-08"));
        let mut done = task(2);
        done.schedule.planned = Some(date("2026-11-01"));
        done.completed_at = Some(ts("2026-11-02T00:00:00Z"));
        let world = World {
            projects: vec![planned],
            tasks: vec![inside.clone(), tomorrow.clone(), done.clone()],
            ..World::default()
        };
        let result = world.run();
        assert!(state(&result, &inside).today);
        assert!(!state(&result, &tomorrow).today);
        assert!(
            !state(&result, &done).today,
            "only open tasks are for today"
        );
    }

    /// ADR 0006: measure before indexing. Run with
    /// `cargo test --release -p samu-core -- --ignored --nocapture`.
    #[test]
    #[ignore = "a measurement, not a check"]
    fn measure_five_thousand_tasks() {
        let mut world = World::default();
        for p in 0..100 {
            let kind = if p % 2 == 0 {
                ProjectKind::Sequential
            } else {
                ProjectKind::Parallel
            };
            let project = project(kind);
            for i in 0..50 {
                let mut t = task(i);
                t.project = Some(project.id);
                if i % 7 == 0 {
                    t.schedule.defer = Some(date("2026-12-01"));
                }
                if i % 5 == 0 {
                    t.schedule.due = Some(date("2026-11-08"));
                }
                world.tasks.push(t);
            }
            world.projects.push(project);
        }
        let start = std::time::Instant::now();
        let result = world.run();
        let elapsed = start.elapsed();
        assert_eq!(result.tasks.len(), 5000);
        eprintln!("5000 tasks in 100 projects: {elapsed:?}");
    }

    #[test]
    fn a_group_summary_counts_its_children_and_names_the_next() {
        let mut group = task(0);
        group.sequential = true;
        let mut first = task(1);
        first.parent = Some(group.id);
        first.completed_at = Some(ts("2026-11-02T00:00:00Z"));
        let mut second = task(2);
        second.parent = Some(group.id);
        let mut third = task(3);
        third.parent = Some(group.id);
        let mut dropped = task(4);
        dropped.parent = Some(group.id);
        dropped.dropped_at = Some(ts("2026-11-02T00:00:00Z"));
        let lone = task(5);
        let tasks = vec![
            group.clone(),
            third,
            dropped,
            second.clone(),
            first,
            lone.clone(),
        ];
        let snapshot = Snapshot {
            folders: &[],
            projects: &[],
            tasks: &tasks,
            tags: &[],
            clocks: &[],
            links: &[],
            scopes: &[],
        };
        let evaluation = evaluate(&snapshot, ts(NOW), &berlin());
        let summary = group_summary(&tasks, &evaluation, &group).unwrap();
        assert_eq!(
            summary,
            GroupSummary {
                open: 2,
                done: 1,
                available: 1,
                next: Some(second.id)
            }
        );
        assert!(group_summary(&tasks, &evaluation, &lone).is_none());
        group.sequential = false;
        assert_eq!(
            group_summary(&tasks, &evaluation, &group).unwrap().next,
            None
        );
    }
}
