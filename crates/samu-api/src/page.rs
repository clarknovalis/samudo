//! Objects of any kind and the page `GET /changes` answers with (ADR 0019 §5).

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::{
    AccountSettings, Appointment, Attachment, Calendar, Clock, Comment, Folder, HabitEntry, Id,
    Link, Perspective, Project, Reminder, Scope, Tag, Task, Template,
};

/// An object of any kind: `{"kind": "task", "object": {…}}`.
///
/// Read by hand, not derived: serde's derive cannot fall back to
/// [`AnyObject::Unknown`] for a kind it does not know when that kind
/// carries an object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "object", rename_all = "snake_case")]
pub enum AnyObject {
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
    /// A kind from a newer samu (ADR 0015).
    #[serde(other)]
    Unknown,
}

impl<'de> Deserialize<'de> for AnyObject {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            kind: String,
            #[serde(default)]
            object: Value,
        }
        let raw = Raw::deserialize(deserializer)?;
        let object = raw.object;
        let typed = match raw.kind.as_str() {
            "folder" => serde_json::from_value(object).map(Self::Folder),
            "project" => serde_json::from_value(object).map(Self::Project),
            "task" => serde_json::from_value(object).map(Self::Task),
            "tag" => serde_json::from_value(object).map(Self::Tag),
            "link" => serde_json::from_value(object).map(Self::Link),
            "clock" => serde_json::from_value(object).map(Self::Clock),
            "reminder" => serde_json::from_value(object).map(Self::Reminder),
            "scope" => serde_json::from_value(object).map(Self::Scope),
            "comment" => serde_json::from_value(object).map(Self::Comment),
            "attachment" => serde_json::from_value(object).map(Self::Attachment),
            "settings" => serde_json::from_value(object).map(Self::Settings),
            "calendar" => serde_json::from_value(object).map(Self::Calendar),
            "appointment" => serde_json::from_value(object).map(Self::Appointment),
            "template" => serde_json::from_value(object).map(Self::Template),
            "perspective" => serde_json::from_value(object).map(Self::Perspective),
            "habit_entry" => serde_json::from_value(object).map(Self::HabitEntry),
            _ => return Ok(Self::Unknown),
        };
        typed.map_err(D::Error::custom)
    }
}

impl AnyObject {
    /// The id; none for an unknown kind.
    #[must_use]
    pub fn id(&self) -> Option<Id> {
        Some(match self {
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
            Self::Unknown => return None,
        })
    }
}

/// What changed after a change number (ADR 0004 §5): the objects as they are
/// now and the ids of the deleted ones. With `reset`, the change number asked
/// for is too old and the client loads everything once.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangesPage {
    /// The account's change number this page is current to.
    pub seq: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reset: bool,
    #[serde(default)]
    pub objects: Vec<AnyObject>,
    #[serde(default)]
    pub gone: Vec<Id>,
}
