// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Properties beyond `samu set` (ADR 0054): `samu prop` shows a task's own
//! and inherited ones, `samu open` opens a value with this device's command
//! or where it leads, `samu open-uri` is the handler for `samu-open:`, and
//! `samu settings link` sets the account's templates.

use std::collections::BTreeMap;

use anyhow::{Context, Result, anyhow, bail};
use samu_api::{AnyObject, Change, ChangesPage, Id, Inherited, TaskStatus};
use serde_json::{Value, json};

use crate::client::Client;
use crate::config::Config;
use crate::output::{Format, emit};
use crate::{forecast, placeholder, resolve, short, signed_in, write};
use samu_client::Error as Failure;

/// The scheme the web hands a click to (ADR 0054 §6).
pub(crate) const SCHEME: &str = "samu-open";

/// Keys samu writes itself; shown last (ADR 0054 §2).
const SYSTEM: &[&str] = &["org_id", "org_origin", "source"];

/// Whether a value is a URI by itself; the rule of `samu-core`
/// (`properties::is_uri`), which the CLI does not depend on.
fn is_uri(value: &str) -> bool {
    let Some((scheme, rest)) = value.split_once(':') else {
        return false;
    };
    let mut bytes = scheme.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_alphabetic())
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
        && !rest.is_empty()
        && !value.chars().any(char::is_whitespace)
}

fn encode(text: &str) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// Where a value leads (ADR 0054 §4), as `samu-core` decides it.
fn link_for(value: &str, template: Option<&str>) -> Option<String> {
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

/// The account's link templates by key (ADR 0054 §4), from all of its
/// objects. A session with a grant does not see the settings: its values
/// lead only where they are URIs themselves.
pub(crate) fn templates_in(objects: &[AnyObject]) -> BTreeMap<String, String> {
    objects
        .iter()
        .find_map(|o| match o {
            AnyObject::Settings(s) => Some(s.property_links.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

fn templates(client: &Client) -> Result<BTreeMap<String, String>> {
    let page: ChangesPage =
        serde_json::from_value(client.call("GET", "/changes?after=0", None)?.json()?)?;
    Ok(templates_in(&page.objects))
}

/// One property of an object: its own or inherited.
struct Row {
    key: String,
    value: String,
    from: Option<Id>,
}

/// The own properties of a task or project and, for a task, the inherited
/// ones as the server computed them.
fn rows_of(client: &Client, id: Id) -> Result<Vec<Row>> {
    let object: AnyObject = serde_json::from_value(
        client
            .call("GET", &format!("/objects/{id}"), None)?
            .json()?,
    )?;
    let statuses: Vec<TaskStatus> = if matches!(object, AnyObject::Task(_)) {
        serde_json::from_value(client.call("GET", "/available?all=true", None)?.json()?)?
    } else {
        Vec::new()
    };
    rows(&object, &statuses)
}

/// The rows of a task or project; `statuses` hold what a task inherits.
fn rows(object: &AnyObject, statuses: &[TaskStatus]) -> Result<Vec<Row>> {
    let (own, inherited): (BTreeMap<String, String>, BTreeMap<String, Inherited>) = match object {
        AnyObject::Project(p) => (p.properties.clone(), BTreeMap::new()),
        AnyObject::Task(t) => {
            let inherited = statuses
                .iter()
                .find(|s| s.task.id == t.id)
                .map(|s| s.inherited.clone())
                .unwrap_or_default();
            (t.properties.clone(), inherited)
        }
        other => bail!(
            "{} is neither a task nor a project",
            other.id().map(|id| id.to_string()).unwrap_or_default()
        ),
    };
    let mut rows: Vec<Row> = own
        .into_iter()
        .map(|(key, value)| Row {
            key,
            value,
            from: None,
        })
        .chain(inherited.into_iter().map(|(key, i)| Row {
            key,
            value: i.value,
            from: Some(i.from),
        }))
        .collect();
    rows.sort_by(|a, b| {
        (SYSTEM.contains(&a.key.as_str()), &a.key).cmp(&(SYSTEM.contains(&b.key.as_str()), &b.key))
    });
    Ok(rows)
}

/// `samu prop <id>`: the properties of a task or project, own and inherited.
pub(crate) fn show(config: &Config, format: Format, id: &str) -> Result<()> {
    let client = signed_in(config)?;
    let id = resolve(&client, id)?;
    let records = records(rows_of(&client, id)?, &templates(&client)?);
    emit(format, &records, &["key", "value", "from", "link"])
}

/// The properties of `object` as `samu prop` shows them, from what the
/// caller already holds.
pub(crate) fn records_of(
    object: &AnyObject,
    statuses: &[TaskStatus],
    templates: &BTreeMap<String, String>,
) -> Result<Vec<Value>> {
    Ok(records(rows(object, statuses)?, templates))
}

fn records(rows: Vec<Row>, templates: &BTreeMap<String, String>) -> Vec<Value> {
    rows.into_iter()
        .map(|row| {
            let link = link_for(&row.value, templates.get(&row.key).map(String::as_str));
            json!({
                "key": row.key,
                "value": row.value,
                "from": row.from.map(|f| short(&f)),
                "link": link,
            })
        })
        .collect()
}

/// What opening a value runs: this device's command for the key, else
/// `xdg-open` on where the value leads.
fn command_for(
    config: &Config,
    key: &str,
    value: &str,
    template: Option<&str>,
) -> Option<Vec<String>> {
    if let Some(argv) = config.open.get(key).filter(|a| !a.is_empty()) {
        return Some(argv.iter().map(|a| a.replace("{value}", value)).collect());
    }
    link_for(value, template).map(|target| vec!["xdg-open".to_owned(), target])
}

/// `samu open <id> <key>` and the handler: finds the value, own or
/// inherited, and runs what opens it. With `print` it only says what.
pub(crate) fn open(
    config: &Config,
    format: Format,
    id: &str,
    key: &str,
    print: bool,
) -> Result<()> {
    let client = signed_in(config)?;
    let id = resolve(&client, id)?;
    let row = rows_of(&client, id)?
        .into_iter()
        .find(|r| r.key == key)
        .ok_or_else(|| Failure::NotFound(format!("{} has no property {key:?}", short(&id))))?;
    let templates = templates(&client)?;
    let template = templates.get(key).map(String::as_str);
    let Some(argv) = command_for(config, key, &row.value, template) else {
        return Err(Failure::NotFound(format!(
            "{key} = {:?} leads nowhere: no command for {key} in the config file, \
             no template (samu settings link {key} …), and the value is no URI",
            row.value
        ))
        .into());
    };
    if print {
        return emit(
            format,
            &[json!({"key": key, "value": row.value, "command": argv})],
            &["key", "value", "command"],
        );
    }
    let status = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .status()
        .with_context(|| format!("running {}", argv[0]))?;
    if !status.success() {
        bail!("{} ended with {status}", argv[0]);
    }
    Ok(())
}

/// The object of a `samu:<uuid>` address (ADR 0075 §2); none for another
/// scheme.
fn object_uri(uri: &str) -> Option<Result<Id>> {
    let (scheme, rest) = uri.trim().split_once(':')?;
    if !scheme.eq_ignore_ascii_case(samu_api::URI_SCHEME) {
        return None;
    }
    let id = rest.trim_start_matches('/').trim_end_matches('/');
    Some(
        id.parse()
            .map_err(|_| anyhow!("{uri:?}: {id:?} is no id ({}:<uuid>)", samu_api::URI_SCHEME)),
    )
}

/// `samu open-uri samu:<uuid>`: the object in the web UI, `<web>/o/<uuid>`,
/// with `xdg-open`. Needs no session: the page signs in itself.
fn open_object(config: &Config, format: Format, id: Id, print: bool) -> Result<()> {
    let target = format!("{}/o/{id}", config.web()?.trim_end_matches('/'));
    let argv = ["xdg-open".to_owned(), target];
    if print {
        return emit(
            format,
            &[json!({"id": id.to_string(), "command": argv})],
            &["id", "command"],
        );
    }
    let status = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .status()
        .with_context(|| format!("running {}", argv[0]))?;
    if !status.success() {
        bail!("{} ended with {status}", argv[0]);
    }
    Ok(())
}

/// The object and key of a `samu-open:<uuid>/<key>` address. It carries no
/// value: the handler reads it from the server (ADR 0054 §6).
fn parse_uri(uri: &str) -> Result<(Id, String)> {
    let rest = uri
        .strip_prefix(SCHEME)
        .and_then(|r| r.strip_prefix(':'))
        .ok_or_else(|| anyhow!("{uri:?} is no {SCHEME}: address"))?;
    let rest = rest.trim_start_matches('/');
    let (id, key) = rest
        .split_once('/')
        .ok_or_else(|| anyhow!("{uri:?}: {SCHEME}:<uuid>/<key>"))?;
    let id: Id = id
        .parse()
        .map_err(|_| anyhow!("{uri:?}: {id:?} is no id"))?;
    let key = key.trim_end_matches('/');
    let valid = key.len() <= 64
        && key.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
    if !valid {
        bail!("{uri:?}: {key:?} is no property key");
    }
    Ok((id, key.to_owned()))
}

/// `samu open-uri samu-open:<uuid>/<key>`. Started by the browser there
/// is no terminal to read a failure on; with `notify` it also goes to the
/// desktop's notifications.
pub(crate) fn open_uri(
    config: &Config,
    format: Format,
    uri: &str,
    print: bool,
    notify: bool,
) -> Result<()> {
    let result = match object_uri(uri) {
        Some(id) => id.and_then(|id| open_object(config, format, id, print)),
        None => {
            parse_uri(uri).and_then(|(id, key)| open(config, format, &id.to_string(), &key, print))
        }
    };
    if let Err(error) = &result
        && notify
    {
        let _ = std::process::Command::new("notify-send")
            .args(["samudo", &format!("{error:#}")])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    result
}

/// `samu settings link [<key> [<template>]] [--unset]`: the templates of the
/// account (ADR 0054 §3).
pub(crate) fn link(
    config: &Config,
    format: Format,
    key: Option<String>,
    template: Option<String>,
    unset: bool,
) -> Result<()> {
    let client = signed_in(config)?;
    let (settings, _) = forecast::settings_of(&client)?;
    let Some(key) = key else {
        let records: Vec<Value> = settings
            .property_links
            .iter()
            .map(|(k, t)| json!({"key": k, "template": t}))
            .collect();
        return emit(format, &records, &["key", "template"]);
    };
    let value = match (template, unset) {
        (Some(_), true) => bail!("a template or --unset, not both"),
        (None, false) => {
            let template = settings
                .property_links
                .get(&key)
                .ok_or_else(|| Failure::NotFound(format!("no template for {key:?}")))?;
            return emit(
                format,
                &[json!({"key": key, "template": template})],
                &["key", "template"],
            );
        }
        (None, true) => Value::Null,
        (Some(t), false) if !t.contains("{value}") => {
            bail!("{t:?} has no {{value}} for the value to go in")
        }
        (Some(t), false) => Value::String(t),
    };
    write(
        &client,
        &[Change::Set {
            object: settings.id,
            field: format!("property_links.{key}"),
            value,
            note: None,
            hlc: placeholder(),
        }],
    )?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests may unwrap")]
mod tests {
    use super::*;

    #[test]
    fn where_a_value_leads_as_in_core() {
        let jira = Some("https://jira.example.org/browse/{value}");
        assert_eq!(
            link_for("HAUS-12", jira).as_deref(),
            Some("https://jira.example.org/browse/HAUS-12")
        );
        assert_eq!(
            link_for("a b/ä", Some("https://x.example/?q={value}")).as_deref(),
            Some("https://x.example/?q=a%20b%2F%C3%A4")
        );
        assert_eq!(
            link_for("mailto:a@example.org", jira).as_deref(),
            Some("mailto:a@example.org")
        );
        assert_eq!(link_for("HAUS-12", None), None);
        assert_eq!(link_for("10:30", None), None);
        assert_eq!(link_for("Notiz: oben", None), None);
    }

    #[test]
    fn a_samu_open_address() {
        let id = "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6c05";
        for uri in [
            format!("samu-open:{id}/jira"),
            format!("samu-open://{id}/jira/"),
        ] {
            let (got, key) = parse_uri(&uri).unwrap();
            assert_eq!((got.to_string().as_str(), key.as_str()), (id, "jira"));
        }
        for bad in [
            "samu:x/jira".to_owned(),
            format!("samu-open:{id}"),
            format!("samu-open:{id}/Jira"),
            "samu-open:nope/jira".to_owned(),
            format!("samu-open:{id}/jira;rm"),
        ] {
            assert!(parse_uri(&bad).is_err(), "{bad}");
        }
    }
}
