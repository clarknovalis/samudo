// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Properties beyond storing them (ADR 0054): which a task inherits from
//! its action groups and project, and where a value leads when clicked.

use std::collections::{BTreeMap, HashMap, HashSet};

use samu_api::{Id, Inherited, Project, Task};

/// Keys samu writes itself (ADR 0023 §5, ADR 0051). They are shown last and
/// faint, and never inherited.
pub const SYSTEM: &[&str] = &["org_id", "org_origin", "source"];

/// Whether `key` is one samu writes itself.
#[must_use]
pub fn is_system(key: &str) -> bool {
    SYSTEM.contains(&key)
}

/// Whether `key` may name a property (ADR 0023 §1): a lower-case letter,
/// then lower-case letters, digits, `_` and `-`, at most 64 in all.
#[must_use]
pub fn is_key(key: &str) -> bool {
    let mut bytes = key.bytes();
    key.len() <= 64
        && bytes.next().is_some_and(|b| b.is_ascii_lowercase())
        && bytes.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// The parents of tasks and the properties of projects, read once, so the
/// inherited properties of many tasks are quick to find.
#[derive(Debug)]
pub struct Inheritance<'a> {
    tasks: HashMap<Id, &'a Task>,
    projects: HashMap<Id, &'a Project>,
}

impl<'a> Inheritance<'a> {
    #[must_use]
    pub fn new(tasks: &'a [Task], projects: &'a [Project]) -> Self {
        Self {
            tasks: tasks.iter().map(|t| (t.id, t)).collect(),
            projects: projects.iter().map(|p| (p.id, p)).collect(),
        }
    }

    /// What `task` inherits and does not have itself (ADR 0054 §7): from
    /// its action groups, nearest first, then from its project. System keys
    /// do not inherit. A cycle of parents stops the walk.
    #[must_use]
    pub fn of(&self, task: &Task) -> BTreeMap<String, Inherited> {
        let mut found: BTreeMap<String, Inherited> = BTreeMap::new();
        let mut take = |from: Id, properties: &BTreeMap<String, String>| {
            for (key, value) in properties {
                if is_system(key) || task.properties.contains_key(key) {
                    continue;
                }
                found.entry(key.clone()).or_insert_with(|| Inherited {
                    value: value.clone(),
                    from,
                });
            }
        };
        let mut seen = HashSet::from([task.id]);
        let mut at = task.parent;
        while let Some(id) = at {
            if !seen.insert(id) {
                break;
            }
            let Some(group) = self.tasks.get(&id) else {
                break;
            };
            take(group.id, &group.properties);
            at = group.parent;
        }
        if let Some(project) = task.project.and_then(|p| self.projects.get(&p)) {
            take(project.id, &project.properties);
        }
        found
    }

    /// The value of `key` at `task`: its own, else the inherited one.
    #[must_use]
    pub fn value(&self, task: &Task, key: &str) -> Option<String> {
        task.properties
            .get(key)
            .cloned()
            .or_else(|| self.of(task).remove(key).map(|i| i.value))
    }
}

/// Whether a value is a URI by itself (ADR 0054 §1): a scheme after
/// RFC 3986 – a letter, then letters, digits, `+`, `-`, `.` – a colon,
/// something after it, and no white space anywhere.
#[must_use]
pub fn is_uri(value: &str) -> bool {
    let Some((scheme, rest)) = value.split_once(':') else {
        return false;
    };
    let mut bytes = scheme.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_alphabetic())
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
        && !rest.is_empty()
        && !value.chars().any(char::is_whitespace)
}

/// `text` with every byte but the unreserved ones of RFC 3986 as `%XX`.
#[must_use]
pub fn encode(text: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            // Writing to a String does not fail.
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// Where a property's value leads (ADR 0054 §4): the value itself when it
/// is a URI, else the key's template with the value URL-encoded for
/// `{value}`, else nowhere.
#[must_use]
pub fn link_for(value: &str, template: Option<&str>) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if is_uri(value) {
        return Some(value.to_owned());
    }
    let template = template?.trim();
    (!template.is_empty() && template.contains("{value}"))
        .then(|| template.replace("{value}", &encode(value)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests may unwrap")]
mod tests {
    use super::*;

    fn task(n: u8, parent: Option<u8>, project: Option<u8>, props: &[(&str, &str)]) -> Task {
        serde_json::from_value(serde_json::json!({
            "id": id(n),
            "created": "2026-11-09T08:00:00Z",
            "modified": "2026-11-09T08:00:00Z",
            "title": format!("t{n}"),
            "parent": parent.map(id),
            "project": project.map(id),
            "properties": props.iter().copied().collect::<BTreeMap<_, _>>(),
        }))
        .unwrap()
    }

    fn project(n: u8, props: &[(&str, &str)]) -> Project {
        serde_json::from_value(serde_json::json!({
            "id": id(n),
            "created": "2026-11-09T08:00:00Z",
            "modified": "2026-11-09T08:00:00Z",
            "title": format!("p{n}"),
            "kind": "parallel",
            "status": "active",
            "properties": props.iter().copied().collect::<BTreeMap<_, _>>(),
        }))
        .unwrap()
    }

    fn id(n: u8) -> Id {
        format!("01936f5e-8a00-7c3e-9b1a-3f2d4c5b6c{n:02x}")
            .parse()
            .unwrap()
    }

    #[test]
    fn the_nearest_wins_and_its_own_overrides() {
        let projects = [project(
            1,
            &[("jira", "P-1"), ("team", "rot"), ("source", "org-import")],
        )];
        let tasks = [
            task(2, None, Some(1), &[("jira", "G-2"), ("epic", "E")]),
            task(3, Some(2), Some(1), &[("epic", "eigen")]),
        ];
        let inh = Inheritance::new(&tasks, &projects);
        let got = inh.of(&tasks[1]);
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got["jira"].value, "G-2");
        assert_eq!(got["jira"].from, id(2));
        assert_eq!(got["team"].from, id(1));
        assert!(!got.contains_key("epic"), "its own is not inherited");
        assert!(!got.contains_key("source"), "system keys stay");
        assert_eq!(inh.value(&tasks[1], "epic").as_deref(), Some("eigen"));
        assert_eq!(inh.value(&tasks[1], "team").as_deref(), Some("rot"));
        assert_eq!(inh.value(&tasks[1], "nichts"), None);
    }

    #[test]
    fn a_cycle_of_parents_ends() {
        let tasks = [
            task(2, Some(3), None, &[("a", "2")]),
            task(3, Some(2), None, &[("b", "3")]),
        ];
        let inh = Inheritance::new(&tasks, &[]);
        let got = inh.of(&tasks[0]);
        assert_eq!(got.keys().collect::<Vec<_>>(), ["b"]);
    }

    #[test]
    fn keys() {
        assert!(is_key("jira"));
        assert!(is_key("org_id"));
        assert!(is_key("a-1"));
        assert!(!is_key(""));
        assert!(!is_key("Jira"));
        assert!(!is_key("1a"));
        assert!(!is_key("a.b"));
        assert!(!is_key(&"a".repeat(65)));
    }

    #[test]
    fn where_a_value_leads() {
        let jira = Some("https://jira.example.org/browse/{value}");
        assert_eq!(
            link_for("HAUS-12", jira).as_deref(),
            Some("https://jira.example.org/browse/HAUS-12")
        );
        assert_eq!(
            link_for("a b/ä", Some("https://x.example/?q={value}")).as_deref(),
            Some("https://x.example/?q=a%20b%2F%C3%A4")
        );
        let uri = "catastr:01936f5e-8a00-7c3e-9b1a-3f2d4c5b6a01#page=3";
        assert_eq!(link_for(uri, jira).as_deref(), Some(uri));
        assert_eq!(link_for("HAUS-12", None), None);
        assert_eq!(link_for("HAUS-12", Some("ohne platzhalter")), None);
        assert_eq!(link_for("  ", jira), None);
    }

    #[test]
    fn what_is_a_uri() {
        assert!(is_uri("https://example.org/"));
        assert!(is_uri("mailto:a@example.org"));
        assert!(is_uri("samu:01936f5e-8a00-7c3e-9b1a-3f2d4c5b6a01"));
        assert!(!is_uri("HAUS-12"));
        assert!(!is_uri("10:30"));
        assert!(!is_uri("Notiz: siehe oben"));
        assert!(!is_uri("leer:"));
    }
}
