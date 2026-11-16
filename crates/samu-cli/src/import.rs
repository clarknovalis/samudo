// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `samu import org`: the open tasks of an Org agenda as changes (ADR 0002
//! §4, ADR 0012 §4, ADR 0023). It may run as often as wanted, with any
//! directories, into one account: what it brought before it knows by
//! `org_id` or `org_origin` and leaves alone (ADR 0051). [`plan`] is pure: files in,
//! changes and messages out. Reading the directory and sending live in
//! [`files_of`] and `main`.
//!
//! What maps to what was decided with the plan of 07.11.2026: `inbox.org`
//! is the inbox, every other file a single-actions project, a heading
//! tagged `:prj:` a project, an open heading under an open one an action
//! group; NEXT and `[#A]` flag, BACKLOG is the on-hold tag `Someday`,
//! WAITING the on-hold tag `Waiting`; SCHEDULED is `planned`, DEADLINE
//! `due`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};
use samu_api::{AnyObject, Change, Hlc, Id, ObjectKind};
use serde_json::{Value, json};

use crate::org::{self, Heading, Keywords, RepeatKind, Stamp, State};

/// What `source` says on everything the import makes (ADR 0023 §5).
pub(crate) const SOURCE: &str = "org-import";

/// Where in the files an object came from: `dir/file.org` for a file's
/// project, `dir/file.org::Above/Heading` for a heading (ADR 0051 §2).
const ORIGIN: &str = "org_origin";

/// Tags that say where a heading goes, not what it is.
const STRUCTURAL_TAGS: &[&str] = &["prj", "inbox"];

const SOMEDAY: &str = "Someday";
const WAITING: &str = "Waiting";

/// What the import made of the files.
#[derive(Debug, Default)]
pub(crate) struct Plan {
    pub(crate) changes: Vec<Change>,
    /// `file:line: message`, for stderr.
    pub(crate) messages: Vec<String>,
    pub(crate) projects: usize,
    pub(crate) tasks: usize,
    pub(crate) tags: usize,
    pub(crate) with_org_id: usize,
    /// Open headings in the account already, left as they are (ADR 0051).
    pub(crate) known: usize,
}

/// The objects already in the account that the import looks at.
#[derive(Debug, Default)]
pub(crate) struct Existing {
    /// Tags by title.
    pub(crate) tags: BTreeMap<String, Id>,
    /// What is there by `id:<org_id>` and `origin:<org_origin>` (ADR 0051
    /// §2).
    known: BTreeMap<String, Made>,
    /// Imported before `org_origin` (ADR 0051 §3): projects and tasks by
    /// title, wherever they were moved since.
    old_projects: BTreeMap<String, Id>,
    old_tasks: BTreeMap<String, Made>,
    /// Projects without `org_origin` by title (from the first import or
    /// made by hand): a file's project is the one named like the file
    /// (ADR 0051 §3). One with an origin belongs to its own directory.
    project_titles: BTreeMap<String, Id>,
    /// The next free position per container (`None` = the inbox), and
    /// the number of projects: new ones go behind.
    positions: BTreeMap<Option<Id>, i64>,
    projects: i64,
}

impl Existing {
    pub(crate) fn from_objects(objects: &[AnyObject]) -> Self {
        let mut existing = Self::default();
        for object in objects {
            match object {
                AnyObject::Tag(tag) => {
                    existing.tags.entry(tag.title.clone()).or_insert(tag.id);
                }
                AnyObject::Task(task) => {
                    let made = Made::Task(task.id, task.project);
                    existing.remember(&task.properties, made);
                    if is_old_import(&task.properties) {
                        existing.old_tasks.entry(task.title.clone()).or_insert(made);
                    }
                    let next = existing
                        .positions
                        .entry(task.parent.or(task.project))
                        .or_insert(0);
                    *next = (*next).max(task.position + 1);
                }
                AnyObject::Project(project) => {
                    existing.remember(&project.properties, Made::Project(project.id));
                    if is_old_import(&project.properties) {
                        existing
                            .old_projects
                            .entry(project.title.clone())
                            .or_insert(project.id);
                    }
                    if !project.properties.contains_key(ORIGIN) {
                        existing
                            .project_titles
                            .entry(project.title.clone())
                            .or_insert(project.id);
                    }
                    existing.projects = existing.projects.max(project.position + 1);
                }
                _ => {}
            }
        }
        existing
    }

    fn remember(&mut self, properties: &BTreeMap<String, String>, made: Made) {
        if let Some(id) = properties.get("org_id") {
            self.known.entry(format!("id:{id}")).or_insert(made);
        }
        if let Some(origin) = properties.get(ORIGIN) {
            self.known.entry(format!("origin:{origin}")).or_insert(made);
        }
    }

    /// What the account has for a heading: by its `:ID:`, else by where
    /// it stands in the files.
    fn find(&self, org_id: Option<&str>, origin: &str) -> Option<Made> {
        org_id
            .and_then(|id| self.known.get(&format!("id:{id}")))
            .or_else(|| self.known.get(&format!("origin:{origin}")))
            .copied()
    }

    /// What the account has for an open heading, a task or a `:prj:`
    /// project: as [`Existing::find`], else from an import before origins
    /// by its title (ADR 0051 §3).
    fn find_heading(&self, heading: &Heading, origin: &str) -> Option<Made> {
        self.find(heading.property("ID"), origin).or_else(|| {
            if heading.tags.iter().any(|t| t == "prj") {
                self.old_projects
                    .get(&heading.title)
                    .map(|id| Made::Project(*id))
            } else {
                self.old_tasks.get(&heading.title).copied()
            }
        })
    }
}

/// Made by an import from before `org_origin` (ADR 0051 §3).
fn is_old_import(properties: &BTreeMap<String, String>) -> bool {
    properties.get("source").map(String::as_str) == Some(SOURCE) && !properties.contains_key(ORIGIN)
}

/// The Org files under `dir`, by path relative to it, sorted; `.git` and
/// other hidden directories left out.
pub(crate) fn files_of(dir: &Path) -> Result<Vec<(String, String)>> {
    fn walk(dir: &Path, found: &mut Vec<PathBuf>) -> Result<()> {
        for entry in std::fs::read_dir(dir).with_context(|| format!("{}", dir.display()))? {
            let path = entry?.path();
            let hidden = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.'));
            if hidden {
                continue;
            }
            if path.is_dir() {
                walk(&path, found)?;
            } else if path.extension().is_some_and(|e| e == "org") {
                found.push(path);
            }
        }
        Ok(())
    }
    let mut paths = Vec::new();
    walk(dir, &mut paths)?;
    paths.sort();
    let mut files = Vec::new();
    for path in paths {
        let relative = path
            .strip_prefix(dir)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let text = std::fs::read_to_string(&path).with_context(|| format!("{}", path.display()))?;
        files.push((relative, text));
    }
    Ok(files)
}

/// Where an imported heading went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Made {
    Project(Id),
    /// A task, with the project it is in.
    Task(Id, Option<Id>),
}

struct Builder<'a> {
    zone: &'a TimeZone,
    now: Timestamp,
    plan: Plan,
    tags: BTreeMap<String, Id>,
    /// The next position per container (`None` = the inbox), behind what
    /// the account has.
    positions: BTreeMap<Option<Id>, i64>,
    /// The next position of a project.
    project_position: i64,
    /// Tags left out on request, like `prv` from `#+FILETAGS:` in an
    /// account that is private anyway (ADR 0005).
    drop_tags: Vec<String>,
}

impl Builder<'_> {
    fn hlc() -> Hlc {
        Hlc::new(0, 0, "cli")
    }

    fn position(&mut self, container: Option<Id>) -> i64 {
        let next = self.positions.entry(container).or_insert(0);
        let position = *next;
        *next += 1;
        position
    }

    fn tag(&mut self, title: &str, on_hold: bool) -> Id {
        if let Some(id) = self.tags.get(title) {
            return *id;
        }
        let id = Id::new();
        let now = self.now.to_string();
        let position = i64::try_from(self.plan.tags).unwrap_or(0);
        self.plan.changes.push(Change::Create {
            object: id,
            kind: ObjectKind::Tag,
            value: json!({
                "id": id,
                "created": now,
                "modified": now,
                "title": title,
                "status": if on_hold { "on_hold" } else { "active" },
                "exclusive": false,
                "position": position,
            }),
            hlc: Self::hlc(),
        });
        self.tags.insert(title.to_string(), id);
        self.plan.tags += 1;
        id
    }

    fn when(&self, stamp: &Stamp) -> Result<Value, String> {
        match stamp.time {
            None => Ok(json!({"date": stamp.date})),
            Some(time) => {
                let zoned: Zoned = stamp
                    .date
                    .to_datetime(time)
                    .to_zoned(self.zone.clone())
                    .map_err(|e| e.to_string())?;
                Ok(json!({"at": zoned}))
            }
        }
    }

    fn created(&self, heading: &Heading) -> String {
        let from = heading
            .property("CREATED")
            .map(str::to_string)
            .or_else(|| heading.body.iter().find(|l| !l.trim().is_empty()).cloned());
        from.and_then(|text| org::inactive_stamp(&text))
            .and_then(|(date, time)| {
                date.to_datetime(time.unwrap_or_default())
                    .to_zoned(self.zone.clone())
                    .ok()
            })
            .map_or_else(|| self.now.to_string(), |z| z.timestamp().to_string())
    }

    /// The fields a task and a project share.
    fn common(
        &mut self,
        heading: &Heading,
        tags: &[String],
        at: &str,
        origin: &str,
    ) -> serde_json::Map<String, Value> {
        let now = self.now.to_string();
        let mut value = serde_json::Map::new();
        let title = if heading.title.is_empty() {
            "(ohne Titel)".to_string()
        } else {
            heading.title.clone()
        };
        value.insert("created".into(), json!(self.created(heading)));
        value.insert("modified".into(), json!(now));
        value.insert("title".into(), json!(title));
        let note = note_of(&heading.body);
        if let Some(note) = note {
            value.insert("note".into(), json!(note));
        }
        let keyword = heading.keyword.as_deref().unwrap_or("");
        let flagged = keyword == "NEXT" || heading.priority == Some('A');
        value.insert("flagged".into(), json!(flagged));

        let mut names: Vec<(String, bool)> = tags
            .iter()
            .filter(|t| !STRUCTURAL_TAGS.contains(&t.as_str()) && !self.drop_tags.contains(t))
            .map(|t| (t.clone(), false))
            .collect();
        match keyword {
            "BACKLOG" => names.push((SOMEDAY.into(), true)),
            "WAITING" | "WAIT" | "HOLD" => names.push((WAITING.into(), true)),
            _ => {}
        }
        let mut taggings = Vec::new();
        for (name, on_hold) in names {
            let id = self.tag(&name, on_hold);
            if !taggings.iter().any(|t: &Value| t["tag"] == json!(id)) {
                let position = taggings.len();
                taggings.push(json!({"tag": id, "position": position}));
            }
        }
        if !taggings.is_empty() {
            value.insert("tags".into(), Value::Array(taggings));
        }

        if let Some(stamp) = &heading.scheduled {
            self.date_field(&mut value, "planned", stamp, at);
        }
        if let Some(stamp) = &heading.deadline {
            self.date_field(&mut value, "due", stamp, at);
            if let Some((n, unit)) = stamp.warning {
                value.insert("due_warning".into(), json!(period(n, unit)));
            }
        }
        let repeat = heading
            .scheduled
            .as_ref()
            .and_then(|s| s.repeat)
            .or_else(|| heading.deadline.as_ref().and_then(|d| d.repeat));
        if let Some(repeater) = repeat {
            match rrule(repeater.n, repeater.unit) {
                Some(rule) => {
                    let (mode, catch_up) = match repeater.kind {
                        RepeatKind::Plus => ("fixed", false),
                        RepeatKind::CatchUp => ("fixed", true),
                        RepeatKind::FromDone => ("due_after_completion", false),
                    };
                    value.insert(
                        "repeat".into(),
                        json!({"rule": rule, "mode": mode, "catch_up": catch_up}),
                    );
                }
                None => self.plan.messages.push(format!(
                    "{at}: the repeater every {}{} has no rule here, left out",
                    repeater.n, repeater.unit
                )),
            }
        }

        let mut properties = serde_json::Map::new();
        properties.insert("source".into(), json!(SOURCE));
        properties.insert(ORIGIN.into(), json!(origin));
        if let Some(id) = heading.property("ID") {
            properties.insert("org_id".into(), json!(id));
            self.plan.with_org_id += 1;
        }
        value.insert("properties".into(), Value::Object(properties));
        for problem in &heading.problems {
            self.plan
                .messages
                .push(format!("{at}: {problem}, left out"));
        }
        value
    }

    fn date_field(
        &mut self,
        value: &mut serde_json::Map<String, Value>,
        field: &str,
        stamp: &Stamp,
        at: &str,
    ) {
        match self.when(stamp) {
            Ok(when) => {
                value.insert(field.into(), when);
            }
            Err(problem) => self
                .plan
                .messages
                .push(format!("{at}: {field}: {problem}, left out")),
        }
        if stamp.has_end {
            self.plan.messages.push(format!(
                "{at}: {field}: the end of the time range is left out"
            ));
        }
    }

    fn project(
        &mut self,
        title: Option<&str>,
        heading: Option<(&Heading, &[String], &str)>,
        origin: &str,
    ) -> Id {
        let id = Id::new();
        let now = self.now.to_string();
        let mut value = match heading {
            Some((heading, tags, at)) => self.common(heading, tags, at, origin),
            None => {
                let mut value = serde_json::Map::new();
                value.insert("created".into(), json!(now));
                value.insert("modified".into(), json!(now));
                value.insert("title".into(), json!(title.unwrap_or("")));
                value.insert("flagged".into(), json!(false));
                value.insert(
                    "properties".into(),
                    json!({"source": SOURCE, ORIGIN: origin}),
                );
                value
            }
        };
        let kind = if heading.is_some() {
            "parallel"
        } else {
            "single_actions"
        };
        value.insert("id".into(), json!(id));
        value.insert("kind".into(), json!(kind));
        value.insert("status".into(), json!("active"));
        value.insert("position".into(), json!(self.project_position));
        self.project_position += 1;
        self.plan.changes.push(Change::Create {
            object: id,
            kind: ObjectKind::Project,
            value: Value::Object(value),
            hlc: Self::hlc(),
        });
        self.plan.projects += 1;
        id
    }

    fn task(
        &mut self,
        heading: &Heading,
        tags: &[String],
        at: &str,
        project: Option<Id>,
        parent: Option<Id>,
        origin: &str,
    ) -> Id {
        let id = Id::new();
        let mut value = self.common(heading, tags, at, origin);
        value.insert("id".into(), json!(id));
        if let Some(project) = project {
            value.insert("project".into(), json!(project));
        }
        if let Some(parent) = parent {
            value.insert("parent".into(), json!(parent));
        }
        value.insert("sequential".into(), json!(false));
        value.insert("complete_with_last_child".into(), json!(false));
        let position = self.position(parent.or(project));
        value.insert("position".into(), json!(position));
        self.plan.changes.push(Change::Create {
            object: id,
            kind: ObjectKind::Task,
            value: Value::Object(value),
            hlc: Self::hlc(),
        });
        self.plan.tasks += 1;
        id
    }
}

/// Headings that are not imported keep their text in the note of the
/// nearest open heading above them: each with its own text, in order. The
/// subtree of an open heading below belongs to that one.
fn with_subtrees(headings: &[Heading]) -> Vec<Heading> {
    let mut out: Vec<Heading> = headings.to_vec();
    for (i, heading) in headings.iter().enumerate() {
        if heading.state != State::Open {
            continue;
        }
        let mut inside_open: Option<usize> = None;
        for below in &headings[i + 1..] {
            if below.level <= heading.level {
                break;
            }
            if inside_open.is_some_and(|level| below.level > level) {
                continue;
            }
            inside_open = None;
            if below.state == State::Open {
                inside_open = Some(below.level);
                continue;
            }
            out[i].body.extend(below.raw.iter().cloned());
            out[i].body.extend(below.body.iter().cloned());
        }
    }
    out
}

/// The note of a heading: its text, without leading and trailing blank
/// lines; none if nothing is left.
fn note_of(body: &[String]) -> Option<String> {
    let start = body.iter().position(|l| !l.trim().is_empty())?;
    let end = body.iter().rposition(|l| !l.trim().is_empty())?;
    Some(body[start..=end].join("\n"))
}

/// An Org duration as ISO 8601.
fn period(n: u32, unit: char) -> String {
    match unit {
        'h' => format!("PT{n}H"),
        'w' => format!("P{n}W"),
        'm' => format!("P{n}M"),
        'y' => format!("P{n}Y"),
        _ => format!("P{n}D"),
    }
}

/// A repeater as RRULE (ADR 0007 §3); hours have none in samu.
fn rrule(n: u32, unit: char) -> Option<String> {
    let freq = match unit {
        'd' => "DAILY",
        'w' => "WEEKLY",
        'm' => "MONTHLY",
        'y' => "YEARLY",
        _ => return None,
    };
    Some(if n > 1 {
        format!("FREQ={freq};INTERVAL={n}")
    } else {
        format!("FREQ={freq}")
    })
}

/// The changes that bring the open headings of `files`, read from the
/// directory named `dir`, into an account. What the account has from an
/// earlier run stays as it is and takes what is new below it (ADR 0051);
/// tags already there by the same title are reused; tags in `drop_tags`
/// are left out.
pub(crate) fn plan(
    files: &[(String, String)],
    dir: &str,
    zone: &TimeZone,
    now: Timestamp,
    existing: &Existing,
    drop_tags: &[String],
) -> Plan {
    let mut builder = Builder {
        zone,
        now,
        plan: Plan::default(),
        tags: existing.tags.clone(),
        positions: existing.positions.clone(),
        project_position: existing.projects,
        drop_tags: drop_tags.to_vec(),
    };
    let fallback = Keywords::fallback();
    for (path, text) in files {
        let file = org::parse(text, &fallback);
        let inbox = path == "inbox.org";
        let mut file_project: Option<Id> = None;
        // Per level above the current heading: the heading's tags, what it
        // became, and its title.
        let mut stack: Vec<(usize, Vec<String>, Slot, String)> = Vec::new();
        let headings = with_subtrees(&file.headings);
        for heading in &headings {
            while stack
                .last()
                .is_some_and(|(level, _, _, _)| *level >= heading.level)
            {
                stack.pop();
            }
            if heading.state != State::Open {
                stack.push((
                    heading.level,
                    heading.tags.clone(),
                    Slot::Skipped,
                    heading.title.clone(),
                ));
                continue;
            }
            let at = format!("{path}:{}", heading.line);
            let trail: Vec<String> = stack
                .iter()
                .map(|(_, _, _, title)| title.clone())
                .chain([heading.title.clone()])
                .collect();
            let origin = format!("{dir}/{path}::{}", trail.join("/"));
            // Tags up to the nearest imported ancestor, which carries the
            // rest itself (tags inherit in samu, ADR 0017).
            let mut tags = heading.tags.clone();
            let mut above: Option<usize> = None;
            for (i, (_, ancestor_tags, slot, _)) in stack.iter().enumerate().rev() {
                if !matches!(slot, Slot::Skipped) {
                    above = Some(i);
                    break;
                }
                tags.extend(ancestor_tags.iter().cloned());
            }
            if above.is_none() {
                tags.extend(file.filetags.iter().cloned());
            }
            let slot = if let Some(made) = existing.find_heading(heading, &origin) {
                builder.plan.known += 1;
                Slot::Made(made)
            } else if heading.tags.iter().any(|t| t == "prj") {
                let pending = Pending {
                    heading: heading.clone(),
                    tags: tags.clone(),
                    at: at.clone(),
                    origin: origin.clone(),
                };
                // Made when the first new task goes into it: a `:prj:`
                // heading without open tasks makes no project, like a file
                // without them, and one renamed in samu does not come
                // twice (ADR 0051 §4).
                Slot::Pending(Box::new(pending))
            } else {
                let container = above.and_then(|i| resolve(&mut builder, &mut stack[i].2));
                let (project, parent) = match container {
                    Some(Made::Project(project)) => (Some(project), None),
                    Some(Made::Task(task, project)) => (project, Some(task)),
                    None if inbox => (None, None),
                    None => {
                        let project = match file_project {
                            Some(project) => project,
                            None => {
                                let project = file_project_of(&mut builder, existing, dir, path);
                                file_project = Some(project);
                                project
                            }
                        };
                        (Some(project), None)
                    }
                };
                Slot::Made(Made::Task(
                    builder.task(heading, &tags, &at, project, parent, &origin),
                    project,
                ))
            };
            stack.push((
                heading.level,
                heading.tags.clone(),
                slot,
                heading.title.clone(),
            ));
        }
    }
    builder.plan
}

/// What stands at a level above a heading.
enum Slot {
    /// Not imported: done, or plain text.
    Skipped,
    Made(Made),
    /// A `:prj:` heading not made yet (ADR 0051 §4).
    Pending(Box<Pending>),
}

/// A `:prj:` heading kept back until something new goes into it.
struct Pending {
    heading: Heading,
    tags: Vec<String>,
    at: String,
    origin: String,
}

impl Pending {
    fn make(&self, builder: &mut Builder<'_>) -> Id {
        builder.project(
            None,
            Some((&self.heading, &self.tags, &self.at)),
            &self.origin,
        )
    }
}

/// What a slot above stands for, making a kept-back project now; none for
/// a skipped one.
fn resolve(builder: &mut Builder<'_>, slot: &mut Slot) -> Option<Made> {
    match slot {
        Slot::Made(made) => Some(*made),
        Slot::Pending(pending) => {
            let made = Made::Project(pending.make(builder));
            *slot = Slot::Made(made);
            Some(made)
        }
        Slot::Skipped => None,
    }
}

/// The project a file becomes: the one the account has for it, else a new
/// one named after the file.
fn file_project_of(builder: &mut Builder<'_>, existing: &Existing, dir: &str, path: &str) -> Id {
    let origin = format!("{dir}/{path}");
    let title = path.strip_suffix(".org").unwrap_or(path);
    match existing.find(None, &origin) {
        Some(Made::Project(id)) => id,
        _ => match existing.project_titles.get(title) {
            Some(id) => *id,
            None => builder.project(Some(title), None, &origin),
        },
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::*;

    fn berlin() -> TimeZone {
        TimeZone::get("Europe/Berlin").unwrap()
    }

    fn now() -> Timestamp {
        "2026-11-07T09:00:00Z".parse().unwrap()
    }

    fn run(files: &[(&str, &str)]) -> Plan {
        let files: Vec<(String, String)> = files
            .iter()
            .map(|(p, t)| ((*p).to_string(), (*t).to_string()))
            .collect();
        plan(
            &files,
            "agenda",
            &berlin(),
            now(),
            &Existing::default(),
            &[],
        )
    }

    /// The account after `first` was sent: its creates read as objects.
    fn account_after(first: &Plan) -> Vec<AnyObject> {
        first
            .changes
            .iter()
            .filter_map(|c| match c {
                Change::Create { kind, value, .. } => {
                    let raw = json!({"kind": kind, "object": value});
                    Some(serde_json::from_value(raw).unwrap())
                }
                _ => None,
            })
            .collect()
    }

    fn run_into(objects: &[AnyObject], dir: &str, files: &[(&str, &str)]) -> Plan {
        let files: Vec<(String, String)> = files
            .iter()
            .map(|(p, t)| ((*p).to_string(), (*t).to_string()))
            .collect();
        plan(
            &files,
            dir,
            &berlin(),
            now(),
            &Existing::from_objects(objects),
            &[],
        )
    }

    /// The created objects of one kind, as JSON.
    fn of(plan: &Plan, kind: ObjectKind) -> Vec<Value> {
        plan.changes
            .iter()
            .filter_map(|c| match c {
                Change::Create { kind: k, value, .. } if *k == kind => Some(value.clone()),
                _ => None,
            })
            .collect()
    }

    fn titled<'a>(objects: &'a [Value], title: &str) -> &'a Value {
        objects
            .iter()
            .find(|o| o["title"] == title)
            .unwrap_or_else(|| panic!("no {title:?} in {objects:#?}"))
    }

    #[test]
    fn the_inbox_stays_the_inbox_and_a_file_becomes_a_project() {
        let plan = run(&[
            ("inbox.org", "* TODO Milch\n* DONE Brot\n"),
            (
                "gtd.org",
                "* Abschnitt\n* NEXT Rauchmelder\n* TODO Keller\n",
            ),
        ]);
        let projects = of(&plan, ObjectKind::Project);
        let tasks = of(&plan, ObjectKind::Task);
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0]["title"], "gtd");
        assert_eq!(projects[0]["kind"], "single_actions");
        assert_eq!(tasks.len(), 3, "done and plain headings stay out");
        assert!(titled(&tasks, "Milch").get("project").is_none());
        let rauch = titled(&tasks, "Rauchmelder");
        assert_eq!(rauch["project"], projects[0]["id"]);
        assert_eq!(rauch["flagged"], true);
        assert_eq!(rauch["position"], 0);
        assert_eq!(titled(&tasks, "Keller")["position"], 1);
        assert_eq!(
            rauch["properties"],
            json!({"source": SOURCE, "org_origin": "agenda/gtd.org::Rauchmelder"})
        );
    }

    #[test]
    fn a_prj_heading_without_open_tasks_makes_no_project() {
        let plan = run(&[("admin.org", "* Admin :prj:\n** DONE alt\n")]);
        assert!(plan.changes.is_empty(), "{:#?}", plan.changes);
    }

    #[test]
    fn a_file_without_open_tasks_makes_no_project() {
        let plan = run(&[("birthdays.org", "* Cosma\n<2008-03-01 +1y>\n* DONE x\n")]);
        assert!(plan.changes.is_empty());
    }

    #[test]
    fn a_prj_heading_is_a_project_and_nesting_is_an_action_group() {
        let plan = run(&[(
            "admin_tasks.org",
            "* REVIEW [/] admin-tasks :prj:it:\n\
             ** TODO Zertifikate\n\
             *** TODO erneuern\n\
             ** DONE alt\n\
             *** TODO unter Erledigtem\n",
        )]);
        let projects = of(&plan, ObjectKind::Project);
        assert_eq!(projects.len(), 1, "no file project: everything is in :prj:");
        let project = &projects[0];
        assert_eq!(project["title"], "admin-tasks");
        assert_eq!(project["kind"], "parallel");
        let tags = of(&plan, ObjectKind::Tag);
        assert_eq!(tags.len(), 1, "prj is structural: {tags:#?}");
        assert_eq!(tags[0]["title"], "it");
        assert_eq!(project["tags"][0]["tag"], tags[0]["id"]);

        let tasks = of(&plan, ObjectKind::Task);
        let group = titled(&tasks, "Zertifikate");
        assert_eq!(group["project"], project["id"]);
        assert!(group.get("tags").is_none(), "inherited from the project");
        let child = titled(&tasks, "erneuern");
        assert_eq!(child["parent"], group["id"]);
        assert_eq!(child["project"], project["id"]);
        let orphan = titled(&tasks, "unter Erledigtem");
        assert_eq!(orphan["project"], project["id"]);
        assert!(orphan.get("parent").is_none());
    }

    #[test]
    fn keywords_become_flags_and_on_hold_tags_and_tags_are_made_once() {
        let plan = run(&[(
            "it.org",
            "#+FILETAGS: :haus:\n\
             * BACKLOG dns :it:\n\
             * WAITING Antwort :it:\n\
             * TODO [#A] dringend\n\
             * TODO [#B] normal\n",
        )]);
        let tags = of(&plan, ObjectKind::Tag);
        let titles: Vec<&str> = tags.iter().map(|t| t["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["it", "haus", "Someday", "Waiting"]);
        assert_eq!(titled(&tags, "Someday")["status"], "on_hold");
        assert_eq!(titled(&tags, "it")["status"], "active");
        let tasks = of(&plan, ObjectKind::Task);
        let dns = titled(&tasks, "dns");
        assert_eq!(dns["tags"].as_array().unwrap().len(), 3);
        assert_eq!(dns["flagged"], false);
        assert_eq!(titled(&tasks, "dringend")["flagged"], true);
        assert_eq!(titled(&tasks, "normal")["flagged"], false);
    }

    #[test]
    fn existing_tags_are_reused() {
        let mut existing = Existing::default();
        let it = Id::new();
        existing.tags.insert("it".into(), it);
        let files = vec![("it.org".to_string(), "* TODO x :it:\n".to_string())];
        let plan = plan(&files, "agenda", &berlin(), now(), &existing, &[]);
        assert!(of(&plan, ObjectKind::Tag).is_empty());
        assert_eq!(of(&plan, ObjectKind::Task)[0]["tags"][0]["tag"], json!(it));
    }

    #[test]
    fn dropped_tags_are_left_out_also_from_filetags() {
        let files = vec![(
            "it.org".to_string(),
            "#+FILETAGS: :prv:\n* TODO x :it:prv:\n* TODO y\n".to_string(),
        )];
        let plan = plan(
            &files,
            "agenda",
            &berlin(),
            now(),
            &Existing::default(),
            &["prv".into()],
        );
        let tags = of(&plan, ObjectKind::Tag);
        let titles: Vec<&str> = tags.iter().map(|t| t["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["it"]);
        let tasks = of(&plan, ObjectKind::Task);
        assert_eq!(titled(&tasks, "x")["tags"].as_array().unwrap().len(), 1);
        assert!(titled(&tasks, "y").get("tags").is_none());
    }

    #[test]
    fn dates_times_warnings_and_repeats() {
        let plan = run(&[(
            "gtd.org",
            "* REVIEW Sonntag Wochen REVIEW\n\
             SCHEDULED: <2026-05-25 Mon 19:30 .+1w>\n\
             * TODO Rechnung\n\
             DEADLINE: <2026-12-01 Tue -3d>\n\
             * TODO Geburtstag\n\
             SCHEDULED: <2026-03-01 Sun ++1y>\n\
             * TODO stündlich\n\
             SCHEDULED: <2026-03-01 Sun 10:00-11:00 +2h>\n",
        )]);
        let tasks = of(&plan, ObjectKind::Task);
        let review = titled(&tasks, "Sonntag Wochen REVIEW");
        assert_eq!(
            review["planned"],
            json!({"at": "2026-05-25T19:30:00+02:00[Europe/Berlin]"})
        );
        assert_eq!(
            review["repeat"],
            json!({"rule": "FREQ=WEEKLY", "mode": "due_after_completion", "catch_up": false})
        );
        let bill = titled(&tasks, "Rechnung");
        assert_eq!(bill["due"], json!({"date": "2026-12-01"}));
        assert_eq!(bill["due_warning"], "P3D");
        let birthday = titled(&tasks, "Geburtstag");
        assert_eq!(birthday["repeat"]["mode"], "fixed");
        assert_eq!(birthday["repeat"]["catch_up"], true);
        assert_eq!(birthday["repeat"]["rule"], "FREQ=YEARLY");
        let hourly = titled(&tasks, "stündlich");
        assert!(hourly.get("repeat").is_none());
        assert!(hourly.get("planned").is_some());
        assert_eq!(plan.messages.len(), 2, "{:#?}", plan.messages);
        assert!(plan.messages.iter().all(|m| m.starts_with("gtd.org:7: ")));
    }

    #[test]
    fn id_note_and_capture_time_are_kept() {
        let plan = run(&[(
            "inbox.org",
            "* TODO Rolladen\n\
             :PROPERTIES:\n\
             :ID:       3f0c2a1e-9d4b-4c7a\n\
             :END:\n\
             [2026-04-21 Tue 10:00]\n\
             \n\
             Gurt gerissen.\n\
             ** DONE Gurt bestellt\n\
             geliefert\n\
             * TODO\n",
        )]);
        let tasks = of(&plan, ObjectKind::Task);
        let task = titled(&tasks, "Rolladen");
        assert_eq!(
            task["properties"],
            json!({"org_id": "3f0c2a1e-9d4b-4c7a", "source": SOURCE, "org_origin": "agenda/inbox.org::Rolladen"})
        );
        assert_eq!(task["created"], "2026-04-21T08:00:00Z");
        assert_eq!(
            task["note"],
            "[2026-04-21 Tue 10:00]\n\nGurt gerissen.\n** DONE Gurt bestellt\ngeliefert"
        );
        assert_eq!(plan.with_org_id, 1);
        let empty = titled(&tasks, "(ohne Titel)");
        assert_eq!(empty["created"], now().to_string());
        assert!(empty.get("note").is_none());
    }

    #[test]
    fn text_below_stays_in_the_note_up_to_the_next_imported_heading() {
        let plan = run(&[(
            "inbox.org",
            "* TODO Verkaufen\n\
             Plan\n\
             ** Phase 1\n\
             *** Fotos\n\
             ** CANCELED Phase 0\n\
             CLOSED: [2026-05-01 Fri 10:00]\n\
             :LOGBOOK:\n\
             - x\n\
             :END:\n\
             alt\n\
             ** TODO Kamera listen\n\
             *** Notiz zur Kamera\n\
             ** Danach\n\
             * Abschnitt\n\
             Fremder Text\n",
        )]);
        let tasks = of(&plan, ObjectKind::Task);
        assert_eq!(
            titled(&tasks, "Verkaufen")["note"],
            "Plan\n** Phase 1\n*** Fotos\n** CANCELED Phase 0\n\
             CLOSED: [2026-05-01 Fri 10:00]\nalt\n** Danach"
        );
        let camera = titled(&tasks, "Kamera listen");
        assert_eq!(camera["note"], "*** Notiz zur Kamera");
        assert_eq!(camera["parent"], titled(&tasks, "Verkaufen")["id"]);
    }

    #[test]
    fn every_change_reads_as_its_wire_type() {
        let plan = run(&[
            ("inbox.org", "* NEXT a :x:\nSCHEDULED: <2026-11-08 Sun>\n"),
            (
                "p.org",
                "* REVIEW p :prj:\nDEADLINE: <2026-11-09 Mon>\n** TODO b\n",
            ),
        ]);
        for change in &plan.changes {
            let Change::Create { kind, value, .. } = change else {
                panic!("{change:?}");
            };
            let checked = match kind {
                ObjectKind::Task => serde_json::from_value::<samu_api::Task>(value.clone()).err(),
                ObjectKind::Project => {
                    serde_json::from_value::<samu_api::Project>(value.clone()).err()
                }
                ObjectKind::Tag => serde_json::from_value::<samu_api::Tag>(value.clone()).err(),
                other => panic!("{other:?}"),
            };
            assert!(checked.is_none(), "{value:#}: {checked:?}");
        }
        assert_eq!((plan.projects, plan.tasks, plan.tags), (1, 2, 1));
    }

    const AGENDA: &str =
        "* Abschnitt\n** TODO Keller :haus:\n* TODO Dach\n:PROPERTIES:\n:ID: dach-1\n:END:\n";

    #[test]
    fn a_second_run_over_the_same_files_makes_nothing() {
        let first = run_into(
            &[],
            "agenda",
            &[("inbox.org", "* TODO a\n"), ("gtd.org", AGENDA)],
        );
        assert_eq!((first.tasks, first.projects, first.known), (3, 1, 0));
        let keller = titled(&of(&first, ObjectKind::Task), "Keller").clone();
        assert_eq!(
            keller["properties"]["org_origin"],
            "agenda/gtd.org::Abschnitt/Keller"
        );
        assert_eq!(
            of(&first, ObjectKind::Project)[0]["properties"]["org_origin"],
            "agenda/gtd.org"
        );
        let account = account_after(&first);
        let again = run_into(
            &account,
            "agenda",
            &[("inbox.org", "* TODO a\n"), ("gtd.org", AGENDA)],
        );
        assert!(again.changes.is_empty(), "{:#?}", again.changes);
        assert_eq!(again.known, 3);
    }

    #[test]
    fn another_directory_with_the_same_file_names_comes_in() {
        let first = run_into(&[], "prv", &[("inbox.org", "* TODO a\n")]);
        let second = run_into(
            &account_after(&first),
            "wrk",
            &[("inbox.org", "* TODO a\n")],
        );
        assert_eq!((second.tasks, second.known), (1, 0));
    }

    #[test]
    fn an_org_id_is_known_wherever_it_moved() {
        let first = run_into(&[], "agenda", &[("gtd.org", AGENDA)]);
        // The same :ID: in another file of another directory.
        let moved = "* TODO Dach, anders\n:PROPERTIES:\n:ID: dach-1\n:END:\n";
        let second = run_into(&account_after(&first), "wrk", &[("neu.org", moved)]);
        assert_eq!((second.tasks, second.projects, second.known), (0, 0, 1));
    }

    #[test]
    fn something_new_goes_under_what_is_known_and_behind_it() {
        let first = run_into(&[], "agenda", &[("gtd.org", AGENDA)]);
        let account = account_after(&first);
        let project = of(&first, ObjectKind::Project)[0]["id"].clone();
        let dach = titled(&of(&first, ObjectKind::Task), "Dach")["id"].clone();
        let grown = format!("{AGENDA}** TODO Ziegel\n* TODO Garage\n");
        let second = run_into(&account, "agenda", &[("gtd.org", &grown)]);
        assert_eq!((second.tasks, second.projects, second.known), (2, 0, 2));
        let tasks = of(&second, ObjectKind::Task);
        let ziegel = titled(&tasks, "Ziegel");
        assert_eq!(ziegel["parent"], dach);
        assert_eq!(ziegel["position"], 0);
        let garage = titled(&tasks, "Garage");
        assert_eq!(garage["project"], project);
        assert_eq!(garage["position"], 2, "behind Keller and Dach");
    }

    #[test]
    fn an_import_from_before_origins_is_known_by_title() {
        let first = run_into(
            &[],
            "agenda",
            &[("inbox.org", "* TODO a\n"), ("gtd.org", AGENDA)],
        );
        // What the run of 08.11.2026 left: no org_origin.
        let account: Vec<AnyObject> = account_after(&first)
            .into_iter()
            .map(|o| match o {
                AnyObject::Task(mut t) => {
                    t.properties.remove(ORIGIN);
                    AnyObject::Task(t)
                }
                AnyObject::Project(mut p) => {
                    p.properties.remove(ORIGIN);
                    AnyObject::Project(p)
                }
                other => other,
            })
            .collect();
        let again = run_into(
            &account,
            "agenda",
            &[("inbox.org", "* TODO a\n* TODO b\n"), ("gtd.org", AGENDA)],
        );
        assert_eq!((again.tasks, again.projects, again.known), (1, 0, 3));
        assert_eq!(of(&again, ObjectKind::Task)[0]["title"], "b");
    }

    /// A real account's case of 09.11.2026: tasks of the first import moved into
    /// the inbox, the `:prj:` project renamed. Nothing comes twice, and the
    /// renamed project is made again only for something new in it.
    #[test]
    fn an_old_import_moved_and_renamed_since_is_still_known() {
        let files = [
            ("gtd.org", "* TODO Keller\n"),
            ("admin.org", "* Admin :prj:\n** TODO Backup\n"),
        ];
        let first = run_into(&[], "agenda", &files);
        let account: Vec<AnyObject> = account_after(&first)
            .into_iter()
            .map(|o| match o {
                AnyObject::Task(mut t) => {
                    t.properties.remove(ORIGIN);
                    if t.title == "Keller" {
                        t.project = None;
                    }
                    AnyObject::Task(t)
                }
                AnyObject::Project(mut p) => {
                    p.properties.remove(ORIGIN);
                    p.title = format!("{} (umbenannt)", p.title);
                    AnyObject::Project(p)
                }
                other => other,
            })
            .collect();
        let again = run_into(&account, "agenda", &files);
        assert!(again.changes.is_empty(), "{:#?}", again.changes);
        assert_eq!(again.known, 2);

        let grown = [
            ("gtd.org", "* TODO Keller\n"),
            ("admin.org", "* Admin :prj:\n** TODO Backup\n** TODO Neu\n"),
        ];
        let more = run_into(&account, "agenda", &grown);
        assert_eq!((more.tasks, more.projects, more.known), (1, 1, 2));
    }
}
