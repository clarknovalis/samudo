//! An export as one Org file to read (ADR 0074 §8): folders, projects and
//! tasks as an outline, with keyword, priority, tags, dates, properties,
//! note, comments and attachments. Only for reading, in Emacs say, while
//! samu is away; never read back (ADR 0002 §1). Text out, no I/O.

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;

use jiff::Timestamp;
use jiff::tz::TimeZone;
use samu_api::{
    AnyObject, Attachment, Comment, EXPORT_BLOBS, Export, Folder, FolderStatus, Id, Priority,
    Project, ProjectKind, ProjectStatus, RepeatMode, Schedule, Tag, Task, When,
};

/// What the outline needs, by id.
struct Index<'a> {
    zone: TimeZone,
    folders: Vec<&'a Folder>,
    projects: Vec<&'a Project>,
    tasks: Vec<&'a Task>,
    tags: HashMap<Id, &'a Tag>,
    comments: HashMap<Id, Vec<&'a Comment>>,
    attachments: HashMap<Id, Vec<&'a Attachment>>,
}

impl<'a> Index<'a> {
    fn new(export: &'a Export) -> Self {
        let zone = TimeZone::get(&export.manifest.zone).unwrap_or(TimeZone::UTC);
        let mut index = Self {
            zone,
            folders: Vec::new(),
            projects: Vec::new(),
            tasks: Vec::new(),
            tags: HashMap::new(),
            comments: HashMap::new(),
            attachments: HashMap::new(),
        };
        for exported in &export.objects {
            match &exported.object {
                AnyObject::Folder(f) => index.folders.push(f),
                AnyObject::Project(p) => index.projects.push(p),
                AnyObject::Task(t) => index.tasks.push(t),
                AnyObject::Tag(t) => {
                    index.tags.insert(t.id, t);
                }
                AnyObject::Comment(c) => index.comments.entry(c.owner).or_default().push(c),
                AnyObject::Attachment(a) => {
                    index.attachments.entry(a.owner).or_default().push(a);
                }
                _ => {}
            }
        }
        index.folders.sort_by_key(|f| (f.position, f.id));
        index.projects.sort_by_key(|p| (p.position, p.id));
        index.tasks.sort_by_key(|t| (t.position, t.id));
        for comments in index.comments.values_mut() {
            comments.sort_by_key(|c| (c.created, c.id));
        }
        for attachments in index.attachments.values_mut() {
            attachments.sort_by_key(|a| (a.created, a.id));
        }
        index
    }

    /// `[2026-11-11 Wed 12:00]` in the account's zone.
    fn inactive(&self, at: Timestamp) -> String {
        let local = at.to_zoned(self.zone.clone());
        format!("[{}]", local.strftime("%Y-%m-%d %a %H:%M"))
    }

    fn tag_names(&self, tags: &[samu_api::Tagging]) -> Vec<String> {
        let mut tags: Vec<&samu_api::Tagging> = tags.iter().collect();
        tags.sort_by_key(|t| (t.position, t.tag));
        tags.iter()
            .filter_map(|t| self.tags.get(&t.tag))
            .map(|t| org_tag(&t.title))
            .collect()
    }
}

/// An Org tag takes letters, digits, `_`, `@`, `#` and `%`; the rest
/// becomes `_`.
fn org_tag(title: &str) -> String {
    title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || "_@#%".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// A heading title on one line.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Body text that cannot open a heading or end a drawer: a line starting
/// with `*` or `:END:` gets a space in front.
fn body(out: &mut String, text: &str) {
    for line in text.lines() {
        if line.starts_with('*') || line.trim().eq_ignore_ascii_case(":end:") {
            out.push(' ');
        }
        out.push_str(line);
        out.push('\n');
    }
}

/// `<2026-11-20 Fri>` or `<2026-11-20 Fri 09:00>` in the time's own zone.
fn active(when: &When) -> Option<String> {
    match when {
        When::Date(date) => Some(format!("<{}>", date.strftime("%Y-%m-%d %a"))),
        When::At(at) => Some(format!("<{}>", at.strftime("%Y-%m-%d %a %H:%M"))),
        When::Unknown => None,
    }
}

fn heading(
    out: &mut String,
    level: usize,
    keyword: Option<&str>,
    priority: Option<Priority>,
    title: &str,
    tags: &[String],
) {
    out.push_str(&"*".repeat(level));
    out.push(' ');
    if let Some(keyword) = keyword {
        out.push_str(keyword);
        out.push(' ');
    }
    match priority {
        Some(Priority::A) => out.push_str("[#A] "),
        Some(Priority::B) => out.push_str("[#B] "),
        Some(Priority::C) => out.push_str("[#C] "),
        _ => {}
    }
    out.push_str(&one_line(title));
    if !tags.is_empty() {
        let _ = write!(out, " :{}:", tags.join(":"));
    }
    out.push('\n');
}

/// The planning line: `CLOSED:`, `DEADLINE:` from due, `SCHEDULED:` from
/// planned. Defer has no Org keyword; it goes into the properties.
fn planning(out: &mut String, index: &Index<'_>, schedule: &Schedule, closed: Option<Timestamp>) {
    let mut parts = Vec::new();
    if let Some(at) = closed {
        parts.push(format!("CLOSED: {}", index.inactive(at)));
    }
    if let Some(due) = schedule.due.as_ref().and_then(active) {
        parts.push(format!("DEADLINE: {due}"));
    }
    if let Some(planned) = schedule.planned.as_ref().and_then(active) {
        parts.push(format!("SCHEDULED: {planned}"));
    }
    if !parts.is_empty() {
        out.push_str(&parts.join(" "));
        out.push('\n');
    }
}

fn drawer(out: &mut String, properties: &BTreeMap<String, String>) {
    out.push_str(":PROPERTIES:\n");
    for (key, value) in properties {
        let _ = writeln!(out, ":{key}: {}", one_line(value));
    }
    out.push_str(":END:\n");
}

/// The samu fields without an Org form, then the object's own properties.
fn properties_of(
    index: &Index<'_>,
    id: Id,
    created: Timestamp,
    schedule: &Schedule,
    own: &BTreeMap<String, String>,
) -> BTreeMap<String, String> {
    let mut props = BTreeMap::new();
    props.insert("ID".into(), id.to_string());
    props.insert("CREATED".into(), index.inactive(created));
    if let Some(defer) = schedule.defer.as_ref().and_then(active) {
        props.insert("DEFER".into(), defer);
    }
    if let Some(repeat) = &schedule.repeat {
        let mode = match repeat.mode {
            RepeatMode::Fixed => "fixed",
            RepeatMode::DeferAfterCompletion => "defer_after_completion",
            RepeatMode::DueAfterCompletion => "due_after_completion",
            _ => "unknown",
        };
        props.insert("REPEAT".into(), format!("{} ({mode})", repeat.rule));
    }
    for (key, value) in own {
        // An own key that is spelled like one of samu's stays apart.
        let key = if props.contains_key(&key.to_uppercase()) {
            format!("samu_{key}")
        } else {
            key.clone()
        };
        props.insert(key, value.clone());
    }
    props
}

/// Comments and attachments of an object, as a list under its text.
fn hanging(out: &mut String, index: &Index<'_>, owner: Id) {
    if let Some(comments) = index.comments.get(&owner) {
        for c in comments {
            let by = c
                .created_by
                .as_ref()
                .map(|a| format!(" {} ({})", a.device, author_kind(a.kind)))
                .unwrap_or_default();
            let mut lines = c.body.lines();
            let _ = writeln!(
                out,
                "- {}{by}: {}",
                index.inactive(c.created),
                lines.next().unwrap_or("")
            );
            for line in lines {
                let _ = writeln!(out, "  {line}");
            }
            hanging_files(out, index, c.id, "  ");
        }
    }
    hanging_files(out, index, owner, "");
}

fn hanging_files(out: &mut String, index: &Index<'_>, owner: Id, indent: &str) {
    if let Some(attachments) = index.attachments.get(&owner) {
        for a in attachments {
            let _ = writeln!(
                out,
                "{indent}- [[file:{EXPORT_BLOBS}/{}][{}]] ({})",
                a.blob,
                one_line(&a.name),
                a.media_type
            );
        }
    }
}

fn author_kind(kind: samu_api::AuthorKind) -> &'static str {
    match kind {
        samu_api::AuthorKind::Agent => "agent",
        _ => "human",
    }
}

fn task_keyword(task: &Task) -> &'static str {
    if task.completed_at.is_some() {
        "DONE"
    } else if task.dropped_at.is_some() {
        "CANCELLED"
    } else {
        "TODO"
    }
}

fn write_task(out: &mut String, index: &Index<'_>, task: &Task, level: usize) {
    heading(
        out,
        level,
        Some(task_keyword(task)),
        task.priority,
        &task.title,
        &index.tag_names(&task.tags),
    );
    planning(
        out,
        index,
        &task.schedule,
        task.completed_at.or(task.dropped_at),
    );
    let mut props = properties_of(
        index,
        task.id,
        task.created,
        &task.schedule,
        &task.properties,
    );
    if task.flagged {
        props.insert("FLAGGED".into(), "t".into());
    }
    if let Some(estimate) = &task.estimate {
        props.insert("EFFORT".into(), estimate.0.to_string());
    }
    drawer(out, &props);
    if let Some(note) = &task.note {
        body(out, note);
    }
    hanging(out, index, task.id);
    for child in index.tasks.iter().filter(|t| t.parent == Some(task.id)) {
        write_task(out, index, child, level + 1);
    }
}

fn write_project(out: &mut String, index: &Index<'_>, project: &Project, level: usize) {
    let keyword = match project.status {
        ProjectStatus::Completed => Some("DONE"),
        ProjectStatus::Dropped => Some("CANCELLED"),
        _ => None,
    };
    let mut tags = index.tag_names(&project.tags);
    tags.insert(0, "project".into());
    let title = match &project.icon {
        Some(icon) => format!("{icon} {}", project.title),
        None => project.title.clone(),
    };
    heading(out, level, keyword, None, &title, &tags);
    planning(
        out,
        index,
        &project.schedule,
        project.completed_at.or(project.dropped_at),
    );
    let mut props = properties_of(
        index,
        project.id,
        project.created,
        &project.schedule,
        &project.properties,
    );
    let kind = match project.kind {
        ProjectKind::Parallel => "parallel",
        ProjectKind::Sequential => "sequential",
        ProjectKind::SingleActions => "single_actions",
        _ => "unknown",
    };
    props.insert("KIND".into(), kind.into());
    if project.status == ProjectStatus::OnHold {
        props.insert("STATUS".into(), "on_hold".into());
    }
    drawer(out, &props);
    if let Some(note) = &project.note {
        body(out, note);
    }
    hanging(out, index, project.id);
    for task in index
        .tasks
        .iter()
        .filter(|t| t.project == Some(project.id) && t.parent.is_none())
    {
        write_task(out, index, task, level + 1);
    }
}

fn write_folder(out: &mut String, index: &Index<'_>, folder: &Folder, level: usize) {
    let title = match &folder.icon {
        Some(icon) => format!("{icon} {}", folder.title),
        None => folder.title.clone(),
    };
    heading(out, level, None, None, &title, &["folder".to_string()]);
    let mut props = BTreeMap::new();
    props.insert("ID".to_string(), folder.id.to_string());
    if folder.status == FolderStatus::Dropped {
        props.insert("STATUS".into(), "dropped".into());
    }
    drawer(out, &props);
    for child in index.folders.iter().filter(|f| f.parent == Some(folder.id)) {
        write_folder(out, index, child, level + 1);
    }
    for project in index
        .projects
        .iter()
        .filter(|p| p.folder == Some(folder.id))
    {
        write_project(out, index, project, level + 1);
    }
}

/// The whole export as one Org file.
pub(crate) fn render(export: &Export) -> String {
    let index = Index::new(export);
    let mut out = String::new();
    let _ = writeln!(out, "#+TITLE: samudo – {}", export.manifest.account);
    // No date: the same state gives the same file, so a git mirror commits
    // only real changes (ADR 0074 §3); the time is in manifest.json.
    out.push_str("#+TODO: TODO | DONE CANCELLED\n");
    out.push_str("#+STARTUP: overview\n");
    out.push_str(
        "# Written from the JSON of this export; read only, never read back (ADR 0074 §8).\n\n",
    );

    out.push_str("* Inbox\n");
    for task in index
        .tasks
        .iter()
        .filter(|t| t.project.is_none() && t.parent.is_none())
    {
        write_task(&mut out, &index, task, 2);
    }
    let folders: HashMap<Id, ()> = index.folders.iter().map(|f| (f.id, ())).collect();
    for folder in index
        .folders
        .iter()
        .filter(|f| f.parent.is_none_or(|p| !folders.contains_key(&p)))
    {
        write_folder(&mut out, &index, folder, 1);
    }
    for project in index
        .projects
        .iter()
        .filter(|p| p.folder.is_none_or(|f| !folders.contains_key(&f)))
    {
        write_project(&mut out, &index, project, 1);
    }
    // A task whose project or parent is not in the export still shows.
    let projects: HashMap<Id, ()> = index.projects.iter().map(|p| (p.id, ())).collect();
    let tasks: HashMap<Id, ()> = index.tasks.iter().map(|t| (t.id, ())).collect();
    let lost: Vec<&&Task> = index
        .tasks
        .iter()
        .filter(|t| match (t.parent, t.project) {
            (Some(parent), _) => !tasks.contains_key(&parent),
            (None, Some(project)) => !projects.contains_key(&project),
            (None, None) => false,
        })
        .collect();
    if !lost.is_empty() {
        out.push_str("* Without their place\n");
        for task in lost {
            write_task(&mut out, &index, task, 2);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests may unwrap")]

    use super::*;

    fn export() -> Export {
        let id = |n: u32| format!("01936f5e-8a00-7c3e-9b1a-{n:012}");
        let sync = serde_json::json!({"seq": 1, "stamps": {}});
        let o = |kind: &str, object: serde_json::Value| serde_json::json!({"kind": kind, "object": object, "sync": sync});
        let at = "2026-11-10T08:00:00Z";
        serde_json::from_value(serde_json::json!({
            "manifest": {"format": 1, "api": "0.1.33", "account": "alice",
                         "exported_at": "2026-11-11T12:00:00Z", "seq": 9,
                         "zone": "Europe/Berlin", "due_soon": "P2D", "remind_time": "09:00",
                         "counts": {}},
            "objects": [
                o("folder", serde_json::json!({"id": id(1), "created": at, "modified": at,
                    "title": "Haus", "status": "active", "position": 0})),
                o("tag", serde_json::json!({"id": id(2), "created": at, "modified": at,
                    "title": "Ort/Büro", "status": "active", "position": 0})),
                o("project", serde_json::json!({"id": id(3), "created": at, "modified": at,
                    "title": "Dachrinne", "kind": "sequential", "status": "active",
                    "flagged": false, "folder": id(1), "position": 0})),
                o("task", serde_json::json!({"id": id(4), "created": at, "modified": at,
                    "title": "Angebot\neinholen", "project": id(3), "flagged": true,
                    "sequential": false, "complete_with_last_child": false, "position": 0,
                    "priority": "A", "due": {"date": "2026-11-20"},
                    "planned": {"at": "2026-11-18T09:00:00+01:00[Europe/Berlin]"},
                    "defer": {"date": "2026-11-15"},
                    "tags": [{"tag": id(2), "position": 0}],
                    "properties": {"jira": "SAMU-1", "id": "eigen"},
                    "note": "zwei Firmen\n* keine Überschrift"})),
                o("task", serde_json::json!({"id": id(5), "created": at, "modified": at,
                    "title": "Rinne messen", "project": id(3), "parent": id(4),
                    "flagged": false, "sequential": false, "complete_with_last_child": false,
                    "position": 0, "completed_at": "2026-11-11T09:30:00Z"})),
                o("task", serde_json::json!({"id": id(6), "created": at, "modified": at,
                    "title": "Milch", "flagged": false, "sequential": false,
                    "complete_with_last_child": false, "position": 0})),
                o("comment", serde_json::json!({"id": id(7), "created": at, "modified": at,
                    "created_by": {"device": "claude", "kind": "agent"},
                    "owner": id(4), "body": "Stufe 1\nzweite Zeile"})),
                o("attachment", serde_json::json!({"id": id(8), "created": at, "modified": at,
                    "owner": id(4), "blob": "a".repeat(64), "name": "tafel.jpg",
                    "media_type": "image/jpeg", "size": 3})),
            ],
            "logbook": [],
            "files": []
        }))
        .unwrap()
    }

    #[test]
    fn the_outline_follows_folders_projects_and_subtasks() {
        let org = render(&export());
        let headings: Vec<&str> = org.lines().filter(|l| l.starts_with('*')).collect();
        assert_eq!(
            headings,
            [
                "* Inbox",
                "** TODO Milch",
                "* Haus :folder:",
                "** Dachrinne :project:",
                "*** TODO [#A] Angebot einholen :Ort_Büro:",
                "**** DONE Rinne messen",
            ]
        );
    }

    #[test]
    fn dates_properties_note_and_comments_are_in_org_form() {
        let org = render(&export());
        assert!(
            org.contains("DEADLINE: <2026-11-20 Fri> SCHEDULED: <2026-11-18 Wed 09:00>\n"),
            "{org}"
        );
        // In the account's zone: 09:30Z is 10:30 in Berlin.
        assert!(org.contains("CLOSED: [2026-11-11 Wed 10:30]\n"), "{org}");
        assert!(org.contains(":ID: 01936f5e-8a00-7c3e-9b1a-000000000004\n"));
        assert!(org.contains(":DEFER: <2026-11-15 Sun>\n"));
        assert!(org.contains(":FLAGGED: t\n"));
        assert!(org.contains(":jira: SAMU-1\n"));
        // An own `id` does not overwrite the ID.
        assert!(org.contains(":samu_id: eigen\n"), "{org}");
        // A note line with a star does not become a heading.
        assert!(org.contains("\n * keine Überschrift\n"));
        assert!(org.contains("- [2026-11-10 Tue 09:00] claude (agent): Stufe 1\n  zweite Zeile\n"));
        assert!(org.contains(&format!(
            "- [[file:files/{}][tafel.jpg]] (image/jpeg)\n",
            "a".repeat(64)
        )));
    }

    /// Regression: `#+DATE` carried the time of the export, so the git
    /// mirror on the server committed every 15 minutes without a change.
    #[test]
    fn the_same_state_at_another_time_gives_the_same_file() {
        let first = export();
        let mut later = first.clone();
        later.manifest.exported_at = "2026-11-12T08:00:00Z".parse().unwrap();
        assert_eq!(render(&first), render(&later));
    }

    #[test]
    fn every_task_is_in_the_file_once() {
        let org = render(&export());
        for n in [4, 5, 6] {
            let id = format!(":ID: 01936f5e-8a00-7c3e-9b1a-{n:012}\n");
            assert_eq!(org.matches(&id).count(), 1, "{id}");
        }
    }
}
