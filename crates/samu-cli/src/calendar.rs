//! `samu calendar …` and `samu appointments` (ADR 0047, 0048): calendars
//! are the user's objects over `POST /changes`; the source goes to
//! `PUT /calendars/{id}/source` and never comes back; the occurrences of
//! some days are `GET /appointments`, unfolded by the server. CalDAV
//! calendars are found with `POST /calendars/discover`, more from the
//! server of one already there with `POST /calendars/{id}/discover`
//! (ADR 0100 §1).

use anyhow::{Result, bail};
use clap::{Subcommand, ValueEnum};
use samu_api::{
    Alert, AnyObject, AppointmentInstance, AppointmentStatus, Calendar, CalendarImport,
    CalendarKind, CalendarSource, Change, ChangesPage, DeleteRequest, DiscoveredCalendar, Id,
    ObjectKind, When,
};
use samu_client::{Client, Error as Failure};
use serde_json::{Value, json};

use crate::config::Config;
use crate::output::{Format, emit};
use crate::{delete_object, password, placeholder, scopes, short, signed_in, write};

#[derive(Debug, Subcommand)]
pub(crate) enum CalendarCommand {
    /// The calendars, with when they last changed and what went wrong.
    List,
    /// Add a calendar; with `--url` its source is set and it is fetched.
    Add {
        title: String,
        /// `https://…` or `webcal://…`; a secret address is fine, it stays
        /// on the server.
        #[arg(long)]
        url: Option<String>,
        #[arg(long, value_enum, default_value_t = Kind::Ics)]
        kind: Kind,
        #[arg(long)]
        username: Option<String>,
        /// Read the password from the first line of stdin instead of asking.
        #[arg(long)]
        password_stdin: bool,
        /// User and password from this CalDAV calendar on the same host;
        /// they stay on the server (ADR 0100 §2).
        #[arg(long, value_name = "CALENDAR", conflicts_with_all = ["username", "password_stdin"])]
        from: Option<String>,
        /// A CSS colour, like `#3b82f6`.
        #[arg(long)]
        color: Option<String>,
        /// Minutes between two fetches, at least 5.
        #[arg(long, default_value_t = 15)]
        refresh: u32,
        /// Where „open in the calendar“ leads without a URL of its own:
        /// `{uid}`, `{date}`, `{y}`, `{m}`, `{d}` are filled in.
        #[arg(long)]
        link_template: Option<String>,
    },
    /// Change one field: title, color, enabled, refresh, link_template,
    /// position, kind. Without a value an optional field is cleared.
    Set {
        calendar: String,
        field: String,
        value: Option<String>,
    },
    /// Set where a calendar comes from (PUT /calendars/{id}/source).
    Source {
        calendar: String,
        url: String,
        #[arg(long)]
        username: Option<String>,
        #[arg(long)]
        password_stdin: bool,
    },
    /// Find the calendars at a CalDAV address (POST /calendars/discover):
    /// the bare host, the principal or a collection; or with `--from` the
    /// others on the server of a calendar already there, with its stored
    /// user and password (POST /calendars/{id}/discover). Makes nothing
    /// unless `--add` is given.
    Discover {
        #[arg(required_unless_present = "from", conflicts_with = "from")]
        url: Option<String>,
        #[arg(long)]
        username: Option<String>,
        #[arg(long)]
        password_stdin: bool,
        /// A CalDAV calendar already there: find next to it, and add with
        /// its user and password, which stay on the server (ADR 0100 §1–2).
        #[arg(long, value_name = "CALENDAR", conflicts_with_all = ["username", "password_stdin"])]
        from: Option<String>,
        /// Add the calendars found, each with its source and fetched; with
        /// titles only those. Without titles, those subscribed already are
        /// left out.
        #[arg(long, num_args = 0.., value_name = "TITLE")]
        add: Option<Vec<String>>,
        /// The colour of the calendars added, instead of the server's.
        #[arg(long, requires = "add")]
        color: Option<String>,
    },
    /// Import an iCalendar file into a writable CalDAV calendar
    /// (POST /calendars/{id}/import, ADR 0100 §4): each UID goes to the
    /// CalDAV server as it is in the file; one the calendar holds is
    /// replaced. What was left out or failed goes to stderr; exit 1 when
    /// anything failed.
    Import {
        calendar: String,
        /// The file, or `-` for stdin.
        file: String,
    },
    /// Fetch now (POST /calendars/{id}/refresh). Exit 1 when the calendar
    /// reports an error afterwards.
    Refresh { calendar: String },
    /// Delete a calendar and its appointments.
    Delete { calendar: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Kind {
    Ics,
    Caldav,
}

impl Kind {
    fn wire(self) -> CalendarKind {
        match self {
            Self::Ics => CalendarKind::Ics,
            Self::Caldav => CalendarKind::Caldav,
        }
    }
}

pub(crate) fn calendars(client: &Client) -> Result<Vec<Calendar>> {
    let page: ChangesPage =
        serde_json::from_value(client.call("GET", "/changes?after=0", None)?.json()?)?;
    let mut out: Vec<Calendar> = page
        .objects
        .into_iter()
        .filter_map(|o| match o {
            AnyObject::Calendar(c) => Some(c),
            _ => None,
        })
        .collect();
    out.sort_by(|a, b| a.position.cmp(&b.position).then(a.title.cmp(&b.title)));
    Ok(out)
}

/// A calendar by id, the end of one, or its title.
fn resolve(client: &Client, reference: &str) -> Result<Calendar> {
    let all = calendars(client)?;
    let wanted = reference.to_lowercase();
    let by_title: Vec<&Calendar> = all
        .iter()
        .filter(|c| c.title.to_lowercase() == wanted)
        .collect();
    let found: Vec<&Calendar> = if by_title.is_empty() {
        all.iter()
            .filter(|c| c.id.to_string().ends_with(&wanted))
            .collect()
    } else {
        by_title
    };
    match found.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(Failure::NotFound(format!("no calendar {reference:?}")).into()),
        many => bail!(
            "{reference:?} matches {} calendars, name one by its id: {}",
            many.len(),
            many.iter()
                .map(|c| format!("{} ({})", short(&c.id), c.title))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn kind_name(kind: CalendarKind) -> &'static str {
    match kind {
        CalendarKind::Ics => "ics",
        CalendarKind::Caldav => "caldav",
        CalendarKind::Unknown => "unknown",
    }
}

fn record(c: &Calendar) -> Value {
    json!({
        "id": c.id,
        "short": short(&c.id),
        "title": c.title,
        "kind": kind_name(c.kind),
        "enabled": c.enabled,
        "refresh": c.refresh,
        "color": c.color,
        "link_template": c.link_template,
        "host": c.host,
        "fetched_at": c.fetched_at.map(|t| t.to_string()),
        "error": c.error,
        "writable": c.writable,
    })
}

const COLUMNS: &[&str] = &[
    "short",
    "title",
    "kind",
    "writable",
    "host",
    "fetched_at",
    "error",
];

fn put_source(client: &Client, id: Id, source: &CalendarSource) -> Result<()> {
    client.call(
        "PUT",
        &format!("/calendars/{id}/source"),
        Some(&serde_json::to_value(source)?),
    )?;
    Ok(())
}

fn source(url: String, username: Option<String>, password_stdin: bool) -> Result<CalendarSource> {
    let password = match &username {
        Some(_) => Some(password(password_stdin)?),
        None => None,
    };
    Ok(CalendarSource {
        url,
        username,
        password,
        credentials_from: None,
    })
}

/// A source with user and password of another calendar (ADR 0100 §2).
fn borrowed(url: String, from: Id) -> CalendarSource {
    CalendarSource {
        url,
        username: None,
        password: None,
        credentials_from: Some(from),
    }
}

fn refresh(client: &Client, id: Id) -> Result<Calendar> {
    let answer = client.call("POST", &format!("/calendars/{id}/refresh"), None)?;
    Ok(serde_json::from_value(answer.json()?)?)
}

/// A new calendar, written over `POST /changes`; without a source yet.
fn create(
    client: &Client,
    title: String,
    kind: CalendarKind,
    color: Option<String>,
    minutes: u32,
    link_template: Option<String>,
) -> Result<Calendar> {
    let now = jiff::Timestamp::now();
    let calendar = Calendar {
        id: Id::new(),
        created: now,
        modified: now,
        title,
        kind,
        color,
        enabled: true,
        refresh: minutes,
        link_template,
        position: 0,
        fetched_at: None,
        error: None,
        host: None,
        writable: false,
    };
    write(
        client,
        &[Change::Create {
            object: calendar.id,
            kind: ObjectKind::Calendar,
            value: serde_json::to_value(&calendar)?,
            hlc: placeholder(),
        }],
    )?;
    Ok(calendar)
}

/// The found calendars `--add` names: all not subscribed yet without
/// titles, else each title once, by name regardless of case. A title found
/// nowhere is an error before anything is added.
fn chosen(found: &[DiscoveredCalendar], titles: &[String]) -> Result<Vec<DiscoveredCalendar>> {
    if titles.is_empty() {
        return Ok(found.iter().filter(|f| !f.known).cloned().collect());
    }
    let mut out = Vec::new();
    for title in titles {
        let wanted = title.to_lowercase();
        let Some(one) = found.iter().find(|f| f.title.to_lowercase() == wanted) else {
            bail!(
                "no calendar {title:?} there; found: {}",
                found
                    .iter()
                    .map(|f| f.title.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        };
        if !out.contains(one) {
            out.push(one.clone());
        }
    }
    Ok(out)
}

/// The value of `set` as the field wants it.
fn field_value(field: &str, value: Option<&str>) -> Result<Value> {
    Ok(match (field, value) {
        ("title" | "kind", None) => bail!("{field} needs a value"),
        ("title" | "kind" | "color" | "link_template", Some(v)) => json!(v),
        ("color" | "link_template", None) => Value::Null,
        ("enabled", Some(v)) => json!(match v {
            "true" | "yes" | "on" | "1" => true,
            "false" | "no" | "off" | "0" => false,
            _ => bail!("enabled: true or false, not {v:?}"),
        }),
        ("refresh" | "position", Some(v)) => json!(
            v.parse::<i64>()
                .map_err(|_| anyhow::anyhow!("{field}: a number, not {v:?}"))?
        ),
        ("enabled" | "refresh" | "position", None) => bail!("{field} needs a value"),
        _ => bail!("{field}: one of title, color, enabled, refresh, link_template, position, kind"),
    })
}

pub(crate) fn run(config: &Config, format: Format, command: CalendarCommand) -> Result<()> {
    let client = signed_in(config)?;
    match command {
        CalendarCommand::List => {
            let records: Vec<Value> = calendars(&client)?.iter().map(record).collect();
            emit(format, &records, COLUMNS)
        }
        CalendarCommand::Add {
            title,
            url,
            kind,
            username,
            password_stdin,
            from,
            color,
            refresh: minutes,
            link_template,
        } => {
            if minutes < 5 {
                bail!("--refresh: at least 5 minutes");
            }
            let source = match (url, from) {
                (Some(url), Some(from)) => Some(borrowed(url, resolve(&client, &from)?.id)),
                (Some(url), None) => Some(source(url, username, password_stdin)?),
                (None, Some(_)) => bail!("--from needs --url"),
                (None, None) => None,
            };
            let calendar = create(&client, title, kind.wire(), color, minutes, link_template)?;
            let shown = match source {
                Some(source) => {
                    put_source(&client, calendar.id, &source)?;
                    refresh(&client, calendar.id)?
                }
                None => calendar,
            };
            if let Some(error) = &shown.error {
                eprintln!("{}: {error}", shown.title);
            }
            emit(format, &[record(&shown)], COLUMNS)
        }
        CalendarCommand::Set {
            calendar,
            field,
            value,
        } => {
            let calendar = resolve(&client, &calendar)?;
            let value = field_value(&field, value.as_deref())?;
            crate::set_field(&client, calendar.id, &field, value)?;
            let after = resolve(&client, &calendar.id.to_string())?;
            emit(format, &[record(&after)], COLUMNS)
        }
        CalendarCommand::Source {
            calendar,
            url,
            username,
            password_stdin,
        } => {
            let calendar = resolve(&client, &calendar)?;
            put_source(
                &client,
                calendar.id,
                &source(url, username, password_stdin)?,
            )?;
            let after = refresh(&client, calendar.id)?;
            if let Some(error) = &after.error {
                eprintln!("{}: {error}", after.title);
            }
            emit(format, &[record(&after)], COLUMNS)
        }
        CalendarCommand::Discover {
            url,
            username,
            password_stdin,
            from,
            add,
            color,
        } => {
            let (found, source, from) = match (url, from) {
                (_, Some(from)) => {
                    let from = resolve(&client, &from)?.id;
                    let answer =
                        client.call("POST", &format!("/calendars/{from}/discover"), None)?;
                    let found: Vec<DiscoveredCalendar> = serde_json::from_value(answer.json()?)?;
                    (found, None, Some(from))
                }
                (Some(url), None) => {
                    let source = source(url, username, password_stdin)?;
                    let answer = client.call(
                        "POST",
                        "/calendars/discover",
                        Some(&serde_json::to_value(&source)?),
                    )?;
                    let found: Vec<DiscoveredCalendar> = serde_json::from_value(answer.json()?)?;
                    (found, Some(source), None)
                }
                (None, None) => bail!("give an address or --from"),
            };
            let Some(titles) = add else {
                let records: Vec<Value> = found
                    .iter()
                    .map(|f| {
                        json!({"title": f.title, "color": f.color, "known": f.known, "url": f.url})
                    })
                    .collect();
                return emit(format, &records, &["title", "color", "known", "url"]);
            };
            let mut records = Vec::new();
            let mut failed = false;
            let picked = chosen(&found, &titles)?;
            if picked.is_empty() {
                eprintln!("every calendar there is subscribed already");
            }
            for one in picked {
                let calendar = create(
                    &client,
                    one.title,
                    CalendarKind::Caldav,
                    color.clone().or(one.color),
                    15,
                    None,
                )?;
                let source = match (&source, from) {
                    (_, Some(from)) => borrowed(one.url, from),
                    (Some(source), None) => CalendarSource {
                        url: one.url,
                        ..source.clone()
                    },
                    (None, None) => bail!("give an address or --from"),
                };
                put_source(&client, calendar.id, &source)?;
                let after = refresh(&client, calendar.id)?;
                if let Some(error) = &after.error {
                    eprintln!("{}: {error}", after.title);
                    failed = true;
                }
                records.push(record(&after));
            }
            emit(format, &records, COLUMNS)?;
            if failed {
                bail!("not every calendar could be fetched");
            }
            Ok(())
        }
        CalendarCommand::Import { calendar, file } => {
            let calendar = resolve(&client, &calendar)?;
            let (body, name) = if file == "-" {
                let mut body = Vec::new();
                std::io::Read::read_to_end(&mut std::io::stdin(), &mut body)?;
                (body, None)
            } else {
                let body =
                    std::fs::read(&file).map_err(|e| anyhow::anyhow!("cannot read {file}: {e}"))?;
                let name = std::path::Path::new(&file)
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned());
                (body, name)
            };
            let mut path = format!("/calendars/{}/import", calendar.id);
            if let Some(name) = name {
                path.push_str(&format!("?name={}", query_value(&name)));
            }
            let answer = client.call_bytes("POST", &path, "text/calendar", &[], &body)?;
            let imported: CalendarImport = serde_json::from_slice(&answer.body)?;
            for issue in &imported.skipped {
                eprintln!(
                    "skipped {}: {}",
                    issue.uid.as_deref().unwrap_or("-"),
                    issue.reason
                );
            }
            for issue in &imported.failed {
                eprintln!(
                    "failed {}: {}",
                    issue.uid.as_deref().unwrap_or("-"),
                    issue.reason
                );
            }
            let record = json!({
                "calendar": calendar.title,
                "created": imported.created,
                "replaced": imported.replaced,
                "skipped": imported.skipped.len(),
                "failed": imported.failed.len(),
            });
            if matches!(format, Format::Json | Format::Jsonl) {
                println!("{}", serde_json::to_string(&imported)?);
            } else {
                emit(
                    format,
                    &[record],
                    &["calendar", "created", "replaced", "skipped", "failed"],
                )?;
            }
            if !imported.failed.is_empty() {
                bail!("{} of the file did not go in", imported.failed.len());
            }
            Ok(())
        }
        CalendarCommand::Refresh { calendar } => {
            let calendar = resolve(&client, &calendar)?;
            let after = refresh(&client, calendar.id)?;
            emit(format, &[record(&after)], COLUMNS)?;
            if let Some(error) = &after.error {
                bail!("{}: {error}", after.title);
            }
            Ok(())
        }
        CalendarCommand::Delete { calendar } => {
            let calendar = resolve(&client, &calendar)?;
            delete_object(&client, calendar.id, &DeleteRequest::default())?;
            eprintln!("deleted {} and its appointments", calendar.title);
            Ok(())
        }
    }
}

/// `2026-11-20` for a whole day, `2026-11-20 09:00` for a time in `zone`,
/// this computer's: a call at 15:00 UTC is at 16:00 in Berlin.
pub(crate) fn when(when: Option<&When>, zone: &jiff::tz::TimeZone) -> Option<String> {
    match when? {
        When::Date(date) => Some(date.to_string()),
        When::At(at) => Some(
            at.with_time_zone(zone.clone())
                .strftime("%Y-%m-%d %H:%M")
                .to_string(),
        ),
        When::Unknown => None,
    }
}

/// `samu appointments`: the occurrences from `from` to `to`, by start.
pub(crate) fn appointments(
    config: &Config,
    format: Format,
    from: Option<&str>,
    to: Option<&str>,
    scope: Option<&str>,
    no_scope: bool,
) -> Result<()> {
    let client = signed_in(config)?;
    let mut query = Vec::new();
    for (name, value) in [("from", from), ("to", to)] {
        if let Some(value) = value {
            let day: jiff::civil::Date = value
                .parse()
                .map_err(|e| anyhow::anyhow!("--{name} {value:?} is not a day: {e}"))?;
            query.push(format!("{name}={day}"));
        }
    }
    if let Some(id) = scopes::query_scope(&client, config, scope, no_scope)? {
        query.push(format!("scope={id}"));
    }
    let path = format!("/appointments?{}", query.join("&"));
    let instances: Vec<AppointmentInstance> =
        serde_json::from_value(client.call("GET", &path, None)?.json()?)?;
    let titles: std::collections::HashMap<Id, String> = calendars(&client)?
        .into_iter()
        .map(|c| (c.id, c.title))
        .collect();
    let zone = jiff::tz::TimeZone::system();
    let records: Vec<Value> = instances
        .iter()
        .map(|i| {
            json!({
                "appointment": i.appointment,
                "calendar": titles.get(&i.calendar),
                "title": i.title,
                "start": when(Some(&i.start), &zone),
                "end": when(i.end.as_ref(), &zone),
                "all_day": matches!(i.start, When::Date(_)),
                "location": i.location,
                "link": i.link,
                "tentative": i.status == AppointmentStatus::Tentative,
                "busy": i.busy,
            })
        })
        .collect();
    emit(
        format,
        &records,
        &["start", "end", "title", "calendar", "location"],
    )
}

/// An instant for `--from` / `--to` of `samu alarms`: RFC 3339, or a day
/// (`YYYY-MM-DD`), its midnight on this computer.
fn instant(name: &str, value: &str) -> Result<jiff::Timestamp> {
    if let Ok(at) = value.parse::<jiff::Timestamp>() {
        return Ok(at);
    }
    let day: jiff::civil::Date = value
        .parse()
        .map_err(|_| anyhow::anyhow!("--{name} {value:?} is neither a time nor a day"))?;
    Ok(day.to_zoned(jiff::tz::TimeZone::system())?.timestamp())
}

/// `samu alarms`: the reminders of tasks and appointments that go off
/// from `from` (now) to `to` (a day later), by time (GET /alarms, ADR 0101
/// §5).
pub(crate) fn alarms(
    config: &Config,
    format: Format,
    (from, to): (Option<&str>, Option<&str>),
    tasks: bool,
    appointments: bool,
) -> Result<()> {
    let client = signed_in(config)?;
    let mut query = vec![
        format!("tasks={tasks}"),
        format!("appointments={appointments}"),
    ];
    for (name, value) in [("from", from), ("to", to)] {
        if let Some(value) = value {
            query.push(format!("{name}={}", instant(name, value)?));
        }
    }
    let path = format!("/alarms?{}", query.join("&"));
    let alerts: Vec<Alert> = serde_json::from_value(client.call("GET", &path, None)?.json()?)?;
    let titles: std::collections::HashMap<Id, String> =
        if alerts.iter().any(|a| a.calendar.is_some()) {
            calendars(&client)?
                .into_iter()
                .map(|c| (c.id, c.title))
                .collect()
        } else {
            std::collections::HashMap::new()
        };
    let zone = jiff::tz::TimeZone::system();
    let records: Vec<Value> = alerts
        .iter()
        .map(|a| {
            json!({
                "at": a.at.to_zoned(zone.clone()).strftime("%Y-%m-%d %H:%M").to_string(),
                "kind": if a.appointment.is_some() { "appointment" } else { "task" },
                "id": a.appointment.or(a.owner),
                "title": a.title,
                "start": when(a.start.as_ref(), &zone),
                "calendar": a.calendar.and_then(|c| titles.get(&c)),
                "location": a.location,
            })
        })
        .collect();
    emit(
        format,
        &records,
        &["at", "kind", "title", "start", "location"],
    )
}

/// A text as one value of a query, percent-encoded but for what is safe.
fn query_value(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::*;

    #[test]
    fn a_time_is_shown_in_the_zone_given_and_a_day_as_it_is() {
        let berlin = jiff::tz::TimeZone::get("Europe/Berlin").unwrap();
        let utc = When::At("2026-11-21T15:00:00+00:00[UTC]".parse().unwrap());
        assert_eq!(
            when(Some(&utc), &berlin).as_deref(),
            Some("2026-11-21 16:00")
        );
        let day = When::Date(jiff::civil::date(2026, 12, 5));
        assert_eq!(when(Some(&day), &berlin).as_deref(), Some("2026-12-05"));
        assert_eq!(when(None, &berlin), None);
    }

    #[test]
    fn add_takes_all_found_or_the_titles_named_and_refuses_one_not_there() {
        let found = |title: &str| DiscoveredCalendar {
            url: format!("https://dav.example.org/{title}/"),
            title: title.to_string(),
            color: None,
            known: false,
        };
        let all = vec![found("Familie"), found("Dienst")];
        assert_eq!(chosen(&all, &[]).unwrap(), all);
        // Without titles, what is subscribed already stays out; named, it
        // is added again.
        let mut some = all.clone();
        some[0].known = true;
        assert_eq!(chosen(&some, &[]).unwrap(), vec![found("Dienst")]);
        assert_eq!(chosen(&some, &["familie".into()]).unwrap().len(), 1);
        let one = chosen(&all, &["dienst".into(), "Dienst".into()]).unwrap();
        assert_eq!(one, vec![found("Dienst")]);
        let err = chosen(&all, &["Urlaub".into()]).unwrap_err().to_string();
        assert!(err.contains("Familie, Dienst"), "{err}");
    }

    #[test]
    fn a_file_name_goes_into_the_query_encoded() {
        assert_eq!(query_value("alt kalender.ics"), "alt%20kalender.ics");
        assert_eq!(query_value("ä&b"), "%C3%A4%26b");
    }

    #[test]
    fn set_takes_the_value_each_field_wants() {
        assert_eq!(field_value("enabled", Some("off")).unwrap(), json!(false));
        assert_eq!(field_value("refresh", Some("30")).unwrap(), json!(30));
        assert_eq!(field_value("color", None).unwrap(), Value::Null);
        assert!(field_value("title", None).is_err());
        assert!(field_value("refresh", Some("bald")).is_err());
        assert!(field_value("host", Some("x")).is_err());
    }
}
