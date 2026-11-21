// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Checklists (ADR 0141): reading pasted lines into items, and turning an
//! item into a subtask and back.
//!
//! The conversions are ordinary changes, applied as one batch, stamped by
//! the server like a delete. They run in the client; the web UI has its
//! own small copy of [`lines`] and the two plans, kept to the same tests.

use jiff::Timestamp;

use crate::api::{
    Attachment, Change, ChecklistItem, Clock, Comment, Id, Link, LinkKind, ObjectKind, Reminder,
    Schedule, Task,
};
use crate::delete::{after_last, placeholder};
use crate::overlay::Objects;

/// One line of pasted text as an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub text: String,
    pub checked: bool,
}

/// Reads pasted or typed text into items, one per non-empty line.
///
/// What a list in Org, Markdown or a note app puts in front of a line is
/// dropped: `- `, `* `, `+ `, `1. `, `1) `, and a checkbox `[ ]`, `[x]`,
/// `[X]` or `[-]`. A ticked box ticks the item. Indentation is ignored,
/// there is one level.
#[must_use]
pub fn lines(text: &str) -> Vec<Line> {
    text.lines().filter_map(line).collect()
}

fn line(raw: &str) -> Option<Line> {
    let mut rest = raw.trim();
    // Bullets and numbers, once: "- [ ] x", "1. x", "* x".
    if let Some(r) = bullet(rest) {
        rest = r;
    }
    let mut checked = false;
    for (mark, ticked) in [("[ ]", false), ("[x]", true), ("[X]", true), ("[-]", false)] {
        if let Some(r) = rest.strip_prefix(mark)
            && (r.is_empty() || r.starts_with(char::is_whitespace))
        {
            rest = r;
            checked = ticked;
            break;
        }
    }
    let text = rest.trim();
    if text.is_empty() {
        None
    } else {
        Some(Line {
            text: text.to_owned(),
            checked,
        })
    }
}

fn bullet(text: &str) -> Option<&str> {
    for mark in ["- ", "* ", "+ ", "• "] {
        if let Some(r) = text.strip_prefix(mark) {
            return Some(r);
        }
    }
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    let after = &text[digits..];
    after
        .strip_prefix(". ")
        .or_else(|| after.strip_prefix(") "))
}

/// Why an item or a subtask is not converted.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChecklistError {
    #[error("no checklist item {0}")]
    NoItem(Id),
    #[error("no task {0}")]
    NoTask(Id),
    #[error("the task is not a subtask")]
    NoParent,
    #[error("the task has subtasks of its own")]
    HasChildren,
    #[error("the task is a habit")]
    Habit,
    #[error("the task repeats")]
    Repeating,
    #[error("the task was dropped")]
    Dropped,
    #[error("the task has {0}, which a checklist item cannot keep")]
    Carries(&'static str),
    #[error("the task has no checklist items")]
    NoItems,
    #[error("the task has no subtask that a checklist item could take over")]
    NoPlainSubtasks,
    #[error("the object does not serialize: {0}")]
    Shape(String),
}

/// What hangs on tasks as objects of their own. A task that is deleted
/// loses them, so a subtask that has any does not become an item.
#[derive(Debug, Clone, Copy, Default)]
pub struct Attached<'a> {
    pub comments: &'a [Comment],
    pub attachments: &'a [Attachment],
    pub clocks: &'a [Clock],
    pub reminders: &'a [Reminder],
    pub links: &'a [Link],
}

impl<'a> From<&'a Objects> for Attached<'a> {
    fn from(objects: &'a Objects) -> Self {
        Self {
            comments: &objects.comments,
            attachments: &objects.attachments,
            clocks: &objects.clocks,
            reminders: &objects.reminders,
            links: &objects.links,
        }
    }
}

impl Attached<'_> {
    /// What hangs on `task` that an item cannot keep, named; `items` are
    /// the checklist items of the account.
    fn on(&self, items: &[ChecklistItem], task: Id) -> Option<&'static str> {
        if items.iter().any(|i| i.owner == task) {
            Some("a checklist of its own")
        } else if self.comments.iter().any(|c| c.owner == task) {
            Some("comments")
        } else if self.attachments.iter().any(|a| a.owner == task) {
            Some("attachments")
        } else if self.clocks.iter().any(|c| c.owner == task) {
            Some("clocked time")
        } else if self.reminders.iter().any(|r| r.owner == task) {
            Some("reminders")
        } else if self.links.iter().any(|l| l.owner == task) {
            Some("links")
        } else if self
            .links
            .iter()
            .any(|l| l.kind == LinkKind::BlockedBy && Id::from_uri(&l.uri) == Some(task))
        {
            // Deleting it would silently unblock the task that waits.
            Some("a task that waits for it")
        } else {
            None
        }
    }
}

/// The changes that make item `item` a subtask `task` of the task it is on:
/// the subtask takes its text as the title, its place at the end, and is
/// completed when the item was ticked; the item goes.
///
/// # Errors
///
/// When the item or its task is not in `items` or `tasks`.
pub fn to_subtask(
    tasks: &[Task],
    items: &[ChecklistItem],
    item: Id,
    task: Id,
    now: Timestamp,
) -> Result<Vec<Change>, ChecklistError> {
    let item = items
        .iter()
        .find(|i| i.id == item)
        .ok_or(ChecklistError::NoItem(item))?;
    let owner = tasks
        .iter()
        .find(|t| t.id == item.owner)
        .ok_or(ChecklistError::NoTask(item.owner))?;
    let new = Task {
        id: task,
        created: now,
        modified: now,
        created_by: None,
        title: item.text.clone(),
        note: None,
        project: owner.project,
        parent: Some(owner.id),
        schedule: Schedule::default(),
        flagged: false,
        estimate: None,
        priority: None,
        completed_at: item.checked.then_some(now),
        dropped_at: None,
        repeat_of: None,
        sequential: false,
        complete_with_last_child: false,
        tags: Vec::new(),
        properties: Default::default(),
        position: after_last(
            tasks
                .iter()
                .filter(|t| t.parent == Some(owner.id))
                .map(|t| t.position),
        ),
        habit: None,
    };
    Ok(vec![
        Change::Create {
            object: task,
            kind: ObjectKind::Task,
            value: serde_json::to_value(&new).map_err(|e| ChecklistError::Shape(e.to_string()))?,
            hlc: placeholder(),
        },
        Change::Delete {
            object: item.id,
            hlc: placeholder(),
        },
    ])
}

/// The changes that make subtask `task` an item `item` of the task it is
/// under, at the end of its list, ticked when it was completed; the subtask
/// goes.
///
/// Only a plain subtask converts: what an item cannot keep would be lost
/// silently, so it is refused and named. That is what the task holds in its
/// fields (a note, dates, tags, an estimate, a flag, a priority, properties)
/// and what hangs on it in `attached` (comments, attachments, clocked time,
/// reminders, links, a task that waits for it) or in `items` (a checklist
/// of its own).
///
/// # Errors
///
/// When the task is no subtask, has children, is a habit, repeats, was
/// dropped, or carries something an item cannot keep.
pub fn from_subtask(
    tasks: &[Task],
    items: &[ChecklistItem],
    attached: &Attached<'_>,
    task: Id,
    item: Id,
    now: Timestamp,
) -> Result<Vec<Change>, ChecklistError> {
    let task = tasks
        .iter()
        .find(|t| t.id == task)
        .ok_or(ChecklistError::NoTask(task))?;
    let parent = task.parent.ok_or(ChecklistError::NoParent)?;
    if tasks.iter().any(|t| t.parent == Some(task.id)) {
        return Err(ChecklistError::HasChildren);
    }
    if task.habit.is_some() {
        return Err(ChecklistError::Habit);
    }
    if task.schedule.repeat.is_some() {
        return Err(ChecklistError::Repeating);
    }
    if task.dropped_at.is_some() {
        return Err(ChecklistError::Dropped);
    }
    let carries = if task.note.as_deref().is_some_and(|n| !n.trim().is_empty()) {
        Some("a note")
    } else if task.schedule != Schedule::default() {
        Some("dates")
    } else if !task.tags.is_empty() {
        Some("tags")
    } else if task.estimate.is_some() {
        Some("an estimate")
    } else if task.flagged {
        Some("a flag")
    } else if task.priority.is_some() {
        Some("a priority")
    } else if !task.properties.is_empty() {
        Some("properties")
    } else {
        attached.on(items, task.id)
    };
    if let Some(what) = carries {
        return Err(ChecklistError::Carries(what));
    }
    let new = ChecklistItem {
        id: item,
        created: now,
        modified: now,
        created_by: None,
        owner: parent,
        text: task.title.clone(),
        checked: task.completed_at.is_some(),
        position: after_last(
            items
                .iter()
                .filter(|i| i.owner == parent)
                .map(|i| i.position),
        ),
    };
    Ok(vec![
        Change::Create {
            object: item,
            kind: ObjectKind::ChecklistItem,
            value: serde_json::to_value(&new).map_err(|e| ChecklistError::Shape(e.to_string()))?,
            hlc: placeholder(),
        },
        Change::Delete {
            object: task.id,
            hlc: placeholder(),
        },
    ])
}

/// What a bulk conversion of subtasks into items did.
#[derive(Debug, Clone, PartialEq)]
pub struct Bulk {
    /// The batch: for each converted subtask a create and a delete.
    pub changes: Vec<Change>,
    /// The subtasks that stay, and why.
    pub skipped: Vec<(Id, ChecklistError)>,
}

/// The changes that make every item of `owner` a subtask, in the order of
/// the list, each after the one before. `id` makes the new ids.
///
/// # Errors
///
/// When the task has no items, or one does not serialize.
pub fn all_to_subtasks(
    tasks: &[Task],
    items: &[ChecklistItem],
    owner: Id,
    now: Timestamp,
    mut id: impl FnMut() -> Id,
) -> Result<Vec<Change>, ChecklistError> {
    let mut mine: Vec<&ChecklistItem> = items.iter().filter(|i| i.owner == owner).collect();
    if mine.is_empty() {
        return Err(ChecklistError::NoItems);
    }
    mine.sort_by_key(|i| (i.position, i.id));
    let mut tasks = tasks.to_vec();
    let mut changes = Vec::new();
    for item in mine {
        let made = to_subtask(&tasks, items, item.id, id(), now)?;
        if let Some(Change::Create { value, .. }) = made.first() {
            let task: Task = serde_json::from_value(value.clone())
                .map_err(|e| ChecklistError::Shape(e.to_string()))?;
            tasks.push(task);
        }
        changes.extend(made);
    }
    Ok(changes)
}

/// The changes that make every plain subtask of `owner` an item, in the
/// order of the subtasks, each after the one before. Those that carry
/// something an item cannot keep stay and are named in `skipped`.
///
/// # Errors
///
/// When no subtask can be converted; the error is then the reason of the
/// first one, or [`ChecklistError::NoPlainSubtasks`] without subtasks.
pub fn all_from_subtasks(
    tasks: &[Task],
    items: &[ChecklistItem],
    attached: &Attached<'_>,
    owner: Id,
    now: Timestamp,
    mut id: impl FnMut() -> Id,
) -> Result<Bulk, ChecklistError> {
    let mut subs: Vec<&Task> = tasks.iter().filter(|t| t.parent == Some(owner)).collect();
    subs.sort_by_key(|t| (t.position, t.id));
    let mut items = items.to_vec();
    let mut bulk = Bulk {
        changes: Vec::new(),
        skipped: Vec::new(),
    };
    for sub in subs {
        match from_subtask(tasks, &items, attached, sub.id, id(), now) {
            Ok(made) => {
                if let Some(Change::Create { value, .. }) = made.first() {
                    let item: ChecklistItem = serde_json::from_value(value.clone())
                        .map_err(|e| ChecklistError::Shape(e.to_string()))?;
                    items.push(item);
                }
                bulk.changes.extend(made);
            }
            Err(e) => bulk.skipped.push((sub.id, e)),
        }
    }
    if bulk.changes.is_empty() {
        return Err(bulk
            .skipped
            .into_iter()
            .next()
            .map_or(ChecklistError::NoPlainSubtasks, |(_, e)| e));
    }
    Ok(bulk)
}

/// How many of the items are ticked, as `(done, all)`.
#[must_use]
pub fn progress(items: &[ChecklistItem], task: Id) -> (usize, usize) {
    let mine = items.iter().filter(|i| i.owner == task);
    (mine.clone().filter(|i| i.checked).count(), mine.count())
}

/// The change that puts a new item at the end of a task's list.
///
/// # Errors
///
/// When the item does not serialize.
pub fn append(
    items: &[ChecklistItem],
    task: Id,
    id: Id,
    line: &Line,
    now: Timestamp,
) -> Result<Change, ChecklistError> {
    let item = ChecklistItem {
        id,
        created: now,
        modified: now,
        created_by: None,
        owner: task,
        text: line.text.clone(),
        checked: line.checked,
        position: after_last(items.iter().filter(|i| i.owner == task).map(|i| i.position)),
    };
    Ok(Change::Create {
        object: id,
        kind: ObjectKind::ChecklistItem,
        value: serde_json::to_value(&item).map_err(|e| ChecklistError::Shape(e.to_string()))?,
        hlc: placeholder(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::*;

    fn id(n: u8) -> Id {
        format!("01936f5e-8a00-7c3e-9b1a-3f2d4c5b6c{n:02x}")
            .parse()
            .unwrap()
    }

    fn now() -> Timestamp {
        "2026-11-21T09:00:00Z".parse().unwrap()
    }

    fn task(n: u8, parent: Option<u8>, position: i64) -> Task {
        serde_json::from_value(serde_json::json!({
            "id": id(n), "created": now(), "modified": now(), "title": format!("t{n}"),
            "parent": parent.map(id), "position": position,
        }))
        .unwrap()
    }

    fn item(n: u8, owner: u8, position: i64, checked: bool) -> ChecklistItem {
        ChecklistItem {
            id: id(n),
            created: now(),
            modified: now(),
            created_by: None,
            owner: id(owner),
            text: format!("i{n}"),
            checked,
            position,
        }
    }

    #[test]
    fn pasted_lists_become_items() {
        let text = "- [ ] Milch\n* Brot\n  + Käse\n1. Eier\n2) Butter\n[x] Salz\n- [X] Pfeffer\n\n   \n[ ]\nhat [x] drin\n[x]kein Leerzeichen";
        let got: Vec<(String, bool)> = lines(text)
            .into_iter()
            .map(|l| (l.text, l.checked))
            .collect();
        assert_eq!(
            got,
            vec![
                ("Milch".to_owned(), false),
                ("Brot".to_owned(), false),
                ("Käse".to_owned(), false),
                ("Eier".to_owned(), false),
                ("Butter".to_owned(), false),
                ("Salz".to_owned(), true),
                ("Pfeffer".to_owned(), true),
                ("hat [x] drin".to_owned(), false),
                ("[x]kein Leerzeichen".to_owned(), false),
            ]
        );
    }

    #[test]
    fn an_item_becomes_the_last_subtask_and_goes() {
        let tasks = [task(1, None, 0), task(2, Some(1), 4)];
        let items = [item(10, 1, 0, true)];
        let changes = to_subtask(&tasks, &items, id(10), id(3), now()).unwrap();
        let [
            Change::Create { kind, value, .. },
            Change::Delete { object, .. },
        ] = &changes[..]
        else {
            panic!("{changes:?}");
        };
        assert_eq!(*kind, ObjectKind::Task);
        assert_eq!(*object, id(10));
        assert_eq!(value["title"], "i10");
        assert_eq!(value["parent"], id(1).to_string());
        assert_eq!(value["position"], 5);
        assert!(value["completed_at"].is_string(), "ticked means completed");
        assert!(matches!(
            to_subtask(&tasks, &items, id(11), id(3), now()),
            Err(ChecklistError::NoItem(_))
        ));
    }

    #[test]
    fn a_plain_subtask_becomes_the_last_item_and_goes() {
        let tasks = [task(1, None, 0), task(2, Some(1), 0)];
        let items = [item(10, 1, 7, false)];
        let changes =
            from_subtask(&tasks, &items, &Attached::default(), id(2), id(11), now()).unwrap();
        let [
            Change::Create { kind, value, .. },
            Change::Delete { object, .. },
        ] = &changes[..]
        else {
            panic!("{changes:?}");
        };
        assert_eq!(*kind, ObjectKind::ChecklistItem);
        assert_eq!(*object, id(2));
        assert_eq!(value["text"], "t2");
        assert_eq!(value["owner"], id(1).to_string());
        assert_eq!(value["position"], 8);
        assert!(value.get("checked").is_none());
    }

    #[test]
    fn what_an_item_cannot_keep_is_refused() {
        let mut flagged = task(2, Some(1), 0);
        flagged.flagged = true;
        let mut noted = task(3, Some(1), 1);
        noted.note = Some("wichtig".into());
        let tasks = [task(1, None, 0), flagged, noted, task(4, Some(3), 0)];
        let nothing = Attached::default();
        let refuse = |n| from_subtask(&tasks, &[], &nothing, id(n), id(20), now()).unwrap_err();
        assert_eq!(refuse(2), ChecklistError::Carries("a flag"));
        assert_eq!(refuse(3), ChecklistError::HasChildren);
        assert_eq!(refuse(1), ChecklistError::NoParent);
        let mut noted = task(5, Some(1), 0);
        noted.note = Some("wichtig".into());
        assert_eq!(
            from_subtask(
                &[task(1, None, 0), noted],
                &[],
                &nothing,
                id(5),
                id(20),
                now()
            )
            .unwrap_err(),
            ChecklistError::Carries("a note")
        );
    }

    /// An object of its own on task `owner`, from the least JSON it reads.
    fn on(n: u8, owner: u8, more: serde_json::Value) -> serde_json::Value {
        let mut value = serde_json::json!({
            "id": id(n), "created": now(), "modified": now(), "owner": id(owner),
        });
        for (key, v) in more.as_object().unwrap() {
            value[key] = v.clone();
        }
        value
    }

    /// What hangs on the subtask as an object of its own goes with the
    /// task when it is deleted, so each kind is refused and named.
    #[test]
    fn what_hangs_on_a_subtask_is_refused() {
        use serde_json::json;
        let tasks = [task(1, None, 0), task(2, Some(1), 0), task(3, Some(1), 1)];
        let comments =
            [serde_json::from_value::<Comment>(on(50, 2, json!({"body": "x"}))).unwrap()];
        let attachments = [serde_json::from_value::<Attachment>(on(
            51,
            2,
            json!({"blob": "0".repeat(64), "name": "a.txt", "media_type": "text/plain", "size": 1}),
        ))
        .unwrap()];
        let clocks =
            [
                serde_json::from_value::<Clock>(on(52, 2, json!({"start": now(), "end": now()})))
                    .unwrap(),
            ];
        let reminders =
            [
                serde_json::from_value::<Reminder>(on(53, 2, json!({"anchor": "at", "at": now()})))
                    .unwrap(),
            ];
        let links =
            [
                serde_json::from_value::<Link>(on(54, 2, json!({"uri": "https://example.org/"})))
                    .unwrap(),
            ];
        let waiting = [serde_json::from_value::<Link>(on(
            55,
            3,
            json!({"uri": format!("samu:{}", id(2)), "kind": "blocked_by"}),
        ))
        .unwrap()];
        let cases = [
            (
                Attached {
                    comments: &comments,
                    ..Attached::default()
                },
                "comments",
            ),
            (
                Attached {
                    attachments: &attachments,
                    ..Attached::default()
                },
                "attachments",
            ),
            (
                Attached {
                    clocks: &clocks,
                    ..Attached::default()
                },
                "clocked time",
            ),
            (
                Attached {
                    reminders: &reminders,
                    ..Attached::default()
                },
                "reminders",
            ),
            (
                Attached {
                    links: &links,
                    ..Attached::default()
                },
                "links",
            ),
            (
                Attached {
                    links: &waiting,
                    ..Attached::default()
                },
                "a task that waits for it",
            ),
        ];
        for (attached, what) in &cases {
            assert_eq!(
                from_subtask(&tasks, &[], attached, id(2), id(20), now()).unwrap_err(),
                ChecklistError::Carries(what)
            );
            // The sibling has nothing on it and converts; the task that
            // waits has a link of its own.
            if *what != "a task that waits for it" {
                from_subtask(&tasks, &[], attached, id(3), id(20), now()).unwrap();
            }
        }
        // A checklist of its own would go with the task.
        let own = [item(10, 2, 0, false)];
        let nothing = Attached::default();
        assert_eq!(
            from_subtask(&tasks, &own, &nothing, id(2), id(20), now()).unwrap_err(),
            ChecklistError::Carries("a checklist of its own")
        );
        // The whole list: the subtask with a comment stays and is named.
        let bulk = all_from_subtasks(&tasks, &[], &cases[0].0, id(1), now(), || id(30)).unwrap();
        assert_eq!(bulk.changes.len(), 2);
        assert_eq!(
            bulk.skipped,
            vec![(id(2), ChecklistError::Carries("comments"))]
        );
    }

    #[test]
    fn all_items_become_subtasks_in_order() {
        let tasks = [task(1, None, 0), task(2, Some(1), 5)];
        let items = [
            item(10, 1, 1, true),
            item(11, 1, 0, false),
            item(12, 9, 0, false),
        ];
        let mut n = 30;
        let changes = all_to_subtasks(&tasks, &items, id(1), now(), || {
            n += 1;
            id(n)
        })
        .unwrap();
        let mut created = Vec::new();
        for c in &changes {
            if let Change::Create { value, .. } = c {
                let t: Task = serde_json::from_value(value.clone()).unwrap();
                created.push((t.title, t.position, t.completed_at.is_some()));
            }
        }
        assert_eq!(changes.len(), 4);
        assert_eq!(created.len(), 2);
        assert_eq!(created[0].0, "i11");
        assert_eq!(created[1].0, "i10");
        assert!(created[1].2 && !created[0].2);
        assert!(5 < created[0].1 && created[0].1 < created[1].1);
        assert_eq!(
            all_to_subtasks(&tasks, &items, id(2), now(), || id(40)).unwrap_err(),
            ChecklistError::NoItems
        );
    }

    #[test]
    fn plain_subtasks_become_items_and_the_rest_stays() {
        let mut flagged = task(3, Some(1), 1);
        flagged.flagged = true;
        let tasks = [
            task(1, None, 0),
            task(2, Some(1), 0),
            flagged,
            task(4, Some(1), 2),
        ];
        let items = [item(10, 1, 7, false)];
        let mut n = 30;
        let nothing = Attached::default();
        let bulk = all_from_subtasks(&tasks, &items, &nothing, id(1), now(), || {
            n += 1;
            id(n)
        })
        .unwrap();
        assert_eq!(bulk.changes.len(), 4);
        assert_eq!(
            bulk.skipped,
            vec![(id(3), ChecklistError::Carries("a flag"))]
        );
        let mut positions = Vec::new();
        for c in &bulk.changes {
            if let Change::Create { value, .. } = c {
                let i: ChecklistItem = serde_json::from_value(value.clone()).unwrap();
                positions.push((i.text, i.position));
            }
        }
        assert_eq!(positions[0].0, "t2");
        assert_eq!(positions[1].0, "t4");
        assert!(7 < positions[0].1 && positions[0].1 < positions[1].1);
        // Nothing plain: the reason of the first one.
        let only = [task(1, None, 0), tasks[2].clone()];
        assert_eq!(
            all_from_subtasks(&only, &[], &nothing, id(1), now(), || id(40)).unwrap_err(),
            ChecklistError::Carries("a flag")
        );
        assert_eq!(
            all_from_subtasks(&[task(1, None, 0)], &[], &nothing, id(1), now(), || id(40))
                .unwrap_err(),
            ChecklistError::NoPlainSubtasks
        );
    }

    #[test]
    fn progress_counts_one_task() {
        let items = [
            item(10, 1, 0, true),
            item(11, 1, 1, false),
            item(12, 2, 0, true),
        ];
        assert_eq!(progress(&items, id(1)), (1, 2));
        assert_eq!(progress(&items, id(3)), (0, 0));
    }
}
