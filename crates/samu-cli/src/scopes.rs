// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `samu scope …` (ADR 0035): scopes are objects of their own, made and
//! changed over `POST /changes`. Which one is chosen belongs to this device
//! (§4) and is kept next to the session token, never on the server. What a
//! scope shows is computed by the server; `list` and `projects` only send
//! `scope=`.

use anyhow::{Result, bail};
use clap::{Subcommand, ValueEnum};
use samu_api::{
    AnyObject, Change, ChangesPage, Folder, Id, InspectorPart, NoteMode, ObjectKind, Project,
    ScheduledScope, Scope, ScopeCapture, ScopeChip, ScopeLayout, ScopeWindow, Tag, Weekday,
};
use serde_json::{Value, json};

use crate::config::Config;
use crate::output::{Format, emit};
use crate::{placeholder, short, signed_in, write};
use samu_client::{Client, Error as Failure};
use samu_core::query::{self, ParseError};

#[derive(Debug, Subcommand)]
pub(crate) enum ScopeCommand {
    /// Make a scope: only what its members cover, or everything else.
    Add {
        title: String,
        /// Show only what the members cover.
        #[arg(long, required_unless_present = "except", conflicts_with = "except")]
        only: bool,
        /// Show everything the members do not cover.
        #[arg(long)]
        except: bool,
        /// The inbox is a member too (ADR 0043 §1): shown with `--only`,
        /// hidden with `--except`.
        #[arg(long)]
        inbox: bool,
        /// Folders, projects and tags by title or the start or end of an id; prefix
        /// `folder:`, `project:` or `tag:` where a title names more than one.
        members: Vec<String>,
    },
    /// Every scope, with its mode and members; `*` marks the one chosen here.
    List,
    /// One scope's members, with their kind.
    Show { scope: String },
    /// Give a scope a new title.
    Rename { scope: String, title: String },
    /// Switch a scope between only and except.
    Mode { scope: String, mode: Mode },
    /// Tick the inbox as a member or untick it (ADR 0043 §1).
    Inbox { scope: String, tick: Tick },
    /// The colour the views show while the scope is chosen (ADR 0060):
    /// without one, print it; `--clear` takes it off.
    Color {
        scope: String,
        #[arg(value_enum, conflicts_with = "clear")]
        color: Option<Color>,
        #[arg(long)]
        clear: bool,
    },
    /// How the views show the scope (ADR 0072): which parts of the
    /// inspector are shown, the rest folded, and how the note shows.
    /// Without options, print it; `--clear` takes the layout off.
    Layout {
        scope: String,
        /// The parts shown, comma separated: properties, availability,
        /// state, project, priority, defer, due, planned, repeat, series,
        /// estimate, tags, note, links, reminders, clock, comments,
        /// attachments, logbook.
        #[arg(long, value_delimiter = ',', conflicts_with = "clear")]
        inspector: Option<Vec<String>>,
        /// How the note shows.
        #[arg(long, value_enum, conflicts_with = "clear")]
        note: Option<NoteArg>,
        #[arg(long)]
        clear: bool,
    },
    /// Set a scope's filter in the search language (ADR 0043 §2), which a
    /// task must meet besides the members: `fällig:7t ist:markiert -@Warten
    /// #Projekt ordner:Name zustand:Name ~<15m`. Chips of one kind OR,
    /// kinds AND, `-` negates. Without words, print it; `--clear` empties it.
    /// What a task captured while this scope is chosen gets when its line
    /// names none (ADR 0043 §6): a project and tags. Without options, print
    /// it; `--clear` takes it off.
    Capture {
        scope: String,
        #[arg(long, conflicts_with = "clear")]
        project: Option<String>,
        /// A tag by title or path; may be given more than once.
        #[arg(long = "tag", conflicts_with = "clear")]
        tags: Vec<String>,
        #[arg(long)]
        clear: bool,
    },
    /// When a device that follows the schedule chooses this scope
    /// (ADR 0043 §7): windows like `mo-fr 08:00-17:00` or `sa 22:00-02:00`
    /// (ends the next day), in the account's zone. Days `mo di mi do fr sa
    /// so`. Without windows, print them; `--clear` takes them off.
    Schedule {
        scope: String,
        windows: Vec<String>,
        #[arg(long, conflicts_with = "windows")]
        clear: bool,
    },
    /// What the schedules say now (GET /scopes/scheduled).
    Now,
    Filter {
        scope: String,
        #[arg(allow_hyphen_values = true, conflicts_with = "clear")]
        words: Vec<String>,
        #[arg(long)]
        clear: bool,
    },
    /// Add members to a scope or take them off.
    #[command(subcommand)]
    Member(MemberCommand),
    /// Delete a scope.
    Delete { scope: String },
    /// Choose the scope `list`, `projects` and `add` use on this device;
    /// `auto` follows the schedule (ADR 0043 §7); with `--none` no scope;
    /// with neither, print the one chosen.
    Use {
        #[arg(conflicts_with = "none")]
        scope: Option<String>,
        #[arg(long)]
        none: bool,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum MemberCommand {
    Add {
        scope: String,
        #[arg(required = true)]
        members: Vec<String>,
    },
    Remove {
        scope: String,
        #[arg(required = true)]
        members: Vec<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Mode {
    Only,
    Except,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Tick {
    On,
    Off,
}

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Self::Only => "only",
            Self::Except => "except",
        }
    }
}

/// What names refer to: the scopes and what can be a member.
pub(crate) struct Snapshot {
    pub(crate) scopes: Vec<Scope>,
    pub(crate) folders: Vec<Folder>,
    pub(crate) projects: Vec<Project>,
    pub(crate) tags: Vec<Tag>,
    /// ADR 0090.
    pub(crate) perspectives: Vec<samu_api::Perspective>,
}

impl Snapshot {
    /// What samu-core reads names from; no tasks, links or clocks.
    pub(crate) fn core(&self) -> samu_core::availability::Snapshot<'_> {
        samu_core::availability::Snapshot {
            folders: &self.folders,
            projects: &self.projects,
            tags: &self.tags,
            scopes: &self.scopes,
            ..Default::default()
        }
    }
}

pub(crate) fn snapshot(client: &Client) -> Result<Snapshot> {
    let page: ChangesPage =
        serde_json::from_value(client.call("GET", "/changes?after=0", None)?.json()?)?;
    let mut snapshot = Snapshot {
        scopes: Vec::new(),
        folders: Vec::new(),
        projects: Vec::new(),
        tags: Vec::new(),
        perspectives: Vec::new(),
    };
    for object in page.objects {
        match object {
            AnyObject::Scope(s) => snapshot.scopes.push(s),
            AnyObject::Folder(f) => snapshot.folders.push(f),
            AnyObject::Project(p) => snapshot.projects.push(p),
            AnyObject::Tag(t) => snapshot.tags.push(t),
            AnyObject::Perspective(p) => snapshot.perspectives.push(p),
            _ => {}
        }
    }
    snapshot.scopes.sort_by_key(|s| (s.position, s.id));
    snapshot.perspectives.sort_by_key(|p| (p.position, p.id));
    Ok(snapshot)
}

/// One of `found`, or why there is not exactly one.
fn one<T: Copy>(name: &str, what: &str, found: &[(T, Id)]) -> Result<T> {
    match found {
        [(item, _)] => Ok(*item),
        [] => Err(Failure::NotFound(format!("no {what} {name:?}")).into()),
        many => bail!(
            "{name:?} names {} {what}s; use the start or end of an id or a prefix: {}",
            many.len(),
            many.iter()
                .map(|(_, id)| short(id))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Title (any case) first, then the start or end of an id.
fn by_name<'a, T>(items: &'a [T], name: &str, of: impl Fn(&T) -> (&str, Id)) -> Vec<&'a T> {
    let by_title: Vec<&T> = items
        .iter()
        .filter(|item| of(item).0.eq_ignore_ascii_case(name))
        .collect();
    if !by_title.is_empty() {
        return by_title;
    }
    let wanted = name.to_ascii_lowercase();
    items
        .iter()
        .filter(|item| crate::id_matches(&of(item).1, &wanted))
        .collect()
}

fn find_scope<'a>(scopes: &'a [Scope], name: &str) -> Result<&'a Scope> {
    let found: Vec<(&Scope, Id)> = by_name(scopes, name, |s| (s.title.as_str(), s.id))
        .into_iter()
        .map(|s| (s, s.id))
        .collect();
    one(name, "scope", &found)
}

/// A scope by title or the start or end of its id, for `samu set <id> own_scope`
/// (ADR 0094 §8).
pub(crate) fn scope_named(client: &Client, name: &str) -> Result<Id> {
    Ok(find_scope(&snapshot(client)?.scopes, name)?.id)
}

/// The titles of the scopes, for the mark `only:<scope>` (ADR 0094 §8).
pub(crate) fn scope_titles(client: &Client) -> Result<std::collections::HashMap<Id, String>> {
    Ok(snapshot(client)?
        .scopes
        .into_iter()
        .map(|s| (s.id, s.title))
        .collect())
}

/// The mark of what has an own scope: `only:<title>`, or `only:?` for a
/// scope that does not exist (ADR 0094 §5, §8).
pub(crate) fn only_mark(
    own: Option<Id>,
    titles: &std::collections::HashMap<Id, String>,
) -> Option<String> {
    own.map(|id| format!("only:{}", titles.get(&id).map_or("?", String::as_str)))
}

/// A member by name: a folder, a project or a tag.
fn find_member(snapshot: &Snapshot, name: &str) -> Result<Id> {
    let (kinds, name) = match name.split_once(':') {
        Some(("folder", rest)) => (["folder"].as_slice(), rest),
        Some(("project", rest)) => (["project"].as_slice(), rest),
        Some(("tag", rest)) => (["tag"].as_slice(), rest),
        _ => (["folder", "project", "tag"].as_slice(), name),
    };
    let name = name.trim_start_matches(['@', '#']);
    let mut found: Vec<(Id, Id)> = Vec::new();
    if kinds.contains(&"folder") {
        found.extend(
            by_name(&snapshot.folders, name, |f| (f.title.as_str(), f.id))
                .iter()
                .map(|f| (f.id, f.id)),
        );
    }
    if kinds.contains(&"project") {
        found.extend(
            by_name(&snapshot.projects, name, |p| (p.title.as_str(), p.id))
                .iter()
                .map(|p| (p.id, p.id)),
        );
    }
    if kinds.contains(&"tag") {
        found.extend(
            by_name(&snapshot.tags, name, |t| (t.title.as_str(), t.id))
                .iter()
                .map(|t| (t.id, t.id)),
        );
    }
    one(name, "folder, project or tag", &found)
}

/// The kind and title of a member; `gone` for one deleted since.
fn describe(snapshot: &Snapshot, id: Id) -> (&'static str, String) {
    if let Some(f) = snapshot.folders.iter().find(|f| f.id == id) {
        return ("folder", f.title.clone());
    }
    if let Some(p) = snapshot.projects.iter().find(|p| p.id == id) {
        return ("project", p.title.clone());
    }
    if let Some(t) = snapshot.tags.iter().find(|t| t.id == id) {
        return ("tag", t.title.clone());
    }
    ("gone", short(&id))
}

/// A folder's or tag's path from the top, `/` between.
fn path<T>(items: &[T], id: Id, of: impl Fn(&T) -> (Id, Option<Id>, &str)) -> String {
    let mut parts = Vec::new();
    let mut at = Some(id);
    while let Some(here) = at {
        let Some(item) = items.iter().find(|i| of(i).0 == here) else {
            break;
        };
        let (_, parent, title) = of(item);
        if parts.len() > 32 {
            break;
        }
        parts.push(title.to_owned());
        at = parent;
    }
    parts.reverse();
    parts.join("/")
}

fn tag_path(snapshot: &Snapshot, id: Id) -> String {
    path(&snapshot.tags, id, |t| (t.id, t.parent, t.title.as_str()))
}

/// By path first, then by title alone when that is unique; as `one` says.
fn by_path<T>(items: &[T], name: &str, of: impl Fn(&T) -> (Id, String, &str)) -> Vec<(Id, Id)> {
    let want = name.to_lowercase();
    let exact: Vec<(Id, Id)> = items
        .iter()
        .filter(|i| of(i).1.to_lowercase() == want)
        .map(|i| (of(i).0, of(i).0))
        .collect();
    if !exact.is_empty() {
        return exact;
    }
    items
        .iter()
        .filter(|i| of(i).2.to_lowercase() == want)
        .map(|i| (of(i).0, of(i).0))
        .collect()
}

/// One word of the search language as a chip, names resolved to ids and
/// the value checked by samu-core, as the web and the server read it.
pub(crate) fn parse_chip(snapshot: &Snapshot, word: &str) -> Result<ScopeChip> {
    query::parse_chip(word, &snapshot.core()).map_err(|e| refused(word, e))
}

/// Why a word is no chip, a name that is not there as not found.
pub(crate) fn refused(word: &str, error: ParseError) -> anyhow::Error {
    match error {
        ParseError::Free(_) => {
            anyhow::anyhow!("{word:?} is no chip; free text has no place in a filter")
        }
        ParseError::NotFound { .. } => Failure::NotFound(error.to_string()).into(),
        e => e.into(),
    }
}

/// Words as chips, each argument split as the search splits it; `oder`
/// starts a new group (ADR 0121 §3).
pub(crate) fn parse_chips(snapshot: &Snapshot, words: &[String]) -> Result<Vec<ScopeChip>> {
    let mut groups = query::Groups::default();
    for word in words.iter().flat_map(|w| query::split_marked(w)) {
        if !word.quoted && query::is_or(&word.text) {
            groups.or();
        } else {
            groups.push(parse_chip(snapshot, &word.text)?);
        }
    }
    Ok(groups.into_chips())
}

/// The days of the week in order, German first, English accepted.
const DAYS: &[(&str, &str, Weekday)] = &[
    ("mo", "mo", Weekday::Mo),
    ("di", "tu", Weekday::Tu),
    ("mi", "we", Weekday::We),
    ("do", "th", Weekday::Th),
    ("fr", "fr", Weekday::Fr),
    ("sa", "sa", Weekday::Sa),
    ("so", "su", Weekday::Su),
];

fn day_index(word: &str) -> Option<usize> {
    let word = word.to_lowercase();
    DAYS.iter()
        .position(|(de, en, _)| *de == word || *en == word)
}

/// `mo-fr 08:00-17:00`, `sa,so 00:00-00:00`: days as a list or a range
/// (one that wraps, like `fr-mo`, goes round), then from-to.
fn parse_window(text: &str) -> Result<ScopeWindow> {
    let Some((days_text, times)) = text.trim().split_once(char::is_whitespace) else {
        bail!("{text:?}: days, then from-to, like mo-fr 08:00-17:00");
    };
    let mut days = Vec::new();
    for part in days_text.split(',') {
        let range = match part.split_once('-') {
            Some((a, b)) => (day_index(a), day_index(b)),
            None => (day_index(part), day_index(part)),
        };
        let (Some(a), Some(b)) = range else {
            bail!("{part:?} is no day; mo di mi do fr sa so");
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
        bail!("{times:?}: from-to, like 08:00-17:00");
    };
    let time = |t: &str| -> Result<jiff::civil::Time> {
        let t = t.trim();
        let full = if t.len() == 5 {
            format!("{t}:00")
        } else {
            t.to_owned()
        };
        full.parse()
            .map_err(|e| anyhow::anyhow!("{t:?} is no time: {e}"))
    };
    Ok(ScopeWindow {
        days,
        from: time(from)?,
        to: time(to)?,
    })
}

/// A window as `parse_window` reads it.
fn window_text(window: &ScopeWindow) -> String {
    let days: Vec<&str> = window
        .days
        .iter()
        .map(|d| {
            DAYS.iter()
                .find(|(_, _, w)| w == d)
                .map_or("?", |(de, _, _)| *de)
        })
        .collect();
    format!(
        "{} {}-{}",
        days.join(","),
        window.from.strftime("%H:%M"),
        window.to.strftime("%H:%M")
    )
}

/// The scope this device uses: by name, chosen, or the schedule's when
/// it follows it. One that is gone counts as none.
pub(crate) fn effective_scope(client: &Client, config: &Config) -> Result<Option<Id>> {
    if config.follows_schedule()? {
        let now: ScheduledScope =
            serde_json::from_value(client.call("GET", "/scopes/scheduled", None)?.json()?)?;
        return Ok(now.scope);
    }
    config.chosen_scope()
}

fn mode_name(scope: &Scope) -> String {
    serde_json::to_value(scope.mode)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn member_change(scope: Id, element: Id, add: bool) -> Change {
    if add {
        Change::Add {
            object: scope,
            field: "members".into(),
            element,
            position: 0,
            note: None,
            hlc: placeholder(),
        }
    } else {
        Change::Remove {
            object: scope,
            field: "members".into(),
            element,
            note: None,
            hlc: placeholder(),
        }
    }
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

/// The scope `list` and `projects` send: `--scope` by name, none with
/// `--no-scope`, else the one chosen on this device. A chosen scope that is
/// gone counts as none (ADR 0035 §4), with a word on stderr.
pub(crate) fn query_scope(
    client: &Client,
    config: &Config,
    named: Option<&str>,
    none: bool,
) -> Result<Option<Id>> {
    if none {
        return Ok(None);
    }
    if let Some(name) = named {
        let snapshot = snapshot(client)?;
        return Ok(Some(find_scope(&snapshot.scopes, name)?.id));
    }
    if config.follows_schedule()? {
        return effective_scope(client, config);
    }
    let Some(chosen) = config.chosen_scope()? else {
        return Ok(None);
    };
    let snapshot = snapshot(client)?;
    if snapshot.scopes.iter().any(|s| s.id == chosen) {
        Ok(Some(chosen))
    } else {
        eprintln!("the scope chosen here is gone; showing everything");
        Ok(None)
    }
}

pub(crate) fn run(config: &Config, format: Format, command: ScopeCommand) -> Result<()> {
    let client = signed_in(config)?;
    match command {
        ScopeCommand::Add {
            title,
            only,
            except: _,
            inbox,
            members,
        } => {
            let snapshot = snapshot(&client)?;
            let members = members
                .iter()
                .map(|m| find_member(&snapshot, m))
                .collect::<Result<Vec<_>>>()?;
            let id = Id::new();
            let now = jiff::Timestamp::now().to_string();
            let scope = json!({
                "id": id,
                "created": now,
                "modified": now,
                "title": title,
                "mode": if only { "only" } else { "except" },
                "members": members,
                "inbox": inbox,
                "position": snapshot.scopes.len(),
            });
            write(
                &client,
                &[Change::Create {
                    object: id,
                    kind: ObjectKind::Scope,
                    value: scope,
                    hlc: placeholder(),
                }],
            )?;
            emit(format, &[json!({"id": id, "short": short(&id)})], &["id"])?;
        }
        ScopeCommand::List => {
            let snapshot = snapshot(&client)?;
            let chosen = config.chosen_scope()?;
            let records: Vec<Value> = snapshot
                .scopes
                .iter()
                .map(|s| {
                    let members: Vec<String> = s
                        .members
                        .iter()
                        .map(|m| describe(&snapshot, *m).1)
                        .collect();
                    json!({
                        "id": s.id,
                        "short": short(&s.id),
                        "chosen": if chosen == Some(s.id) { "*" } else { "" },
                        "title": s.title,
                        "mode": mode_name(s),
                        "inbox": if s.inbox { "inbox" } else { "" },
                        "color": s.color,
                        "members": members.join(", "),
                    })
                })
                .collect();
            emit(
                format,
                &records,
                &[
                    "chosen", "short", "title", "mode", "inbox", "members", "color",
                ],
            )?;
        }
        ScopeCommand::Show { scope } => {
            let snapshot = snapshot(&client)?;
            let scope = find_scope(&snapshot.scopes, &scope)?;
            let records: Vec<Value> = scope
                .members
                .iter()
                .map(|m| {
                    let (kind, title) = describe(&snapshot, *m);
                    json!({"id": m, "short": short(m), "kind": kind, "title": title})
                })
                .collect();
            let inbox = if scope.inbox { ", inbox ticked" } else { "" };
            eprintln!("{} ({}{inbox})", scope.title, mode_name(scope));
            emit(format, &records, &["short", "kind", "title"])?;
        }
        ScopeCommand::Rename { scope, title } => {
            let snapshot = snapshot(&client)?;
            let id = find_scope(&snapshot.scopes, &scope)?.id;
            write(&client, &[set(id, "title", json!(title))])?;
        }
        ScopeCommand::Mode { scope, mode } => {
            let snapshot = snapshot(&client)?;
            let id = find_scope(&snapshot.scopes, &scope)?.id;
            write(&client, &[set(id, "mode", json!(mode.name()))])?;
        }
        ScopeCommand::Inbox { scope, tick } => {
            let snapshot = snapshot(&client)?;
            let id = find_scope(&snapshot.scopes, &scope)?.id;
            write(&client, &[set(id, "inbox", json!(tick == Tick::On))])?;
        }
        ScopeCommand::Color {
            scope,
            color,
            clear,
        } => {
            let snapshot = snapshot(&client)?;
            let found = find_scope(&snapshot.scopes, &scope)?;
            match (color, clear) {
                (_, true) => write(&client, &[set(found.id, "color", Value::Null)])?,
                (Some(color), false) => {
                    write(&client, &[set(found.id, "color", json!(color.wire()))])?
                }
                (None, false) => {
                    emit(
                        format,
                        &[json!({"id": found.id, "color": found.color})],
                        &["color"],
                    )?;
                    return Ok(());
                }
            };
        }
        ScopeCommand::Layout {
            scope,
            inspector,
            note,
            clear,
        } => {
            let snapshot = snapshot(&client)?;
            let found = find_scope(&snapshot.scopes, &scope)?;
            if clear {
                write(&client, &[set(found.id, "layout", Value::Null)])?;
                return Ok(());
            }
            let current = found.layout.clone().unwrap_or_default();
            if inspector.is_none() && note.is_none() {
                let shown = current.inspector.as_ref();
                let parts: Vec<Value> = shown
                    .map(|i| i.shown.iter().map(|p| json!(p)).collect())
                    .unwrap_or_default();
                let parts: Vec<&str> = parts.iter().filter_map(Value::as_str).collect();
                emit(
                    format,
                    &[json!({
                        "id": found.id,
                        "inspector": parts.join(","),
                        "note": shown.and_then(|i| i.note),
                        "layout": found.layout,
                    })],
                    &["inspector", "note"],
                )?;
                return Ok(());
            }
            let mut inspector_layout = current.inspector.clone().unwrap_or_default();
            if let Some(names) = inspector {
                inspector_layout.shown = parts_of(&names)?;
            }
            if let Some(note) = note {
                inspector_layout.note = Some(note.wire());
            }
            // Other parts of the layout stay as they are (ADR 0072 §1).
            let mut layout: ScopeLayout = current;
            layout.inspector = Some(inspector_layout);
            write(&client, &[set(found.id, "layout", json!(layout))])?;
        }
        ScopeCommand::Capture {
            scope,
            project,
            tags,
            clear,
        } => {
            let snapshot = snapshot(&client)?;
            let found = find_scope(&snapshot.scopes, &scope)?;
            if !clear && project.is_none() && tags.is_empty() {
                let capture = found.capture.clone().unwrap_or_default();
                let project = capture
                    .project
                    .map(|p| describe(&snapshot, p).1)
                    .unwrap_or_default();
                let tags: Vec<String> = capture
                    .tags
                    .iter()
                    .map(|t| tag_path(&snapshot, *t))
                    .collect();
                emit(
                    format,
                    &[json!({"id": found.id, "project": project, "tags": tags.join(", ")})],
                    &["project", "tags"],
                )?;
                return Ok(());
            }
            let value = if clear {
                Value::Null
            } else {
                let project = project
                    .map(|name| {
                        let found =
                            by_name(&snapshot.projects, &name, |p| (p.title.as_str(), p.id));
                        let found: Vec<(Id, Id)> = found.iter().map(|p| (p.id, p.id)).collect();
                        one(&name, "project", &found)
                    })
                    .transpose()?;
                let tags = tags
                    .iter()
                    .map(|name| {
                        let found = by_path(&snapshot.tags, name, |t| {
                            (t.id, tag_path(&snapshot, t.id), t.title.as_str())
                        });
                        one(name, "tag", &found)
                    })
                    .collect::<Result<Vec<_>>>()?;
                serde_json::to_value(ScopeCapture { project, tags })?
            };
            write(&client, &[set(found.id, "capture", value)])?;
        }
        ScopeCommand::Schedule {
            scope,
            windows,
            clear,
        } => {
            let snapshot = snapshot(&client)?;
            let found = find_scope(&snapshot.scopes, &scope)?;
            if !clear && windows.is_empty() {
                let records: Vec<Value> = found
                    .schedule
                    .iter()
                    .map(|w| json!({"window": window_text(w), "days": w.days, "from": w.from, "to": w.to}))
                    .collect();
                emit(format, &records, &["window"])?;
                return Ok(());
            }
            let windows = windows
                .iter()
                .map(|w| parse_window(w))
                .collect::<Result<Vec<_>>>()?;
            write(
                &client,
                &[set(found.id, "schedule", serde_json::to_value(&windows)?)],
            )?;
        }
        ScopeCommand::Now => {
            let now: ScheduledScope =
                serde_json::from_value(client.call("GET", "/scopes/scheduled", None)?.json()?)?;
            let snapshot = snapshot(&client)?;
            let title = now
                .scope
                .and_then(|id| snapshot.scopes.iter().find(|s| s.id == id))
                .map(|s| s.title.clone())
                .unwrap_or_default();
            emit(
                format,
                &[json!({
                    "scope": now.scope,
                    "title": title,
                    "since": now.since,
                    "until": now.until,
                })],
                &["title", "since", "until"],
            )?;
        }
        ScopeCommand::Filter {
            scope,
            words,
            clear,
        } => {
            let snapshot = snapshot(&client)?;
            let found = find_scope(&snapshot.scopes, &scope)?;
            if !clear && words.is_empty() {
                let text = query::words(&found.filter, &snapshot.core());
                emit(
                    format,
                    &[json!({"id": found.id, "filter": text, "chips": found.filter})],
                    &["filter"],
                )?;
                return Ok(());
            }
            let chips = parse_chips(&snapshot, &words)?;
            write(
                &client,
                &[set(found.id, "filter", serde_json::to_value(&chips)?)],
            )?;
        }
        ScopeCommand::Member(command) => {
            let (scope, members, add) = match command {
                MemberCommand::Add { scope, members } => (scope, members, true),
                MemberCommand::Remove { scope, members } => (scope, members, false),
            };
            let snapshot = snapshot(&client)?;
            let scope = find_scope(&snapshot.scopes, &scope)?;
            let mut changes = Vec::new();
            for name in &members {
                // A member deleted since can still be taken off by its id.
                let id = match find_member(&snapshot, name) {
                    Ok(id) => id,
                    Err(_) if !add => scope
                        .members
                        .iter()
                        .copied()
                        .find(|m| crate::id_matches(m, &name.to_ascii_lowercase()))
                        .ok_or_else(|| Failure::NotFound(format!("no member {name:?}")))?,
                    Err(e) => return Err(e),
                };
                changes.push(member_change(scope.id, id, add));
            }
            write(&client, &changes)?;
        }
        ScopeCommand::Delete { scope } => {
            let snapshot = snapshot(&client)?;
            let id = find_scope(&snapshot.scopes, &scope)?.id;
            write(
                &client,
                &[Change::Delete {
                    object: id,
                    hlc: placeholder(),
                }],
            )?;
            if config.chosen_scope()? == Some(id) {
                config.choose_scope(None)?;
            }
        }
        ScopeCommand::Use { scope, none } => {
            if none {
                config.choose_scope(None)?;
            } else if scope
                .as_deref()
                .is_some_and(|s| s.eq_ignore_ascii_case("auto"))
            {
                config.follow_schedule()?;
                eprintln!("scope here: as the schedule says");
            } else if let Some(name) = scope {
                let snapshot = snapshot(&client)?;
                let found = find_scope(&snapshot.scopes, &name)?;
                config.choose_scope(Some(found.id))?;
                eprintln!("scope here: {}", found.title);
            } else if let Some(chosen) = config.chosen_scope()? {
                let snapshot = snapshot(&client)?;
                match snapshot.scopes.iter().find(|s| s.id == chosen) {
                    Some(s) => emit(
                        format,
                        &[json!({"id": s.id, "short": short(&s.id), "title": s.title})],
                        &["short", "title"],
                    )?,
                    None => eprintln!("the scope chosen here is gone"),
                }
            }
        }
    }
    Ok(())
}

/// How the note shows in the inspector (ADR 0072 §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum NoteArg {
    Default,
    Rendered,
}

impl NoteArg {
    fn wire(self) -> NoteMode {
        match self {
            Self::Default => NoteMode::Default,
            Self::Rendered => NoteMode::Rendered,
        }
    }
}

/// Parts of the inspector by their wire names; an unknown one is an error
/// that lists the known (ADR 0072 §2).
fn parts_of(names: &[String]) -> Result<Vec<InspectorPart>> {
    let mut parts = Vec::new();
    for name in names.iter().map(|n| n.trim()).filter(|n| !n.is_empty()) {
        let part: InspectorPart = serde_json::from_value(json!(name.to_lowercase()))?;
        if part == InspectorPart::Unknown {
            bail!(
                "{name:?} is no part of the inspector: properties, availability, state, \
                 project, priority, defer, due, planned, repeat, series, estimate, tags, note, \
                 links, reminders, clock, comments, attachments, logbook"
            );
        }
        if !parts.contains(&part) {
            parts.push(part);
        }
    }
    Ok(parts)
}

/// The palette of a scope (ADR 0060 §1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Color {
    Violet,
    Blue,
    Teal,
    Green,
    Yellow,
    Orange,
    Red,
    Pink,
}

impl Color {
    fn wire(self) -> &'static str {
        match self {
            Self::Violet => "violet",
            Self::Blue => "blue",
            Self::Teal => "teal",
            Self::Green => "green",
            Self::Yellow => "yellow",
            Self::Orange => "orange",
            Self::Red => "red",
            Self::Pink => "pink",
        }
    }
}
