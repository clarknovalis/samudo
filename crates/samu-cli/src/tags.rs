// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `samu tag …` and `samu state`: tags and workflows (ADR 0024). The
//! command line depends on samu-api only, so the few rules of a workflow
//! (which group, which order, the next state) are written out here once
//! more; web and Android take them from samu-core (`workflow.rs`), and the
//! tests below hold the two to the same answers.

use anyhow::{Result, bail};
use clap::{Subcommand, ValueEnum};
use clap_complete::engine::ArgValueCompleter;
use samu_api::{AnyObject, Change, ChangesPage, Id, ObjectKind, Project, Tag, TagStatus, Task};
use serde_json::{Value, json};

use crate::client::Client;
use crate::config::Config;
use crate::output::{Format, emit};
use crate::{placeholder, short, signed_in, write};
use samu_client::Error as Failure;

#[derive(Debug, Subcommand)]
pub(crate) enum TagCommand {
    /// Make a tag; with `--exclusive` a workflow, its children the states.
    /// `Ort/Büro` makes Büro under Ort (ADR 0027 §3).
    Add {
        title: String,
        /// The tag above, by path, title or the start or end of its id.
        #[arg(long)]
        parent: Option<String>,
        /// Its children exclude each other: a workflow (ADR 0024 §1).
        #[arg(long)]
        exclusive: bool,
        /// What carries it is not available (Someday, Waiting).
        #[arg(long)]
        on_hold: bool,
        /// One character to pick this state by.
        #[arg(long)]
        key: Option<String>,
        /// Ask for a note on entering this state.
        #[arg(long)]
        ask_note: bool,
        /// The workflow of projects that name none.
        #[arg(long)]
        default_workflow: bool,
    },
    /// Change what a tag is afterwards: a workflow or not, the default
    /// workflow, its status, and for a state its key and whether it asks
    /// for a note. Says on stderr whether it changed a workflow, a state of
    /// one, or a plain tag, since a group and its states have the same
    /// fields.
    Set {
        #[arg(add = ArgValueCompleter::new(crate::complete::tag_title))]
        tag: String,
        /// Its children exclude each other: a workflow (ADR 0024 §1).
        #[arg(long)]
        exclusive: Option<bool>,
        /// The workflow of projects that name none; the one before stops
        /// being it.
        #[arg(long)]
        default_workflow: Option<bool>,
        /// On hold: what carries it is not available (Someday, Waiting).
        #[arg(long, value_enum)]
        status: Option<SetStatus>,
        /// One character to pick this state by; `none` takes it off.
        #[arg(long)]
        key: Option<String>,
        /// Ask for a note on entering this state.
        #[arg(long)]
        ask_note: Option<bool>,
    },
    /// Every tag as a tree, with status, key and workflow marks.
    List,
    /// Give a tag a new title.
    Rename {
        #[arg(add = ArgValueCompleter::new(crate::complete::tag_title))]
        tag: String,
        title: String,
    },
    /// Hang a tag under another, or with `--top` at the top.
    Move {
        #[arg(add = ArgValueCompleter::new(crate::complete::tag_title))]
        tag: String,
        /// The new tag above, by title or the start or end of its id.
        #[arg(long, required_unless_present = "top", conflicts_with = "top")]
        parent: Option<String>,
        #[arg(long)]
        top: bool,
    },
    /// Take a tag off every task, project and scope, then delete it
    /// (ADR 0036 §4). Refused while it has children or is a project's
    /// workflow.
    Delete {
        #[arg(add = ArgValueCompleter::new(crate::complete::tag_title))]
        tag: String,
    },
}

/// The status `samu tag set` gives a tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum SetStatus {
    Active,
    OnHold,
    Dropped,
}

/// What `samu tag set` was asked to change.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct TagFields {
    pub(crate) exclusive: Option<bool>,
    pub(crate) default_workflow: Option<bool>,
    pub(crate) status: Option<SetStatus>,
    /// `Some(None)` takes the key off.
    pub(crate) key: Option<Option<String>>,
    pub(crate) ask_note: Option<bool>,
}

/// What a tag is, in the words of `samu tag set`'s message.
fn role(tags: &[Tag], tag: &Tag, exclusive: bool) -> String {
    if exclusive {
        return "the workflow".into();
    }
    match tag.parent.and_then(|p| tags.iter().find(|t| t.id == p)) {
        Some(group) if group.exclusive => format!("a state of {:?}", tag_path(tags, group.id)),
        _ => "the tag".into(),
    }
}

/// The changes `samu tag set` writes, and the lines it says about them.
/// Refuses what would leave a workflow half made: a default that is no
/// workflow, a workflow that projects use taken back, a key or a note on
/// what is no state.
pub(crate) fn tag_changes(
    snapshot: &Snapshot,
    id: Id,
    fields: &TagFields,
) -> Result<(Vec<Change>, Vec<String>)> {
    let tags = &snapshot.tags;
    let Some(tag) = tags.iter().find(|t| t.id == id) else {
        bail!("{id} is not a tag");
    };
    let path = tag_path(tags, id);
    let exclusive = fields.exclusive.unwrap_or(tag.exclusive);
    let is_state = tag
        .parent
        .and_then(|p| tags.iter().find(|t| t.id == p))
        .is_some_and(|g| g.exclusive);
    if *fields == TagFields::default() {
        bail!("nothing to set: --exclusive, --default-workflow, --status, --key or --ask-note");
    }
    let mut changes = Vec::new();
    let mut said = Vec::new();
    if let Some(on) = fields.exclusive
        && on != tag.exclusive
    {
        if !on {
            let using = snapshot
                .projects
                .iter()
                .filter(|p| p.workflow == Some(id))
                .count();
            if using > 0 {
                bail!(
                    "{path:?} is the workflow of {using} project(s); give them another first \
                     (samu project set <project> --workflow …)"
                );
            }
        }
        changes.push(set(id, "exclusive", json!(on)));
        said.push(format!(
            "{path:?} is {}",
            if on {
                "a workflow now, its children the states"
            } else {
                "no workflow any more"
            }
        ));
        if !on && tag.default_workflow && fields.default_workflow != Some(true) {
            changes.push(set(id, "default_workflow", json!(false)));
            said.push(format!("{path:?} is no longer the default workflow"));
        }
    }
    if let Some(on) = fields.default_workflow {
        if on && !exclusive {
            bail!("{path:?} is no workflow; make it one with --exclusive true");
        }
        if on != tag.default_workflow {
            changes.push(set(id, "default_workflow", json!(on)));
            said.push(format!(
                "{path:?} is {} the default workflow",
                if on { "now" } else { "no longer" }
            ));
        }
        if on {
            for other in tags.iter().filter(|t| t.default_workflow && t.id != id) {
                changes.push(set(other.id, "default_workflow", json!(false)));
                said.push(format!(
                    "{:?} is no longer the default workflow",
                    tag_path(tags, other.id)
                ));
            }
        }
    }
    if let Some(status) = fields.status {
        let wire = match status {
            SetStatus::Active => "active",
            SetStatus::OnHold => "on_hold",
            SetStatus::Dropped => "dropped",
        };
        changes.push(set(id, "status", json!(wire)));
        said.push(format!(
            "{}: {path:?} is {wire}",
            role(tags, tag, exclusive)
        ));
    }
    if (fields.key.is_some() || fields.ask_note.is_some()) && !is_state {
        bail!("{path:?} is no state: --key and --ask-note are for the children of a workflow");
    }
    if let Some(key) = &fields.key {
        if let Some(k) = key {
            if k.chars().count() != 1 {
                bail!("--key is one character");
            }
            if let Some(same) = tags.iter().find(|t| {
                t.id != id && t.parent == tag.parent && t.key.as_deref() == Some(k.as_str())
            }) {
                bail!("{k:?} is the key of {:?} already", tag_path(tags, same.id));
            }
        }
        changes.push(set(id, "key", json!(key)));
        said.push(match key {
            Some(k) => format!("{path:?} has the key {k:?}"),
            None => format!("{path:?} has no key"),
        });
    }
    if let Some(on) = fields.ask_note {
        changes.push(set(id, "ask_note", json!(on)));
        said.push(format!(
            "{path:?} {} for a note",
            if on { "asks" } else { "does not ask" }
        ));
    }
    if changes.is_empty() {
        said.push(format!("{path:?} is like that already"));
    }
    Ok((changes, said))
}

/// Where `samu state` takes a task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Target {
    Next,
    None,
    /// By title or key.
    Named(String),
}

pub(crate) struct Snapshot {
    pub(crate) tags: Vec<Tag>,
    pub(crate) tasks: Vec<Task>,
    pub(crate) projects: Vec<Project>,
}

pub(crate) fn snapshot(client: &Client) -> Result<Snapshot> {
    let page: ChangesPage =
        serde_json::from_value(client.call("GET", "/changes?after=0", None)?.json()?)?;
    let mut snapshot = Snapshot {
        tags: Vec::new(),
        tasks: Vec::new(),
        projects: Vec::new(),
    };
    for object in page.objects {
        match object {
            AnyObject::Tag(t) => snapshot.tags.push(t),
            AnyObject::Task(t) => snapshot.tasks.push(t),
            AnyObject::Project(p) => snapshot.projects.push(p),
            _ => {}
        }
    }
    Ok(snapshot)
}

/// A tag by its path (`Ort/Büro`, any case, ADR 0027 §3), its title or the
/// end of its id. The path comes first, so `Ort/Büro` names Büro under Ort
/// even while an old tag titled „Ort/Büro“ is still around; that one is
/// still found by its id.
pub(crate) fn find_tag<'a>(tags: &'a [Tag], name: &str) -> Result<&'a Tag> {
    let by_path: Vec<&Tag> = if name.contains('/') {
        tags.iter()
            .filter(|t| {
                t.parent.is_some() && tag_path(tags, t.id).to_lowercase() == name.to_lowercase()
            })
            .collect()
    } else {
        Vec::new()
    };
    let by_title: Vec<&Tag> = if by_path.is_empty() {
        tags.iter()
            .filter(|t| t.title.eq_ignore_ascii_case(name))
            .collect()
    } else {
        by_path
    };
    let found: Vec<&Tag> = if by_title.is_empty() {
        let wanted = name.to_ascii_lowercase();
        tags.iter()
            .filter(|t| crate::id_matches(&t.id, &wanted))
            .collect()
    } else {
        by_title
    };
    match found.as_slice() {
        [tag] => Ok(tag),
        [] => Err(Failure::NotFound(format!("no tag {name:?}")).into()),
        many => bail!(
            "{name:?} names {} tags; use the start or end of an id: {}",
            many.len(),
            many.iter()
                .map(|t| short(&t.id))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// A tag's path from the top, `Ort/Büro`.
pub(crate) fn tag_path(tags: &[Tag], id: Id) -> String {
    let mut parts = Vec::new();
    let mut current = Some(id);
    // A cycle in broken data must not loop forever (ADR 0017 §9).
    for _ in 0..=tags.len() {
        let Some(tag) = current.and_then(|c| tags.iter().find(|t| t.id == c)) else {
            break;
        };
        parts.push(tag.title.as_str());
        current = tag.parent;
    }
    parts.reverse();
    parts.join("/")
}

/// Where a new tag goes and its title: `a/b/c` is `c` under the tag at
/// `a/b`, below `parent` when one is given, as `@a/b/c` when capturing
/// (ADR 0027 §3). The steps above must be there; a title is never made of
/// a path, and two tags of one title side by side are refused, since
/// either would make a name or a path mean two tags.
fn placed(tags: &[Tag], title: &str, parent: Option<Id>) -> Result<(Option<Id>, String)> {
    let steps: Vec<&str> = title.split('/').map(str::trim).collect();
    let Some((leaf, above)) = steps.split_last() else {
        bail!("a tag needs a title");
    };
    if leaf.is_empty() || above.iter().any(|s| s.is_empty()) {
        bail!("{title:?}: a title, or a path like Ort/Büro without empty steps");
    }
    let mut at = parent;
    for step in above {
        let found: Vec<&Tag> = tags
            .iter()
            .filter(|t| t.parent == at && t.title.eq_ignore_ascii_case(step))
            .collect();
        at = match found.as_slice() {
            [one] => Some(one.id),
            [] => {
                let under = at
                    .map(|id| format!(" under {:?}", tag_path(tags, id)))
                    .unwrap_or_default();
                return Err(
                    Failure::NotFound(format!("no tag {step:?}{under}; make it first")).into(),
                );
            }
            _ => bail!(
                "{step:?} names more than one tag; use --parent with the start or end of an id"
            ),
        };
    }
    if let Some(same) = tags
        .iter()
        .find(|t| t.parent == at && t.title.eq_ignore_ascii_case(leaf))
    {
        bail!(
            "{:?} is there already ({})",
            tag_path(tags, same.id),
            short(&same.id)
        );
    }
    Ok((at, (*leaf).to_owned()))
}

pub(crate) fn workflow_of(project: Option<&Project>, tags: &[Tag]) -> Option<Id> {
    if let Some(own) = project.and_then(|p| p.workflow) {
        return Some(own);
    }
    tags.iter()
        .filter(|t| t.exclusive && t.default_workflow && t.status != TagStatus::Dropped)
        .min_by_key(|t| (t.position, t.id))
        .map(|t| t.id)
}

/// As `samu_core::workflow::states`.
pub(crate) fn states(group: Id, tags: &[Tag]) -> Vec<&Tag> {
    let mut children: Vec<&Tag> = tags
        .iter()
        .filter(|t| t.parent == Some(group) && t.status != TagStatus::Dropped)
        .collect();
    children.sort_by_key(|t| (t.position, t.id));
    children
}

/// As `samu_core::workflow::next_state`: round to none.
pub(crate) fn next_state(states: &[&Tag], current: Option<Id>) -> Option<Id> {
    match current.and_then(|c| states.iter().position(|s| s.id == c)) {
        None => states.first().map(|t| t.id),
        Some(i) => states.get(i + 1).map(|t| t.id),
    }
}

/// Puts a task into a state of its workflow; what it is in afterwards.
pub(crate) fn set_state(
    client: &Client,
    id: Id,
    target: &Target,
    note: Option<String>,
) -> Result<Value> {
    let snapshot = snapshot(client)?;
    let Some(task) = snapshot.tasks.iter().find(|t| t.id == id) else {
        bail!("{id} is not a task");
    };
    let project = task
        .project
        .and_then(|p| snapshot.projects.iter().find(|q| q.id == p));
    let Some(group) = workflow_of(project, &snapshot.tags) else {
        bail!(
            "no workflow: give the project one (samu set <project> workflow <tag>) \
             or mark a group with --default-workflow"
        );
    };
    let states = states(group, &snapshot.tags);
    let mut mine: Vec<_> = task
        .tags
        .iter()
        .filter(|t| states.iter().any(|s| s.id == t.tag))
        .collect();
    mine.sort_by_key(|t| t.position);
    let current = mine.first().map(|t| t.tag);
    let wanted = match target {
        Target::Next => next_state(&states, current),
        Target::None => None,
        Target::Named(name) => {
            let found = states.iter().find(|s| {
                s.title.eq_ignore_ascii_case(name) || s.key.as_deref() == Some(name.as_str())
            });
            match found {
                Some(state) => Some(state.id),
                None => bail!(
                    "{name:?} is not a state here; the states are: {}",
                    states
                        .iter()
                        .map(|s| s.title.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
        }
    };
    let change = match (wanted, current) {
        (Some(new), Some(old)) if new == old => None,
        (Some(new), _) => Some(Change::Add {
            object: id,
            field: "tags".into(),
            element: new,
            position: 0,
            note,
            hlc: placeholder(),
        }),
        (None, Some(old)) => Some(Change::Remove {
            object: id,
            field: "tags".into(),
            element: old,
            note,
            hlc: placeholder(),
        }),
        (None, None) => None,
    };
    if let Some(change) = change {
        write(client, &[change])?;
    }
    let title = wanted
        .and_then(|w| states.iter().find(|s| s.id == w))
        .map_or("", |s| s.title.as_str());
    Ok(json!({"state": title}))
}

pub(crate) fn run(config: &Config, format: Format, command: TagCommand) -> Result<()> {
    let client = signed_in(config)?;
    match command {
        TagCommand::Add {
            title,
            parent,
            exclusive,
            on_hold,
            key,
            ask_note,
            default_workflow,
        } => {
            if key.as_ref().is_some_and(|k| k.chars().count() != 1) {
                bail!("--key is one character");
            }
            let snapshot = snapshot(&client)?;
            let parent = parent
                .map(|p| find_tag(&snapshot.tags, &p).map(|t| t.id))
                .transpose()?;
            let (parent, title) = placed(&snapshot.tags, &title, parent)?;
            let position = snapshot.tags.iter().filter(|t| t.parent == parent).count();
            let id = Id::new();
            let now = jiff::Timestamp::now().to_string();
            let mut tag = json!({
                "id": id,
                "created": now,
                "modified": now,
                "title": title,
                "status": if on_hold { "on_hold" } else { "active" },
                "exclusive": exclusive,
                "ask_note": ask_note,
                "default_workflow": default_workflow,
                "position": position,
            });
            if let Some(parent) = parent {
                tag["parent"] = json!(parent);
            }
            if let Some(key) = key {
                tag["key"] = json!(key);
            }
            write(
                &client,
                &[Change::Create {
                    object: id,
                    kind: ObjectKind::Tag,
                    value: tag,
                    hlc: placeholder(),
                }],
            )?;
            emit(format, &[json!({"id": id, "short": short(&id)})], &["id"])?;
        }
        TagCommand::Set {
            tag,
            exclusive,
            default_workflow,
            status,
            key,
            ask_note,
        } => {
            let snapshot = snapshot(&client)?;
            let id = find_tag(&snapshot.tags, &tag)?.id;
            let fields = TagFields {
                exclusive,
                default_workflow,
                status,
                key: key.map(|k| (k != "none").then_some(k)),
                ask_note,
            };
            let (changes, said) = tag_changes(&snapshot, id, &fields)?;
            if !changes.is_empty() {
                write(&client, &changes)?;
            }
            for line in said {
                eprintln!("{line}");
            }
        }
        TagCommand::Rename { tag, title } => {
            if title.contains('/') {
                bail!(
                    "a title has no \"/\": it would read as a path; samu tag move hangs a tag elsewhere"
                );
            }
            let snapshot = snapshot(&client)?;
            let id = find_tag(&snapshot.tags, &tag)?.id;
            write(&client, &[set(id, "title", json!(title))])?;
        }
        TagCommand::Move { tag, parent, top } => {
            let snapshot = snapshot(&client)?;
            let id = find_tag(&snapshot.tags, &tag)?.id;
            let parent = match (parent, top) {
                (_, true) | (None, _) => None,
                (Some(p), false) => Some(find_tag(&snapshot.tags, &p)?.id),
            };
            if let Some(parent) = parent
                && below(&snapshot.tags, parent, id)
            {
                bail!("a tag cannot go under itself or its own children");
            }
            let position = snapshot
                .tags
                .iter()
                .filter(|t| t.parent == parent && t.id != id)
                .count();
            write(
                &client,
                &[
                    set(id, "parent", json!(parent)),
                    set(id, "position", json!(position)),
                ],
            )?;
        }
        TagCommand::Delete { tag } => {
            let snapshot = snapshot(&client)?;
            let id = find_tag(&snapshot.tags, &tag)?.id;
            // The server takes it off everything first (ADR 0036 §4).
            crate::delete_object(&client, id, &samu_api::DeleteRequest::default())?;
        }
        TagCommand::List => {
            let snapshot = snapshot(&client)?;
            let titles = crate::scopes::scope_titles(&client)?;
            let mut records = Vec::new();
            tree(&snapshot.tags, &titles, None, "", &mut records);
            emit(
                format,
                &records,
                &["short", "path", "status", "key", "marks"],
            )?;
        }
    }
    Ok(())
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

/// Whether `tag` is `ancestor` or hangs somewhere below it.
fn below(tags: &[Tag], tag: Id, ancestor: Id) -> bool {
    let mut current = Some(tag);
    // A cycle in broken data must not loop forever (ADR 0017 §9).
    for _ in 0..=tags.len() {
        match current {
            Some(id) if id == ancestor => return true,
            Some(id) => current = tags.iter().find(|t| t.id == id).and_then(|t| t.parent),
            None => return false,
        }
    }
    true
}

/// Depth first, by position: one record per tag with its path.
fn tree(
    tags: &[Tag],
    titles: &std::collections::HashMap<Id, String>,
    parent: Option<Id>,
    prefix: &str,
    records: &mut Vec<Value>,
) {
    let mut children: Vec<&Tag> = tags.iter().filter(|t| t.parent == parent).collect();
    children.sort_by_key(|t| (t.position, t.id));
    for tag in children {
        let path = format!("{prefix}{}", tag.title);
        let mut marks = Vec::new();
        if tag.exclusive {
            marks.push("workflow");
        }
        if tag.default_workflow {
            marks.push("default");
        }
        if tag.ask_note {
            marks.push("note");
        }
        // ADR 0094 §8.
        let only = crate::scopes::only_mark(tag.own_scope, titles);
        if let Some(only) = &only {
            marks.push(only);
        }
        records.push(json!({
            "id": tag.id,
            "short": short(&tag.id),
            "path": path,
            "status": tag.status,
            "key": tag.key,
            "marks": marks.join(","),
        }));
        // A cycle in broken data must not recurse forever (ADR 0017 §9).
        if records.len() <= tags.len() {
            tree(tags, titles, Some(tag.id), &format!("{path}/"), records);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::*;

    fn tag(title: &str, parent: Option<Id>, position: i64) -> Tag {
        Tag {
            scope_only: false,
            own_scope: None,
            id: Id::new(),
            created: "2026-11-07T08:00:00Z".parse().unwrap(),
            modified: "2026-11-07T08:00:00Z".parse().unwrap(),
            title: title.into(),
            parent,
            status: TagStatus::Active,
            exclusive: false,
            key: None,
            ask_note: false,
            default_workflow: false,
            position,
        }
    }

    /// The same answers as samu-core's workflow module.
    #[test]
    fn the_rules_match_samu_core() {
        let mut group = tag("Status", None, 0);
        group.exclusive = true;
        group.default_workflow = true;
        let a = tag("In Arbeit", Some(group.id), 0);
        let b = tag("Wartet", Some(group.id), 1);
        let mut gone = tag("Alt", Some(group.id), 2);
        gone.status = TagStatus::Dropped;
        let tags = vec![b.clone(), gone, group.clone(), a.clone()];
        assert_eq!(workflow_of(None, &tags), Some(group.id));
        let ordered: Vec<Id> = states(group.id, &tags).iter().map(|t| t.id).collect();
        assert_eq!(ordered, vec![a.id, b.id]);
        let states = states(group.id, &tags);
        assert_eq!(next_state(&states, None), Some(a.id));
        assert_eq!(next_state(&states, Some(a.id)), Some(b.id));
        assert_eq!(next_state(&states, Some(b.id)), None);
    }

    #[test]
    fn a_tag_is_found_by_title_or_the_end_of_its_id() {
        let tags = vec![tag("Wartet", None, 0), tag("Irgendwann", None, 1)];
        assert_eq!(find_tag(&tags, "wartet").unwrap().id, tags[0].id);
        let end = short(&tags[1].id);
        assert_eq!(find_tag(&tags, &end).unwrap().id, tags[1].id);
        assert!(find_tag(&tags, "nichts").is_err());
    }

    #[test]
    fn a_tag_never_goes_under_itself() {
        let a = tag("a", None, 0);
        let b = tag("b", Some(a.id), 0);
        let c = tag("c", None, 1);
        let tags = vec![a.clone(), b.clone(), c.clone()];
        assert!(below(&tags, a.id, a.id));
        assert!(below(&tags, b.id, a.id));
        assert!(!below(&tags, c.id, a.id));
        assert!(!below(&tags, a.id, b.id));
    }

    #[test]
    fn a_cycle_in_tags_does_not_recurse_forever() {
        let mut a = tag("a", None, 0);
        let mut b = tag("b", Some(a.id), 0);
        a.parent = Some(b.id);
        b.parent = Some(a.id);
        let root = tag("root", None, 0);
        let mut c = tag("c", Some(root.id), 0);
        c.parent = Some(root.id);
        let tags = vec![a, b, root, c];
        let mut records = Vec::new();
        tree(&tags, &Default::default(), None, "", &mut records);
        assert_eq!(records.len(), 2);
    }

    #[test]
    fn a_path_makes_the_tag_under_its_parent() {
        let ort = tag("Ort", None, 0);
        let buero = tag("Büro", Some(ort.id), 0);
        let tags = vec![ort.clone(), buero.clone()];
        // The finding of 08.11.2026: Ort/Büro was one tag at the top.
        assert_eq!(
            placed(&tags, "Ort/Zuhause", None).unwrap(),
            (Some(ort.id), "Zuhause".into())
        );
        assert_eq!(
            placed(&tags, "ort / Büro/Raum 3", None).unwrap(),
            (Some(buero.id), "Raum 3".into())
        );
        assert_eq!(
            placed(&tags, "Zuhause", Some(ort.id)).unwrap(),
            (Some(ort.id), "Zuhause".into())
        );
        assert_eq!(
            placed(&tags, "Büro/Raum", Some(ort.id)).unwrap(),
            (Some(buero.id), "Raum".into())
        );
        assert_eq!(placed(&tags, "Neu", None).unwrap(), (None, "Neu".into()));
        // What is missing above is not made; what is there is not made twice.
        assert!(placed(&tags, "Zeit/Abends", None).is_err());
        assert!(placed(&tags, "Ort/büro", None).is_err());
        assert!(placed(&tags, "Ort", None).is_err());
        for bad in ["", "Ort/", "/Büro", "Ort//Büro"] {
            assert!(placed(&tags, bad, None).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_tag_is_found_by_its_path_before_its_title() {
        let ort = tag("Ort", None, 0);
        let buero = tag("Büro", Some(ort.id), 0);
        let firma = tag("Firma", None, 1);
        let firma_buero = tag("Büro", Some(firma.id), 0);
        // What the old `samu tag add Ort/Büro` left behind.
        let old = tag("Ort/Büro", None, 2);
        let tags = vec![ort, buero.clone(), firma, firma_buero.clone(), old.clone()];
        assert_eq!(find_tag(&tags, "Ort/Büro").unwrap().id, buero.id);
        assert_eq!(find_tag(&tags, "firma/büro").unwrap().id, firma_buero.id);
        assert!(find_tag(&tags, "Büro").is_err(), "two of that title");
        assert_eq!(find_tag(&tags, &short(&old.id)).unwrap().id, old.id);
        assert_eq!(tag_path(&tags, firma_buero.id), "Firma/Büro");
    }

    fn changed(changes: &[Change]) -> Vec<(Id, String, Value)> {
        changes
            .iter()
            .map(|c| match c {
                Change::Set {
                    object,
                    field,
                    value,
                    ..
                } => (*object, field.clone(), value.clone()),
                other => panic!("only sets: {other:?}"),
            })
            .collect()
    }

    fn workflows() -> (Tag, Tag, Tag, Tag) {
        let mut status = tag("Status", None, 0);
        status.exclusive = true;
        status.default_workflow = true;
        let waiting = tag("Wartet", Some(status.id), 0);
        let mut dev = tag("Entwicklung", None, 1);
        dev.exclusive = true;
        let plain = tag("Büro", None, 2);
        (status, waiting, dev, plain)
    }

    fn snap(tags: Vec<Tag>, projects: Vec<Project>) -> Snapshot {
        Snapshot {
            tags,
            tasks: Vec::new(),
            projects,
        }
    }

    #[test]
    fn a_new_default_workflow_takes_over_from_the_old() {
        let (status, waiting, dev, plain) = workflows();
        let s = snap(
            vec![status.clone(), waiting, dev.clone(), plain],
            Vec::new(),
        );
        let fields = TagFields {
            default_workflow: Some(true),
            ..TagFields::default()
        };
        let (changes, said) = tag_changes(&s, dev.id, &fields).unwrap();
        assert_eq!(
            changed(&changes),
            vec![
                (dev.id, "default_workflow".into(), json!(true)),
                (status.id, "default_workflow".into(), json!(false)),
            ]
        );
        assert_eq!(said.len(), 2);
    }

    #[test]
    fn the_message_tells_a_workflow_from_its_state() {
        let (status, waiting, dev, plain) = workflows();
        let s = snap(
            vec![status.clone(), waiting.clone(), dev, plain.clone()],
            Vec::new(),
        );
        let hold = TagFields {
            status: Some(SetStatus::OnHold),
            ..TagFields::default()
        };
        let (changes, said) = tag_changes(&s, waiting.id, &hold).unwrap();
        assert_eq!(
            changed(&changes),
            vec![(waiting.id, "status".into(), json!("on_hold"))]
        );
        assert_eq!(
            said,
            vec![r#"a state of "Status": "Status/Wartet" is on_hold"#]
        );
        let (_, said) = tag_changes(&s, status.id, &hold).unwrap();
        assert_eq!(said, vec![r#"the workflow: "Status" is on_hold"#]);
        let (_, said) = tag_changes(&s, plain.id, &hold).unwrap();
        assert_eq!(said, vec![r#"the tag: "Büro" is on_hold"#]);
    }

    #[test]
    fn a_key_and_a_note_belong_to_states_only() {
        let (status, mut waiting, dev, plain) = workflows();
        waiting.key = Some("w".into());
        let next = tag("Später", Some(status.id), 1);
        let s = snap(
            vec![
                status.clone(),
                waiting.clone(),
                next.clone(),
                dev,
                plain.clone(),
            ],
            Vec::new(),
        );
        let key = |k: Option<&str>| TagFields {
            key: Some(k.map(str::to_owned)),
            ..TagFields::default()
        };
        assert!(tag_changes(&s, plain.id, &key(Some("b"))).is_err());
        assert!(tag_changes(&s, status.id, &key(Some("s"))).is_err());
        assert!(
            tag_changes(&s, next.id, &key(Some("w"))).is_err(),
            "taken by Wartet"
        );
        assert!(tag_changes(&s, next.id, &key(Some("sp"))).is_err());
        let (changes, _) = tag_changes(&s, next.id, &key(Some("s"))).unwrap();
        assert_eq!(changed(&changes), vec![(next.id, "key".into(), json!("s"))]);
        let (changes, _) = tag_changes(&s, waiting.id, &key(None)).unwrap();
        assert_eq!(
            changed(&changes),
            vec![(waiting.id, "key".into(), Value::Null)]
        );
        let note = TagFields {
            ask_note: Some(true),
            ..TagFields::default()
        };
        assert!(tag_changes(&s, plain.id, &note).is_err());
        let (changes, _) = tag_changes(&s, waiting.id, &note).unwrap();
        assert_eq!(
            changed(&changes),
            vec![(waiting.id, "ask_note".into(), json!(true))]
        );
    }

    #[test]
    fn a_workflow_in_use_stays_one() {
        let (status, waiting, dev, plain) = workflows();
        let project: Project = serde_json::from_value(json!({
            "id": Id::new(),
            "created": "2026-11-07T08:00:00Z",
            "modified": "2026-11-07T08:00:00Z",
            "title": "samudo",
            "kind": "parallel",
            "status": "active",
            "workflow": dev.id,
        }))
        .unwrap();
        let s = snap(
            vec![status.clone(), waiting, dev.clone(), plain.clone()],
            vec![project],
        );
        let off = TagFields {
            exclusive: Some(false),
            ..TagFields::default()
        };
        assert!(tag_changes(&s, dev.id, &off).is_err());
        // The default stops being one with it.
        let (changes, _) = tag_changes(&s, status.id, &off).unwrap();
        assert_eq!(
            changed(&changes),
            vec![
                (status.id, "exclusive".into(), json!(false)),
                (status.id, "default_workflow".into(), json!(false)),
            ]
        );
        let default = TagFields {
            default_workflow: Some(true),
            ..TagFields::default()
        };
        assert!(tag_changes(&s, plain.id, &default).is_err(), "no workflow");
        assert!(
            tag_changes(&s, plain.id, &TagFields::default()).is_err(),
            "nothing"
        );
        let on = TagFields {
            exclusive: Some(true),
            default_workflow: Some(true),
            ..TagFields::default()
        };
        let (changes, _) = tag_changes(&s, plain.id, &on).unwrap();
        assert_eq!(changes.len(), 3, "exclusive, default, the old default off");
    }

    /// The cases samu-core and the web must give as well; the command
    /// line says its refusals in words, so only whether it refuses counts.
    #[test]
    fn the_shared_cases_hold() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testdata/tagfields/cases.json"
        );
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut names = std::collections::HashMap::new();
        let mut tags = Vec::new();
        for t in doc["tags"].as_array().unwrap() {
            let parent = t["parent"].as_str().map(|p| names[p]);
            let mut made = tag(
                t["title"].as_str().unwrap(),
                parent,
                t["position"].as_i64().unwrap(),
            );
            made.exclusive = t["exclusive"].as_bool().unwrap_or(false);
            made.default_workflow = t["default_workflow"].as_bool().unwrap_or(false);
            made.key = t["key"].as_str().map(str::to_owned);
            names.insert(t["id"].as_str().unwrap().to_owned(), made.id);
            tags.push(made);
        }
        let projects = doc["projects"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| {
                serde_json::from_value(json!({
                    "id": Id::new(),
                    "created": "2026-11-19T08:00:00Z",
                    "modified": "2026-11-19T08:00:00Z",
                    "title": "p",
                    "kind": "parallel",
                    "status": "active",
                    "workflow": names[p["workflow"].as_str().unwrap()],
                }))
                .unwrap()
            })
            .collect();
        let s = snap(tags, projects);
        for case in doc["cases"].as_array().unwrap() {
            let what = &case["what"];
            let f = &case["fields"];
            let fields = TagFields {
                exclusive: f["exclusive"].as_bool(),
                default_workflow: f["default_workflow"].as_bool(),
                status: f["status"].as_str().map(|s| match s {
                    "active" => SetStatus::Active,
                    "on_hold" => SetStatus::OnHold,
                    _ => SetStatus::Dropped,
                }),
                key: f.get("key").map(|k| k.as_str().map(str::to_owned)),
                ask_note: f["ask_note"].as_bool(),
            };
            let got = tag_changes(&s, names[case["tag"].as_str().unwrap()], &fields);
            if case["error"].is_string() {
                assert!(got.is_err(), "{what}");
                continue;
            }
            let want: Vec<(Id, String, Value)> = case["changes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| {
                    (
                        names[c[0].as_str().unwrap()],
                        c[1].as_str().unwrap().to_owned(),
                        c[2].clone(),
                    )
                })
                .collect();
            assert_eq!(changed(&got.unwrap().0), want, "{what}");
        }
    }
}
