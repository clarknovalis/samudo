// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `samu project …` and `samu folder …`: make and rename them (ADR 0036
//! §7). Deleting goes through `samu delete`, which the server composes.

use anyhow::{Result, bail};
use clap::{Subcommand, ValueEnum};
use clap_complete::engine::ArgValueCompleter;
use samu_api::{
    AnyObject, Change, ChangesPage, Folder, Id, ObjectKind, Project, Tag, ToProjectRequest,
    ToProjectResponse,
};
use samu_client::{Client, Error as Failure};
use serde_json::{Value, json};

use crate::config::Config;
use crate::output::{Format, emit};
use crate::{placeholder, resolve_project, short, signed_in, write};

#[derive(Debug, Subcommand)]
pub(crate) enum ProjectCommand {
    /// Make a project, last at its level.
    Add {
        title: String,
        /// The folder it goes in, by title or the start or end of its id; the top
        /// without one.
        #[arg(long)]
        folder: Option<String>,
        #[arg(long, value_enum, default_value_t = Kind::Parallel)]
        kind: Kind,
    },
    /// Give a project a new title.
    Rename {
        #[arg(add = ArgValueCompleter::new(crate::complete::project))]
        project: String,
        title: String,
    },
    /// Put a project into a folder, or with `--top` at the top, last there
    /// (ADR 0037 §1).
    Move {
        #[arg(add = ArgValueCompleter::new(crate::complete::project))]
        project: String,
        /// The folder, by title or the start or end of its id.
        #[arg(long, required_unless_present = "top", conflicts_with = "top")]
        folder: Option<String>,
        #[arg(long)]
        top: bool,
    },
    /// Turn a task into a project (ADR 0126): the project takes its title,
    /// note, dates, flag and tags, its children go on top; the task stays,
    /// with its comments, clocks and links, last in the project.
    FromTask {
        #[arg(add = ArgValueCompleter::new(crate::complete::task))]
        task: String,
        /// The folder, by title or the start or end of its id; else the folder of
        /// the task's project.
        #[arg(long)]
        folder: Option<String>,
        /// Put before the title of the task that stays; not for a task with
        /// the property `source`, whose title a sync writes.
        #[arg(long, default_value = "Notizen: ", conflicts_with = "keep_title")]
        prefix: String,
        /// Leave the task's title as it is.
        #[arg(long)]
        keep_title: bool,
        /// Say what would happen; write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Set what a project is (ADR 0037 §3).
    Set {
        #[arg(add = ArgValueCompleter::new(crate::complete::project))]
        project: String,
        #[arg(long, value_enum)]
        kind: Option<Kind>,
        #[arg(long, value_enum)]
        status: Option<Status>,
        /// An exclusive tag group by title or the start or end of its id; `none` for
        /// the default.
        #[arg(long)]
        workflow: Option<String>,
        /// How often it comes up for review: `1w`, `2w`, `1mo`, `3mo` or
        /// `none`.
        #[arg(long)]
        review: Option<String>,
    },
    /// A project's icon, an emoji (ADR 0059): without one, print it;
    /// `--clear` takes it off.
    Icon {
        #[arg(add = ArgValueCompleter::new(crate::complete::project))]
        project: String,
        #[arg(conflicts_with = "clear")]
        icon: Option<String>,
        #[arg(long)]
        clear: bool,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum Status {
    Active,
    OnHold,
    Completed,
    Dropped,
}

impl Status {
    fn wire(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::OnHold => "on_hold",
            Self::Completed => "completed",
            Self::Dropped => "dropped",
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum Kind {
    Parallel,
    Sequential,
    SingleActions,
}

impl Kind {
    fn wire(self) -> &'static str {
        match self {
            Self::Parallel => "parallel",
            Self::Sequential => "sequential",
            Self::SingleActions => "single_actions",
        }
    }
}

#[derive(Debug, Subcommand)]
pub(crate) enum FolderCommand {
    /// Make a folder, last at its level.
    Add {
        title: String,
        /// The folder above, by title or the start or end of its id.
        #[arg(long)]
        parent: Option<String>,
    },
    /// Give a folder a new title.
    Rename { folder: String, title: String },
    /// A folder's icon, an emoji (ADR 0059): without one, print it;
    /// `--clear` takes it off.
    Icon {
        folder: String,
        #[arg(conflicts_with = "clear")]
        icon: Option<String>,
        #[arg(long)]
        clear: bool,
    },
    /// Hang a folder into another, or with `--top` at the top, last there.
    /// Never below itself (ADR 0037 §2).
    Move {
        folder: String,
        /// The folder above, by title or the start or end of its id.
        #[arg(long, required_unless_present = "top", conflicts_with = "top")]
        parent: Option<String>,
        #[arg(long)]
        top: bool,
    },
    /// The folders the views show, as a tree (GET /available/folders,
    /// ADR 0037 §4): through the scope chosen here unless told otherwise.
    List {
        /// Through this scope instead of the one chosen here.
        #[arg(long, conflicts_with = "no_scope")]
        scope: Option<String>,
        /// Without a scope, whatever is chosen here.
        #[arg(long)]
        no_scope: bool,
    },
}

struct Places {
    folders: Vec<Folder>,
    projects: Vec<Project>,
    tags: Vec<Tag>,
}

fn places(client: &Client) -> Result<Places> {
    let page: ChangesPage =
        serde_json::from_value(client.call("GET", "/changes?after=0", None)?.json()?)?;
    let mut places = Places {
        folders: Vec::new(),
        projects: Vec::new(),
        tags: Vec::new(),
    };
    for object in page.objects {
        match object {
            AnyObject::Folder(f) => places.folders.push(f),
            AnyObject::Project(p) => places.projects.push(p),
            AnyObject::Tag(t) => places.tags.push(t),
            _ => {}
        }
    }
    Ok(places)
}

/// A folder by its title (any case) or the start or end of its id.
fn find_folder<'a>(folders: &'a [Folder], name: &str) -> Result<&'a Folder> {
    let by_title: Vec<&Folder> = folders
        .iter()
        .filter(|f| f.title.eq_ignore_ascii_case(name))
        .collect();
    let found: Vec<&Folder> = if by_title.is_empty() {
        let wanted = name.to_ascii_lowercase();
        folders
            .iter()
            .filter(|f| crate::id_matches(&f.id, &wanted))
            .collect()
    } else {
        by_title
    };
    match found.as_slice() {
        [folder] => Ok(folder),
        [] => Err(Failure::NotFound(format!("no folder {name:?}")).into()),
        many => bail!(
            "{name:?} names {} folders; use the start or end of an id: {}",
            many.len(),
            many.iter()
                .map(|f| short(&f.id))
                .collect::<Vec<_>>()
                .join(", ")
        ),
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

fn rename(client: &Client, id: Id, title: &str) -> Result<()> {
    write(client, &[set(id, "title", json!(title))])?;
    Ok(())
}

/// Print an icon, or set or clear it (ADR 0059).
fn icon(
    client: &Client,
    format: Format,
    id: Id,
    current: Option<&str>,
    icon: Option<String>,
    clear: bool,
) -> Result<()> {
    match (icon, clear) {
        (_, true) => write(client, &[set(id, "icon", Value::Null)]).map(|_| ()),
        (Some(icon), false) => {
            let icon = icon.trim();
            if icon.is_empty() || icon.len() > 32 {
                bail!("an icon is an emoji, 1 to 32 bytes, not {icon:?}");
            }
            write(client, &[set(id, "icon", json!(icon))]).map(|_| ())
        }
        (None, false) => emit(
            format,
            &[json!({"id": id, "icon": current.unwrap_or_default()})],
            &["icon"],
        ),
    }
}

fn id_or_null(id: Option<Id>) -> Value {
    id.map_or(Value::Null, |id| json!(id))
}

/// `1w`, `2w`, `1mo`, `3mo` as ISO 8601 spans; `none` clears.
fn review_value(text: &str) -> Result<Value> {
    Ok(match text {
        "none" => Value::Null,
        "1w" => json!("P1W"),
        "2w" => json!("P2W"),
        "1mo" => json!("P1M"),
        "3mo" => json!("P3M"),
        other => bail!("--review is 1w, 2w, 1mo, 3mo or none, not {other:?}"),
    })
}

fn create(client: &Client, format: Format, kind: ObjectKind, value: Value) -> Result<()> {
    let id: Id = serde_json::from_value(value["id"].clone())?;
    write(
        client,
        &[Change::Create {
            object: id,
            kind,
            value,
            hlc: placeholder(),
        }],
    )?;
    emit(format, &[json!({"id": id, "short": short(&id)})], &["id"])
}

fn next_position(positions: impl Iterator<Item = i64>) -> i64 {
    positions.max().map_or(0, |p| p.saturating_add(1))
}

pub(crate) fn project(config: &Config, format: Format, command: ProjectCommand) -> Result<()> {
    let client = signed_in(config)?;
    match command {
        ProjectCommand::Add {
            title,
            folder,
            kind,
        } => {
            let places = places(&client)?;
            let folder = folder
                .map(|f| find_folder(&places.folders, &f).map(|f| f.id))
                .transpose()?;
            let position = next_position(
                places
                    .projects
                    .iter()
                    .filter(|p| p.folder == folder)
                    .map(|p| p.position),
            );
            let now = jiff::Timestamp::now().to_string();
            let mut value = json!({
                "id": Id::new(),
                "created": now,
                "modified": now,
                "title": title,
                "kind": kind.wire(),
                "status": "active",
                "flagged": false,
                "position": position,
            });
            if let Some(folder) = folder {
                value["folder"] = json!(folder);
            }
            create(&client, format, ObjectKind::Project, value)
        }
        ProjectCommand::FromTask {
            task,
            folder,
            prefix,
            keep_title,
            dry_run,
        } => {
            let id = crate::resolve(&client, &task)?;
            let folder = match folder {
                Some(f) => Some(find_folder(&places(&client)?.folders, &f)?.id),
                None => None,
            };
            let request = ToProjectRequest {
                folder,
                prefix: (!keep_title).then_some(prefix),
                preview: dry_run,
            };
            let answer = client.call(
                "POST",
                &format!("/objects/{id}/to-project"),
                Some(&serde_json::to_value(&request)?),
            )?;
            let answer: ToProjectResponse = serde_json::from_value(answer.json()?)?;
            emit(
                format,
                &[json!({
                    "id": answer.project,
                    "short": short(&answer.project),
                    "moved": answer.moved,
                    "seq": answer.seq,
                })],
                &["id", "moved"],
            )
        }
        ProjectCommand::Rename { project, title } => {
            let id = resolve_project(&client, &project)?;
            rename(&client, id, &title)
        }
        ProjectCommand::Icon {
            project,
            icon: new,
            clear,
        } => {
            let id = resolve_project(&client, &project)?;
            let places = places(&client)?;
            let current = places
                .projects
                .iter()
                .find(|p| p.id == id)
                .and_then(|p| p.icon.clone());
            icon(&client, format, id, current.as_deref(), new, clear)
        }
        ProjectCommand::Move {
            project,
            folder,
            top,
        } => {
            let id = resolve_project(&client, &project)?;
            let places = places(&client)?;
            let folder = match (folder, top) {
                (_, true) | (None, _) => None,
                (Some(f), false) => Some(find_folder(&places.folders, &f)?.id),
            };
            let position = next_position(
                places
                    .projects
                    .iter()
                    .filter(|p| p.folder == folder && p.id != id)
                    .map(|p| p.position),
            );
            write(
                &client,
                &[
                    set(id, "folder", id_or_null(folder)),
                    set(id, "position", json!(position)),
                ],
            )?;
            Ok(())
        }
        ProjectCommand::Set {
            project,
            kind,
            status,
            workflow,
            review,
        } => {
            let id = resolve_project(&client, &project)?;
            let mut changes = Vec::new();
            if let Some(kind) = kind {
                changes.push(set(id, "kind", json!(kind.wire())));
            }
            if let Some(status) = status {
                // Done and dropped carry their moment, like `samu done`
                // (ADR 0037 §3); back to open clears both.
                let now = json!(jiff::Timestamp::now().to_string());
                let (completed, dropped) = match status {
                    Status::Completed => (now, Value::Null),
                    Status::Dropped => (Value::Null, now),
                    Status::Active | Status::OnHold => (Value::Null, Value::Null),
                };
                changes.push(set(id, "status", json!(status.wire())));
                changes.push(set(id, "completed_at", completed));
                changes.push(set(id, "dropped_at", dropped));
            }
            if let Some(workflow) = workflow {
                let value = if workflow == "none" {
                    Value::Null
                } else {
                    let places = places(&client)?;
                    json!(crate::tags::find_tag(&places.tags, &workflow)?.id)
                };
                changes.push(set(id, "workflow", value));
            }
            if let Some(review) = review {
                changes.push(set(id, "review_interval", review_value(&review)?));
            }
            if changes.is_empty() {
                bail!("nothing to set: --kind, --status, --workflow or --review");
            }
            write(&client, &changes)?;
            Ok(())
        }
    }
}

pub(crate) fn folder(config: &Config, format: Format, command: FolderCommand) -> Result<()> {
    let client = signed_in(config)?;
    match command {
        FolderCommand::Add { title, parent } => {
            let places = places(&client)?;
            let parent = parent
                .map(|f| find_folder(&places.folders, &f).map(|f| f.id))
                .transpose()?;
            let position = next_position(
                places
                    .folders
                    .iter()
                    .filter(|f| f.parent == parent)
                    .map(|f| f.position),
            );
            let now = jiff::Timestamp::now().to_string();
            let mut value = json!({
                "id": Id::new(),
                "created": now,
                "modified": now,
                "title": title,
                "status": "active",
                "position": position,
            });
            if let Some(parent) = parent {
                value["parent"] = json!(parent);
            }
            create(&client, format, ObjectKind::Folder, value)
        }
        FolderCommand::Rename { folder, title } => {
            let places = places(&client)?;
            let id = find_folder(&places.folders, &folder)?.id;
            rename(&client, id, &title)
        }
        FolderCommand::Icon {
            folder,
            icon: new,
            clear,
        } => {
            let places = places(&client)?;
            let found = find_folder(&places.folders, &folder)?;
            icon(&client, format, found.id, found.icon.as_deref(), new, clear)
        }
        FolderCommand::Move {
            folder,
            parent,
            top,
        } => {
            let places = places(&client)?;
            let id = find_folder(&places.folders, &folder)?.id;
            let parent = match (parent, top) {
                (_, true) | (None, _) => None,
                (Some(f), false) => Some(find_folder(&places.folders, &f)?.id),
            };
            let position = next_position(
                places
                    .folders
                    .iter()
                    .filter(|f| f.parent == parent && f.id != id)
                    .map(|f| f.position),
            );
            // The server refuses a folder below itself (ADR 0037 §2).
            write(
                &client,
                &[
                    set(id, "parent", id_or_null(parent)),
                    set(id, "position", json!(position)),
                ],
            )?;
            Ok(())
        }
        FolderCommand::List { scope, no_scope } => {
            let scope = crate::scopes::query_scope(&client, config, scope.as_deref(), no_scope)?;
            let path = scope.map_or_else(
                || "/available/folders".to_string(),
                |s| format!("/available/folders?scope={s}"),
            );
            let shown: Vec<Folder> =
                serde_json::from_value(client.call("GET", &path, None)?.json()?)?;
            let ids: std::collections::HashSet<Id> = shown.iter().map(|f| f.id).collect();
            // A folder whose parent the scope hides stands at the top.
            let rooted: Vec<Folder> = shown
                .iter()
                .cloned()
                .map(|mut f| {
                    if f.parent.is_some_and(|p| !ids.contains(&p)) {
                        f.parent = None;
                    }
                    f
                })
                .collect();
            let titles = crate::scopes::scope_titles(&client)?;
            let mut records = Vec::new();
            tree(&rooted, &titles, None, "", &mut records, 0);
            emit(format, &records, &["short", "path", "status", "marks"])
        }
    }
}

/// Depth first, by position: one record per folder with its path. A cycle
/// in broken data ends at the depth of the whole list.
fn tree(
    folders: &[Folder],
    titles: &std::collections::HashMap<Id, String>,
    parent: Option<Id>,
    prefix: &str,
    records: &mut Vec<Value>,
    depth: usize,
) {
    if depth > folders.len() {
        return;
    }
    let mut children: Vec<&Folder> = folders.iter().filter(|f| f.parent == parent).collect();
    children.sort_by_key(|f| (f.position, f.id));
    for folder in children {
        let path = format!("{prefix}{}", folder.title);
        records.push(json!({
            "id": folder.id,
            "short": short(&folder.id),
            "path": path,
            "status": folder.status,
            // ADR 0094 §8.
            "marks": crate::scopes::only_mark(folder.own_scope, titles).unwrap_or_default(),
        }));
        tree(
            folders,
            titles,
            Some(folder.id),
            &format!("{path}/"),
            records,
            depth + 1,
        );
    }
}
