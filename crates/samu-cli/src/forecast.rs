// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `samu forecast` (GET /forecast) and `samu settings`: Forecast as the
//! server computes it, and the account's settings object (ADR 0045), shown
//! and set over `POST /changes` like any object.

use anyhow::{Result, bail};
use clap::Subcommand;
use samu_api::{
    AccountSettings, AnyObject, Change, ChangesPage, ForecastKind, ForecastSection, ForecastWhy, Id,
};
use serde_json::{Value, json};

use crate::config::Config;
use crate::output::{Format, emit};
use crate::{placeholder, scopes, short, signed_in, write};

/// Forecast, section by section, a row per task with why it is there.
pub(crate) fn run(
    config: &Config,
    format: Format,
    scope: Option<&str>,
    no_scope: bool,
) -> Result<()> {
    let client = signed_in(config)?;
    let path = match scopes::query_scope(&client, config, scope, no_scope)? {
        Some(id) => format!("/forecast?scope={id}"),
        None => "/forecast".to_owned(),
    };
    let sections: Vec<ForecastSection> =
        serde_json::from_value(client.call("GET", &path, None)?.json()?)?;
    // As JSON a row carries what `list` shows of a task and the whole of an
    // appointment, for a client that draws its own view (the DMS widget,
    // ADR 0083); the other formats show only the columns.
    let whole = matches!(format, Format::Json | Format::Jsonl);
    let (names, calendars) = if whole {
        let statuses: Vec<samu_api::TaskStatus> = sections
            .iter()
            .flat_map(|s| s.rows.iter().map(|r| r.status.clone()))
            .collect();
        let page: ChangesPage =
            serde_json::from_value(client.call("GET", "/changes?after=0", None)?.json()?)?;
        let calendars: std::collections::HashMap<Id, String> = crate::calendar::calendars(&client)?
            .into_iter()
            .map(|c| (c.id, c.title))
            .collect();
        (Some(crate::Names::new(&page, &statuses)), calendars)
    } else {
        (None, std::collections::HashMap::new())
    };
    let zone = jiff::tz::TimeZone::system();
    let mut records = Vec::new();
    for section in &sections {
        let name = match section.section {
            ForecastKind::Overdue => "überfällig".to_owned(),
            ForecastKind::Day => section.date.map(|d| d.to_string()).unwrap_or_default(),
            ForecastKind::Flagged => "markiert".to_owned(),
            ForecastKind::Later => "später".to_owned(),
            ForecastKind::Unknown => "?".to_owned(),
        };
        let kind = serde_json::to_value(section.section).unwrap_or(Value::Null);
        let day = section.date.map(|d| d.to_string());
        // The appointments first, as the views show them (ADR 0047 §10).
        for appointment in &section.appointments {
            let date = match &appointment.start {
                samu_api::When::At(at) => at
                    .with_time_zone(zone.clone())
                    .strftime("%H:%M")
                    .to_string(),
                _ => "ganztägig".to_owned(),
            };
            let mut record = json!({
                "section": name,
                "why": "termin",
                "date": date,
                "id": appointment.appointment,
                "short": short(&appointment.appointment),
                "title": appointment.title,
                "location": appointment.location,
                "link": appointment.link,
            });
            if whole && let Some(object) = record.as_object_mut() {
                object.insert("kind".into(), json!("appointment"));
                object.insert("section_kind".into(), kind.clone());
                object.insert("day".into(), json!(day));
                object.insert(
                    "start".into(),
                    json!(crate::calendar::when(Some(&appointment.start), &zone)),
                );
                object.insert(
                    "end".into(),
                    json!(crate::calendar::when(appointment.end.as_ref(), &zone)),
                );
                object.insert(
                    "all_day".into(),
                    json!(matches!(appointment.start, samu_api::When::Date(_))),
                );
                object.insert(
                    "calendar".into(),
                    json!(calendars.get(&appointment.calendar)),
                );
            }
            records.push(record);
        }
        for row in &section.rows {
            let why = match row.why {
                ForecastWhy::Due => "fällig",
                ForecastWhy::Planned => "geplant",
                ForecastWhy::Defer => "verfügbar ab",
                ForecastWhy::Tag => "Tag",
                ForecastWhy::Done => "erledigt",
                ForecastWhy::Flagged => "markiert",
                ForecastWhy::Unknown => "?",
            };
            let task = &row.status.task;
            let mut record = json!({
                "section": name,
                "why": why,
                "date": row.date.map(|d| d.to_string()).unwrap_or_default(),
                "id": task.id,
                "short": short(&task.id),
                "title": task.title,
            });
            if let (Some(names), Some(object)) = (&names, record.as_object_mut()) {
                let listed = crate::status_record(&row.status, names);
                for key in [
                    "flag",
                    "priority",
                    "due",
                    "due_text",
                    "project",
                    "tags",
                    "state",
                    "today",
                    "available",
                ] {
                    object.insert(key.into(), listed[key].clone());
                }
                object.insert("kind".into(), json!("task"));
                object.insert("section_kind".into(), kind.clone());
                object.insert("day".into(), json!(day));
            }
            records.push(record);
        }
    }
    emit(
        format,
        &records,
        &["section", "short", "why", "date", "title"],
    )
}

#[derive(Debug, Subcommand)]
pub(crate) enum SettingsCommand {
    /// Set one setting: due-soon (P2D), remind-time (09:00), forecast-days
    /// (1..366), forecast-flagged, forecast-defer, forecast-done,
    /// forecast-events (on|off), forecast-tag (a tag by title or path, or
    /// none), default-calendar (a calendar by title or id, or none; ADR
    /// 0068), keys (the web's key bindings as one JSON object, action id
    /// to binding or null; `{}` is the defaults; ADR 0079), report-worked
    /// (events or fields: what the report counts as worked on; ADR 0080),
    /// hit-children (open or folded: how a group that a search finds
    /// starts when its children are no hits; ADR 0021).
    Set { key: String, value: String },
    /// Where a property's value leads (ADR 0054 §3): without arguments the
    /// templates, with a key its template, with a template (containing
    /// `{value}`) sets it, `--unset` removes it.
    Link {
        key: Option<String>,
        template: Option<String>,
        #[arg(long)]
        unset: bool,
    },
}

pub(crate) fn settings_of(
    client: &crate::client::Client,
) -> Result<(AccountSettings, Vec<samu_api::Tag>)> {
    let page: ChangesPage =
        serde_json::from_value(client.call("GET", "/changes?after=0", None)?.json()?)?;
    let mut settings = None;
    let mut tags = Vec::new();
    for object in page.objects {
        match object {
            AnyObject::Settings(s) => settings = Some(s),
            AnyObject::Tag(t) => tags.push(t),
            _ => {}
        }
    }
    let Some(settings) = settings else {
        bail!("the server has no settings object; it is older than samu-api 0.1.15");
    };
    Ok((settings, tags))
}

fn tag_path(tags: &[samu_api::Tag], id: Id) -> String {
    let mut parts = Vec::new();
    let mut at = Some(id);
    while let Some(here) = at {
        let Some(tag) = tags.iter().find(|t| t.id == here) else {
            break;
        };
        if parts.len() > 32 {
            break;
        }
        parts.push(tag.title.clone());
        at = tag.parent;
    }
    parts.reverse();
    parts.join("/")
}

fn on_off(value: &str) -> Result<bool> {
    match value {
        "on" | "an" | "true" | "ja" => Ok(true),
        "off" | "aus" | "false" | "nein" => Ok(false),
        _ => bail!("{value:?}: on or off"),
    }
}

/// `samu settings`: show them; `samu settings set <key> <value>`.
pub(crate) fn settings(
    config: &Config,
    format: Format,
    command: Option<SettingsCommand>,
) -> Result<()> {
    if let Some(SettingsCommand::Link {
        key,
        template,
        unset,
    }) = command
    {
        return crate::props::link(config, format, key, template, unset);
    }
    let client = signed_in(config)?;
    let (settings, tags) = settings_of(&client)?;
    let Some(SettingsCommand::Set { key, value }) = command else {
        let tag = settings
            .forecast_tag
            .map(|t| tag_path(&tags, t))
            .unwrap_or_default();
        let on = |b: bool| if b { "on" } else { "off" };
        let records = [
            ("due-soon", settings.due_soon.0.to_string()),
            (
                "remind-time",
                settings.remind_time.strftime("%H:%M").to_string(),
            ),
            ("forecast-days", settings.forecast_days.to_string()),
            ("forecast-flagged", on(settings.forecast_flagged).to_owned()),
            ("forecast-tag", tag),
            ("forecast-defer", on(settings.forecast_defer).to_owned()),
            ("forecast-done", on(settings.forecast_done).to_owned()),
            ("forecast-events", on(settings.forecast_events).to_owned()),
            (
                "default-calendar",
                settings
                    .default_calendar
                    .map(|c| c.to_string())
                    .unwrap_or_default(),
            ),
            ("keys", serde_json::to_string(&settings.keys)?),
            (
                "report-worked",
                match settings.report_worked {
                    Some(samu_api::ReportWorked::Fields) => "fields",
                    Some(samu_api::ReportWorked::Unknown) => "unknown",
                    Some(samu_api::ReportWorked::Events) | None => "events",
                }
                .to_owned(),
            ),
            (
                "hit-children",
                match settings.hit_children {
                    Some(samu_api::HitChildren::Folded) => "folded",
                    Some(samu_api::HitChildren::Unknown) => "unknown",
                    Some(samu_api::HitChildren::Open) | None => "open",
                }
                .to_owned(),
            ),
        ]
        .into_iter()
        .map(|(k, v)| json!({"key": k, "value": v}))
        .collect::<Vec<_>>();
        return emit(format, &records, &["key", "value"]);
    };
    let (field, json): (&str, Value) = match key.as_str() {
        "due-soon" => {
            let span: jiff::Span = value
                .parse()
                .map_err(|e| anyhow::anyhow!("{value:?}: {e}"))?;
            ("due_soon", json!(span.to_string()))
        }
        "remind-time" => {
            let full = if value.len() == 5 {
                format!("{value}:00")
            } else {
                value.clone()
            };
            let time: jiff::civil::Time = full
                .parse()
                .map_err(|e| anyhow::anyhow!("{value:?}: {e}"))?;
            ("remind_time", json!(time))
        }
        "forecast-days" => {
            let days: u32 = value
                .parse()
                .map_err(|_| anyhow::anyhow!("{value:?}: a number of days"))?;
            ("forecast_days", json!(days))
        }
        "forecast-flagged" => ("forecast_flagged", json!(on_off(&value)?)),
        "forecast-defer" => ("forecast_defer", json!(on_off(&value)?)),
        "forecast-done" => ("forecast_done", json!(on_off(&value)?)),
        "forecast-events" => ("forecast_events", json!(on_off(&value)?)),
        "forecast-tag" if value == "none" || value.is_empty() => ("forecast_tag", Value::Null),
        "forecast-tag" => {
            let want = value.to_lowercase();
            let by_path: Vec<Id> = tags
                .iter()
                .filter(|t| tag_path(&tags, t.id).to_lowercase() == want)
                .map(|t| t.id)
                .collect();
            let found = if by_path.is_empty() {
                tags.iter()
                    .filter(|t| t.title.to_lowercase() == want)
                    .map(|t| t.id)
                    .collect()
            } else {
                by_path
            };
            match found.as_slice() {
                [one] => ("forecast_tag", json!(one)),
                [] => return Err(samu_client::Error::NotFound(format!("no tag {value:?}")).into()),
                _ => bail!("{value:?} names more than one tag; give the path"),
            }
        }
        // ADR 0068 §10: by title, id or the start or end of one.
        "default-calendar" if value == "none" || value.is_empty() => {
            ("default_calendar", Value::Null)
        }
        "default-calendar" => {
            let page: ChangesPage =
                serde_json::from_value(client.call("GET", "/changes?after=0", None)?.json()?)?;
            let want = value.to_lowercase();
            let found: Vec<Id> = page
                .objects
                .iter()
                .filter_map(|o| match o {
                    AnyObject::Calendar(c)
                        if c.title.to_lowercase() == want || crate::id_matches(&c.id, &want) =>
                    {
                        Some(c.id)
                    }
                    _ => None,
                })
                .collect();
            match found.as_slice() {
                [one] => ("default_calendar", json!(one)),
                [] => {
                    return Err(
                        samu_client::Error::NotFound(format!("no calendar {value:?}")).into(),
                    );
                }
                _ => bail!("{value:?} names more than one calendar; give its id"),
            }
        }
        // ADR 0079 §2: the differences from the defaults, as a whole.
        "keys" => {
            let keys: std::collections::BTreeMap<String, Option<String>> =
                serde_json::from_str(&value).map_err(|e| {
                    anyhow::anyhow!("{value:?}: a JSON object of action id to key or null: {e}")
                })?;
            ("keys", json!(keys))
        }
        // ADR 0080 §1.
        "report-worked" => match value.as_str() {
            "events" | "fields" => ("report_worked", json!(value)),
            _ => bail!("report-worked is events or fields, not {value:?}"),
        },
        // ADR 0021 §6.
        "hit-children" => match value.as_str() {
            "open" | "folded" => ("hit_children", json!(value)),
            _ => bail!("hit-children is open or folded, not {value:?}"),
        },
        _ => bail!("no setting {key:?}; see samu settings set --help"),
    };
    write(
        &client,
        &[Change::Set {
            object: settings.id,
            field: field.into(),
            value: json,
            note: None,
            hlc: placeholder(),
        }],
    )?;
    Ok(())
}
