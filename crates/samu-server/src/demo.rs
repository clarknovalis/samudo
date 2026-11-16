// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The demo content (ADR 0099 §6): `demo/de.toml` and `demo/en.toml`,
//! embedded, written into an empty account.
//!
//! Every object is made as a client would make it over `POST /changes`
//! (`samu add`, `samu tag add`, …), with the server's stamps and the device
//! `demo`; the quick capture is samu-core's, as `POST /capture` reads it.
//! `{+N}` and `{-N}` in the content are the day N days from today in the
//! account's zone, so the demo is fresh on the day it is made: due dates
//! lie ahead, the history behind.
//!
//! An account with content of its own gets only what can be kept apart
//! (ADR 0099 §7): everything under a folder `Demo` and a tag `Demo`, the
//! inbox as a project there, no settings, perspectives, scopes or
//! templates.

use std::collections::{BTreeMap, HashMap};

use anyhow::{Context, Result, anyhow, bail};
use jiff::Timestamp;
use jiff::civil::Date;
use samu_store::core::api::{
    Attachment, Author, AuthorKind, CaptureRequest, Change, ChangeOutcome, Comment, HabitEntry,
    HabitEntryStatus, Hlc, Id, ObjectKind, Perspective, PerspectiveArrange, PerspectiveFilter,
    PerspectiveGroup, PerspectiveSort, PerspectiveView, Reminder, ReminderAnchor, ScopeCapture,
    ScopeWindow, Template, TemplateReminder, Weekday,
};
use samu_store::core::clock::{next_stamp, restamp};
use samu_store::core::{blockers, query, views, workflow};
use samu_store::{AccountStore, BlobDir, OwnedSnapshot};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const DE: &str = include_str!("../demo/de.toml");
const EN: &str = include_str!("../demo/en.toml");

/// The device the demo's changes carry.
pub(crate) const DEVICE: &str = "demo";

/// The language of the demo content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Lang {
    De,
    En,
}

impl Lang {
    fn text(self) -> &'static str {
        match self {
            Self::De => DE,
            Self::En => EN,
        }
    }

    /// The project the inbox becomes under the folder `Demo`.
    const fn inbox(self) -> &'static str {
        match self {
            Self::De => "Eingang",
            Self::En => "Inbox",
        }
    }
}

/// The folder and the root tag the demo goes under in an account with
/// content of its own.
pub(crate) const SHIELD: &str = "Demo";

/// The account has a folder or a root tag [`SHIELD`] already: the demo
/// would mix with it (ADR 0099 §7, a conflict).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Taken(pub(crate) String);

impl std::fmt::Display for Taken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Taken {}

/// How many of each were made, for the line the command prints.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Counts(pub(crate) BTreeMap<&'static str, usize>);

impl Counts {
    fn add(&mut self, what: &'static str) {
        *self.0.entry(what).or_default() += 1;
    }
}

impl std::fmt::Display for Counts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let parts: Vec<String> = self.0.iter().map(|(k, n)| format!("{n} {k}")).collect();
        f.write_str(&parts.join(", "))
    }
}

// --- the content file ------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Content {
    #[serde(default)]
    inbox: Vec<TaskEntry>,
    #[serde(default)]
    blocks: Vec<[String; 2]>,
    #[serde(default)]
    settings: BTreeMap<String, String>,
    #[serde(default)]
    property_links: BTreeMap<String, String>,
    #[serde(default)]
    tags: Vec<TagEntry>,
    #[serde(default)]
    folders: Vec<FolderEntry>,
    #[serde(default)]
    projects: Vec<ProjectEntry>,
    #[serde(default)]
    perspectives: Vec<PerspectiveEntry>,
    #[serde(default)]
    scopes: Vec<ScopeEntry>,
    #[serde(default)]
    templates: Vec<TemplateEntry>,
    #[serde(default)]
    pub(crate) calendars: Vec<CalendarEntry>,
    #[serde(default)]
    pub(crate) appointments: Vec<AppointmentEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TagEntry {
    path: String,
    #[serde(default)]
    exclusive: bool,
    #[serde(default)]
    on_hold: bool,
    #[serde(default)]
    ask_note: bool,
    key: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FolderEntry {
    title: String,
    parent: Option<String>,
    icon: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProjectEntry {
    title: String,
    folder: Option<String>,
    kind: Option<String>,
    icon: Option<String>,
    note: Option<String>,
    workflow: Option<String>,
    review: Option<String>,
    #[serde(default)]
    reviewed: bool,
    #[serde(default)]
    props: BTreeMap<String, String>,
    status: Option<String>,
    #[serde(default)]
    tasks: Vec<TaskEntry>,
}

/// A task: a line of quick capture, or a table with more.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TaskEntry {
    Line(String),
    Table(Box<TaskTable>),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskTable {
    line: String,
    key: Option<String>,
    #[serde(default)]
    children: Vec<TaskEntry>,
    state: Option<String>,
    #[serde(default)]
    done: bool,
    #[serde(default)]
    comments: Vec<String>,
    #[serde(default)]
    remind: Vec<RemindEntry>,
    #[serde(default)]
    clocks: Vec<String>,
    #[serde(default)]
    attach: Vec<AttachEntry>,
    /// A habit's past, oldest first and ending yesterday: `x` done, a digit
    /// that many, `-` failed on purpose, `.` nothing; spaces are ignored.
    history: Option<String>,
    /// A habit's pause, from and until.
    pause: Option<[String; 2]>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RemindEntry {
    /// A length of time before the anchor, like `10m` or `1d`.
    before: Option<String>,
    /// `due` (the default), `planned` or `defer`.
    of: Option<String>,
    /// Every day at this time of day, like `07:30`.
    daily: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttachEntry {
    name: String,
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PerspectiveEntry {
    name: String,
    icon: Option<String>,
    search: String,
    filter: Option<String>,
    group: Option<String>,
    sort: Option<String>,
    then: Option<String>,
    #[serde(default)]
    board: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopeEntry {
    title: String,
    /// `only` (the default) or `except`.
    mode: Option<String>,
    #[serde(default)]
    inbox: bool,
    /// `folder:<title>`, `project:<title>` or `tag:<title>`.
    members: Vec<String>,
    color: Option<String>,
    /// Windows like `mo-fr 08:00-17:00`.
    #[serde(default)]
    schedule: Vec<String>,
    capture_project: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TemplateEntry {
    name: String,
    text: String,
    /// `due:1d`, `planned`, `daily@20:00`.
    #[serde(default)]
    remind: Vec<String>,
    key: Option<String>,
}

/// A CalDAV calendar at `<caldav>/<account>/<path>` (only with `--caldav`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CalendarEntry {
    pub(crate) title: String,
    pub(crate) path: String,
    pub(crate) color: Option<String>,
    #[serde(default)]
    pub(crate) default: bool,
}

/// An appointment in a calendar of the demo (only with `--caldav`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AppointmentEntry {
    pub(crate) title: String,
    /// A day, or a day and a time in the account's zone.
    pub(crate) start: String,
    pub(crate) end: Option<String>,
    pub(crate) location: Option<String>,
    pub(crate) note: Option<String>,
    pub(crate) rrule: Option<String>,
    /// By title; the default calendar without one.
    pub(crate) calendar: Option<String>,
    #[serde(default)]
    pub(crate) free: bool,
}

/// `{+N}`/`{-N}` replaced by the day N days from `today`.
pub(crate) fn with_days(text: &str, today: Date) -> Result<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let (head, tail) = rest.split_at(open);
        out.push_str(head);
        let days = tail.find('}').and_then(|close| {
            let number = &tail[1..close];
            let n = number
                .starts_with(['+', '-'])
                .then(|| number.parse::<i64>().ok())
                .flatten()?;
            Some((close, n))
        });
        match days {
            Some((close, n)) => {
                let day = today
                    .checked_add(jiff::Span::new().days(n))
                    .with_context(|| format!("{n} days from {today}"))?;
                out.push_str(&day.to_string());
                rest = &tail[close + 1..];
            }
            // Not a day, like `{value}`: kept as it is.
            None => {
                out.push('{');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    Ok(out)
}

/// The content of one language with its days filled in.
pub(crate) fn content(lang: Lang, today: Date) -> Result<Content> {
    let text = with_days(lang.text(), today)?;
    toml::from_str(&text).with_context(|| format!("the demo content {lang:?}"))
}

// --- writing ---------------------------------------------------------------

/// Writes the demo content (without calendars) into an empty account.
/// Refuses an account that holds tasks or projects already.
pub(crate) fn seed(store: &mut AccountStore, blobs: &BlobDir, lang: Lang) -> Result<Counts> {
    let mut seeder = Seeder::new(store, blobs, lang)?;
    seeder.ensure_empty()?;
    seeder.whole()?;
    Ok(seeder.counts)
}

/// `POST /demo` (ADR 0099 §7): everything into an account without tasks,
/// projects, folders and tags; into any other only what lies under the
/// folder and the tag [`SHIELD`]. True when it was everything. A folder
/// or root tag [`SHIELD`] that is there already is a [`Taken`].
pub(crate) fn seed_into(
    store: &mut AccountStore,
    blobs: &BlobDir,
    lang: Lang,
) -> Result<(bool, Counts)> {
    let mut seeder = Seeder::new(store, blobs, lang)?;
    let objects = seeder.objects()?;
    let live_tags = objects
        .tags
        .iter()
        .any(|t| t.status != samu_store::core::api::TagStatus::Dropped);
    let bare = objects.tasks.is_empty()
        && objects.projects.is_empty()
        && objects.folders.is_empty()
        && !live_tags;
    drop(objects);
    if bare {
        seeder.whole()?;
        return Ok((true, seeder.counts));
    }
    seeder.shielded(lang)?;
    Ok((false, seeder.counts))
}

/// `@name` and `@"a b"` of a quick-capture line as paths under
/// [`SHIELD`], so a tag the line names is never one of the account's own.
pub(crate) fn shield_tags(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 16);
    let mut at_start = true;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        out.push(c);
        if c == '@' && at_start && chars.peek().is_some_and(|n| !n.is_whitespace()) {
            if chars.peek() == Some(&'"') {
                out.push('"');
                chars.next();
            }
            out.push_str(SHIELD);
            out.push('/');
        }
        at_start = c.is_whitespace();
    }
    out
}

pub(crate) struct Seeder<'a> {
    store: &'a mut AccountStore,
    blobs: &'a BlobDir,
    today: Date,
    content: Content,
    /// The `key`s of tasks, for `blocks`.
    keys: HashMap<String, Id>,
    /// The folders made here, by title: the account may have its own of
    /// the same name.
    folders: HashMap<String, Id>,
    /// Set when the demo goes under [`SHIELD`].
    shield: Option<Shield>,
    counts: Counts,
}

/// Where the demo goes in an account with content of its own.
struct Shield {
    folder: Id,
}

fn author() -> Author {
    Author {
        device: DEVICE.into(),
        kind: AuthorKind::Human,
    }
}

fn placeholder() -> Hlc {
    Hlc::new(0, 0, DEVICE)
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

fn create(kind: ObjectKind, object: Id, value: Value) -> Change {
    Change::Create {
        object,
        kind,
        value,
        hlc: placeholder(),
    }
}

fn create_of<T: serde::Serialize>(kind: ObjectKind, object: Id, value: &T) -> Result<Change> {
    Ok(create(kind, object, serde_json::to_value(value)?))
}

/// A word of the API's vocabulary, as serde reads it; a word it does not
/// know is an error, not `Unknown`.
fn word<T: DeserializeOwned + serde::Serialize>(text: &str, what: &str) -> Result<T> {
    let wire = text.to_lowercase().replace('-', "_");
    let parsed: Option<T> = serde_json::from_value(json!(wire)).ok();
    match parsed {
        Some(value) if serde_json::to_value(&value)? == json!(wire) => Ok(value),
        _ => bail!("{text:?} is no {what}"),
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// A tag's path from the top, `Ort/Büro`.
fn tag_path(objects: &OwnedSnapshot, id: Id) -> String {
    let mut parts = Vec::new();
    let mut current = Some(id);
    for _ in 0..=objects.tags.len() {
        let Some(tag) = current.and_then(|c| objects.tags.iter().find(|t| t.id == c)) else {
            break;
        };
        parts.push(tag.title.as_str());
        current = tag.parent;
    }
    parts.reverse();
    parts.join("/")
}

/// One of several found by name, or why not.
fn one(found: &[Id], name: &str, what: &str) -> Result<Id> {
    match found {
        [id] => Ok(*id),
        [] => bail!("no {what} {name:?}"),
        _ => bail!("{name:?} names several of kind {what}"),
    }
}

impl<'a> Seeder<'a> {
    fn new(store: &'a mut AccountStore, blobs: &'a BlobDir, lang: Lang) -> Result<Self> {
        let today = Timestamp::now().to_zoned(store.settings()?.zone).date();
        Ok(Seeder {
            store,
            blobs,
            today,
            content: content(lang, today)?,
            keys: HashMap::new(),
            folders: HashMap::new(),
            shield: None,
            counts: Counts::default(),
        })
    }

    /// Everything, into an empty account.
    fn whole(&mut self) -> Result<()> {
        let content = std::mem::take(&mut self.content);
        self.settings(&content)?;
        self.tags(&content.tags)?;
        self.folders(&content.folders)?;
        self.projects(&content.projects)?;
        for task in &content.inbox {
            self.task(task, None, None)?;
        }
        self.blocks(&content.blocks)?;
        self.perspectives(&content.perspectives)?;
        self.scopes(&content.scopes)?;
        self.templates(&content.templates)?;
        Ok(())
    }

    /// What can be kept apart, under the folder and the tag [`SHIELD`].
    fn shielded(&mut self, lang: Lang) -> Result<()> {
        let objects = self.objects()?;
        if objects
            .folders
            .iter()
            .any(|f| f.title.eq_ignore_ascii_case(SHIELD))
            || objects
                .tags
                .iter()
                .any(|t| t.parent.is_none() && t.title.eq_ignore_ascii_case(SHIELD))
        {
            return Err(Taken(format!(
                "there is a folder or a tag {SHIELD:?} already; delete it first, or rename it"
            ))
            .into());
        }
        drop(objects);
        let content = std::mem::take(&mut self.content);
        let folder = self.folder_entry(SHIELD, None, Some("🧪"))?;
        self.tag_entry(&TagEntry {
            path: SHIELD.into(),
            exclusive: false,
            on_hold: false,
            ask_note: false,
            key: None,
        })?;
        let inbox = self.project_entry(&ProjectEntry {
            title: lang.inbox().into(),
            folder: Some(SHIELD.into()),
            kind: None,
            icon: Some("📥".into()),
            note: None,
            workflow: None,
            review: None,
            reviewed: false,
            props: BTreeMap::new(),
            status: None,
            tasks: Vec::new(),
        })?;
        self.shield = Some(Shield { folder });
        self.tags(&content.tags)?;
        self.folders(&content.folders)?;
        self.projects(&content.projects)?;
        for task in &content.inbox {
            self.task(task, Some(inbox), None)?;
        }
        self.blocks(&content.blocks)?;
        Ok(())
    }

    /// A tag path of the content as it lies in the account.
    fn shielded_path(&self, path: &str) -> String {
        if self.shield.is_some() {
            format!("{SHIELD}/{path}")
        } else {
            path.to_string()
        }
    }

    /// Applies one change as `POST /changes?stamp=server` does; anything
    /// but applied is an error.
    pub(crate) fn apply(&mut self, change: Change) -> Result<()> {
        let now = Timestamp::now();
        let seen = self.store.observed()?;
        let change = restamp(change, next_stamp(DEVICE, None, seen.as_ref(), now));
        match self.store.apply(&change, &author(), now)? {
            ChangeOutcome::Applied { .. } => Ok(()),
            other => Err(anyhow!(
                "not applied: {} for {}",
                serde_json::to_string(&other)?,
                serde_json::to_string(&change)?
            )),
        }
    }

    pub(crate) fn objects(&self) -> Result<std::sync::Arc<OwnedSnapshot>> {
        Ok(self.store.snapshot()?)
    }

    fn ensure_empty(&self) -> Result<()> {
        let objects = self.objects()?;
        if !objects.tasks.is_empty() || !objects.projects.is_empty() {
            bail!(
                "the account is not empty ({} tasks, {} projects): the demo goes only into a new one",
                objects.tasks.len(),
                objects.projects.len()
            );
        }
        Ok(())
    }

    pub(crate) fn settings_id(&self) -> Result<Id> {
        self.objects()?
            .settings
            .as_ref()
            .map(|s| s.id)
            .ok_or_else(|| anyhow!("the account has no settings object"))
    }

    // --- structure --------------------------------------------------------

    /// As `samu settings set` and `samu settings link`.
    fn settings(&mut self, content: &Content) -> Result<()> {
        let id = self.settings_id()?;
        for (key, value) in &content.settings {
            let on_off = |v: &str| match v {
                "on" => Ok(json!(true)),
                "off" => Ok(json!(false)),
                _ => Err(anyhow!("{key} = {v:?}: on or off")),
            };
            let (field, value) = match key.as_str() {
                "due-soon" => {
                    let span: jiff::Span = value.parse().map_err(|e| anyhow!("{value:?}: {e}"))?;
                    ("due_soon", json!(span.to_string()))
                }
                "forecast-flagged" => ("forecast_flagged", on_off(value)?),
                "forecast-defer" => ("forecast_defer", on_off(value)?),
                "forecast-done" => ("forecast_done", on_off(value)?),
                "forecast-events" => ("forecast_events", on_off(value)?),
                other => bail!("the demo knows no setting {other:?}"),
            };
            self.apply(set(id, field, value))?;
        }
        for (key, template) in &content.property_links {
            if !template.contains("{value}") {
                bail!("{template:?} has no {{value}}");
            }
            self.apply(set(id, &format!("property_links.{key}"), json!(template)))?;
        }
        Ok(())
    }

    /// A tag by its path, any case.
    fn tag(&self, objects: &OwnedSnapshot, path: &str) -> Result<Id> {
        let want = path.to_lowercase();
        let found: Vec<Id> = objects
            .tags
            .iter()
            .filter(|t| tag_path(objects, t.id).to_lowercase() == want)
            .map(|t| t.id)
            .collect();
        one(&found, path, "tag")
    }

    /// As `samu tag add <path>`.
    fn tags(&mut self, tags: &[TagEntry]) -> Result<()> {
        for tag in tags {
            self.tag_entry(tag)?;
        }
        Ok(())
    }

    fn tag_entry(&mut self, tag: &TagEntry) -> Result<()> {
        let objects = self.objects()?;
        let path = self.shielded_path(&tag.path);
        let (parent, title) = match path.rsplit_once('/') {
            Some((above, title)) => (Some(self.tag(&objects, above)?), title),
            None => (None, path.as_str()),
        };
        if let Some(key) = &tag.key
            && key.chars().count() != 1
        {
            bail!("tag {:?}: a key is one character", tag.path);
        }
        let position = objects.tags.iter().filter(|t| t.parent == parent).count();
        let id = Id::new();
        let now = Timestamp::now().to_string();
        let mut value = json!({
            "id": id,
            "created": now,
            "modified": now,
            "title": title,
            "status": if tag.on_hold { "on_hold" } else { "active" },
            "exclusive": tag.exclusive,
            "ask_note": tag.ask_note,
            "default_workflow": false,
            "position": position,
        });
        if let Some(parent) = parent {
            value["parent"] = json!(parent);
        }
        if let Some(key) = &tag.key {
            value["key"] = json!(key);
        }
        self.apply(create(ObjectKind::Tag, id, value))?;
        self.counts.add("tags");
        Ok(())
    }

    /// A folder the demo made, by title.
    fn folder(&self, title: &str) -> Result<Id> {
        self.folders
            .get(&title.to_lowercase())
            .copied()
            .ok_or_else(|| anyhow!("no folder {title:?}"))
    }

    fn project_named(objects: &OwnedSnapshot, title: &str) -> Result<Id> {
        let found: Vec<Id> = objects
            .projects
            .iter()
            .filter(|p| p.title.eq_ignore_ascii_case(title))
            .map(|p| p.id)
            .collect();
        one(&found, title, "project")
    }

    /// As `samu folder add` and `samu folder icon`.
    fn folders(&mut self, folders: &[FolderEntry]) -> Result<()> {
        for folder in folders {
            let parent = match (&folder.parent, &self.shield) {
                (Some(p), _) => Some(self.folder(p)?),
                (None, Some(shield)) => Some(shield.folder),
                (None, None) => None,
            };
            self.folder_entry(&folder.title, parent, folder.icon.as_deref())?;
        }
        Ok(())
    }

    fn folder_entry(&mut self, title: &str, parent: Option<Id>, icon: Option<&str>) -> Result<Id> {
        let key = title.to_lowercase();
        if self.folders.contains_key(&key) {
            bail!("the demo has two folders {title:?}");
        }
        let position = self
            .objects()?
            .folders
            .iter()
            .filter(|f| f.parent == parent)
            .map(|f| f.position)
            .max()
            .map_or(0, |p| p.saturating_add(1));
        let id = Id::new();
        let now = Timestamp::now().to_string();
        let mut value = json!({
            "id": id,
            "created": now,
            "modified": now,
            "title": title,
            "status": "active",
            "position": position,
        });
        if let Some(parent) = parent {
            value["parent"] = json!(parent);
        }
        self.apply(create(ObjectKind::Folder, id, value))?;
        if let Some(icon) = icon {
            self.apply(set(id, "icon", json!(icon)))?;
        }
        self.folders.insert(key, id);
        self.counts.add("folders");
        Ok(id)
    }

    /// As `samu project add`, `project icon`, `set … note`, `project set`,
    /// `review`, then its tasks, and its status last: tasks still go into
    /// an active project.
    fn projects(&mut self, projects: &[ProjectEntry]) -> Result<()> {
        for project in projects {
            self.project_entry(project)?;
        }
        Ok(())
    }

    fn project_entry(&mut self, project: &ProjectEntry) -> Result<Id> {
        let folder = match (&project.folder, &self.shield) {
            (Some(f), _) => Some(self.folder(f)?),
            (None, Some(shield)) => Some(shield.folder),
            (None, None) => None,
        };
        let objects = self.objects()?;
        let position = objects
            .projects
            .iter()
            .filter(|p| p.folder == folder)
            .map(|p| p.position)
            .max()
            .map_or(0, |p| p.saturating_add(1));
        let kind: samu_store::core::api::ProjectKind = word(
            project.kind.as_deref().unwrap_or("parallel"),
            "project kind",
        )?;
        let id = Id::new();
        let now = Timestamp::now().to_string();
        let mut value = json!({
            "id": id,
            "created": now,
            "modified": now,
            "title": project.title,
            "kind": kind,
            "status": "active",
            "flagged": false,
            "position": position,
        });
        if let Some(folder) = folder {
            value["folder"] = json!(folder);
        }
        self.apply(create(ObjectKind::Project, id, value))?;
        self.counts.add("projects");
        if let Some(icon) = &project.icon {
            self.apply(set(id, "icon", json!(icon)))?;
        }
        if let Some(note) = &project.note {
            self.apply(set(id, "note", json!(note.trim())))?;
        }
        if let Some(workflow) = &project.workflow {
            let tag = self.tag(&objects, &self.shielded_path(workflow))?;
            self.apply(set(id, "workflow", json!(tag)))?;
        }
        if let Some(review) = &project.review {
            let period = match review.as_str() {
                "1w" => "P1W",
                "2w" => "P2W",
                "1mo" => "P1M",
                "3mo" => "P3M",
                other => bail!("review is 1w, 2w, 1mo or 3mo, not {other:?}"),
            };
            self.apply(set(id, "review_interval", json!(period)))?;
        }
        for (key, value) in &project.props {
            self.apply(set(id, &format!("properties.{key}"), json!(value)))?;
        }
        if project.reviewed {
            let now = json!(Timestamp::now().to_string());
            self.apply(set(id, "last_reviewed", now))?;
            self.apply(set(id, "next_review", Value::Null))?;
        }
        for task in &project.tasks {
            self.task(task, Some(id), None)?;
        }
        if let Some(status) = &project.status {
            let status: samu_store::core::api::ProjectStatus = word(status, "project status")?;
            let now = json!(Timestamp::now().to_string());
            let wire = serde_json::to_value(status)?;
            let (completed, dropped) = match wire.as_str() {
                Some("completed") => (now, Value::Null),
                Some("dropped") => (Value::Null, now),
                _ => (Value::Null, Value::Null),
            };
            self.apply(set(id, "status", wire))?;
            self.apply(set(id, "completed_at", completed))?;
            self.apply(set(id, "dropped_at", dropped))?;
        }
        Ok(id)
    }

    // --- tasks ------------------------------------------------------------

    /// Runs a capture as `POST /capture` does and writes its changes.
    fn capture(&mut self, request: &CaptureRequest) -> Result<Id> {
        let objects = self.objects()?;
        let settings = self.store.settings()?;
        let views::Capture { answer, changes } = views::capture(
            &objects,
            &settings,
            request,
            Timestamp::now(),
            &placeholder(),
            &mut Id::new,
        )
        .map_err(|e| anyhow!("{:?}: {e}", request.text))?;
        if !answer.warnings.is_empty() {
            bail!("{:?}: {:?}", request.text, answer.warnings);
        }
        for change in changes {
            self.apply(change)?;
        }
        Ok(answer.task.id)
    }

    /// As `samu add` with the line, then what the table adds.
    fn task(&mut self, entry: &TaskEntry, project: Option<Id>, parent: Option<Id>) -> Result<Id> {
        let line = match entry {
            TaskEntry::Line(line) => line,
            TaskEntry::Table(table) => &table.line,
        };
        let text = if self.shield.is_some() {
            shield_tags(line)
        } else {
            line.clone()
        };
        let id = self.capture(&CaptureRequest {
            text,
            project: if parent.is_some() { None } else { project },
            parent,
            scope: None,
            template: None,
            task: None,
            preview: false,
        })?;
        self.counts.add("tasks");
        let TaskEntry::Table(table) = entry else {
            return Ok(id);
        };
        if let Some(key) = &table.key {
            self.keys.insert(key.clone(), id);
        }
        for child in &table.children {
            self.task(child, None, Some(id))?;
        }
        if let Some(state) = &table.state {
            self.state(id, state)?;
        }
        self.extras(id, table)?;
        if table.done {
            self.apply(set(id, "completed_at", json!(Timestamp::now().to_string())))?;
        }
        Ok(id)
    }

    /// As `samu state <id> <state>`: the state by title or key.
    fn state(&mut self, id: Id, name: &str) -> Result<()> {
        let objects = self.objects()?;
        let task = objects
            .tasks
            .iter()
            .find(|t| t.id == id)
            .ok_or_else(|| anyhow!("{id} is not a task"))?;
        let project = task
            .project
            .and_then(|p| objects.projects.iter().find(|q| q.id == p));
        let group = workflow::workflow_of(project, &objects.tags)
            .ok_or_else(|| anyhow!("{:?} has no workflow for {name:?}", task.title))?;
        let states = workflow::states(group, &objects.tags);
        let wanted = objects
            .tags
            .iter()
            .filter(|t| states.contains(&t.id))
            .find(|t| t.title.eq_ignore_ascii_case(name) || t.key.as_deref() == Some(name))
            .map(|t| t.id)
            .ok_or_else(|| anyhow!("{name:?} is not a state of {:?}", task.title))?;
        if workflow::current_state(&task.tags, &states) == Some(wanted) {
            return Ok(());
        }
        self.apply(Change::Add {
            object: id,
            field: "tags".into(),
            element: wanted,
            position: 0,
            note: None,
            hlc: placeholder(),
        })
    }

    /// Comments, reminders, clocks, attachments and a habit's past.
    fn extras(&mut self, id: Id, table: &TaskTable) -> Result<()> {
        for text in &table.comments {
            let now = Timestamp::now();
            let comment = Comment {
                id: Id::new(),
                created: now,
                modified: now,
                created_by: None,
                owner: id,
                body: text.trim().to_string(),
            };
            self.apply(create_of(ObjectKind::Comment, comment.id, &comment)?)?;
            self.counts.add("comments");
        }
        for remind in &table.remind {
            let reminder = reminder(id, remind)?;
            self.apply(create_of(ObjectKind::Reminder, reminder.id, &reminder)?)?;
            self.counts.add("reminders");
        }
        for clock in &table.clocks {
            // As `samu clock add <id> <time>`: what follows `z:`.
            self.capture(&CaptureRequest {
                text: format!("z:{clock}"),
                project: None,
                parent: None,
                scope: None,
                template: None,
                task: Some(id),
                preview: false,
            })?;
            self.counts.add("clocks");
        }
        for attachment in &table.attach {
            self.attach(id, attachment)?;
            self.counts.add("attachments");
        }
        if let Some(history) = &table.history {
            self.history(id, history)?;
        }
        if let Some([from, until]) = &table.pause {
            let from: Date = from.parse().with_context(|| format!("pause {from:?}"))?;
            let until: Date = until.parse().with_context(|| format!("pause {until:?}"))?;
            self.entry(id, from, HabitEntryStatus::Pause, None, Some(until))?;
        }
        Ok(())
    }

    /// As `samu attach`: the file into the account's blobs, then the
    /// attachment.
    fn attach(&mut self, owner: Id, attachment: &AttachEntry) -> Result<()> {
        let bytes = attachment.text.trim_start().as_bytes();
        let hash = hex(&Sha256::digest(bytes));
        let written = self
            .blobs
            .put(&hash, bytes)?
            .map_err(|refused| anyhow!("{}: {refused}", attachment.name))?;
        let media_type = "text/plain";
        let now = Timestamp::now();
        self.store
            .add_blob(&written.hash, written.size, media_type, now)?;
        let attachment = Attachment {
            id: Id::new(),
            created: now,
            modified: now,
            created_by: None,
            owner,
            blob: written.hash,
            name: attachment.name.clone(),
            media_type: media_type.into(),
            size: written.size,
        };
        self.apply(create_of(
            ObjectKind::Attachment,
            attachment.id,
            &attachment,
        )?)
    }

    fn history(&mut self, habit: Id, marks: &str) -> Result<()> {
        let marks: Vec<char> = marks.chars().filter(|c| !c.is_whitespace()).collect();
        let len = i64::try_from(marks.len())?;
        for (i, mark) in (0..).zip(&marks) {
            let day = self.today.checked_sub(jiff::Span::new().days(len - i))?;
            let (status, amount) = match mark {
                'x' => (HabitEntryStatus::Done, None),
                '-' => (HabitEntryStatus::Fail, None),
                '.' => continue,
                digit => match digit.to_digit(10) {
                    Some(n) if n > 0 => (HabitEntryStatus::Done, Some(n)),
                    _ => bail!("{mark:?} in a habit's history: x, a digit, - or ."),
                },
            };
            self.entry(habit, day, status, amount, None)?;
            self.counts.add("habit entries");
        }
        Ok(())
    }

    /// As `samu habit done|fail|pause`.
    fn entry(
        &mut self,
        habit: Id,
        day: Date,
        status: HabitEntryStatus,
        amount: Option<u32>,
        until: Option<Date>,
    ) -> Result<()> {
        let now = Timestamp::now();
        let entry = HabitEntry {
            id: Id::new(),
            created: now,
            modified: now,
            created_by: None,
            habit,
            day,
            status,
            amount,
            until,
        };
        self.apply(create_of(ObjectKind::HabitEntry, entry.id, &entry)?)
    }

    /// As `samu block <task> <blocker>`, by the tasks' keys.
    fn blocks(&mut self, blocks: &[[String; 2]]) -> Result<()> {
        for [task, blocker] in blocks {
            let key = |k: &String| {
                self.keys
                    .get(k)
                    .copied()
                    .ok_or_else(|| anyhow!("no task with key {k:?}"))
            };
            let (owner, target) = (key(task)?, key(blocker)?);
            let objects = self.objects()?;
            blockers::check(&objects.tasks, &objects.links, owner, target)
                .map_err(|why| anyhow!("{task} blocked by {blocker}: {why}"))?;
            let link = blockers::link(owner, target, Timestamp::now());
            self.apply(create_of(ObjectKind::Link, link.id, &link)?)?;
            self.counts.add("blocks");
        }
        Ok(())
    }

    // --- views ------------------------------------------------------------

    /// As `samu perspective add` (and `arrange` for a second sorting): the
    /// search read as the web reads it, chips and free text.
    fn perspectives(&mut self, perspectives: &[PerspectiveEntry]) -> Result<()> {
        for p in perspectives {
            let objects = self.objects()?;
            let (chips, text) = query::split_search(&p.search, &objects.snapshot());
            let arrange = if p.group.is_some() || p.sort.is_some() || p.then.is_some() {
                Some(PerspectiveArrange {
                    group: p
                        .group
                        .as_deref()
                        .map(|g| word::<PerspectiveGroup>(g, "grouping"))
                        .transpose()?
                        .unwrap_or_default(),
                    sort: p
                        .sort
                        .as_deref()
                        .map(|s| word::<PerspectiveSort>(s, "sorting"))
                        .transpose()?
                        .unwrap_or_default(),
                    then: p
                        .then
                        .as_deref()
                        .map(|s| word::<PerspectiveSort>(s, "sorting"))
                        .transpose()?,
                    ..PerspectiveArrange::default()
                })
            } else {
                None
            };
            let position = objects
                .perspectives
                .iter()
                .map(|p| p.position + 1)
                .max()
                .unwrap_or(0);
            let now = Timestamp::now();
            let perspective = Perspective {
                id: Id::new(),
                created: now,
                modified: now,
                title: p.name.clone(),
                icon: p.icon.clone(),
                text,
                chips,
                filter: p
                    .filter
                    .as_deref()
                    .map(|f| word::<PerspectiveFilter>(f, "filter"))
                    .transpose()?
                    .unwrap_or_default(),
                arrange,
                view: if p.board {
                    PerspectiveView::Board
                } else {
                    PerspectiveView::List
                },
                board: None,
                table: None,
                blocks: Vec::new(),
                dedupe: true,
                position,
            };
            self.apply(create_of(
                ObjectKind::Perspective,
                perspective.id,
                &perspective,
            )?)?;
            self.counts.add("perspectives");
        }
        Ok(())
    }

    /// A scope's member: `folder:`, `project:` or `tag:` and a title.
    fn member(objects: &OwnedSnapshot, name: &str) -> Result<Id> {
        let (kind, title) = name.split_once(':').unwrap_or(("", name));
        let mut found = Vec::new();
        if matches!(kind, "" | "folder") {
            found.extend(
                objects
                    .folders
                    .iter()
                    .filter(|f| f.title.eq_ignore_ascii_case(title))
                    .map(|f| f.id),
            );
        }
        if matches!(kind, "" | "project") {
            found.extend(
                objects
                    .projects
                    .iter()
                    .filter(|p| p.title.eq_ignore_ascii_case(title))
                    .map(|p| p.id),
            );
        }
        if matches!(kind, "" | "tag") {
            found.extend(
                objects
                    .tags
                    .iter()
                    .filter(|t| t.title.eq_ignore_ascii_case(title))
                    .map(|t| t.id),
            );
        }
        one(&found, name, "folder, project or tag")
    }

    /// As `samu scope add`, `scope color`, `scope schedule`, `scope capture`.
    fn scopes(&mut self, scopes: &[ScopeEntry]) -> Result<()> {
        for s in scopes {
            let objects = self.objects()?;
            let members = s
                .members
                .iter()
                .map(|m| Self::member(&objects, m))
                .collect::<Result<Vec<_>>>()?;
            let mode = s.mode.as_deref().unwrap_or("only");
            if !matches!(mode, "only" | "except") {
                bail!("scope {:?}: mode is only or except", s.title);
            }
            let id = Id::new();
            let now = Timestamp::now().to_string();
            self.apply(create(
                ObjectKind::Scope,
                id,
                json!({
                    "id": id,
                    "created": now,
                    "modified": now,
                    "title": s.title,
                    "mode": mode,
                    "members": members,
                    "inbox": s.inbox,
                    "position": objects.scopes.len(),
                }),
            ))?;
            if let Some(color) = &s.color {
                self.apply(set(id, "color", json!(color)))?;
            }
            if !s.schedule.is_empty() {
                let windows = s
                    .schedule
                    .iter()
                    .map(|w| window(w))
                    .collect::<Result<Vec<_>>>()?;
                self.apply(set(id, "schedule", serde_json::to_value(&windows)?))?;
            }
            if let Some(project) = &s.capture_project {
                let capture = ScopeCapture {
                    project: Some(Self::project_named(&objects, project)?),
                    tags: Vec::new(),
                };
                self.apply(set(id, "capture", serde_json::to_value(capture)?))?;
            }
            self.counts.add("scopes");
        }
        Ok(())
    }

    /// As `samu template add`.
    fn templates(&mut self, templates: &[TemplateEntry]) -> Result<()> {
        for t in templates {
            let objects = self.objects()?;
            let reminders = t
                .remind
                .iter()
                .map(|r| template_reminder(r))
                .collect::<Result<Vec<_>>>()?;
            let position = objects
                .templates
                .iter()
                .map(|t| t.position + 1)
                .max()
                .unwrap_or(0);
            let now = Timestamp::now();
            let template = Template {
                id: Id::new(),
                created: now,
                modified: now,
                title: t.name.clone(),
                text: t.text.clone(),
                reminders,
                position,
                key: t.key.clone(),
            };
            self.apply(create_of(ObjectKind::Template, template.id, &template)?)?;
            self.counts.add("templates");
        }
        Ok(())
    }
}

/// As `samu remind`: `before` the anchor `of`, or `daily` at a time.
fn reminder(owner: Id, remind: &RemindEntry) -> Result<Reminder> {
    let now = Timestamp::now();
    if let Some(time) = &remind.daily {
        return Ok(Reminder {
            id: Id::new(),
            created: now,
            modified: now,
            owner,
            anchor: ReminderAnchor::Daily,
            at: None,
            before: None,
            time: Some(time_of_day(time)?),
        });
    }
    let anchor = match remind.of.as_deref().unwrap_or("due") {
        "due" => ReminderAnchor::Due,
        "planned" => ReminderAnchor::Planned,
        "defer" => ReminderAnchor::Defer,
        other => bail!("a reminder is of due, planned or defer, not {other:?}"),
    };
    let before = remind
        .before
        .as_deref()
        .map(|b| {
            b.parse()
                .map(samu_store::core::api::Period)
                .map_err(|e| anyhow!("{b:?} is no length of time: {e}"))
        })
        .transpose()?;
    Ok(Reminder {
        id: Id::new(),
        created: now,
        modified: now,
        owner,
        anchor,
        at: None,
        before,
        time: None,
    })
}

/// As `samu template add --remind`: `due:1d`, `planned`, `daily@20:00`.
fn template_reminder(text: &str) -> Result<TemplateReminder> {
    if let Some((anchor, time)) = text.split_once('@') {
        if anchor.trim() != "daily" {
            bail!("{anchor:?}: only daily takes a time of day");
        }
        return Ok(TemplateReminder {
            anchor: ReminderAnchor::Daily,
            before: None,
            time: Some(time_of_day(time)?),
        });
    }
    let (anchor, before) = text.split_once(':').unwrap_or((text, ""));
    let anchor = match anchor.trim() {
        "planned" => ReminderAnchor::Planned,
        "due" => ReminderAnchor::Due,
        "defer" => ReminderAnchor::Defer,
        other => bail!("{other:?}: a reminder is on planned, due or defer"),
    };
    let before = match before.trim() {
        "" => None,
        b => Some(samu_store::core::api::Period(
            b.parse()
                .map_err(|e| anyhow!("{b:?} is no length of time: {e}"))?,
        )),
    };
    Ok(TemplateReminder {
        anchor,
        before,
        time: None,
    })
}

/// `20:00` or `20:00:00`.
fn time_of_day(text: &str) -> Result<jiff::civil::Time> {
    let text = text.trim();
    let full = if text.len() == 5 {
        format!("{text}:00")
    } else {
        text.to_owned()
    };
    full.parse()
        .map_err(|e| anyhow!("{text:?} is no time of day: {e}"))
}

const DAYS: [(&str, &str, Weekday); 7] = [
    ("mo", "mo", Weekday::Mo),
    ("di", "tu", Weekday::Tu),
    ("mi", "we", Weekday::We),
    ("do", "th", Weekday::Th),
    ("fr", "fr", Weekday::Fr),
    ("sa", "sa", Weekday::Sa),
    ("so", "su", Weekday::Su),
];

/// As `samu scope schedule`: `mo-fr 08:00-17:00`, days as a list or a
/// range (one that wraps goes round), then from-to.
fn window(text: &str) -> Result<ScopeWindow> {
    let index = |word: &str| {
        let word = word.to_lowercase();
        DAYS.iter()
            .position(|(de, en, _)| *de == word || *en == word)
            .ok_or_else(|| anyhow!("{word:?} is no day"))
    };
    let Some((days_text, times)) = text.trim().split_once(char::is_whitespace) else {
        bail!("{text:?}: days, then from-to");
    };
    let mut days = Vec::new();
    for part in days_text.split(',') {
        let (a, b) = match part.split_once('-') {
            Some((a, b)) => (index(a)?, index(b)?),
            None => (index(part)?, index(part)?),
        };
        let mut i = a;
        loop {
            if !days.contains(&DAYS[i].2) {
                days.push(DAYS[i].2);
            }
            if i == b {
                break;
            }
            i = (i + 1) % DAYS.len();
        }
    }
    let Some((from, to)) = times.trim().split_once('-') else {
        bail!("{times:?}: from-to");
    };
    Ok(ScopeWindow {
        days,
        from: time_of_day(from)?,
        to: time_of_day(to)?,
    })
}

// --- calendars (only from `account add --caldav`) ----------------------------

/// Where the demo's calendars go: `<base>/<account>/<path>` on a CalDAV
/// server whose users are the accounts, with one password for all.
pub(crate) struct CalDav {
    pub(crate) base: String,
    pub(crate) password: String,
}

/// Makes a collection on the CalDAV server; one that is there is fine.
async fn mkcalendar(
    client: &reqwest::Client,
    url: &str,
    user: &str,
    password: &str,
    entry: &CalendarEntry,
) -> Result<()> {
    let mut props = format!("<D:displayname>{}</D:displayname>", xml_text(&entry.title));
    if let Some(color) = &entry.color {
        props.push_str(&format!(
            "<I:calendar-color>{}</I:calendar-color>",
            xml_text(color)
        ));
    }
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
         <C:mkcalendar xmlns:D=\"DAV:\" xmlns:C=\"urn:ietf:params:xml:ns:caldav\" \
         xmlns:I=\"http://apple.com/ns/ical/\"><D:set><D:prop>{props}\
         <C:supported-calendar-component-set><C:comp name=\"VEVENT\"/>\
         </C:supported-calendar-component-set></D:prop></D:set></C:mkcalendar>"
    );
    let method = reqwest::Method::from_bytes(b"MKCALENDAR")?;
    let response = client
        .request(method, url)
        .basic_auth(user, Some(password))
        .header("Content-Type", "application/xml")
        .body(body)
        .send()
        .await
        .with_context(|| format!("MKCALENDAR {url}"))?;
    let status = response.status();
    // 405: the collection is there already.
    if !status.is_success() && status != reqwest::StatusCode::METHOD_NOT_ALLOWED {
        bail!("MKCALENDAR {url}: {status}");
    }
    Ok(())
}

fn xml_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// A day alone, or a day and a time in the zone, as `samu appointment add`
/// reads it.
fn when(text: &str, zone: &jiff::tz::TimeZone) -> Result<samu_store::core::api::When> {
    use samu_store::core::api::When;
    let text = text.trim();
    if text.len() == 10
        && let Ok(day) = text.parse::<Date>()
    {
        return Ok(When::Date(day));
    }
    let wall: jiff::civil::DateTime = text
        .replacen(' ', "T", 1)
        .parse()
        .map_err(|e| anyhow!("{text:?} is neither a day nor a day with a time: {e}"))?;
    Ok(When::At(wall.to_zoned(zone.clone())?))
}

/// Without an end: the next day, or an hour later.
fn default_end(start: &samu_store::core::api::When) -> Option<samu_store::core::api::When> {
    use samu_store::core::api::When;
    match start {
        When::Date(d) => d.tomorrow().ok().map(When::Date),
        When::At(at) => at
            .checked_add(jiff::SignedDuration::from_hours(1))
            .ok()
            .map(When::At),
        When::Unknown => None,
    }
}

/// The demo's calendars on the CalDAV server, each with its source and
/// marked writable, then its appointments. The server writes these to the
/// calendar once it runs (ADR 0068 §6).
pub(crate) fn calendars(
    store: &mut AccountStore,
    blobs: &BlobDir,
    lang: Lang,
    account: &str,
    caldav: &CalDav,
    key: &samu_store::SecretKey,
) -> Result<Counts> {
    use samu_store::core::api::{
        Appointment, AppointmentStatus, Calendar, CalendarKind, CalendarSource,
    };
    let zone = store.settings()?.zone;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut seeder = Seeder::new(store, blobs, lang)?;
    let content = std::mem::take(&mut seeder.content);
    let settings = seeder.settings_id()?;
    let mut by_title = HashMap::new();
    let mut default = None;
    for (position, entry) in (0..).zip(&content.calendars) {
        let url = format!(
            "{}/{account}/{}",
            caldav.base.trim_end_matches('/'),
            entry.path.trim_start_matches('/')
        );
        runtime.block_on(mkcalendar(&client, &url, account, &caldav.password, entry))?;
        let now = Timestamp::now();
        let calendar = Calendar {
            id: Id::new(),
            created: now,
            modified: now,
            title: entry.title.clone(),
            kind: CalendarKind::Caldav,
            color: entry.color.clone(),
            enabled: true,
            refresh: 15,
            link_template: None,
            position,
            fetched_at: None,
            error: None,
            host: None,
            writable: false,
        };
        seeder.apply(create_of(ObjectKind::Calendar, calendar.id, &calendar)?)?;
        let source = CalendarSource {
            url,
            username: Some(account.to_string()),
            password: Some(caldav.password.clone()),
            credentials_from: None,
        };
        seeder
            .store
            .set_calendar_source(calendar.id, &source, key, now)?
            .map_err(|why| anyhow!("calendar {:?}: {why}", entry.title))?;
        // Made just now with these credentials: samu may write to it.
        seeder.store.set_writable(calendar.id, true, now)?;
        if entry.default {
            seeder.apply(set(settings, "default_calendar", json!(calendar.id)))?;
            default = Some(calendar.id);
        }
        by_title.insert(entry.title.to_lowercase(), calendar.id);
        seeder.counts.add("calendars");
    }
    for a in &content.appointments {
        let calendar = match &a.calendar {
            Some(title) => *by_title
                .get(&title.to_lowercase())
                .ok_or_else(|| anyhow!("no calendar {title:?}"))?,
            None => default.ok_or_else(|| anyhow!("{:?}: no default calendar", a.title))?,
        };
        let start = when(&a.start, &zone)?;
        let end = match &a.end {
            Some(end) => Some(when(end, &zone)?),
            None => default_end(&start),
        };
        let now = Timestamp::now();
        let id = Id::new();
        let appointment = Appointment {
            id,
            created: now,
            modified: now,
            calendar,
            uid: id.to_string(),
            recurrence_id: None,
            title: a.title.clone(),
            start,
            end,
            location: a.location.as_deref().map(str::trim).map(String::from),
            note: a.note.as_deref().map(str::trim).map(String::from),
            url: None,
            status: AppointmentStatus::Confirmed,
            busy: !a.free,
            rrule: a.rrule.clone(),
            rdate: Vec::new(),
            exdate: Vec::new(),
            meeting: false,
            alarms: Vec::new(),
        };
        seeder.apply(create_of(ObjectKind::Appointment, id, &appointment)?)?;
        seeder.counts.add("appointments");
    }
    Ok(seeder.counts)
}
