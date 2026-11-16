//! Rows ↔ `samu-api` objects.
//!
//! One description of the columns per kind, used both ways: an object is
//! turned into its JSON form (which is the wire form, ADR 0014) and the
//! columns are read off that, and a row becomes JSON and then the typed
//! object through serde. So a field cannot be written one way and read
//! another, and `set` is checked by the same serde that checks the wire
//! (ADR 0018 §6).

use rusqlite::types::Value as Sql;
use serde_json::{Map, Value as Json};
use uuid::Uuid;

use crate::core::api::{
    AccountSettings, Appointment, Attachment, Calendar, Clock, Comment, Folder, HabitEntry, Id,
    Link, ObjectKind, Perspective, Project, Reminder, Scope, Tag, Task, Template,
};

/// How one JSON field is kept in the row.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Ty {
    /// Text, or NULL when missing.
    Text,
    Int,
    Bool,
    /// A uuid as BLOB(16).
    Uuid,
    /// A date in two forms: `<column>_date`, or `<column>_at` plus
    /// `<column>_zone` (ADR 0007 §1).
    When,
    /// `repeat_rule`, `repeat_mode`, `repeat_catch_up`.
    Repeat,
    /// The field's JSON as text, NULL when missing, null or an empty list.
    Json,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Column {
    /// The key in the JSON form.
    key: &'static str,
    /// The column, or the prefix of the columns for `When` and `Repeat`.
    column: &'static str,
    ty: Ty,
}

const fn col(key: &'static str, ty: Ty) -> Column {
    Column {
        key,
        column: key,
        ty,
    }
}

const FOLDER: &[Column] = &[
    col("title", Ty::Text),
    col("parent", Ty::Uuid),
    col("status", Ty::Text),
    col("icon", Ty::Text),
    // ADR 0094; `scope_only` is derived from it when read.
    col("own_scope", Ty::Uuid),
    col("position", Ty::Int),
];

const PROJECT: &[Column] = &[
    col("title", Ty::Text),
    col("icon", Ty::Text),
    // ADR 0094; `scope_only` is derived from it when read.
    col("own_scope", Ty::Uuid),
    col("note", Ty::Text),
    col("folder", Ty::Uuid),
    col("workflow", Ty::Uuid),
    col("kind", Ty::Text),
    col("status", Ty::Text),
    col("defer", Ty::When),
    col("due", Ty::When),
    col("planned", Ty::When),
    col("due_warning", Ty::Text),
    col("planned_duration", Ty::Text),
    col("repeat", Ty::Repeat),
    col("review_interval", Ty::Text),
    col("next_review", Ty::Text),
    col("last_reviewed", Ty::Text),
    col("flagged", Ty::Bool),
    col("estimate", Ty::Text),
    col("completed_at", Ty::Text),
    col("dropped_at", Ty::Text),
    col("position", Ty::Int),
];

const TASK: &[Column] = &[
    col("title", Ty::Text),
    col("note", Ty::Text),
    col("project", Ty::Uuid),
    col("parent", Ty::Uuid),
    col("defer", Ty::When),
    col("due", Ty::When),
    col("planned", Ty::When),
    col("due_warning", Ty::Text),
    col("planned_duration", Ty::Text),
    col("repeat", Ty::Repeat),
    col("flagged", Ty::Bool),
    col("estimate", Ty::Text),
    col("priority", Ty::Text),
    col("completed_at", Ty::Text),
    col("dropped_at", Ty::Text),
    col("repeat_of", Ty::Uuid),
    col("sequential", Ty::Bool),
    col("complete_with_last_child", Ty::Bool),
    col("position", Ty::Int),
    // ADR 0091.
    col("habit", Ty::Json),
];

const TAG: &[Column] = &[
    col("title", Ty::Text),
    col("parent", Ty::Uuid),
    col("status", Ty::Text),
    col("exclusive", Ty::Bool),
    // ADR 0094; `scope_only` is derived from it when read.
    col("own_scope", Ty::Uuid),
    col("key", Ty::Text),
    col("ask_note", Ty::Bool),
    col("default_workflow", Ty::Bool),
    col("position", Ty::Int),
];

const LINK: &[Column] = &[
    col("owner", Ty::Uuid),
    col("uri", Ty::Text),
    col("label", Ty::Text),
    col("kind", Ty::Text),
];

const CLOCK: &[Column] = &[
    col("owner", Ty::Uuid),
    Column {
        key: "start",
        column: "start_at",
        ty: Ty::Text,
    },
    Column {
        key: "end",
        column: "end_at",
        ty: Ty::Text,
    },
    col("note", Ty::Text),
];

const REMINDER: &[Column] = &[
    col("owner", Ty::Uuid),
    col("anchor", Ty::Text),
    col("at", Ty::Text),
    col("before", Ty::Text),
    // ADR 0093.
    col("time", Ty::Text),
];

const SCOPE: &[Column] = &[
    col("title", Ty::Text),
    col("mode", Ty::Text),
    col("inbox", Ty::Bool),
    col("filter", Ty::Json),
    col("capture", Ty::Json),
    col("schedule", Ty::Json),
    // ADR 0047 §12.
    col("calendars", Ty::Json),
    // ADR 0060.
    col("color", Ty::Text),
    // ADR 0072.
    col("layout", Ty::Json),
    col("position", Ty::Int),
];

// ADR 0073.
const TEMPLATE: &[Column] = &[
    col("title", Ty::Text),
    col("text", Ty::Text),
    col("reminders", Ty::Json),
    col("position", Ty::Int),
    // ADR 0084.
    col("key", Ty::Text),
];

// ADR 0090.
const PERSPECTIVE: &[Column] = &[
    col("title", Ty::Text),
    col("icon", Ty::Text),
    col("text", Ty::Text),
    col("chips", Ty::Json),
    col("filter", Ty::Text),
    col("arrange", Ty::Json),
    col("view", Ty::Text),
    col("board", Ty::Json),
    // ADR 0115 §7; `table` is a word of SQL.
    Column {
        key: "table",
        column: "table_view",
        ty: Ty::Json,
    },
    // ADR 0116 §1; `dedupe` as JSON, so a missing one stays the default.
    col("blocks", Ty::Json),
    col("dedupe", Ty::Json),
    col("position", Ty::Int),
];

// ADR 0091. An entry does not change (§2), see [`settable`].
const HABIT_ENTRY: &[Column] = &[
    col("habit", Ty::Uuid),
    col("day", Ty::Text),
    col("status", Ty::Text),
    // JSON, so that a missing amount stays missing.
    col("amount", Ty::Json),
    col("until", Ty::Text),
];

const SETTINGS: &[Column] = &[
    col("due_soon", Ty::Text),
    col("remind_time", Ty::Text),
    col("forecast_days", Ty::Int),
    col("forecast_flagged", Ty::Bool),
    col("forecast_tag", Ty::Uuid),
    col("forecast_defer", Ty::Bool),
    col("forecast_done", Ty::Bool),
    col("forecast_events", Ty::Bool),
    col("default_calendar", Ty::Uuid),
    col("keys", Ty::Json),
    col("report_worked", Ty::Text),
];

const CALENDAR: &[Column] = &[
    col("title", Ty::Text),
    col("kind", Ty::Text),
    col("color", Ty::Text),
    col("enabled", Ty::Bool),
    col("refresh", Ty::Int),
    col("link_template", Ty::Text),
    col("position", Ty::Int),
    col("fetched_at", Ty::Text),
    col("error", Ty::Text),
    col("host", Ty::Text),
    col("writable", Ty::Bool),
];

/// The fields of a calendar only the server writes (ADR 0047 §2).
pub(crate) const CALENDAR_SERVER_FIELDS: &[&str] = &["fetched_at", "error", "host", "writable"];

const APPOINTMENT: &[Column] = &[
    col("calendar", Ty::Uuid),
    col("uid", Ty::Text),
    col("recurrence_id", Ty::When),
    col("title", Ty::Text),
    col("start", Ty::When),
    col("end", Ty::When),
    col("location", Ty::Text),
    col("note", Ty::Text),
    col("url", Ty::Text),
    col("status", Ty::Text),
    col("busy", Ty::Bool),
    col("rrule", Ty::Text),
    col("rdate", Ty::Json),
    col("exdate", Ty::Json),
    col("meeting", Ty::Bool),
    col("alarms", Ty::Json),
];

const COMMENT: &[Column] = &[col("owner", Ty::Uuid), col("body", Ty::Text)];

const ATTACHMENT: &[Column] = &[
    col("owner", Ty::Uuid),
    col("blob", Ty::Text),
    col("name", Ty::Text),
    col("media_type", Ty::Text),
    col("size", Ty::Int),
];

/// The table and columns of a kind; `None` for `unknown`.
pub(crate) fn layout(kind: ObjectKind) -> Option<(&'static str, &'static [Column])> {
    Some(match kind {
        ObjectKind::Folder => ("folder", FOLDER),
        ObjectKind::Project => ("project", PROJECT),
        ObjectKind::Task => ("task", TASK),
        ObjectKind::Tag => ("tag", TAG),
        ObjectKind::Link => ("link", LINK),
        ObjectKind::Clock => ("clock", CLOCK),
        ObjectKind::Reminder => ("reminder", REMINDER),
        ObjectKind::Scope => ("scope", SCOPE),
        ObjectKind::Comment => ("comment", COMMENT),
        ObjectKind::Attachment => ("attachment", ATTACHMENT),
        ObjectKind::Settings => ("settings", SETTINGS),
        ObjectKind::Calendar => ("calendar", CALENDAR),
        ObjectKind::Appointment => ("appointment", APPOINTMENT),
        ObjectKind::Template => ("template", TEMPLATE),
        ObjectKind::Perspective => ("perspective", PERSPECTIVE),
        ObjectKind::HabitEntry => ("habit_entry", HABIT_ENTRY),
        ObjectKind::Unknown => return None,
    })
}

/// Whether `field` is a field `set` may change on this kind. A comment keeps
/// its owner, and an attachment its owner and its file: only the text and the
/// name change (ADR 0039 §3, §5).
pub(crate) fn settable(kind: ObjectKind, field: &str) -> bool {
    match kind {
        ObjectKind::Comment => field == "body",
        ObjectKind::Attachment => field == "name",
        // An entry is deleted, never changed (ADR 0091 §2).
        ObjectKind::HabitEntry => false,
        // In a writable calendar, these alone (ADR 0068 §3).
        ObjectKind::Appointment => crate::core::api::APPOINTMENT_FIELDS.contains(&field),
        ObjectKind::Calendar if CALENDAR_SERVER_FIELDS.contains(&field) => false,
        _ => layout(kind).is_some_and(|(_, columns)| columns.iter().any(|c| c.key == field)),
    }
}

/// Whether `key` may name a property (ADR 0023 §1): a lower-case letter,
/// then lower-case letters, digits, `_` and `-`, at most 64 in all. The
/// table checks the same.
pub(crate) fn property_key(key: &str) -> bool {
    let mut chars = key.chars();
    key.len() <= 64
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

/// The SQL column names, in the order [`to_sql`] and [`from_sql`] use.
pub(crate) fn sql_columns(columns: &[Column]) -> Vec<String> {
    let mut names = Vec::new();
    for c in columns {
        match c.ty {
            Ty::When => {
                names.push(format!("{}_date", c.column));
                names.push(format!("{}_at", c.column));
                names.push(format!("{}_zone", c.column));
            }
            Ty::Repeat => {
                names.push("repeat_rule".into());
                names.push("repeat_mode".into());
                names.push("repeat_catch_up".into());
            }
            _ => names.push(c.column.into()),
        }
    }
    names
}

fn text(value: Option<&Json>) -> Result<Sql, String> {
    match value {
        None | Some(Json::Null) => Ok(Sql::Null),
        Some(Json::String(s)) => Ok(Sql::Text(s.clone())),
        Some(other) => Err(format!("expected text, got {other}")),
    }
}

/// The zone of a zoned time as jiff writes it in brackets: an IANA name, or
/// an offset for a fixed zone.
fn zone_of(zoned: &jiff::Zoned) -> Result<String, String> {
    let printed = zoned.to_string();
    let start = printed.rfind('[').ok_or("zoned time without zone")?;
    let end = printed.rfind(']').ok_or("zoned time without zone")?;
    printed
        .get(start + 1..end)
        .map(str::to_string)
        .ok_or_else(|| "zoned time without zone".into())
}

/// The columns of one object, from its JSON form.
pub(crate) fn to_sql(object: &Map<String, Json>, columns: &[Column]) -> Result<Vec<Sql>, String> {
    let mut values = Vec::new();
    for c in columns {
        let value = object.get(c.key);
        match c.ty {
            Ty::Text => values.push(text(value)?),
            Ty::Int => values.push(match value {
                None | Some(Json::Null) => Sql::Integer(0),
                Some(v) => Sql::Integer(v.as_i64().ok_or(format!("{}: expected a number", c.key))?),
            }),
            Ty::Bool => values.push(Sql::Integer(i64::from(
                value.and_then(Json::as_bool).unwrap_or(false),
            ))),
            Ty::Uuid => values.push(match value {
                None | Some(Json::Null) => Sql::Null,
                Some(Json::String(s)) => Sql::Blob(
                    Uuid::parse_str(s)
                        .map_err(|e| format!("{}: {e}", c.key))?
                        .as_bytes()
                        .to_vec(),
                ),
                Some(other) => return Err(format!("{}: expected a uuid, got {other}", c.key)),
            }),
            Ty::When => {
                let (date, at, zone) = match value {
                    None | Some(Json::Null) => (Sql::Null, Sql::Null, Sql::Null),
                    Some(Json::Object(form)) => match (form.get("date"), form.get("at")) {
                        (Some(date), None) => (text(Some(date))?, Sql::Null, Sql::Null),
                        (None, Some(Json::String(at))) => {
                            let zoned: jiff::Zoned =
                                at.parse().map_err(|e| format!("{}: {e}", c.key))?;
                            (
                                Sql::Null,
                                Sql::Text(zoned.timestamp().to_string()),
                                Sql::Text(zone_of(&zoned)?),
                            )
                        }
                        // `unknown` is read, never written (ADR 0015 §2).
                        _ => {
                            return Err(format!("{}: a date form this server cannot store", c.key));
                        }
                    },
                    Some(other) => return Err(format!("{}: expected a date, got {other}", c.key)),
                };
                values.extend([date, at, zone]);
            }
            Ty::Repeat => {
                let (rule, mode, catch_up) = match value {
                    None | Some(Json::Null) => (Sql::Null, Sql::Null, Sql::Null),
                    Some(Json::Object(repeat)) => (
                        text(repeat.get("rule"))?,
                        text(repeat.get("mode"))?,
                        Sql::Integer(i64::from(
                            repeat
                                .get("catch_up")
                                .and_then(Json::as_bool)
                                .unwrap_or(false),
                        )),
                    ),
                    Some(other) => return Err(format!("repeat: expected an object, got {other}")),
                };
                values.extend([rule, mode, catch_up]);
            }
            Ty::Json => values.push(match value {
                None | Some(Json::Null) => Sql::Null,
                Some(Json::Array(list)) if list.is_empty() => Sql::Null,
                Some(v) => Sql::Text(v.to_string()),
            }),
        }
    }
    Ok(values)
}

/// The JSON form of a row's columns, in the order of [`sql_columns`].
pub(crate) fn from_sql(values: &[Sql], columns: &[Column]) -> Result<Map<String, Json>, String> {
    let mut object = Map::new();
    let mut at = 0;
    let mut next = || {
        let value = values.get(at).cloned().unwrap_or(Sql::Null);
        at += 1;
        value
    };
    for c in columns {
        let json = match c.ty {
            Ty::Text => match next() {
                Sql::Text(s) => Some(Json::String(s)),
                _ => None,
            },
            Ty::Int => match next() {
                Sql::Integer(i) => Some(Json::from(i)),
                _ => None,
            },
            Ty::Bool => match next() {
                Sql::Integer(i) => Some(Json::Bool(i != 0)),
                _ => None,
            },
            Ty::Uuid => match next() {
                Sql::Blob(bytes) => Some(Json::String(
                    Uuid::from_slice(&bytes)
                        .map_err(|e| format!("{}: {e}", c.key))?
                        .to_string(),
                )),
                _ => None,
            },
            Ty::When => match (next(), next(), next()) {
                (Sql::Text(date), _, _) => Some(serde_json::json!({ "date": date })),
                (_, Sql::Text(at), Sql::Text(zone)) => {
                    let instant: jiff::Timestamp =
                        at.parse().map_err(|e| format!("{}: {e}", c.key))?;
                    let zoned: jiff::Zoned = format!("{instant}[{zone}]")
                        .parse()
                        .map_err(|e| format!("{}: {e}", c.key))?;
                    Some(serde_json::json!({ "at": zoned.to_string() }))
                }
                _ => None,
            },
            Ty::Repeat => match (next(), next(), next()) {
                (Sql::Text(rule), Sql::Text(mode), catch_up) => Some(serde_json::json!({
                    "rule": rule,
                    "mode": mode,
                    "catch_up": matches!(catch_up, Sql::Integer(i) if i != 0),
                })),
                _ => None,
            },
            Ty::Json => match next() {
                Sql::Text(text) => {
                    Some(serde_json::from_str(&text).map_err(|e| format!("{}: {e}", c.key))?)
                }
                _ => None,
            },
        };
        if let Some(json) = json {
            object.insert(c.key.into(), json);
        }
    }
    Ok(object)
}

/// An object of any kind, as `samu-api` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Object {
    Folder(Folder),
    Project(Project),
    Task(Task),
    Tag(Tag),
    Link(Link),
    Clock(Clock),
    Reminder(Reminder),
    Scope(Scope),
    Comment(Comment),
    Attachment(Attachment),
    Settings(AccountSettings),
    Calendar(Calendar),
    Appointment(Appointment),
    Template(Template),
    Perspective(Perspective),
    HabitEntry(HabitEntry),
}

impl Object {
    #[must_use]
    pub fn kind(&self) -> ObjectKind {
        match self {
            Self::Folder(_) => ObjectKind::Folder,
            Self::Project(_) => ObjectKind::Project,
            Self::Task(_) => ObjectKind::Task,
            Self::Tag(_) => ObjectKind::Tag,
            Self::Link(_) => ObjectKind::Link,
            Self::Clock(_) => ObjectKind::Clock,
            Self::Reminder(_) => ObjectKind::Reminder,
            Self::Scope(_) => ObjectKind::Scope,
            Self::Comment(_) => ObjectKind::Comment,
            Self::Attachment(_) => ObjectKind::Attachment,
            Self::Settings(_) => ObjectKind::Settings,
            Self::Calendar(_) => ObjectKind::Calendar,
            Self::Appointment(_) => ObjectKind::Appointment,
            Self::Template(_) => ObjectKind::Template,
            Self::Perspective(_) => ObjectKind::Perspective,
            Self::HabitEntry(_) => ObjectKind::HabitEntry,
        }
    }

    #[must_use]
    pub fn id(&self) -> Id {
        match self {
            Self::Folder(o) => o.id,
            Self::Project(o) => o.id,
            Self::Task(o) => o.id,
            Self::Tag(o) => o.id,
            Self::Link(o) => o.id,
            Self::Clock(o) => o.id,
            Self::Reminder(o) => o.id,
            Self::Scope(o) => o.id,
            Self::Comment(o) => o.id,
            Self::Attachment(o) => o.id,
            Self::Settings(o) => o.id,
            Self::Calendar(o) => o.id,
            Self::Appointment(o) => o.id,
            Self::Template(o) => o.id,
            Self::Perspective(o) => o.id,
            Self::HabitEntry(o) => o.id,
        }
    }

    /// The wire form (ADR 0014).
    pub fn to_json(&self) -> Result<Map<String, Json>, String> {
        let value = match self {
            Self::Folder(o) => serde_json::to_value(o),
            Self::Project(o) => serde_json::to_value(o),
            Self::Task(o) => serde_json::to_value(o),
            Self::Tag(o) => serde_json::to_value(o),
            Self::Link(o) => serde_json::to_value(o),
            Self::Clock(o) => serde_json::to_value(o),
            Self::Reminder(o) => serde_json::to_value(o),
            Self::Scope(o) => serde_json::to_value(o),
            Self::Comment(o) => serde_json::to_value(o),
            Self::Attachment(o) => serde_json::to_value(o),
            Self::Settings(o) => serde_json::to_value(o),
            Self::Calendar(o) => serde_json::to_value(o),
            Self::Appointment(o) => serde_json::to_value(o),
            Self::Template(o) => serde_json::to_value(o),
            Self::Perspective(o) => serde_json::to_value(o),
            Self::HabitEntry(o) => serde_json::to_value(o),
        }
        .map_err(|e| e.to_string())?;
        match value {
            Json::Object(map) => Ok(map),
            _ => Err("an object did not serialise as a JSON object".into()),
        }
    }

    /// Reads the wire form of a kind.
    pub fn from_json(kind: ObjectKind, value: Json) -> Result<Self, String> {
        let typed = match kind {
            ObjectKind::Folder => serde_json::from_value(value).map(Self::Folder),
            ObjectKind::Project => serde_json::from_value(value).map(Self::Project),
            ObjectKind::Task => serde_json::from_value(value).map(Self::Task),
            ObjectKind::Tag => serde_json::from_value(value).map(Self::Tag),
            ObjectKind::Link => serde_json::from_value(value).map(Self::Link),
            ObjectKind::Clock => serde_json::from_value(value).map(Self::Clock),
            ObjectKind::Reminder => serde_json::from_value(value).map(Self::Reminder),
            ObjectKind::Scope => serde_json::from_value(value).map(Self::Scope),
            ObjectKind::Comment => serde_json::from_value(value).map(Self::Comment),
            ObjectKind::Attachment => serde_json::from_value(value).map(Self::Attachment),
            ObjectKind::Settings => serde_json::from_value(value).map(Self::Settings),
            ObjectKind::Calendar => serde_json::from_value(value).map(Self::Calendar),
            ObjectKind::Appointment => serde_json::from_value(value).map(Self::Appointment),
            ObjectKind::Template => serde_json::from_value(value).map(Self::Template),
            ObjectKind::Perspective => serde_json::from_value(value).map(Self::Perspective),
            ObjectKind::HabitEntry => serde_json::from_value(value).map(Self::HabitEntry),
            ObjectKind::Unknown => return Err("an object kind this server does not know".into()),
        };
        typed.map_err(|e| e.to_string())
    }
}

/// The kind as stored in `object.kind`.
pub(crate) fn kind_name(kind: ObjectKind) -> Option<&'static str> {
    layout(kind).map(|(table, _)| table)
}

/// The kind of a stored `object.kind`.
pub(crate) fn kind_of(name: &str) -> ObjectKind {
    match name {
        "folder" => ObjectKind::Folder,
        "project" => ObjectKind::Project,
        "task" => ObjectKind::Task,
        "tag" => ObjectKind::Tag,
        "link" => ObjectKind::Link,
        "clock" => ObjectKind::Clock,
        "reminder" => ObjectKind::Reminder,
        "scope" => ObjectKind::Scope,
        "comment" => ObjectKind::Comment,
        "attachment" => ObjectKind::Attachment,
        "settings" => ObjectKind::Settings,
        "calendar" => ObjectKind::Calendar,
        "appointment" => ObjectKind::Appointment,
        "template" => ObjectKind::Template,
        "perspective" => ObjectKind::Perspective,
        "habit_entry" => ObjectKind::HabitEntry,
        _ => ObjectKind::Unknown,
    }
}

impl From<Object> for crate::core::api::AnyObject {
    fn from(object: Object) -> Self {
        match object {
            Object::Folder(o) => Self::Folder(o),
            Object::Project(o) => Self::Project(o),
            Object::Task(o) => Self::Task(o),
            Object::Tag(o) => Self::Tag(o),
            Object::Link(o) => Self::Link(o),
            Object::Clock(o) => Self::Clock(o),
            Object::Reminder(o) => Self::Reminder(o),
            Object::Scope(o) => Self::Scope(o),
            Object::Comment(o) => Self::Comment(o),
            Object::Attachment(o) => Self::Attachment(o),
            Object::Settings(o) => Self::Settings(o),
            Object::Calendar(o) => Self::Calendar(o),
            Object::Appointment(o) => Self::Appointment(o),
            Object::Template(o) => Self::Template(o),
            Object::Perspective(o) => Self::Perspective(o),
            Object::HabitEntry(o) => Self::HabitEntry(o),
        }
    }
}
