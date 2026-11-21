// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `samu checklist` (ADR 0141): the items of a task's checklist are
//! objects of their own over the ordinary routes; turning one into a
//! subtask and back is a batch of ordinary changes planned by samu-core.

use std::io::Read;

use anyhow::{Result, bail};
use clap::Subcommand;
use samu_api::{AnyObject, Change, ChangesPage, ChecklistItem, Id, Task};
use samu_client::Client;
use samu_core::checklist;
use serde_json::{Value, json};

use crate::config::Config;
use crate::output::{Format, emit};
use crate::{resolve, set_field, short, signed_in, write};

#[derive(Debug, Subcommand)]
pub(crate) enum ChecklistCommand {
    /// The items of a task, in order.
    List { task: String },
    /// Add items at the end: one per line, read from the argument or, for
    /// `-`, from stdin. A pasted Org or Markdown list works: `- [ ] ` and
    /// `1. ` are dropped, `[x]` ticks the item.
    Add {
        task: String,
        /// The text, or `-` for stdin.
        text: String,
    },
    /// Tick an item.
    Check { item: String },
    /// Untick an item.
    Uncheck { item: String },
    /// Change the text of an item.
    Edit { item: String, text: String },
    /// Move an item to a place in its list, counted from 1.
    Move { item: String, place: usize },
    /// Delete an item.
    Rm { item: String },
    /// Make an item a subtask of its task; the item goes.
    ToSubtask { item: String },
    /// Make a plain subtask an item of its parent's list; the subtask
    /// goes. A subtask with a note, dates, tags or the like is refused.
    FromSubtask { task: String },
}

/// The tasks and items of the account.
struct Objects {
    tasks: Vec<Task>,
    items: Vec<ChecklistItem>,
}

fn objects(client: &Client) -> Result<Objects> {
    let page: ChangesPage =
        serde_json::from_value(client.call("GET", "/changes?after=0", None)?.json()?)?;
    let mut out = Objects {
        tasks: Vec::new(),
        items: Vec::new(),
    };
    for object in page.objects {
        match object {
            AnyObject::Task(t) => out.tasks.push(t),
            AnyObject::ChecklistItem(i) => out.items.push(i),
            _ => {}
        }
    }
    Ok(out)
}

/// The items of `owner`, in their order.
fn ordered(items: &[ChecklistItem], owner: Id) -> Vec<&ChecklistItem> {
    let mut mine: Vec<&ChecklistItem> = items.iter().filter(|i| i.owner == owner).collect();
    mine.sort_by_key(|i| (i.position, i.id));
    mine
}

/// The records of `samu checklist list`.
pub(crate) fn records(items: &[ChecklistItem], owner: Id) -> Vec<Value> {
    ordered(items, owner)
        .iter()
        .enumerate()
        .map(|(n, i)| {
            json!({
                "id": i.id, "short": short(&i.id), "place": n + 1,
                "checked": i.checked, "mark": if i.checked { "[x]" } else { "[ ]" },
                "text": i.text,
            })
        })
        .collect()
}

fn text_of(text: &str) -> Result<String> {
    if text != "-" {
        return Ok(text.to_owned());
    }
    let mut body = String::new();
    std::io::stdin().read_to_string(&mut body)?;
    Ok(body)
}

fn find_item(objects: &Objects, id: Id) -> Result<&ChecklistItem> {
    match objects.items.iter().find(|i| i.id == id) {
        Some(item) => Ok(item),
        None => bail!("{id} is not a checklist item"),
    }
}

pub(crate) fn run(config: &Config, format: Format, command: ChecklistCommand) -> Result<()> {
    let client = signed_in(config)?;
    match command {
        ChecklistCommand::List { task } => {
            let task = resolve(&client, &task)?;
            let all = objects(&client)?;
            emit(
                format,
                &records(&all.items, task),
                &["short", "mark", "text"],
            )
        }
        ChecklistCommand::Add { task, text } => {
            let task = resolve(&client, &task)?;
            let all = objects(&client)?;
            if !all.tasks.iter().any(|t| t.id == task) {
                bail!("{task} is not a task");
            }
            let lines = checklist::lines(&text_of(&text)?);
            if lines.is_empty() {
                bail!("nothing to add: no line with a text");
            }
            // Each item after the one before it, as one batch.
            let mut items = all.items;
            let mut changes = Vec::new();
            let mut made = Vec::new();
            for line in &lines {
                let id = Id::new();
                let change = checklist::append(&items, task, id, line, jiff::Timestamp::now())?;
                if let Change::Create { value, .. } = &change {
                    items.push(serde_json::from_value(value.clone())?);
                }
                changes.push(change);
                made.push(json!({"id": id, "short": short(&id), "text": line.text}));
            }
            write(&client, &changes)?;
            emit(format, &made, &["short", "text"])
        }
        ChecklistCommand::Check { item } => tick(&client, &item, true),
        ChecklistCommand::Uncheck { item } => tick(&client, &item, false),
        ChecklistCommand::Edit { item, text } => {
            let id = resolve(&client, &item)?;
            let text = text.trim();
            if text.is_empty() || text.contains('\n') {
                bail!("an item is one line of text");
            }
            find_item(&objects(&client)?, id)?;
            set_field(&client, id, "text", json!(text))
        }
        ChecklistCommand::Move { item, place } => {
            let id = resolve(&client, &item)?;
            let all = objects(&client)?;
            let owner = find_item(&all, id)?.owner;
            let mut order: Vec<Id> = ordered(&all.items, owner).iter().map(|i| i.id).collect();
            if place == 0 || place > order.len() {
                bail!("the place is from 1 to {}", order.len());
            }
            order.retain(|i| *i != id);
            order.insert(place - 1, id);
            // Only what changes its number is written.
            let mut changes = Vec::new();
            for (n, i) in order.iter().enumerate() {
                let position = i64::try_from(n)?;
                if all
                    .items
                    .iter()
                    .any(|x| x.id == *i && x.position != position)
                {
                    changes.push(Change::Set {
                        object: *i,
                        field: "position".into(),
                        value: json!(position),
                        note: None,
                        hlc: crate::placeholder(),
                    });
                }
            }
            if !changes.is_empty() {
                write(&client, &changes)?;
            }
            Ok(())
        }
        ChecklistCommand::Rm { item } => {
            let id = resolve(&client, &item)?;
            find_item(&objects(&client)?, id)?;
            write(
                &client,
                &[Change::Delete {
                    object: id,
                    hlc: crate::placeholder(),
                }],
            )?;
            Ok(())
        }
        ChecklistCommand::ToSubtask { item } => {
            let id = resolve(&client, &item)?;
            let all = objects(&client)?;
            let task = Id::new();
            let changes =
                checklist::to_subtask(&all.tasks, &all.items, id, task, jiff::Timestamp::now())?;
            write(&client, &changes)?;
            emit(
                format,
                &[json!({"id": task, "short": short(&task)})],
                &["short"],
            )
        }
        ChecklistCommand::FromSubtask { task } => {
            let id = resolve(&client, &task)?;
            let all = objects(&client)?;
            let item = Id::new();
            let changes =
                checklist::from_subtask(&all.tasks, &all.items, id, item, jiff::Timestamp::now())?;
            write(&client, &changes)?;
            emit(
                format,
                &[json!({"id": item, "short": short(&item)})],
                &["short"],
            )
        }
    }
}

fn tick(client: &Client, item: &str, checked: bool) -> Result<()> {
    let id = resolve(client, item)?;
    find_item(&objects(client)?, id)?;
    set_field(client, id, "checked", json!(checked))
}
