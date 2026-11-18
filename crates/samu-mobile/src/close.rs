// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Completing or dropping with open tasks below (ADR 0128 §5): the app
//! asks first, then closes them along or lifts them out. Offline, like
//! every change: `samu_core::close::plan` computes it here, the outbox
//! carries it, and the server decides when it arrives.

use std::collections::HashSet;

use jiff::Timestamp;
use samu_api::{Change, ChildFate, CloseField, CloseRequest, Hlc, Id};
use samu_core::clocking::stop_on_close;
use samu_core::overlay::{self, Objects};
use serde_json::Value;

use crate::clocking::ended;
use crate::edit::{descendants, find};
use crate::{MobileError, Samu, input, parse_id};

/// What a close did, or why it waits for a choice.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Closed {
    /// Picked tasks with open tasks below that are not picked themselves;
    /// above zero, nothing was written and the app asks.
    pub asking: u32,
    /// Those open tasks below, all of them counted.
    pub open: u64,
    /// The numbers the changes got in the outbox, for `withdraw`.
    pub numbers: Vec<i64>,
    /// What every changed field of a task was before, for `put_back` once
    /// `withdraw` comes too late.
    pub back: Vec<FieldBack>,
}

/// A field of a task and its value before, as JSON.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FieldBack {
    pub object: String,
    pub field: String,
    pub value: String,
}

fn is_open(objects: &Objects, id: Id) -> bool {
    objects
        .tasks
        .iter()
        .any(|t| t.id == id && t.completed_at.is_none() && t.dropped_at.is_none())
}

/// How many groups are above `id`; the deepest close first, so a group
/// closes after its picked children.
fn depth(objects: &Objects, id: Id) -> usize {
    let mut seen = HashSet::from([id]);
    let mut current = id;
    let mut depth = 0;
    while let Some(parent) = objects
        .tasks
        .iter()
        .find(|t| t.id == current)
        .and_then(|t| t.parent)
    {
        // A cycle in broken data stops here.
        if !seen.insert(parent) {
            break;
        }
        depth += 1;
        current = parent;
    }
    depth
}

/// The value of `field` on task `id` now, as JSON; null when unset.
fn value_of(objects: &Objects, id: Id, field: &str) -> Value {
    objects
        .tasks
        .iter()
        .find(|t| t.id == id)
        .and_then(|t| serde_json::to_value(t).ok())
        .and_then(|v| v.get(field).cloned())
        .unwrap_or(Value::Null)
}

/// The changes closing `ordered` takes, one after another on the objects as
/// the ones before left them, with the clocks that end, and what the
/// changed task fields were before.
fn plan_all(
    objects: &Objects,
    ordered: &[Id],
    request: &CloseRequest,
    now: Timestamp,
    stamp: &mut dyn FnMut() -> Hlc,
) -> Result<(Vec<Change>, Vec<FieldBack>), MobileError> {
    let mut objects = objects.clone();
    let mut changes = Vec::new();
    let mut back: Vec<FieldBack> = Vec::new();
    let mut touched: HashSet<(Id, String)> = HashSet::new();
    for &id in ordered {
        let plan = samu_core::close::plan(&objects.snapshot(), id, request, now)
            .map_err(|e| input(e.to_string()))?;
        let mut these = Vec::new();
        for change in plan.changes {
            let change = restamp(change, stamp());
            let mut clocks = Vec::new();
            if let Change::Set { object, field, .. } = &change {
                // The first value counts: that is what it was before all.
                if touched.insert((*object, field.clone())) {
                    back.push(FieldBack {
                        object: object.to_string(),
                        field: field.clone(),
                        value: value_of(&objects, *object, field).to_string(),
                    });
                }
                if field == "completed_at" || field == "dropped_at" {
                    clocks = ended(stop_on_close(&objects.clocks, *object, now), stamp);
                }
            }
            these.push(change);
            these.extend(clocks);
        }
        for change in &these {
            overlay::apply(&mut objects, change).map_err(|e| input(e.to_string()))?;
        }
        changes.extend(these);
    }
    Ok((changes, back))
}

/// `change` with the device's own stamp: the plan stamps for the server.
fn restamp(change: Change, hlc: Hlc) -> Change {
    match change {
        Change::Set {
            object,
            field,
            value,
            note,
            ..
        } => Change::Set {
            object,
            field,
            value,
            note,
            hlc,
        },
        other => other,
    }
}

#[uniffi::export]
impl Samu {
    /// Completes (`field` `completed`) or drops (`dropped`) the picked
    /// tasks (ADR 0128). `children` is `ask`, `close` or `lift`: with
    /// `ask` and open tasks below that are not picked themselves, nothing
    /// is written and [`Closed::asking`] tells the app to ask. A clock
    /// running on a closed task ends (ADR 0046 §3).
    pub fn close_tasks(
        &self,
        ids: Vec<String>,
        field: String,
        children: String,
    ) -> Result<Closed, MobileError> {
        let field = match field.as_str() {
            "completed" => CloseField::Completed,
            "dropped" => CloseField::Dropped,
            other => return Err(input(format!("{other:?}: completed or dropped"))),
        };
        let children = match children.as_str() {
            "ask" => ChildFate::Ask,
            "close" => ChildFate::Close,
            "lift" => ChildFate::Lift,
            other => return Err(input(format!("{other:?}: ask, close or lift"))),
        };
        let mut ids: Vec<Id> = ids
            .iter()
            .map(|id| parse_id(id))
            .collect::<Result<_, _>>()?;
        let mut seen = HashSet::new();
        ids.retain(|id| seen.insert(*id));

        let request = CloseRequest {
            field,
            children,
            note: None,
            preview: false,
        };
        let mut closed = Closed {
            asking: 0,
            open: 0,
            numbers: Vec::new(),
            back: Vec::new(),
        };
        let mut back = Vec::new();
        closed.numbers = self.queue_numbered(|objects, stamp| {
            for &id in &ids {
                find(objects, id)?;
            }
            let picked: HashSet<Id> = ids.iter().copied().collect();
            for &id in &ids {
                let open = descendants(objects, id)
                    .into_iter()
                    .filter(|t| !picked.contains(t) && is_open(objects, *t))
                    .count() as u64;
                if open > 0 {
                    closed.asking += 1;
                    closed.open += open;
                }
            }
            if closed.asking > 0 && children == ChildFate::Ask {
                return Ok(Vec::new());
            }
            // Picked children count as answered: close them along.
            let request = CloseRequest {
                children: if children == ChildFate::Ask {
                    ChildFate::Close
                } else {
                    children
                },
                ..request.clone()
            };
            let mut ordered = ids.clone();
            ordered.sort_by_key(|id| std::cmp::Reverse(depth(objects, *id)));
            let (changes, b) = plan_all(objects, &ordered, &request, Timestamp::now(), stamp)?;
            back = b;
            Ok(changes)
        })?;
        closed.back = back;
        Ok(closed)
    }

    /// How many open tasks lie below `id`, at any depth: above zero, a
    /// widget or a notification opens the task instead of closing it,
    /// since it cannot ask (ADR 0128 §5).
    pub fn open_below(&self, id: String) -> Result<u64, MobileError> {
        let id = parse_id(&id)?;
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        Ok(descendants(&objects, id)
            .into_iter()
            .filter(|t| is_open(&objects, *t))
            .count() as u64)
    }

    /// Writes the fields back as they were before a close (ADR 0128 §5),
    /// for „Rückgängig“ once the changes have left the phone.
    pub fn put_back(&self, back: Vec<FieldBack>) -> Result<(), MobileError> {
        let back: Vec<(Id, String, Value)> = back
            .into_iter()
            .map(|b| {
                let value = serde_json::from_str(&b.value).map_err(|e| input(e.to_string()))?;
                Ok((parse_id(&b.object)?, b.field, value))
            })
            .collect::<Result<_, MobileError>>()?;
        self.queue(|objects, stamp| {
            let mut changes = Vec::new();
            for (object, field, value) in back {
                if objects.tasks.iter().any(|t| t.id == object) {
                    changes.push(Change::Set {
                        object,
                        field,
                        value,
                        note: None,
                        hlc: stamp(),
                    });
                }
            }
            Ok(changes)
        })
    }
}
