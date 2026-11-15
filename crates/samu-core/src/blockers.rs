//! Setting blockers (ae5a3206): which task may block which, and the link
//! that says so. A blocker is a `Link` with `kind = blocked_by` and the
//! `samu:` address of the blocking task (ADR 0012, ADR 0017 §7); the
//! availability reads it, this module only decides whether a new one makes
//! sense. The web mirrors [`check`] in `web/lib/palette.js`.

use std::collections::{HashMap, HashSet};

use jiff::Timestamp;

use crate::api::{Id, Link, LinkKind, Task};

/// Why a task cannot block another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    #[error("a task cannot block itself")]
    Itself,
    #[error("the blocker is not a task")]
    NotATask,
    #[error("it blocks the task already")]
    Already,
    #[error("that would block in a circle")]
    Cycle,
}

/// The `blocked_by` links of `owner` with the task each one names; links
/// whose address is not a `samu:` id are left out.
pub fn of(links: &[Link], owner: Id) -> Vec<(&Link, Id)> {
    links
        .iter()
        .filter(|l| l.owner == owner && l.kind == LinkKind::BlockedBy)
        .filter_map(|l| Id::from_uri(&l.uri).map(|target| (l, target)))
        .collect()
}

/// Whether `target` may become a blocker of `owner`: not the task itself,
/// a task at all, not one already, and nothing that `target` waits for
/// (through `blocked_by`, at any depth) is `owner`, or the two would wait
/// for each other forever.
pub fn check(tasks: &[Task], links: &[Link], owner: Id, target: Id) -> Result<(), Refusal> {
    if owner == target {
        return Err(Refusal::Itself);
    }
    if !tasks.iter().any(|t| t.id == target) {
        return Err(Refusal::NotATask);
    }
    if of(links, owner).iter().any(|(_, t)| *t == target) {
        return Err(Refusal::Already);
    }
    let mut waits: HashMap<Id, Vec<Id>> = HashMap::new();
    for link in links.iter().filter(|l| l.kind == LinkKind::BlockedBy) {
        if let Some(t) = Id::from_uri(&link.uri) {
            waits.entry(link.owner).or_default().push(t);
        }
    }
    let mut seen = HashSet::new();
    let mut todo = vec![target];
    while let Some(at) = todo.pop() {
        if at == owner {
            return Err(Refusal::Cycle);
        }
        if seen.insert(at) {
            todo.extend(waits.get(&at).into_iter().flatten().copied());
        }
    }
    Ok(())
}

/// The new link: `owner` waits for `target`.
pub fn link(owner: Id, target: Id, now: Timestamp) -> Link {
    Link {
        id: Id::new(),
        created: now,
        modified: now,
        owner,
        uri: target.uri(),
        label: None,
        kind: LinkKind::BlockedBy,
    }
}

/// Whether `id` is a blocker in the lists (the 🚧 before its title): it
/// is open and an open task waits for it. A done or dropped blocker holds
/// nothing up, nor does one that only done or dropped tasks wait for.
pub fn holds_up(tasks: &[Task], links: &[Link], id: Id) -> bool {
    let open = |id: Id| {
        tasks
            .iter()
            .any(|t| t.id == id && t.completed_at.is_none() && t.dropped_at.is_none())
    };
    links
        .iter()
        .any(|l| l.kind == LinkKind::BlockedBy && Id::from_uri(&l.uri) == Some(id) && open(l.owner))
        && open(id)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::*;

    fn task(title: &str) -> Task {
        serde_json::from_value(serde_json::json!({
            "id": Id::new(),
            "created": "2026-11-12T08:00:00Z",
            "modified": "2026-11-12T08:00:00Z",
            "title": title,
        }))
        .expect("a minimal task deserializes")
    }

    fn now() -> Timestamp {
        "2026-11-12T08:00:00Z".parse().expect("a valid instant")
    }

    #[test]
    fn a_task_blocks_another() {
        let (a, b) = (task("a"), task("b"));
        let tasks = vec![a.clone(), b.clone()];
        assert_eq!(check(&tasks, &[], a.id, b.id), Ok(()));
        let l = link(a.id, b.id, now());
        assert_eq!(l.kind, LinkKind::BlockedBy);
        assert_eq!(Id::from_uri(&l.uri), Some(b.id));
        assert_eq!(of(std::slice::from_ref(&l), a.id), vec![(&l, b.id)]);
        assert!(of(&[l], b.id).is_empty());
    }

    #[test]
    fn refusals() {
        let (a, b, c) = (task("a"), task("b"), task("c"));
        let tasks = vec![a.clone(), b.clone(), c.clone()];
        let links = vec![link(a.id, b.id, now()), link(b.id, c.id, now())];
        assert_eq!(check(&tasks, &links, a.id, a.id), Err(Refusal::Itself));
        assert_eq!(
            check(&tasks, &links, a.id, Id::new()),
            Err(Refusal::NotATask)
        );
        assert_eq!(check(&tasks, &links, a.id, b.id), Err(Refusal::Already));
        // c waits for nothing, b waits for c, a for b: c may not wait for a.
        assert_eq!(check(&tasks, &links, c.id, a.id), Err(Refusal::Cycle));
        assert_eq!(check(&tasks, &links, b.id, a.id), Err(Refusal::Cycle));
        assert_eq!(check(&tasks, &links, a.id, c.id), Ok(()));
    }

    #[test]
    fn a_reference_link_is_no_blocker() {
        let (a, b) = (task("a"), task("b"));
        let tasks = vec![a.clone(), b.clone()];
        let mut reference = link(b.id, a.id, now());
        reference.kind = LinkKind::Ref;
        assert_eq!(check(&tasks, &[reference.clone()], a.id, b.id), Ok(()));
        assert!(of(&[reference], b.id).is_empty());
    }

    #[test]
    fn only_an_open_task_that_an_open_one_waits_for_holds_up() {
        let (a, b, c, d) = (task("a"), task("b"), task("c"), task("d"));
        let mut done = task("done");
        done.completed_at = Some(now());
        let mut dropped = task("dropped");
        dropped.dropped_at = Some(now());
        let tasks = vec![
            a.clone(),
            b.clone(),
            c.clone(),
            d.clone(),
            done.clone(),
            dropped.clone(),
        ];
        let mut reference = link(c.id, d.id, now());
        reference.kind = LinkKind::Ref;
        let links = vec![
            link(a.id, b.id, now()),
            link(a.id, done.id, now()),
            link(dropped.id, c.id, now()),
            reference,
        ];
        let holding: Vec<&str> = tasks
            .iter()
            .filter(|t| holds_up(&tasks, &links, t.id))
            .map(|t| t.title.as_str())
            .collect();
        assert_eq!(holding, vec!["b"]);
    }
}
