// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `samu remind` and `samu reminders` (ADR 0030): reminders are objects of
//! their own, made and listed over the ordinary routes. When they go off is
//! computed by samu-core on the phone; this lists what is set.

use anyhow::{Result, bail};
use clap::{Args, ValueEnum};
use samu_api::{AnyObject, Change, ChangesPage, Id, ObjectKind, Reminder, ReminderAnchor};
use samu_client::Client;
use serde_json::{Value, json};

/// What a reminder is relative to, on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Of {
    Due,
    Planned,
    Defer,
}

#[derive(Debug, Args)]
pub(crate) struct Remind {
    /// The task or project.
    pub(crate) id: String,
    /// At an instant: RFC 3339 (`2026-11-20T07:30:00Z`), or a local time
    /// (`2026-11-20T08:30`) in this computer's zone.
    #[arg(long, conflicts_with_all = ["before", "of", "daily"])]
    pub(crate) at: Option<String>,
    /// Every day at this time (`20:00`) in the account's zone; a habit only
    /// while the day's goal is open (ADR 0093).
    #[arg(long, conflicts_with_all = ["before", "of"])]
    pub(crate) daily: Option<String>,
    /// This long before the date, like `2h`, `1d` or `PT30M`.
    #[arg(long)]
    pub(crate) before: Option<String>,
    /// The date it is relative to.
    #[arg(long, value_enum, default_value_t = Of::Due)]
    pub(crate) of: Of,
}

/// The new reminder as `create` carries it.
pub(crate) fn reminder(owner: Id, remind: &Remind) -> Result<Reminder> {
    let now = jiff::Timestamp::now();
    if let Some(text) = &remind.daily {
        return Ok(Reminder {
            id: Id::new(),
            created: now,
            modified: now,
            owner,
            anchor: ReminderAnchor::Daily,
            at: None,
            before: None,
            time: Some(time_of_day(text)?),
        });
    }
    let (anchor, at) = match &remind.at {
        Some(text) => (ReminderAnchor::At, Some(instant(text)?)),
        None => (
            match remind.of {
                Of::Due => ReminderAnchor::Due,
                Of::Planned => ReminderAnchor::Planned,
                Of::Defer => ReminderAnchor::Defer,
            },
            None,
        ),
    };
    let before = match &remind.before {
        Some(text) => Some(samu_api::Period(text.parse().map_err(|e| {
            anyhow::anyhow!("--before {text:?} is not a length of time: {e}")
        })?)),
        None => None,
    };
    Ok(Reminder {
        id: Id::new(),
        created: now,
        modified: now,
        owner,
        anchor,
        at,
        before,
        time: None,
    })
}

/// A time of day: `20:00` or `20:00:00`.
pub(crate) fn time_of_day(text: &str) -> Result<jiff::civil::Time> {
    text.trim()
        .parse()
        .map_err(|e| anyhow::anyhow!("{text:?} is not a time of day like 20:00: {e}"))
}

/// An instant: RFC 3339, or a local date and time in this computer's zone.
pub(crate) fn instant(text: &str) -> Result<jiff::Timestamp> {
    if let Ok(at) = text.parse::<jiff::Timestamp>() {
        return Ok(at);
    }
    match text.parse::<jiff::civil::DateTime>() {
        Ok(local) => Ok(local.to_zoned(jiff::tz::TimeZone::system())?.timestamp()),
        Err(e) => bail!("{text:?} is neither RFC 3339 nor a local date and time: {e}"),
    }
}

/// The change that makes it.
pub(crate) fn create(reminder: &Reminder, hlc: samu_api::Hlc) -> Result<Change> {
    Ok(Change::Create {
        object: reminder.id,
        kind: ObjectKind::Reminder,
        value: serde_json::to_value(reminder)?,
        hlc,
    })
}

/// Every reminder, or those of one owner, with the owner's title.
pub(crate) fn list(client: &Client, owner: Option<Id>) -> Result<Vec<Value>> {
    let page: ChangesPage =
        serde_json::from_value(client.call("GET", "/changes?after=0", None)?.json()?)?;
    Ok(records_in(&page.objects, owner))
}

/// The reminders among `objects`, all or those of one owner, as
/// `samu reminders` shows them.
pub(crate) fn records_in(objects: &[AnyObject], owner: Option<Id>) -> Vec<Value> {
    let title = |id: Id| -> Option<String> {
        objects.iter().find_map(|o| match o {
            AnyObject::Task(t) if t.id == id => Some(t.title.clone()),
            AnyObject::Project(p) if p.id == id => Some(p.title.clone()),
            _ => None,
        })
    };
    let mut records = Vec::new();
    for object in objects {
        let AnyObject::Reminder(r) = object else {
            continue;
        };
        if owner.is_some_and(|o| o != r.owner) {
            continue;
        }
        records.push(record(r, title(r.owner)));
    }
    records
}

/// One reminder for the output; `when` says it in a few words.
pub(crate) fn record(r: &Reminder, owner_title: Option<String>) -> Value {
    let anchor = serde_json::to_value(r.anchor).unwrap_or(Value::Null);
    let when = match (r.anchor, r.at, r.before) {
        (ReminderAnchor::At, Some(at), _) => at.to_string(),
        (ReminderAnchor::Daily, _, _) => match r.time {
            Some(time) => format!("daily {}", time.strftime("%H:%M")),
            None => "daily".into(),
        },
        (_, _, Some(before)) => format!("{} before {}", before.0, cell(&anchor)),
        (_, _, None) => format!("at {}", cell(&anchor)),
    };
    json!({
        "id": r.id,
        "short": crate::short(&r.id),
        "owner": r.owner,
        "title": owner_title,
        "anchor": anchor,
        "at": r.at.map(|t| t.to_string()),
        "before": r.before.map(|p| p.0.to_string()),
        "time": r.time.map(|t| t.strftime("%H:%M").to_string()),
        "when": when,
    })
}

fn cell(value: &Value) -> String {
    value.as_str().unwrap_or("unknown").to_string()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests may unwrap")]

    use super::*;

    fn remind(at: Option<&str>, before: Option<&str>, of: Of) -> Remind {
        Remind {
            id: String::new(),
            at: at.map(str::to_string),
            before: before.map(str::to_string),
            daily: None,
            of,
        }
    }

    #[test]
    fn the_arguments_make_the_reminder() {
        let owner = Id::new();
        let r = reminder(owner, &remind(None, Some("2h"), Of::Due)).unwrap();
        assert_eq!(
            (r.owner, r.anchor, r.at),
            (owner, ReminderAnchor::Due, None)
        );
        assert_eq!(r.before.unwrap().0.get_hours(), 2);

        let r = reminder(owner, &remind(None, None, Of::Planned)).unwrap();
        assert_eq!((r.anchor, r.before), (ReminderAnchor::Planned, None));

        let r = reminder(owner, &remind(Some("2026-11-20T07:30:00Z"), None, Of::Due)).unwrap();
        assert_eq!(r.anchor, ReminderAnchor::At);
        assert_eq!(r.at.unwrap().to_string(), "2026-11-20T07:30:00Z");

        assert!(reminder(owner, &remind(Some("morgen"), None, Of::Due)).is_err());
        assert!(reminder(owner, &remind(None, Some("bald"), Of::Due)).is_err());
    }

    #[test]
    fn a_record_says_when_in_words() {
        let owner = Id::new();
        let r = reminder(owner, &remind(None, Some("PT30M"), Of::Defer)).unwrap();
        assert_eq!(record(&r, None)["when"], "PT30M before defer");
        let r = reminder(owner, &remind(None, None, Of::Due)).unwrap();
        assert_eq!(record(&r, None)["when"], "at due");
    }

    /// ADR 0093: `--daily 20:00` is the anchor `daily` with the time.
    #[test]
    fn daily_takes_a_time_of_day() {
        let owner = Id::new();
        let mut args = remind(None, None, Of::Due);
        args.daily = Some("20:00".into());
        let r = reminder(owner, &args).unwrap();
        assert_eq!(r.anchor, ReminderAnchor::Daily);
        assert_eq!(r.time, Some(jiff::civil::time(20, 0, 0, 0)));
        assert_eq!((r.at, r.before), (None, None));
        assert_eq!(record(&r, None)["when"], "daily 20:00");
        assert_eq!(record(&r, None)["time"], "20:00");
        args.daily = Some("abends".into());
        assert!(reminder(owner, &args).is_err());
    }
}
