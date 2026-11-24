// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The text forms of `samu list` (ADR 0145 §8): a Markdown task list, and
//! the depths both it and the Org form indent by. The web writes the same
//! (web/lib/exporttext.js); both are held to `testdata/export/`. Text
//! out, no I/O.

use std::collections::HashMap;
use std::fmt::Write as _;

use samu_api::{Id, Task};

use crate::export_org::one_line;

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

#[cfg(test)]
mod tests {
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

    #[test]
    fn a_child_alone_stands_at_the_top() {
        let cases = cases();
        let tasks: Vec<&Task> = cases.tasks.iter().skip(1).collect();
        assert_eq!(super::depths(&tasks), vec![0, 1, 0]);
    }
}
