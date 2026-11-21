// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The checklist of a task on the phone (ADR 0141).
//!
//! An item is an ordinary object through the outbox, like a comment. The
//! rules that need the whole list (splitting pasted text, turning an item
//! into a subtask and back) are `samu_core::checklist`; here they are
//! stamped with the phone's own clock and queued.

use jiff::Timestamp;
use samu_api::{Change, ChecklistItem, Id};
use samu_core::checklist;
use samu_core::clock::restamp;
use serde_json::Value;

use crate::{MobileError, Samu, input, parse_id};

/// One item of a checklist.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ChecklistRow {
    pub id: String,
    pub text: String,
    pub checked: bool,
    /// Waiting in the outbox: not yet on the server.
    pub pending: bool,
}

/// The items of `task`, in their order.
fn items_of(items: &[ChecklistItem], task: Id) -> Vec<&ChecklistItem> {
    let mut mine: Vec<&ChecklistItem> = items.iter().filter(|i| i.owner == task).collect();
    mine.sort_by_key(|i| (i.position, i.id));
    mine
}

#[uniffi::export]
impl Samu {
    /// The checklist of a task, in order.
    pub fn checklist(&self, task: String) -> Result<Vec<ChecklistRow>, MobileError> {
        let task = parse_id(&task)?;
        let cache = self.cache()?;
        let (objects, touched) = Self::view(&cache)?;
        if !objects.tasks.iter().any(|t| t.id == task) {
            return Ok(Vec::new());
        }
        Ok(items_of(&objects.checklist_items, task)
            .into_iter()
            .map(|i| ChecklistRow {
                id: i.id.to_string(),
                text: i.text.clone(),
                checked: i.checked,
                pending: touched.contains(&i.id),
            })
            .collect())
    }

    /// New items at the end of a task's list, one per line of `text`;
    /// what a list in Org or Markdown puts in front of a line is dropped
    /// and a ticked box ticks the item. Returns the new ids.
    pub fn checklist_add(&self, task: String, text: String) -> Result<Vec<String>, MobileError> {
        let task = parse_id(&task)?;
        let lines = checklist::lines(&text);
        if lines.is_empty() {
            return Err(input("a checklist item needs a text"));
        }
        let mut ids = Vec::new();
        self.queue(|objects, stamp| {
            if !objects.tasks.iter().any(|t| t.id == task) {
                return Err(input(format!("no task {task}")));
            }
            let now = Timestamp::now();
            let mut items = objects.checklist_items.clone();
            let mut changes = Vec::new();
            for line in &lines {
                let id = Id::new();
                let change = checklist::append(&items, task, id, line, now)
                    .map_err(|e| input(e.to_string()))?;
                let change = restamp(change, stamp());
                // The next line goes after this one.
                if let Change::Create { value, .. } = &change
                    && let Ok(item) = serde_json::from_value::<ChecklistItem>(value.clone())
                {
                    items.push(item);
                }
                ids.push(id.to_string());
                changes.push(change);
            }
            Ok(changes)
        })?;
        Ok(ids)
    }

    /// An item ticked or unticked.
    pub fn checklist_check(&self, id: String, checked: bool) -> Result<(), MobileError> {
        self.set_item(&id, "checked", Value::Bool(checked))
    }

    /// An item's new text.
    pub fn checklist_edit(&self, id: String, text: String) -> Result<(), MobileError> {
        let text = text.trim().to_string();
        if text.is_empty() {
            return Err(input("a checklist item needs a text"));
        }
        self.set_item(&id, "text", Value::String(text))
    }

    /// An item moved to place `to` (0 = first) in its list.
    pub fn checklist_move(&self, id: String, to: u32) -> Result<(), MobileError> {
        let id = parse_id(&id)?;
        self.queue(|objects, stamp| {
            let owner = objects
                .checklist_items
                .iter()
                .find(|i| i.id == id)
                .map(|i| i.owner)
                .ok_or_else(|| input(format!("no checklist item {id}")))?;
            let mut list = items_of(&objects.checklist_items, owner);
            let from = list.iter().position(|i| i.id == id).unwrap_or(0);
            let item = list.remove(from);
            let to = usize::try_from(to).unwrap_or(usize::MAX).min(list.len());
            list.insert(to, item);
            Ok(list
                .iter()
                .enumerate()
                .filter(|(place, i)| i.position != *place as i64)
                .map(|(place, i)| Change::Set {
                    object: i.id,
                    field: "position".into(),
                    value: Value::from(place as i64),
                    note: None,
                    hlc: stamp(),
                })
                .collect())
        })
    }

    /// An item gone.
    pub fn checklist_remove(&self, id: String) -> Result<(), MobileError> {
        let id = parse_id(&id)?;
        self.queue(|objects, stamp| {
            if !objects.checklist_items.iter().any(|i| i.id == id) {
                return Err(input(format!("no checklist item {id}")));
            }
            Ok(vec![Change::Delete {
                object: id,
                hlc: stamp(),
            }])
        })
    }

    /// An item made a subtask of its task, at the end of the subtasks;
    /// returns the new task.
    pub fn checklist_to_subtask(&self, id: String) -> Result<String, MobileError> {
        let id = parse_id(&id)?;
        let task = Id::new();
        self.queue(|objects, stamp| {
            let changes = checklist::to_subtask(
                &objects.tasks,
                &objects.checklist_items,
                id,
                task,
                Timestamp::now(),
            )
            .map_err(|e| input(e.to_string()))?;
            Ok(changes.into_iter().map(|c| restamp(c, stamp())).collect())
        })?;
        Ok(task.to_string())
    }

    /// A subtask made an item at the end of its parent's checklist;
    /// refused, with the reason, when the item could not keep what the
    /// task has. Returns the new item.
    pub fn checklist_from_subtask(&self, task: String) -> Result<String, MobileError> {
        let task = parse_id(&task)?;
        let item = Id::new();
        self.queue(|objects, stamp| {
            let changes = checklist::from_subtask(
                &objects.tasks,
                &objects.checklist_items,
                &checklist::Attached::from(objects),
                task,
                item,
                Timestamp::now(),
            )
            .map_err(|e| input(e.to_string()))?;
            Ok(changes.into_iter().map(|c| restamp(c, stamp())).collect())
        })?;
        Ok(item.to_string())
    }

    /// Every item of the task made a subtask, in the order of the list.
    pub fn checklist_all_to_subtasks(&self, task: String) -> Result<(), MobileError> {
        let task = parse_id(&task)?;
        self.queue(|objects, stamp| {
            let changes = checklist::all_to_subtasks(
                &objects.tasks,
                &objects.checklist_items,
                task,
                Timestamp::now(),
                Id::new,
            )
            .map_err(|e| input(e.to_string()))?;
            Ok(changes.into_iter().map(|c| restamp(c, stamp())).collect())
        })
    }

    /// Every plain subtask of the task made an item. Those that carry
    /// something an item cannot keep stay; the result names each, with the
    /// reason, one line per subtask. Refused when none can be converted.
    pub fn checklist_all_from_subtasks(&self, task: String) -> Result<Vec<String>, MobileError> {
        let task = parse_id(&task)?;
        let mut skipped = Vec::new();
        self.queue(|objects, stamp| {
            let bulk = checklist::all_from_subtasks(
                &objects.tasks,
                &objects.checklist_items,
                &checklist::Attached::from(objects),
                task,
                Timestamp::now(),
                Id::new,
            )
            .map_err(|e| input(e.to_string()))?;
            skipped = bulk
                .skipped
                .iter()
                .map(|(id, why)| {
                    let title = objects
                        .tasks
                        .iter()
                        .find(|t| t.id == *id)
                        .map_or_else(String::new, |t| t.title.clone());
                    format!("{title}: {why}")
                })
                .collect();
            Ok(bulk
                .changes
                .into_iter()
                .map(|c| restamp(c, stamp()))
                .collect())
        })?;
        Ok(skipped)
    }
}

impl Samu {
    fn set_item(&self, id: &str, field: &str, value: Value) -> Result<(), MobileError> {
        let id = parse_id(id)?;
        self.queue(|objects, stamp| {
            if !objects.checklist_items.iter().any(|i| i.id == id) {
                return Err(input(format!("no checklist item {id}")));
            }
            Ok(vec![Change::Set {
                object: id,
                field: field.into(),
                value,
                note: None,
                hlc: stamp(),
            }])
        })
    }
}
