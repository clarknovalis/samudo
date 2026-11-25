// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The text forms of `samu list` (ADR 0145 §8): a Markdown task list, and
//! the depths both it and the Org form indent by. The web writes the same
//! (web/lib/exporttext.js); both are held to `testdata/export/`. Text
//! out, no I/O.

use std::collections::HashMap;
use std::fmt::Write as _;

use jiff::tz::TimeZone;
use samu_api::{Id, Project, Task};

use crate::export_org::{Hanging, author_kind, one_line};

/// The tasks in outline order: each one right after its parent when that
/// is in the list, the rest at the top; siblings keep the order given.
/// The server orders `GET /available` flat (overdue first, then by
/// project and position), where a child may come before its parent.
pub(crate) fn outline<'a>(tasks: &[&'a Task]) -> Vec<&'a Task> {
    let ids: std::collections::HashSet<Id> = tasks.iter().map(|t| t.id).collect();
    let mut children: HashMap<Id, Vec<&'a Task>> = HashMap::new();
    let mut roots = Vec::new();
    for task in tasks {
        match task.parent.filter(|p| ids.contains(p)) {
            Some(parent) => children.entry(parent).or_default().push(task),
            None => roots.push(*task),
        }
    }
    let mut out = Vec::with_capacity(tasks.len());
    let mut stack: Vec<&'a Task> = roots.into_iter().rev().collect();
    while let Some(task) = stack.pop() {
        out.push(task);
        if let Some(below) = children.get(&task.id) {
            stack.extend(below.iter().rev());
        }
    }
    out
}

/// How deep each task stands: one below its parent when that is in the
/// list before it, else at the top.
pub(crate) fn depths(tasks: &[&Task]) -> Vec<usize> {
    let mut seen: HashMap<Id, usize> = HashMap::new();
    tasks
        .iter()
        .map(|task| {
            let depth = task.parent.and_then(|p| seen.get(&p)).map_or(0, |d| d + 1);
            seen.insert(task.id, depth);
            depth
        })
        .collect()
}

/// `- [ ] Title`, `- [x]` for a completed or dropped task, two spaces
/// deeper below a parent in the list.
pub(crate) fn markdown(tasks: &[&Task]) -> String {
    let mut out = String::new();
    for (task, depth) in tasks.iter().zip(depths(tasks)) {
        let closed = task.completed_at.is_some() || task.dropped_at.is_some();
        let _ = writeln!(
            out,
            "{}- [{}] {}",
            "  ".repeat(depth),
            if closed { "x" } else { " " },
            one_line(&task.title)
        );
    }
    out
}

/// The full Markdown form (`samu list --full`, `samu show --format md`):
/// as [`markdown`], with each task's note, comments and attachments
/// below its line, indented to stay in its item. With a project, its
/// title as a heading first, then its note and comments. Stamps in
/// `zone`. For people to read; its look may change (ADR 0145).
pub(crate) fn markdown_full(
    tasks: &[&Task],
    hanging: &Hanging<'_>,
    zone: &TimeZone,
    project: Option<&Project>,
) -> String {
    let mut out = String::new();
    if let Some(project) = project {
        let _ = writeln!(out, "# {}", one_line(&project.title));
        details(
            &mut out,
            "",
            project.note.as_deref(),
            project.id,
            hanging,
            zone,
        );
        if !tasks.is_empty() {
            out.push('\n');
        }
    }
    for (task, depth) in tasks.iter().zip(depths(tasks)) {
        let indent = "  ".repeat(depth);
        let closed = task.completed_at.is_some() || task.dropped_at.is_some();
        let _ = writeln!(
            out,
            "{indent}- [{}] {}",
            if closed { "x" } else { " " },
            one_line(&task.title)
        );
        details(
            &mut out,
            &format!("{indent}  "),
            task.note.as_deref(),
            task.id,
            hanging,
            zone,
        );
    }
    out
}

/// Note, comments and attachments of one object, each part after a blank
/// line, every line behind `indent`; nothing when it has none of them.
fn details(
    out: &mut String,
    indent: &str,
    note: Option<&str>,
    owner: Id,
    hanging: &Hanging<'_>,
    zone: &TimeZone,
) {
    let part = |out: &mut String, lines: Vec<String>| {
        out.push('\n');
        for line in lines {
            if line.is_empty() {
                out.push('\n');
            } else {
                let _ = writeln!(out, "{indent}{line}");
            }
        }
    };
    if let Some(note) = note.map(str::trim_end).filter(|n| !n.trim().is_empty()) {
        part(out, note.lines().map(str::to_owned).collect());
    }
    for comment in hanging.comments.get(&owner).into_iter().flatten() {
        let by = comment
            .created_by
            .as_ref()
            .map(|a| format!(" {} ({})", a.device, author_kind(a.kind)))
            .unwrap_or_default();
        let stamp = comment
            .created
            .to_zoned(zone.clone())
            .strftime("%Y-%m-%d %H:%M");
        let mut lines = comment.body.trim_end().lines();
        let mut quoted = vec![format!(
            "> **{stamp}{by}:** {}",
            lines.next().unwrap_or_default()
        )];
        quoted.extend(lines.map(|l| {
            if l.is_empty() {
                ">".into()
            } else {
                format!("> {l}")
            }
        }));
        for a in hanging.attachments.get(&comment.id).into_iter().flatten() {
            quoted.push(">".into());
            quoted.push(format!(
                "> Attachment: {} ({})",
                one_line(&a.name),
                a.media_type
            ));
        }
        part(out, quoted);
    }
    let files: Vec<String> = hanging
        .attachments
        .get(&owner)
        .into_iter()
        .flatten()
        .map(|a| format!("Attachment: {} ({})", one_line(&a.name), a.media_type))
        .collect();
    if !files.is_empty() {
        // One per line: a hard break, two spaces, between them.
        let last = files.len() - 1;
        part(
            out,
            files
                .into_iter()
                .enumerate()
                .map(|(i, f)| if i < last { format!("{f}  ") } else { f })
                .collect(),
        );
    }
    let had = note.is_some_and(|n| !n.trim().is_empty())
        || hanging.comments.contains_key(&owner)
        || hanging.attachments.contains_key(&owner);
    if had {
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use std::collections::HashMap;

    use jiff::tz::TimeZone;
    use samu_api::{Tag, Task};
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct Cases {
        zone: String,
        tags: Vec<Tag>,
        tasks: Vec<Task>,
    }

    fn cases() -> Cases {
        serde_json::from_str(include_str!("../../../testdata/export/tasks.json"))
            .expect("testdata/export/tasks.json is well formed")
    }

    #[test]
    fn markdown_matches_the_shared_cases() {
        let cases = cases();
        let tasks: Vec<&Task> = cases.tasks.iter().collect();
        assert_eq!(tasks.len(), 4, "the shared cases are there");
        assert_eq!(
            super::markdown(&tasks),
            include_str!("../../../testdata/export/tasks.md")
        );
    }

    #[test]
    fn org_matches_the_shared_cases() {
        let cases = cases();
        let tasks: Vec<&Task> = cases.tasks.iter().collect();
        let tags: HashMap<samu_api::Id, Tag> =
            cases.tags.iter().map(|t| (t.id, t.clone())).collect();
        let zone = TimeZone::get(&cases.zone).expect("a known zone");
        assert_eq!(
            crate::export_org::task_list(&tasks, &tags, zone),
            include_str!("../../../testdata/export/tasks.org")
        );
    }

    /// A comment by an agent on the first task, with an attachment, and
    /// one on the comment; synthetic.
    fn hung() -> Vec<samu_api::AnyObject> {
        serde_json::from_value(serde_json::json!([
            {"kind": "comment", "object": {"id": "01900000-0000-7000-8000-0000000000c1",
                "created": "2026-03-01T12:05:00Z", "modified": "2026-03-01T12:05:00Z",
                "created_by": {"device": "claude", "kind": "agent"},
                "owner": "01900000-0000-7000-8000-00000000000a",
                "body": "Asked the hotel\n\nno answer yet"}},
            {"kind": "attachment", "object": {"id": "01900000-0000-7000-8000-0000000000a1",
                "created": "2026-03-01T12:06:00Z", "modified": "2026-03-01T12:06:00Z",
                "owner": "01900000-0000-7000-8000-0000000000c1", "blob": "b".repeat(64),
                "name": "mail.txt", "media_type": "text/plain", "size": 3}},
            {"kind": "attachment", "object": {"id": "01900000-0000-7000-8000-0000000000a2",
                "created": "2026-03-01T12:07:00Z", "modified": "2026-03-01T12:07:00Z",
                "owner": "01900000-0000-7000-8000-00000000000b", "blob": "c".repeat(64),
                "name": "ticket.pdf", "media_type": "application/pdf", "size": 3}}
        ]))
        .unwrap()
    }

    #[test]
    fn markdown_full_hangs_note_comments_and_attachments_in_the_item() {
        let cases = cases();
        let tasks: Vec<&Task> = cases.tasks.iter().collect();
        let objects = hung();
        let hanging = crate::export_org::Hanging::of(&objects);
        let zone = TimeZone::get(&cases.zone).unwrap();
        let md = super::markdown_full(&tasks, &hanging, &zone, None);
        assert_eq!(
            md,
            "- [ ] Plan the trip\n\
             \n\
             \x20 First line\n\
             \x20 * not a heading\n\
             \x20 :END:\n\
             \n\
             \x20 > **2026-03-01 12:05 claude (agent):** Asked the hotel\n\
             \x20 >\n\
             \x20 > no answer yet\n\
             \x20 >\n\
             \x20 > Attachment: mail.txt (text/plain)\n\
             \n\
             \x20 - [x] Book a train\n\
             \n\
             \x20   Attachment: ticket.pdf (application/pdf)\n\
             \n\
             \x20   - [x] Seat with a table\n\
             - [ ] Pack, \"light\" really\n"
        );
    }

    #[test]
    fn org_full_hangs_comments_without_links() {
        let cases = cases();
        let tasks: Vec<&Task> = cases.tasks.iter().collect();
        let tags: HashMap<samu_api::Id, Tag> =
            cases.tags.iter().map(|t| (t.id, t.clone())).collect();
        let objects = hung();
        let hanging = crate::export_org::Hanging::of(&objects);
        let zone = TimeZone::get(&cases.zone).unwrap();
        let org = crate::export_org::task_list_full(&tasks, &tags, zone, hanging, None);
        // The note as the short form has it, then what hangs below it.
        assert!(
            org.contains(
                ":END:\nFirst line\n * not a heading\n :END:\n\
                 - [2026-03-01 Sun 12:05] claude (agent): Asked the hotel\n  \n  no answer yet\n\
                 \x20 - mail.txt (text/plain)\n** DONE Book a train\n"
            ),
            "{org}"
        );
        assert!(
            org.contains(":END:\n- ticket.pdf (application/pdf)\n*** CANCELLED Seat"),
            "{org}"
        );
        assert!(
            !org.contains("file:"),
            "no link to a folder that is not there: {org}"
        );
        // Without anything hanging, the short form.
        let plain = crate::export_org::task_list(&tasks, &tags, TimeZone::UTC);
        let bare = crate::export_org::task_list_full(
            &tasks,
            &tags,
            TimeZone::UTC,
            crate::export_org::Hanging::default(),
            None,
        );
        assert_eq!(bare, plain);
    }

    #[test]
    fn outline_puts_children_after_their_parent() {
        let cases = cases();
        let t = &cases.tasks;
        // As the server may give them: the grandchild first, the parent last.
        let flat: Vec<&Task> = vec![&t[2], &t[3], &t[1], &t[0]];
        let ordered: Vec<samu_api::Id> = super::outline(&flat).iter().map(|t| t.id).collect();
        assert_eq!(ordered, vec![t[3].id, t[0].id, t[1].id, t[2].id]);
        assert_eq!(super::depths(&super::outline(&flat)), vec![0, 0, 1, 2]);
    }

    #[test]
    fn a_child_alone_stands_at_the_top() {
        let cases = cases();
        let tasks: Vec<&Task> = cases.tasks.iter().skip(1).collect();
        assert_eq!(super::depths(&tasks), vec![0, 1, 0]);
    }
}
