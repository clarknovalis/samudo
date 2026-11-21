// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `samu show <id> --full`: everything the view of one task or project
//! shows, in one record. Each part is the record of the command that shows
//! it alone (`samu list`, `samu comments`, `samu prop`, …), so a client
//! that fills such a view starts one process instead of nine, and the
//! account is fetched once instead of once per command.

use std::collections::HashSet;

use anyhow::{Result, bail};
use samu_api::{
    AnyObject, ChangesPage, HabitStatus, Id, LogEntry, RenderRequest, RenderResponse, RenderText,
    TaskStatus,
};
use serde_json::{Value, json};

use crate::output::{Format, emit};
use crate::{Names, comments, habits, object_record, props, remind, status_record};
use samu_client::{Client, Error as Failure};

/// The open tasks below a task, or in a project, in the order of `statuses`.
fn below<'a>(object: &AnyObject, statuses: &'a [TaskStatus]) -> Vec<&'a TaskStatus> {
    let open: Vec<&TaskStatus> = statuses
        .iter()
        .filter(|s| s.task.completed_at.is_none() && s.task.dropped_at.is_none())
        .collect();
    let mut ids: HashSet<Id> = match object {
        AnyObject::Project(p) => open
            .iter()
            .filter(|s| s.task.project == Some(p.id))
            .map(|s| s.task.id)
            .collect(),
        _ => HashSet::new(),
    };
    let top = object.id();
    loop {
        let before = ids.len();
        for status in &open {
            if status
                .task
                .parent
                .is_some_and(|p| Some(p) == top || ids.contains(&p))
            {
                ids.insert(status.task.id);
            }
        }
        if ids.len() == before {
            break;
        }
    }
    open.into_iter()
        .filter(|s| ids.contains(&s.task.id))
        .collect()
}

/// What each of `texts` reads as, `org` or `markdown`, as the server
/// decides it (`POST /render`); one request for all, none for no text.
fn formats(client: &Client, texts: &[&str]) -> Result<Vec<Value>> {
    if texts.is_empty() {
        return Ok(Vec::new());
    }
    let request = RenderRequest {
        texts: texts
            .iter()
            .map(|text| RenderText {
                text: (*text).to_owned(),
                format: None,
            })
            .collect(),
    };
    let answer = client.call("POST", "/render", Some(&serde_json::to_value(&request)?))?;
    let rendered: RenderResponse = serde_json::from_value(answer.json()?)?;
    Ok(rendered
        .documents
        .iter()
        .map(|document| json!(document.format))
        .collect())
}

pub(crate) fn show(client: &Client, format: Format, id: Id, log: bool) -> Result<()> {
    let page: ChangesPage =
        serde_json::from_value(client.call("GET", "/changes?after=0", None)?.json()?)?;
    let Some(object) = page.objects.iter().find(|o| o.id() == Some(id)) else {
        return Err(Failure::NotFound(format!("nothing has the id {id}")).into());
    };
    if !matches!(object, AnyObject::Task(_) | AnyObject::Project(_)) {
        bail!("{id} is neither a task nor a project: --full shows only those");
    }
    // As `samu list --all --no-scope`: what is due outside is no row.
    let mut statuses: Vec<TaskStatus> =
        serde_json::from_value(client.call("GET", "/available?all=true", None)?.json()?)?;
    statuses.retain(|s| !s.outside);
    let names = Names::new(&page, &statuses);
    let row = statuses
        .iter()
        .find(|s| s.task.id == id)
        .map(|s| status_record(s, &names));
    let below = below(object, &statuses);

    // The habits among the tasks below, and the task's own.
    let habits_below: Vec<Value> = if below.iter().any(|s| s.task.habit.is_some()) {
        let all: Vec<HabitStatus> =
            serde_json::from_value(client.call("GET", "/habits", None)?.json()?)?;
        all.iter()
            .filter(|h| below.iter().any(|s| s.task.id == h.task.id))
            .map(habits::record)
            .collect()
    } else {
        Vec::new()
    };
    let habit = match object {
        AnyObject::Task(task) if task.habit.is_some() => {
            Some(habits::shown(&habits::fetch(client, id, Some(1))?))
        }
        _ => None,
    };

    let mut comment_records = comments::comment_records(&page.objects, id);
    let shown = object_record(object);
    let note = shown["object"]["note"]
        .as_str()
        .filter(|n| !n.trim().is_empty());
    let mut texts: Vec<&str> = note.into_iter().collect();
    let bodies: Vec<String> = comment_records
        .iter()
        .map(|c| c["body"].as_str().unwrap_or_default().to_owned())
        .collect();
    texts.extend(bodies.iter().map(String::as_str));
    let mut read_as = formats(client, &texts)?.into_iter();
    let note_format = note.and_then(|_| read_as.next());
    for (comment, format) in comment_records.iter_mut().zip(read_as) {
        comment["format"] = format;
    }

    let log = if log {
        let answer = client.call("GET", &format!("/objects/{id}/logbook"), None)?;
        let entries: Vec<LogEntry> = serde_json::from_value(answer.json()?)?;
        Some(serde_json::to_value(entries)?)
    } else {
        None
    };

    let record = json!({
        "shown": shown,
        "note_format": note_format,
        "row": row,
        "below": below.iter().map(|s| status_record(s, &names)).collect::<Vec<_>>(),
        "habits": habits_below,
        "habit": habit,
        "reminders": remind::records_in(&page.objects, Some(id)),
        "props": props::records_of(object, &statuses, &props::templates_in(&page.objects))?,
        "comments": comment_records,
        "attachments": comments::attachment_records(&page.objects, id),
        "log": log,
    });
    if format == Format::Plain {
        println!("{}", serde_json::to_string_pretty(&record)?);
        Ok(())
    } else {
        emit(format, &[record], &["shown"])
    }
}
