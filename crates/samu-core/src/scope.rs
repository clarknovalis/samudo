// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Scopes: one lens over every view (ADR 0035).
//!
//! A scope names folders, projects and tags, and says whether the views show
//! only what those cover or everything else. The rule lives here and only
//! here (ADR 0035 §2); the server and the phone ask this module, no client
//! decides on its own.
//!
//! A task is *covered* when one of these is a member: its project, a folder
//! above that project at any depth, one of its tags or a tag above one, or
//! one of its project's tags or a tag above one. A project is covered by the
//! same rule without a task's tags. A task without a project, the inbox, is
//! covered when the scope ticks the inbox, and by nothing else: its tags do
//! not count, so a scope from before the tick shows what it showed
//! (ADR 0043 §1). A member that is not live in the snapshot counts for
//! nothing.
//!
//! The filter (ADR 0043 §2, §3) comes on top, for tasks only: a task is in
//! the scope when the members let it in and it meets the filter
//! ([`crate::query`]). Projects and folders follow the members alone.
//!
//! Before all that comes `own_scope` (ADR 0094): what hits an object that
//! names its own scope is shown in that scope alone, in either mode, and
//! with `only` as if it were ticked. On the way of folders and project the
//! nearest such object counts, among the tags each one; one is enough.
//! An own scope that does not exist counts for nothing (§5).

use std::collections::{HashMap, HashSet};

use samu_api::{Folder, Id, Project, Scope, ScopeMode, Tag, Tagging, Task};

use jiff::Timestamp;
use jiff::tz::TimeZone;

use crate::availability::{DueState, Snapshot, TaskState};
use crate::query::Filter;

/// A scope read against one snapshot, ready to ask about tasks and projects;
/// or no scope at all, which still hides what has an own scope (ADR 0094 §4).
#[derive(Debug)]
pub struct Lens<'a> {
    /// None for no scope: the members do not narrow.
    mode: Option<ScopeMode>,
    /// The inbox is a member (ADR 0043 §1).
    inbox: bool,
    /// Nothing ticked, neither a member nor the inbox: the members do not
    /// narrow, in either mode (ADR 0044). Asked of the stored set, so the
    /// last member deleted does not widen an `only`.
    unticked: bool,
    /// The members that are live in the snapshot.
    members: HashSet<Id>,
    /// The scope looked through, if any: the only one that shows what
    /// names it as its own (ADR 0094 §3).
    current: Option<Id>,
    /// The scopes that exist, for an `own_scope` to count (ADR 0094 §5).
    scopes: HashSet<Id>,
    folders: HashMap<Id, &'a Folder>,
    projects: HashMap<Id, &'a Project>,
    tags: HashMap<Id, &'a Tag>,
    filter: Filter<'a>,
}

impl<'a> Lens<'a> {
    #[must_use]
    /// The scope, or no scope, on the day `now` is in `zone`, which its
    /// filter's relative ranges count from.
    pub fn new(
        scope: Option<&Scope>,
        snapshot: &Snapshot<'a>,
        now: Timestamp,
        zone: &TimeZone,
    ) -> Self {
        let folders: HashMap<Id, &Folder> = snapshot.folders.iter().map(|f| (f.id, f)).collect();
        let projects: HashMap<Id, &Project> = snapshot.projects.iter().map(|p| (p.id, p)).collect();
        let tags: HashMap<Id, &Tag> = snapshot.tags.iter().map(|t| (t.id, t)).collect();
        let members: HashSet<Id> = scope
            .map(|s| s.members.as_slice())
            .unwrap_or_default()
            .iter()
            .copied()
            .filter(|id| {
                folders.contains_key(id) || projects.contains_key(id) || tags.contains_key(id)
            })
            .collect();
        let unticked = scope.is_none_or(|s| s.members.is_empty() && !s.inbox);
        Self {
            mode: scope.map(|s| s.mode),
            inbox: scope.is_some_and(|s| s.inbox),
            unticked,
            members,
            current: scope.map(|s| s.id),
            scopes: snapshot.scopes.iter().map(|s| s.id).collect(),
            folders,
            projects,
            tags,
            filter: Filter::new(
                scope.map(|s| s.filter.as_slice()).unwrap_or_default(),
                snapshot,
                now,
                zone,
            ),
        }
    }

    /// Whether the views show this task: the members let it in and it
    /// meets the filter (ADR 0043 §3), unless it hits an own scope other
    /// than this one (ADR 0094).
    #[must_use]
    pub fn task(&self, task: &Task, state: &TaskState) -> bool {
        self.by_members(task) && self.filter.matches(task, state)
    }

    /// Whether the members, read through the mode, let this task in.
    fn by_members(&self, task: &Task) -> bool {
        let mut hit = HashSet::new();
        self.tags_own(&task.tags, &mut hit);
        // The inbox, an orphan among it (ADR 0036 §6), is a member of its
        // own (ADR 0043 §1). Its tags count for own_scope (ADR 0069 §5).
        let Some(project) = crate::availability::home(task, &self.projects) else {
            return match self.own_here(&hit) {
                Some(own) => self.shows(self.inbox || own),
                None => false,
            };
        };
        self.project_own(project, &mut hit);
        let Some(own) = self.own_here(&hit) else {
            return false;
        };
        let covered = own || self.covers_tags(&task.tags) || self.covers_project(project);
        self.shows(covered)
    }

    /// Outside the lens, yet worth the hint in the header (ADR 0035 §6,
    /// ADR 0069 §7): open, and overdue or due soon.
    #[must_use]
    pub fn due_outside(&self, task: &Task, state: &TaskState) -> bool {
        let open = task.completed_at.is_none() && task.dropped_at.is_none();
        open && matches!(state.due, DueState::Overdue | DueState::Soon) && !self.task(task, state)
    }

    /// Whether the views show this project.
    #[must_use]
    pub fn project(&self, project: &Project) -> bool {
        let mut hit = HashSet::new();
        self.project_own(project.id, &mut hit);
        let Some(own) = self.own_here(&hit) else {
            return false;
        };
        self.shows(own || self.covers_project(project.id))
    }

    /// Whether the views show this folder: when it or something below it
    /// holds a project they show, or when nothing is below it at all and the
    /// folder itself is not hidden.
    #[must_use]
    pub fn folder(&self, folder: &Folder) -> bool {
        let hit: HashSet<Id> = self.folders_own(Some(folder.id)).into_iter().collect();
        let Some(own) = self.own_here(&hit) else {
            return false;
        };
        let mut holds_any = false;
        for project in self.projects.values() {
            if self.folder_above(project.folder, folder.id) {
                holds_any = true;
                if self.project(project) {
                    return true;
                }
            }
        }
        !holds_any && self.shows(own || self.covers_folder(Some(folder.id)))
    }

    /// What the own scopes hit say here (ADR 0094 §3): none when this
    /// lens must not show it; otherwise whether it counts as ticked, which
    /// it does in its own scope with `only`.
    fn own_here(&self, hit: &HashSet<Id>) -> Option<bool> {
        if hit.is_empty() {
            return Some(false);
        }
        self.current
            .filter(|c| hit.contains(c))
            .map(|_| self.mode == Some(ScopeMode::Only))
    }

    /// An own scope that counts: one that exists (ADR 0094 §5).
    fn counting(&self, own: Option<Id>) -> Option<Id> {
        own.filter(|id| self.scopes.contains(id))
    }

    /// The own scopes a project hits: the nearest of itself and the
    /// folders above it, and every one among its tags and those above them
    /// (ADR 0094 §2).
    fn project_own(&self, id: Id, hit: &mut HashSet<Id>) {
        let Some(project) = self.projects.get(&id) else {
            return;
        };
        self.tags_own(&project.tags, hit);
        if let Some(own) = self.counting(project.own_scope) {
            hit.insert(own);
        } else if let Some(own) = self.folders_own(project.folder) {
            hit.insert(own);
        }
    }

    /// The own scope of the nearest folder from `folder` up that has one.
    fn folders_own(&self, mut folder: Option<Id>) -> Option<Id> {
        let mut seen = HashSet::new();
        while let Some(id) = folder {
            if !seen.insert(id) {
                return None;
            }
            let f = self.folders.get(&id)?;
            if let Some(own) = self.counting(f.own_scope) {
                return Some(own);
            }
            folder = f.parent;
        }
        None
    }

    /// The own scopes among `tags` and those above them.
    fn tags_own(&self, tags: &[Tagging], hit: &mut HashSet<Id>) {
        let mut up = HashSet::new();
        for tagging in tags {
            self.tags_up(tagging.tag, &mut up);
        }
        hit.extend(
            up.into_iter()
                .filter_map(|id| self.counting(self.tags.get(&id).and_then(|t| t.own_scope))),
        );
    }

    /// `tag` and every tag above it; stops at a cycle.
    fn tags_up(&self, tag: Id, into: &mut HashSet<Id>) {
        let mut seen = HashSet::new();
        let mut tag = Some(tag);
        while let Some(id) = tag {
            if !seen.insert(id) {
                return;
            }
            into.insert(id);
            tag = self.tags.get(&id).and_then(|t| t.parent);
        }
    }

    fn shows(&self, covered: bool) -> bool {
        if self.unticked {
            return true;
        }
        match self.mode {
            Some(ScopeMode::Only) => covered,
            Some(ScopeMode::Except) => !covered,
            // No scope, or a mode from a newer samu, hides nothing
            // (ADR 0035 §1) but what has an own scope.
            None | Some(ScopeMode::Unknown) => true,
        }
    }

    fn covers_project(&self, id: Id) -> bool {
        if self.members.contains(&id) {
            return true;
        }
        let Some(project) = self.projects.get(&id) else {
            return false;
        };
        self.covers_tags(&project.tags) || self.covers_folder(project.folder)
    }

    /// The folder or one above it is a member.
    fn covers_folder(&self, mut folder: Option<Id>) -> bool {
        let mut seen = HashSet::new();
        while let Some(id) = folder {
            if !seen.insert(id) {
                return false;
            }
            if self.members.contains(&id) {
                return true;
            }
            folder = self.folders.get(&id).and_then(|f| f.parent);
        }
        false
    }

    /// One of the tags or a tag above one is a member.
    fn covers_tags(&self, tags: &[Tagging]) -> bool {
        tags.iter().any(|tagging| {
            let mut seen = HashSet::new();
            let mut tag = Some(tagging.tag);
            while let Some(id) = tag {
                if !seen.insert(id) {
                    return false;
                }
                if self.members.contains(&id) {
                    return true;
                }
                tag = self.tags.get(&id).and_then(|t| t.parent);
            }
            false
        })
    }

    /// `ancestor` is `folder` or above it.
    fn folder_above(&self, mut folder: Option<Id>, ancestor: Id) -> bool {
        let mut seen = HashSet::new();
        while let Some(id) = folder {
            if id == ancestor {
                return true;
            }
            if !seen.insert(id) {
                return false;
            }
            folder = self.folders.get(&id).and_then(|f| f.parent);
        }
        false
    }
}

/// What a scope's schedule says at one instant (ADR 0043 §7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleNow {
    /// The scope whose window holds the instant, the smallest `position`
    /// first; none when no window does.
    pub scope: Option<Id>,
    /// The last time a window began or ended, at or before the instant.
    pub since: Option<Timestamp>,
    /// The next time a window begins or ends, after the instant.
    pub until: Option<Timestamp>,
}

/// The days and times of a window as jiff's, or none for a window this
/// samu cannot read (an unknown day, no day at all).
fn window_days(window: &samu_api::ScopeWindow) -> Option<Vec<jiff::civil::Weekday>> {
    let days: Option<Vec<_>> = window
        .days
        .iter()
        .map(|d| crate::query::weekday(*d))
        .collect();
    days.filter(|d| !d.is_empty())
}

/// Whether the window holds the local `at`. A window whose end is not
/// after its start ends on the next day.
fn holds(
    window: &samu_api::ScopeWindow,
    days: &[jiff::civil::Weekday],
    at: jiff::civil::DateTime,
) -> bool {
    let (date, time) = (at.date(), at.time());
    if window.from < window.to {
        days.contains(&date.weekday()) && window.from <= time && time < window.to
    } else {
        let yesterday = date.yesterday().map(|d| d.weekday()).ok();
        (days.contains(&date.weekday()) && time >= window.from)
            || (yesterday.is_some_and(|d| days.contains(&d)) && time < window.to)
    }
}

/// What the schedules of `scopes` say at `now` in `zone` (ADR 0043 §7):
/// the scope whose window holds `now`, and the edges around it. A device
/// that follows the schedule shows that scope, unless it was chosen by hand
/// since the last edge ([`follow`]).
#[must_use]
pub fn schedule_now(scopes: &[Scope], now: Timestamp, zone: &TimeZone) -> ScheduleNow {
    let local = now.to_zoned(zone.clone());
    let at = local.datetime();
    let mut ordered: Vec<&Scope> = scopes.iter().collect();
    ordered.sort_by_key(|s| (s.position, s.id));
    let mut scope = None;
    let (mut since, mut until): (Option<Timestamp>, Option<Timestamp>) = (None, None);
    let instant = |date: jiff::civil::Date, time: jiff::civil::Time| {
        date.to_datetime(time)
            .to_zoned(zone.clone())
            .ok()
            .map(|z| z.timestamp())
    };
    for s in ordered {
        for window in &s.schedule {
            let Some(days) = window_days(window) else {
                continue;
            };
            if scope.is_none() && holds(window, &days, at) {
                scope = Some(s.id);
            }
            // The edges of the window on the days around today.
            for offset in -8_i64..=8 {
                let Ok(date) = at.date().checked_add(jiff::Span::new().days(offset)) else {
                    continue;
                };
                if !days.contains(&date.weekday()) {
                    continue;
                }
                let end_date = if window.from < window.to {
                    Some(date)
                } else {
                    date.tomorrow().ok()
                };
                let edges = [
                    instant(date, window.from),
                    end_date.and_then(|d| instant(d, window.to)),
                ];
                for edge in edges.into_iter().flatten() {
                    if edge <= now {
                        since = since.max(Some(edge));
                    } else if until.is_none_or(|u| edge < u) {
                        until = Some(edge);
                    }
                }
            }
        }
    }
    ScheduleNow {
        scope,
        since,
        until,
    }
}

/// The scope a device that follows the schedule shows: the one chosen by
/// hand, if that was at or after the schedule's last edge, else the
/// schedule's. The last event wins (ADR 0043 §7); a choice by hand is one,
/// and so is a broadcast from Tasker (§8).
#[must_use]
pub fn follow(hand: Option<(Option<Id>, Timestamp)>, schedule: &ScheduleNow) -> Option<Id> {
    match hand {
        Some((chosen, at)) if schedule.since.is_none_or(|edge| at >= edge) => chosen,
        _ => schedule.scope,
    }
}

/// The days of the week in order, German first, English accepted.
const DAYS: &[(&str, &str, samu_api::Weekday)] = &[
    ("mo", "mo", samu_api::Weekday::Mo),
    ("di", "tu", samu_api::Weekday::Tu),
    ("mi", "we", samu_api::Weekday::We),
    ("do", "th", samu_api::Weekday::Th),
    ("fr", "fr", samu_api::Weekday::Fr),
    ("sa", "sa", samu_api::Weekday::Sa),
    ("so", "su", samu_api::Weekday::Su),
];

/// Why a text is no schedule.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct WindowError(String);

/// Windows from text, `;` between them: `mo-fr 08:00-17:00; sa 22:00-02:00`.
/// Days as a list or a range (`fr-mo` goes round); an empty text is no
/// window. The same reading as `samu scope schedule`.
///
/// # Errors
///
/// [`WindowError`] says which part is wrong.
pub fn parse_windows(text: &str) -> Result<Vec<samu_api::ScopeWindow>, WindowError> {
    let day = |word: &str| {
        let word = word.trim().to_lowercase();
        DAYS.iter()
            .position(|(de, en, _)| *de == word || *en == word)
    };
    let mut windows = Vec::new();
    for part in text.split(';').map(str::trim).filter(|p| !p.is_empty()) {
        let err = || {
            WindowError(format!(
                "{part:?}: days, then from-to, like mo-fr 08:00-17:00"
            ))
        };
        let (days_text, times) = part.split_once(char::is_whitespace).ok_or_else(err)?;
        let mut days = Vec::new();
        for range in days_text.split(',') {
            let (a, b) = match range.split_once('-') {
                Some((a, b)) => (day(a), day(b)),
                None => (day(range), day(range)),
            };
            let (Some(a), Some(b)) = (a, b) else {
                return Err(WindowError(format!(
                    "{range:?} is no day; mo di mi do fr sa so"
                )));
            };
            let mut i = a;
            loop {
                if !days.contains(&DAYS[i].2) {
                    days.push(DAYS[i].2);
                }
                if i == b {
                    break;
                }
                i = (i + 1) % DAYS.len();
            }
        }
        let (from, to) = times.trim().split_once('-').ok_or_else(err)?;
        let time = |t: &str| -> Result<jiff::civil::Time, WindowError> {
            let t = t.trim();
            let full = if t.len() == 5 {
                format!("{t}:00")
            } else {
                t.to_owned()
            };
            full.parse()
                .map_err(|_| WindowError(format!("{t:?} is no time")))
        };
        windows.push(samu_api::ScopeWindow {
            days,
            from: time(from)?,
            to: time(to)?,
        });
    }
    Ok(windows)
}

/// Windows as [`parse_windows`] reads them.
#[must_use]
pub fn windows_text(windows: &[samu_api::ScopeWindow]) -> String {
    windows
        .iter()
        .map(|w| {
            let days: Vec<&str> = w
                .days
                .iter()
                .map(|d| {
                    DAYS.iter()
                        .find(|(_, _, x)| x == d)
                        .map_or("?", |(de, _, _)| *de)
                })
                .collect();
            format!(
                "{} {}-{}",
                days.join(","),
                w.from.strftime("%H:%M"),
                w.to.strftime("%H:%M")
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use samu_api::{FolderStatus, ProjectKind, ProjectStatus, Schedule, TagStatus};

    use super::*;

    fn id(n: u8) -> Id {
        format!("01936f5e-8a00-7c3e-9b1a-0000000000{n:02x}")
            .parse()
            .unwrap()
    }

    fn at() -> Timestamp {
        "2026-11-08T08:00:00Z".parse().unwrap()
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

    fn project(n: u8, folder: Option<u8>, tags: &[u8]) -> Project {
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
            tags: taggings(tags),
            properties: Default::default(),
            position: 0,
        }
    }

    fn task(n: u8, project: Option<u8>, tags: &[u8]) -> Task {
        Task {
            created_by: None,
            habit: None,
            id: id(n),
            created: at(),
            modified: at(),
            title: format!("task {n}"),
            note: None,
            project: project.map(id),
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
            tags: taggings(tags),
            properties: Default::default(),
            position: 0,
        }
    }

    fn tag(n: u8, parent: Option<u8>) -> Tag {
        Tag {
            scope_only: false,
            own_scope: None,
            id: id(n),
            created: at(),
            modified: at(),
            title: format!("tag {n}"),
            parent: parent.map(id),
            status: TagStatus::Active,
            exclusive: false,
            key: None,
            ask_note: false,
            default_workflow: false,
            position: 0,
        }
    }

    fn taggings(tags: &[u8]) -> Vec<Tagging> {
        tags.iter()
            .map(|n| Tagging {
                tag: id(*n),
                position: 0,
            })
            .collect()
    }

    fn scope(mode: ScopeMode, members: &[u8]) -> Scope {
        Scope {
            layout: None,
            color: None,
            id: id(200),
            created: at(),
            modified: at(),
            title: "scope".into(),
            mode,
            members: members.iter().map(|n| id(*n)).collect(),
            inbox: false,
            filter: Vec::new(),
            capture: None,
            schedule: Vec::new(),
            calendars: Vec::new(),
            position: 0,
        }
    }

    fn scope_with(n: u8, mode: ScopeMode, members: &[u8]) -> Scope {
        Scope {
            id: id(n),
            ..scope(mode, members)
        }
    }

    /// Folders 1 (Privat) ⊃ 2 (Haus); projects 10 in 1, 11 in 2, 12 at the
    /// top with tag 30; tags 30 (IT) ⊃ 31 (Server). Scopes 200 (the one
    /// [`scope`] makes), 201 and 202 exist.
    struct World {
        folders: Vec<Folder>,
        projects: Vec<Project>,
        tags: Vec<Tag>,
        scopes: Vec<Scope>,
    }

    fn world() -> World {
        World {
            folders: vec![folder(1, None), folder(2, Some(1))],
            projects: vec![
                project(10, Some(1), &[]),
                project(11, Some(2), &[]),
                project(12, None, &[30]),
            ],
            tags: vec![tag(30, None), tag(31, Some(30))],
            scopes: [200, 201, 202]
                .into_iter()
                .map(|n| scope_with(n, ScopeMode::Only, &[]))
                .collect(),
        }
    }

    fn snapshot(world: &World) -> Snapshot<'_> {
        Snapshot {
            folders: &world.folders,
            projects: &world.projects,
            tags: &world.tags,
            scopes: &world.scopes,
            ..Snapshot::default()
        }
    }

    /// The world with folder 2 (Haus, under Privat) only in scope 201 and
    /// tag 31 (Server, under IT) only in scope 202 (ADR 0094).
    fn secluded_world() -> World {
        let mut world = world();
        world.folders[1].own_scope = Some(id(201));
        world.tags[1].own_scope = Some(id(202));
        world
    }

    fn lens_of<'a>(world: &'a World, scope: Option<&Scope>) -> Lens<'a> {
        Lens::new(scope, &snapshot(world), at(), &TimeZone::UTC)
    }

    #[test]
    fn without_a_scope_what_has_an_own_scope_is_hidden() {
        let world = secluded_world();
        let lens = lens_of(&world, None);
        assert!(lens.by_members(&task(50, Some(10), &[])));
        assert!(!lens.by_members(&task(51, Some(11), &[])), "in folder 2");
        assert!(!lens.by_members(&task(52, Some(10), &[31])), "tag 31");
        assert!(lens.by_members(&task(53, Some(10), &[30])), "IT has none");
        assert!(lens.project(&world.projects[0]));
        assert!(!lens.project(&world.projects[1]));
        assert!(lens.folder(&world.folders[0]));
        assert!(!lens.folder(&world.folders[1]));
        // No own scope anywhere: no scope shows everything, as before.
        let plain = self::world();
        let lens = lens_of(&plain, None);
        assert!(lens.by_members(&task(51, Some(11), &[31])));
        assert!(lens.folder(&plain.folders[1]));
    }

    #[test]
    fn its_own_scope_shows_it_with_only_as_if_ticked() {
        let world = secluded_world();
        // Scope 201 ticks only project 12, not folder 2.
        let lens = lens_of(&world, Some(&scope_with(201, ScopeMode::Only, &[12])));
        assert!(lens.by_members(&task(51, Some(11), &[])));
        assert!(lens.project(&world.projects[1]));
        assert!(lens.folder(&world.folders[1]));
        assert!(lens.by_members(&task(52, Some(12), &[])), "ticked");
        assert!(!lens.by_members(&task(50, Some(10), &[])), "not ticked");
        // Folder 1 holds folder 2, which this scope shows.
        assert!(lens.folder(&world.folders[0]));
    }

    /// The case of 13.11.2026: „alles außer“ in the project's own scope.
    #[test]
    fn its_own_scope_shows_it_with_except_unless_ticked() {
        let world = secluded_world();
        let lens = lens_of(&world, Some(&scope_with(201, ScopeMode::Except, &[10])));
        assert!(lens.by_members(&task(51, Some(11), &[])));
        assert!(lens.project(&world.projects[1]));
        assert!(lens.folder(&world.folders[1]));
        assert!(!lens.by_members(&task(50, Some(10), &[])), "ticked");
        assert!(lens.by_members(&task(52, Some(12), &[])));
        // Ticked there, it is left out like anything else.
        let lens = lens_of(&world, Some(&scope_with(201, ScopeMode::Except, &[2])));
        assert!(!lens.by_members(&task(51, Some(11), &[])));
        assert!(!lens.project(&world.projects[1]));
        // Without a tick it shows everything, this too.
        let lens = lens_of(&world, Some(&scope_with(201, ScopeMode::Except, &[])));
        assert!(lens.by_members(&task(51, Some(11), &[])));
    }

    #[test]
    fn every_other_scope_hides_it_whatever_it_ticks() {
        let world = secluded_world();
        let in_haus = task(51, Some(11), &[]);
        for (mode, members) in [
            // Naming it no longer unlocks it (ADR 0069 §3 is replaced).
            (ScopeMode::Only, &[2][..]),
            (ScopeMode::Only, &[11][..]),
            (ScopeMode::Only, &[][..]),
            (ScopeMode::Except, &[30][..]),
            (ScopeMode::Except, &[][..]),
            (ScopeMode::Unknown, &[2][..]),
        ] {
            let lens = lens_of(&world, Some(&scope(mode, members)));
            assert!(!lens.by_members(&in_haus), "{mode:?} {members:?}");
            assert!(!lens.project(&world.projects[1]), "{mode:?} {members:?}");
            assert!(!lens.folder(&world.folders[1]), "{mode:?} {members:?}");
        }
    }

    #[test]
    fn the_nearest_folder_with_an_own_scope_counts() {
        let mut world = secluded_world();
        world.folders[0].own_scope = Some(id(200));
        let in_haus = task(51, Some(11), &[]);
        let in_privat = task(50, Some(10), &[]);
        let haus = lens_of(&world, Some(&scope_with(201, ScopeMode::Only, &[])));
        assert!(haus.by_members(&in_haus));
        assert!(!haus.by_members(&in_privat));
        let privat = lens_of(&world, Some(&scope(ScopeMode::Only, &[1])));
        assert!(privat.by_members(&in_privat));
        assert!(!privat.by_members(&in_haus), "Haus is nearer");
        assert!(!privat.folder(&world.folders[1]));
        assert!(privat.folder(&world.folders[0]));
    }

    #[test]
    fn a_tag_with_an_own_scope_outweighs_the_inbox() {
        let world = secluded_world();
        let habit = task(50, None, &[31]);
        let plain = task(51, None, &[30]);
        assert!(!lens_of(&world, None).by_members(&habit));
        assert!(lens_of(&world, None).by_members(&plain));
        let mut ticked = scope(ScopeMode::Only, &[]);
        ticked.inbox = true;
        assert!(!lens_of(&world, Some(&ticked)).by_members(&habit));
        assert!(lens_of(&world, Some(&ticked)).by_members(&plain));
        // Its own scope shows it, inbox ticked or not.
        let own = scope_with(202, ScopeMode::Only, &[12]);
        assert!(lens_of(&world, Some(&own)).by_members(&habit));
        let mut own_inbox = own.clone();
        own_inbox.inbox = true;
        assert!(lens_of(&world, Some(&own_inbox)).by_members(&habit));
        let own_except = scope_with(202, ScopeMode::Except, &[12]);
        assert!(lens_of(&world, Some(&own_except)).by_members(&habit));
    }

    #[test]
    fn one_own_scope_hit_is_enough() {
        let world = secluded_world();
        // In folder 2 and tagged 31: both own scopes show it.
        let both = task(50, Some(11), &[31]);
        assert!(lens_of(&world, Some(&scope_with(201, ScopeMode::Only, &[12]))).by_members(&both));
        assert!(lens_of(&world, Some(&scope_with(202, ScopeMode::Only, &[12]))).by_members(&both));
        assert!(!lens_of(&world, None).by_members(&both));
        assert!(!lens_of(&world, Some(&scope(ScopeMode::Except, &[]))).by_members(&both));
    }

    #[test]
    fn an_own_scope_that_does_not_exist_counts_for_nothing() {
        let mut world = world();
        world.folders[1].own_scope = Some(id(209));
        let lens = lens_of(&world, None);
        assert!(lens.by_members(&task(51, Some(11), &[])));
        assert!(lens.folder(&world.folders[1]));
        let lens = lens_of(&world, Some(&scope(ScopeMode::Only, &[2])));
        assert!(lens.by_members(&task(51, Some(11), &[])));
    }

    #[test]
    fn what_has_an_own_scope_and_is_due_counts_for_the_hint_without_a_scope() {
        let world = secluded_world();
        let lens = lens_of(&world, None);
        let state = |due| TaskState {
            available: true,
            reasons: Vec::new(),
            due,
            today: false,
            inbox: false,
        };
        let habit = task(51, Some(11), &[]);
        assert!(lens.due_outside(&habit, &state(DueState::Overdue)));
        assert!(!lens.due_outside(&habit, &state(DueState::None)));
        let plain = task(50, Some(10), &[]);
        assert!(!lens.due_outside(&plain, &state(DueState::Overdue)));
    }

    #[test]
    fn only_a_folder_shows_its_projects_at_every_depth() {
        let world = world();
        let lens = Lens::new(
            Some(&scope(ScopeMode::Only, &[1])),
            &snapshot(&world),
            at(),
            &TimeZone::UTC,
        );
        assert!(lens.by_members(&task(50, Some(10), &[])));
        assert!(lens.by_members(&task(51, Some(11), &[])));
        assert!(!lens.by_members(&task(52, Some(12), &[])));
        assert!(lens.project(&world.projects[1]));
        assert!(!lens.project(&world.projects[2]));
    }

    #[test]
    fn except_a_tag_hides_its_children_and_projects_carrying_it() {
        let world = world();
        let lens = Lens::new(
            Some(&scope(ScopeMode::Except, &[30])),
            &snapshot(&world),
            at(),
            &TimeZone::UTC,
        );
        // A child tag of IT on the task itself.
        assert!(!lens.by_members(&task(50, Some(10), &[31])));
        // No tag on the task, but its project carries IT.
        assert!(!lens.by_members(&task(51, Some(12), &[])));
        assert!(!lens.project(&world.projects[2]));
        // Neither.
        assert!(lens.by_members(&task(52, Some(10), &[])));
        assert!(lens.project(&world.projects[0]));
    }

    #[test]
    fn the_inbox_is_in_as_its_tick_says() {
        let world = world();
        let inbox_task = task(50, None, &[30]);
        let with = |mode, inbox| {
            let mut scope = scope(mode, &[30]);
            scope.inbox = inbox;
            Lens::new(Some(&scope), &snapshot(&world), at(), &TimeZone::UTC).by_members(&inbox_task)
        };
        // ADR 0043 §1: `only` shows it ticked, `except` hides it ticked.
        assert!(with(ScopeMode::Only, true));
        assert!(!with(ScopeMode::Only, false));
        assert!(!with(ScopeMode::Except, true));
        // Its tag IT does not count: `except IT` without the tick shows it,
        // as before the tick.
        assert!(with(ScopeMode::Except, false));
        assert!(with(ScopeMode::Unknown, false));
    }

    #[test]
    fn a_project_as_member_covers_its_tasks() {
        let world = world();
        let lens = Lens::new(
            Some(&scope(ScopeMode::Only, &[11])),
            &snapshot(&world),
            at(),
            &TimeZone::UTC,
        );
        assert!(lens.by_members(&task(50, Some(11), &[])));
        assert!(!lens.by_members(&task(51, Some(10), &[])));
    }

    #[test]
    fn a_member_that_is_gone_counts_for_nothing() {
        let mut world = world();
        // Project 12 is deleted; a task still points at it.
        world.projects.pop();
        let lens = Lens::new(
            Some(&scope(ScopeMode::Only, &[12, 99])),
            &snapshot(&world),
            at(),
            &TimeZone::UTC,
        );
        assert!(!lens.by_members(&task(51, Some(10), &[])));
    }

    #[test]
    fn an_orphan_is_in_the_inbox() {
        let mut world = world();
        // Project 12 is deleted; a task still points at it (ADR 0036 §6).
        world.projects.pop();
        let mut ticked = scope(ScopeMode::Only, &[11]);
        ticked.inbox = true;
        let lens = Lens::new(Some(&ticked), &snapshot(&world), at(), &TimeZone::UTC);
        assert!(lens.by_members(&task(50, Some(12), &[])));
        assert!(
            !lens.by_members(&task(51, Some(10), &[])),
            "the probe hides one"
        );
        let lens = Lens::new(
            Some(&scope(ScopeMode::Only, &[11])),
            &snapshot(&world),
            at(),
            &TimeZone::UTC,
        );
        assert!(!lens.by_members(&task(50, Some(12), &[])));
    }

    #[test]
    fn an_unknown_mode_hides_nothing() {
        let world = world();
        let lens = Lens::new(
            Some(&scope(ScopeMode::Unknown, &[1])),
            &snapshot(&world),
            at(),
            &TimeZone::UTC,
        );
        assert!(lens.by_members(&task(50, Some(12), &[])));
    }

    #[test]
    fn only_open_due_tasks_outside_count_for_the_hint() {
        let world = world();
        let lens = Lens::new(
            Some(&scope(ScopeMode::Only, &[1])),
            &snapshot(&world),
            at(),
            &TimeZone::UTC,
        );
        let due = |due| TaskState {
            available: true,
            reasons: Vec::new(),
            due,
            today: false,
            inbox: false,
        };
        let outside = task(50, Some(12), &[]);
        assert!(lens.due_outside(&outside, &due(DueState::Overdue)));
        assert!(lens.due_outside(&outside, &due(DueState::Soon)));
        assert!(!lens.due_outside(&outside, &due(DueState::None)));
        let mut done = outside.clone();
        done.completed_at = Some(at());
        assert!(!lens.due_outside(&done, &due(DueState::Overdue)));
        let inside = task(51, Some(10), &[]);
        assert!(!lens.due_outside(&inside, &due(DueState::Overdue)));
    }

    /// ADR 0043 §3: members AND filter; what the filter leaves out but is
    /// due counts for the hint.
    #[test]
    fn the_filter_narrows_what_the_members_let_in() {
        let world = world();
        let mut flagged_only = scope(ScopeMode::Only, &[1]);
        flagged_only.filter = vec![samu_api::ScopeChip {
            kind: samu_api::ScopeChipKind::Is,
            value: "markiert".into(),
            not: false,
        }];
        let lens = Lens::new(Some(&flagged_only), &snapshot(&world), at(), &TimeZone::UTC);
        let open = |due| TaskState {
            available: true,
            reasons: Vec::new(),
            due,
            today: false,
            inbox: false,
        };
        let mut flagged = task(50, Some(10), &[]);
        flagged.flagged = true;
        assert!(lens.task(&flagged, &open(DueState::None)));
        let plain = task(51, Some(10), &[]);
        assert!(!lens.task(&plain, &open(DueState::None)), "the filter");
        let mut outside = task(52, Some(12), &[]);
        outside.flagged = true;
        assert!(!lens.task(&outside, &open(DueState::None)), "the members");
        assert!(lens.due_outside(&plain, &open(DueState::Overdue)));
    }

    fn scheduled(n: u8, position: i64, windows: serde_json::Value) -> Scope {
        let mut s = scope(ScopeMode::Only, &[]);
        s.id = id(n);
        s.position = position;
        s.schedule = serde_json::from_value(windows).unwrap();
        s
    }

    fn berlin() -> TimeZone {
        TimeZone::get("Europe/Berlin").unwrap()
    }

    /// ADR 0043 §7: weekdays 08–17, Saturday night over midnight; the
    /// smaller position wins an overlap; edges in the account's zone.
    #[test]
    fn the_schedule_picks_by_window_and_position() {
        let work = scheduled(
            1,
            1,
            serde_json::json!([{"days": ["mo", "tu", "we", "th", "fr"], "from": "08:00:00", "to": "17:00:00"}]),
        );
        let night = scheduled(
            2,
            0,
            serde_json::json!([{"days": ["sa"], "from": "22:00:00", "to": "02:00:00"}]),
        );
        let lunch = scheduled(
            3,
            0,
            serde_json::json!([{"days": ["mo"], "from": "12:00:00", "to": "13:00:00"}]),
        );
        let scopes = [work, night, lunch];
        // Monday 09.11.2026; Berlin is UTC+1.
        let at = |text: &str| -> Timestamp { text.parse().unwrap() };
        let now = schedule_now(&scopes, at("2026-11-09T09:00:00Z"), &berlin());
        assert_eq!(now.scope, Some(id(1)));
        assert_eq!(now.since, Some(at("2026-11-09T07:00:00Z")));
        assert_eq!(now.until, Some(at("2026-11-09T11:00:00Z")), "lunch begins");
        // Lunch overlaps work and has the smaller position.
        assert_eq!(
            schedule_now(&scopes, at("2026-11-09T11:30:00Z"), &berlin()).scope,
            Some(id(3))
        );
        // Monday 20:00: nothing.
        assert_eq!(
            schedule_now(&scopes, at("2026-11-09T19:00:00Z"), &berlin()).scope,
            None
        );
        // Sunday 01:00 in Berlin is still Saturday night's window.
        assert_eq!(
            schedule_now(&scopes, at("2026-11-15T00:00:00Z"), &berlin()).scope,
            Some(id(2))
        );
        assert_eq!(
            schedule_now(&scopes, at("2026-11-15T01:30:00Z"), &berlin()).scope,
            None
        );
        // 00:00 to 00:00 is a whole day.
        let sunday = scheduled(
            4,
            5,
            serde_json::json!([{"days": ["su"], "from": "00:00:00", "to": "00:00:00"}]),
        );
        let late_sunday = schedule_now(
            std::slice::from_ref(&sunday),
            at("2026-11-15T22:30:00Z"),
            &berlin(),
        );
        assert_eq!(late_sunday.scope, Some(id(4)));
        assert_eq!(
            schedule_now(&[sunday], at("2026-11-15T23:30:00Z"), &berlin()).scope,
            None,
            "Monday"
        );
        // Without windows: nothing, no edges.
        let none = schedule_now(
            &[scope(ScopeMode::Only, &[])],
            at("2026-11-09T09:00:00Z"),
            &berlin(),
        );
        assert_eq!((none.scope, none.since, none.until), (None, None, None));
    }

    #[test]
    fn windows_read_and_write_as_text() {
        let windows =
            parse_windows("mo-fr 08:00-17:00; sa 22:00-02:00; fr-mo 00:00-00:00").unwrap();
        assert_eq!(windows.len(), 3);
        assert_eq!(windows[0].days.len(), 5);
        assert_eq!(windows[2].days.len(), 4, "fr sa so mo");
        assert_eq!(
            windows_text(&windows),
            "mo,di,mi,do,fr 08:00-17:00; sa 22:00-02:00; fr,sa,so,mo 00:00-00:00"
        );
        assert_eq!(parse_windows(&windows_text(&windows)).unwrap(), windows);
        assert!(parse_windows("").unwrap().is_empty());
        for bad in ["xx 08:00-09:00", "mo", "mo 8-9", "mo 08:00"] {
            assert!(parse_windows(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_last_event_wins() {
        let at = |text: &str| -> Timestamp { text.parse().unwrap() };
        let schedule = ScheduleNow {
            scope: Some(id(1)),
            since: Some(at("2026-11-09T07:00:00Z")),
            until: Some(at("2026-11-09T16:00:00Z")),
        };
        // Chosen by hand after the window began: the hand holds.
        assert_eq!(
            follow(Some((Some(id(2)), at("2026-11-09T08:00:00Z"))), &schedule),
            Some(id(2))
        );
        assert_eq!(
            follow(Some((None, at("2026-11-09T08:00:00Z"))), &schedule),
            None
        );
        // Before: the window began later and wins.
        assert_eq!(
            follow(Some((Some(id(2)), at("2026-11-09T06:00:00Z"))), &schedule),
            Some(id(1))
        );
        assert_eq!(follow(None, &schedule), Some(id(1)));
    }

    /// ADR 0044: without a tick the members do not narrow, only the
    /// filter; a last member that is gone keeps `only` narrow.
    #[test]
    fn a_scope_without_ticks_narrows_by_its_filter_alone() {
        let world = world();
        let mut flagged = scope(ScopeMode::Only, &[]);
        flagged.filter = vec![samu_api::ScopeChip {
            kind: samu_api::ScopeChipKind::Is,
            value: "markiert".into(),
            not: false,
        }];
        let lens = Lens::new(Some(&flagged), &snapshot(&world), at(), &TimeZone::UTC);
        let open = TaskState {
            available: true,
            reasons: Vec::new(),
            due: DueState::None,
            today: false,
            inbox: false,
        };
        let mut marked = task(50, Some(12), &[]);
        marked.flagged = true;
        assert!(lens.task(&marked, &open));
        assert!(
            !lens.task(&task(51, Some(12), &[]), &open),
            "the filter still"
        );
        let mut in_inbox = task(52, None, &[]);
        in_inbox.flagged = true;
        assert!(lens.task(&in_inbox, &open));
        assert!(lens.project(&world.projects[2]));
        assert!(lens.folder(&world.folders[0]));
        for mode in [ScopeMode::Only, ScopeMode::Except] {
            let plain = Lens::new(
                Some(&scope(mode, &[])),
                &snapshot(&world),
                at(),
                &TimeZone::UTC,
            );
            assert!(plain.by_members(&task(53, Some(10), &[])), "{mode:?}");
        }
        // The only member is gone: `only` stays narrow.
        let gone = Lens::new(
            Some(&scope(ScopeMode::Only, &[99])),
            &snapshot(&world),
            at(),
            &TimeZone::UTC,
        );
        assert!(!gone.by_members(&task(54, Some(10), &[])));
    }

    #[test]
    fn a_cycle_ends_the_walk() {
        let mut world = world();
        world.folders = vec![folder(1, Some(2)), folder(2, Some(1))];
        world.tags = vec![tag(30, Some(31)), tag(31, Some(30))];
        let lens = Lens::new(
            Some(&scope(ScopeMode::Only, &[99])),
            &snapshot(&world),
            at(),
            &TimeZone::UTC,
        );
        assert!(!lens.by_members(&task(50, Some(11), &[31])));
        assert!(!lens.folder(&world.folders[0]));
    }

    #[test]
    fn a_folder_shows_when_it_holds_a_shown_project() {
        let world = world();
        let only_it = Lens::new(
            Some(&scope(ScopeMode::Only, &[30])),
            &snapshot(&world),
            at(),
            &TimeZone::UTC,
        );
        assert!(!only_it.folder(&world.folders[0]));
        let only_house = Lens::new(
            Some(&scope(ScopeMode::Only, &[11])),
            &snapshot(&world),
            at(),
            &TimeZone::UTC,
        );
        assert!(only_house.folder(&world.folders[0]));
        assert!(only_house.folder(&world.folders[1]));
        // An empty folder follows its own membership.
        drop((only_it, only_house));
        let mut world = world;
        world.folders.push(folder(3, None));
        let except_three = Lens::new(
            Some(&scope(ScopeMode::Except, &[3])),
            &snapshot(&world),
            at(),
            &TimeZone::UTC,
        );
        assert!(!except_three.folder(&world.folders[2]));
        assert!(except_three.folder(&world.folders[0]));
    }
}
