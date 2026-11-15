//! `samu block`, `samu unblock` and `samu blockers` (ae5a3206): a task
//! waits for another through a `blocked_by` link (ADR 0017 §7). The links
//! are ordinary objects over `/changes`. Which ones make sense is
//! samu-core's `blockers::check`; the CLI does not depend on samu-core, so
//! [`check`] repeats it, as `tags.rs` does with the workflow rules.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use anyhow::bail;
use samu_api::{AnyObject, Change, ChangesPage, Id, Link, LinkKind, ObjectKind, Task};
use samu_client::{Client, Error as Failure};
use serde_json::{Value, json};

use crate::output::{Format, emit};
use crate::{placeholder, resolve, short, write};

struct Objects {
    tasks: Vec<Task>,
    links: Vec<Link>,
}

fn objects(client: &Client) -> Result<Objects> {
    let page: ChangesPage =
        serde_json::from_value(client.call("GET", "/changes?after=0", None)?.json()?)?;
    let mut out = Objects {
        tasks: Vec::new(),
        links: Vec::new(),
    };
    for object in page.objects {
        match object {
            AnyObject::Task(t) => out.tasks.push(t),
            AnyObject::Link(l) => out.links.push(l),
            _ => {}
        }
    }
    Ok(out)
}

/// `samu block <id> <blocker>`: `id` waits until `blocker` is done.
pub(crate) fn block(client: &Client, format: Format, id: &str, blocker: &str) -> Result<()> {
    let owner = resolve(client, id)?;
    let target = resolve(client, blocker)?;
    let objects = objects(client)?;
    if !objects.tasks.iter().any(|t| t.id == owner) {
        return Err(Failure::NotFound(format!("no task {id:?}")).into());
    }
    if let Err(why) = check(&objects.tasks, &objects.links, owner, target) {
        bail!("{why}");
    }
    let now = jiff::Timestamp::now();
    let link = Link {
        id: Id::new(),
        created: now,
        modified: now,
        owner,
        uri: target.uri(),
        label: None,
        kind: LinkKind::BlockedBy,
    };
    write(
        client,
        &[Change::Create {
            object: link.id,
            kind: ObjectKind::Link,
            value: serde_json::to_value(&link)?,
            hlc: placeholder(),
        }],
    )?;
    emit(
        format,
        &[json!({"id": link.id, "short": short(&link.id)})],
        &["id"],
    )
}

/// `samu unblock <id> <blocker>`: takes the link away again.
pub(crate) fn unblock(client: &Client, id: &str, blocker: &str) -> Result<()> {
    let owner = resolve(client, id)?;
    let target = resolve(client, blocker)?;
    let objects = objects(client)?;
    let changes: Vec<Change> = of(&objects.links, owner)
        .into_iter()
        .filter(|(_, t)| *t == target)
        .map(|(l, _)| Change::Delete {
            object: l.id,
            hlc: placeholder(),
        })
        .collect();
    if changes.is_empty() {
        return Err(Failure::NotFound(format!("{blocker:?} does not block {id:?}")).into());
    }
    write(client, &changes)?;
    Ok(())
}

/// `samu blockers <id>`: the tasks it waits for, open or not.
pub(crate) fn list(client: &Client, format: Format, id: &str) -> Result<()> {
    let owner = resolve(client, id)?;
    let objects = objects(client)?;
    let records: Vec<Value> = of(&objects.links, owner)
        .into_iter()
        .map(|(link, target)| {
            let task = objects.tasks.iter().find(|t| t.id == target);
            json!({
                "id": target,
                "short": short(&target),
                "title": task.map(|t| t.title.clone()),
                "open": task.is_some_and(|t| t.completed_at.is_none() && t.dropped_at.is_none()),
                "link": link.id,
            })
        })
        .collect();
    emit(format, &records, &["short", "open", "title"])
}

/// As `samu_core::blockers::of`.
fn of(links: &[Link], owner: Id) -> Vec<(&Link, Id)> {
    links
        .iter()
        .filter(|l| l.owner == owner && l.kind == LinkKind::BlockedBy)
        .filter_map(|l| Id::from_uri(&l.uri).map(|target| (l, target)))
        .collect()
}

/// As `samu_core::blockers::check`, with its messages.
fn check(tasks: &[Task], links: &[Link], owner: Id, target: Id) -> Result<(), &'static str> {
    if owner == target {
        return Err("a task cannot block itself");
    }
    if !tasks.iter().any(|t| t.id == target) {
        return Err("the blocker is not a task");
    }
    if of(links, owner).iter().any(|(_, t)| *t == target) {
        return Err("it blocks the task already");
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
            return Err("that would block in a circle");
        }
        if seen.insert(at) {
            todo.extend(waits.get(&at).into_iter().flatten().copied());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]
    use super::*;

    fn task() -> Task {
        serde_json::from_value(json!({
            "id": Id::new(),
            "created": "2026-11-12T08:00:00Z",
            "modified": "2026-11-12T08:00:00Z",
            "title": "t",
        }))
        .expect("a minimal task deserializes")
    }

    fn link(owner: Id, target: Id) -> Link {
        serde_json::from_value(json!({
            "id": Id::new(),
            "created": "2026-11-12T08:00:00Z",
            "modified": "2026-11-12T08:00:00Z",
            "owner": owner,
            "uri": target.uri(),
            "kind": "blocked_by",
        }))
        .expect("a link deserializes")
    }

    /// The same answers as samu-core's `blockers` tests.
    #[test]
    fn the_rules_match_samu_core() {
        let (a, b, c) = (task(), task(), task());
        let tasks = vec![a.clone(), b.clone(), c.clone()];
        let links = vec![link(a.id, b.id), link(b.id, c.id)];
        assert!(check(&tasks, &links, a.id, a.id).is_err());
        assert!(check(&tasks, &links, a.id, Id::new()).is_err());
        assert!(check(&tasks, &links, a.id, b.id).is_err());
        assert!(check(&tasks, &links, c.id, a.id).is_err());
        assert!(check(&tasks, &links, b.id, a.id).is_err());
        assert_eq!(check(&tasks, &links, a.id, c.id), Ok(()));
        assert_eq!(of(&links, a.id).len(), 1);
    }
}
