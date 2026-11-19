// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `samu template` (ADR 0073): capture templates are objects of their own,
//! made and changed over `/changes` like a scope. A template is called as
//! `/name` in `samu add`, or with `samu add --template name`; its key
//! calls it after the capture prefix in the web and samu.el (ADR 0084).

use anyhow::{Result, bail};
use clap::Subcommand;
use samu_api::{
    AnyObject, Change, ChangesPage, Id, ObjectKind, Period, ReminderAnchor, Template,
    TemplateReminder,
};
use serde_json::{Value, json};

use crate::config::Config;
use crate::output::{Format, emit};
use crate::{placeholder, short, signed_in, write};
use samu_client::Client;

#[derive(Debug, Subcommand)]
pub(crate) enum TemplateCommand {
    /// Every template with its text and reminders.
    List,
    /// Make a template: a name to call it by and a line of quick capture,
    /// like `#Gewohnheiten @habit p:heute 21:00 every+:1d`. Free text in it
    /// goes before the title of what is captured, like `[EPIC] @epic`.
    Add {
        name: String,
        /// The marks; several words are joined.
        #[arg(required = true)]
        text: Vec<String>,
        /// A reminder for the new task: `planned`, `due` or `defer`, with
        /// `:` and how long before, like `due:1h`. Again for more.
        #[arg(long = "remind")]
        reminders: Vec<String>,
        /// The key that calls it after the capture prefix, like `h` for
        /// `C h`: one letter or digit.
        #[arg(long)]
        key: Option<String>,
    },
    /// Replace a template's text.
    Text {
        name: String,
        #[arg(required = true)]
        text: Vec<String>,
    },
    /// Replace a template's reminders; `--clear` takes them all off.
    Remind {
        name: String,
        /// Like `planned` or `due:1h`.
        #[arg(required_unless_present = "clear")]
        reminders: Vec<String>,
        #[arg(long, conflicts_with = "reminders")]
        clear: bool,
    },
    /// Give a template the key that calls it after the capture prefix
    /// (`C h` in the web), or take it off with `--none`.
    Key {
        name: String,
        /// One letter or digit; `h` and `H` are two keys.
        #[arg(required_unless_present = "none")]
        key: Option<String>,
        #[arg(long, conflicts_with = "key")]
        none: bool,
    },
    /// Give a template another name, which is also its call.
    Rename { name: String, title: String },
    /// Delete a template; what was made with it stays.
    Delete { name: String },
}

/// Whether `name` may name a template (ADR 0073 §1), as the server reads it.
pub(crate) fn callable(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

/// The key as the server takes it (ADR 0084 §1), checked here first so a
/// slip is told before anything is sent; and that no other template has it.
fn free_key(templates: &[Template], key: &str, own: Option<Id>) -> Result<()> {
    let mut chars = key.chars();
    if !(chars.next().is_some_and(char::is_alphanumeric) && chars.next().is_none()) {
        bail!("{key:?}: a template's key is one letter or digit");
    }
    if let Some(other) = templates
        .iter()
        .find(|t| Some(t.id) != own && t.key.as_deref() == Some(key))
    {
        bail!(
            "the key {key:?} is the template {:?}'s already",
            other.title
        );
    }
    Ok(())
}

fn templates(client: &Client) -> Result<Vec<Template>> {
    let page: ChangesPage =
        serde_json::from_value(client.call("GET", "/changes?after=0", None)?.json()?)?;
    let mut found: Vec<Template> = page
        .objects
        .into_iter()
        .filter_map(|o| match o {
            AnyObject::Template(t) => Some(t),
            _ => None,
        })
        .collect();
    found.sort_by_key(|t| (t.position, t.title.to_lowercase()));
    Ok(found)
}

/// A template by its name (any case, a leading `/` allowed) or the start or end of
/// its id.
fn find<'a>(templates: &'a [Template], name: &str) -> Result<&'a Template> {
    let name = name.trim_start_matches('/');
    let lower = name.to_lowercase();
    if let Some(t) = templates.iter().find(|t| t.title.to_lowercase() == lower) {
        return Ok(t);
    }
    let by_id: Vec<&Template> = templates
        .iter()
        .filter(|t| crate::id_matches(&t.id, &lower))
        .collect();
    match by_id.as_slice() {
        [one] => Ok(one),
        [] => bail!("no template {name:?}"),
        _ => bail!("{name:?} names several templates; take more of the id"),
    }
}

/// `planned`, `due:1h`, `defer:30m` or `daily@20:00` (ADR 0093) as a
/// reminder of a template.
fn reminder(text: &str) -> Result<TemplateReminder> {
    if let Some((anchor, time)) = text.split_once('@') {
        if !matches!(anchor.trim().to_lowercase().as_str(), "daily" | "täglich") {
            bail!("{anchor:?}: only daily takes a time of day, like daily@20:00");
        }
        return Ok(TemplateReminder {
            anchor: ReminderAnchor::Daily,
            before: None,
            time: Some(crate::remind::time_of_day(time)?),
        });
    }
    let (anchor, before) = match text.split_once(':') {
        Some((anchor, before)) => (anchor, Some(before)),
        None => (text, None),
    };
    let anchor = match anchor.trim().to_lowercase().as_str() {
        "planned" | "geplant" => ReminderAnchor::Planned,
        "due" | "fällig" => ReminderAnchor::Due,
        "defer" | "start" => ReminderAnchor::Defer,
        "daily" | "täglich" => bail!("daily needs a time of day, like daily@20:00"),
        other => bail!("{other:?}: a reminder is on planned, due or defer, or daily@HH:MM"),
    };
    let before = match before.map(str::trim).filter(|b| !b.is_empty()) {
        Some(b) => {
            Some(Period(b.parse().map_err(|e| {
                anyhow::anyhow!("{b:?} is not a length of time: {e}")
            })?))
        }
        None => None,
    };
    Ok(TemplateReminder {
        anchor,
        before,
        time: None,
    })
}

/// A template's reminders in a few words, like `planned, 1h before due`.
fn reminders_text(reminders: &[TemplateReminder]) -> String {
    reminders
        .iter()
        .map(|r| {
            let anchor = serde_json::to_value(r.anchor)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_else(|| "unknown".into());
            match (r.time, r.before) {
                (Some(time), _) => format!("{anchor}@{}", time.strftime("%H:%M")),
                (None, Some(before)) => format!("{} before {anchor}", before.0),
                (None, None) => anchor,
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn set(object: Id, field: &str, value: Value) -> Change {
    Change::Set {
        object,
        field: field.into(),
        value,
        note: None,
        hlc: placeholder(),
    }
}

pub(crate) fn run(config: &Config, format: Format, command: TemplateCommand) -> Result<()> {
    let client = signed_in(config)?;
    match command {
        TemplateCommand::List => {
            let records: Vec<Value> = templates(&client)?
                .iter()
                .map(|t| {
                    json!({
                        "id": t.id,
                        "short": short(&t.id),
                        "title": t.title,
                        "call": format!("/{}", t.title),
                        "key": t.key,
                        "text": t.text,
                        "reminders": reminders_text(&t.reminders),
                        "template": t,
                    })
                })
                .collect();
            emit(
                format,
                &records,
                &["short", "key", "call", "text", "reminders"],
            )?;
        }
        TemplateCommand::Add {
            name,
            text,
            reminders,
            key,
        } => {
            let name = name.trim_start_matches('/').to_owned();
            if !callable(&name) {
                bail!("{name:?}: a template's name has letters, digits, - and _ only");
            }
            let reminders = reminders
                .iter()
                .map(|r| reminder(r))
                .collect::<Result<Vec<_>>>()?;
            let all = templates(&client)?;
            if let Some(key) = &key {
                free_key(&all, key, None)?;
            }
            let position = all.iter().map(|t| t.position + 1).max().unwrap_or(0);
            let now = jiff::Timestamp::now();
            let template = Template {
                id: Id::new(),
                created: now,
                modified: now,
                title: name,
                text: text.join(" "),
                reminders,
                position,
                key,
            };
            write(
                &client,
                &[Change::Create {
                    object: template.id,
                    kind: ObjectKind::Template,
                    value: serde_json::to_value(&template)?,
                    hlc: placeholder(),
                }],
            )?;
            emit(
                format,
                &[json!({"id": template.id, "call": format!("/{}", template.title)})],
                &["id"],
            )?;
        }
        TemplateCommand::Text { name, text } => {
            let all = templates(&client)?;
            let found = find(&all, &name)?;
            write(&client, &[set(found.id, "text", json!(text.join(" ")))])?;
        }
        TemplateCommand::Remind {
            name,
            reminders,
            clear,
        } => {
            let all = templates(&client)?;
            let found = find(&all, &name)?;
            // A list changes as a whole; none is the empty list.
            let value = if clear {
                json!([])
            } else {
                serde_json::to_value(
                    reminders
                        .iter()
                        .map(|r| reminder(r))
                        .collect::<Result<Vec<_>>>()?,
                )?
            };
            write(&client, &[set(found.id, "reminders", value)])?;
        }
        TemplateCommand::Key { name, key, none } => {
            let all = templates(&client)?;
            let found = find(&all, &name)?;
            let value = match key.filter(|_| !none) {
                Some(key) => {
                    free_key(&all, &key, Some(found.id))?;
                    json!(key)
                }
                None => Value::Null,
            };
            write(&client, &[set(found.id, "key", value)])?;
        }
        TemplateCommand::Rename { name, title } => {
            let all = templates(&client)?;
            let found = find(&all, &name)?;
            let title = title.trim_start_matches('/');
            if !callable(title) {
                bail!("{title:?}: a template's name has letters, digits, - and _ only");
            }
            write(&client, &[set(found.id, "title", json!(title))])?;
        }
        TemplateCommand::Delete { name } => {
            let all = templates(&client)?;
            let found = find(&all, &name)?;
            write(
                &client,
                &[Change::Delete {
                    object: found.id,
                    hlc: placeholder(),
                }],
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests may unwrap")]

    use super::*;

    /// ADR 0093: `daily@20:00` in a template, and back in words.
    #[test]
    fn a_template_reminder_can_be_daily() {
        let r = reminder("daily@20:00").unwrap();
        assert_eq!(r.anchor, ReminderAnchor::Daily);
        assert_eq!(r.time, Some(jiff::civil::time(20, 0, 0, 0)));
        assert_eq!(reminders_text(&[r]), "daily@20:00");
        assert!(reminder("daily").is_err());
        assert!(reminder("due@20:00").is_err());
        assert!(reminder("daily@abends").is_err());
        assert_eq!(reminder("due:1h").unwrap().time, None);
    }
}
