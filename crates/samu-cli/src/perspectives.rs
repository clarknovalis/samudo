//! `samu perspective` (ADR 0090): saved views of the search, made and
//! changed over `/changes` like a scope. `samu list --perspective name`
//! shows what one holds, computed by samu-core on the server.

use anyhow::{Result, bail};
use clap::Subcommand;
use samu_api::{
    Change, Id, ObjectKind, Perspective, PerspectiveArrange, PerspectiveFilter, PerspectiveGroup,
    PerspectiveSort, PerspectiveView, ScopeChip,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::config::Config;
use crate::output::{Format, emit};
use crate::scopes::{DATE_WORDS, Snapshot, chip_word, parse_chip, snapshot, split_words};
use crate::{placeholder, short, signed_in, write};
use samu_client::{Client, Error as Failure};

#[derive(Debug, Subcommand)]
pub(crate) enum PerspectiveCommand {
    /// Every perspective with its search, filter and arrangement.
    List,
    /// Make a perspective from words of the search language, like
    /// `@Büro ~<15m angebot`: chips become chips, other words free text.
    /// A negated chip like `-@Warten` goes after `--`.
    Add {
        name: String,
        /// The search; several words are joined.
        words: Vec<String>,
        /// One emoji for the sidebar.
        #[arg(long)]
        icon: Option<String>,
        /// available, remaining or all.
        #[arg(long)]
        filter: Option<String>,
        /// project, folder, tag, state, priority, date or none.
        #[arg(long)]
        group: Option<String>,
        /// position, due, planned, defer, title, created, modified,
        /// estimate or priority.
        #[arg(long)]
        sort: Option<String>,
        /// Show it as a board.
        #[arg(long)]
        board: bool,
    },
    /// Replace a perspective's search: chips and free text together, as
    /// in `add`; without words it prints them.
    Search {
        name: String,
        #[arg(allow_hyphen_values = true, conflicts_with = "clear")]
        words: Vec<String>,
        /// No chips and no free text: everything in the scope.
        #[arg(long)]
        clear: bool,
    },
    /// Replace only the chips; a word that is no chip is refused.
    Chips {
        name: String,
        #[arg(allow_hyphen_values = true, required_unless_present = "clear")]
        words: Vec<String>,
        #[arg(long, conflicts_with = "words")]
        clear: bool,
    },
    /// Replace only the free text.
    Text {
        name: String,
        #[arg(allow_hyphen_values = true, required_unless_present = "clear")]
        words: Vec<String>,
        #[arg(long, conflicts_with = "words")]
        clear: bool,
    },
    /// available, remaining or all.
    Filter { name: String, filter: String },
    /// Group and sort; `--clear` goes back to project and own order.
    Arrange {
        name: String,
        #[arg(long, required_unless_present = "clear")]
        group: Option<String>,
        #[arg(long)]
        sort: Option<String>,
        /// Sort downward.
        #[arg(long)]
        desc: bool,
        /// A second key for what the first leaves equal.
        #[arg(long)]
        then: Option<String>,
        #[arg(long)]
        then_desc: bool,
        #[arg(long, conflicts_with_all = ["group", "sort", "desc", "then", "then_desc"])]
        clear: bool,
    },
    /// list or board.
    View { name: String, view: String },
    /// Give a perspective another name.
    Rename { name: String, title: String },
    /// Give a perspective an emoji, or take it off with `--none`.
    Icon {
        name: String,
        #[arg(required_unless_present = "none")]
        icon: Option<String>,
        #[arg(long, conflicts_with = "icon")]
        none: bool,
    },
    /// Move a perspective to a place in the sidebar, 1 the first.
    Move { name: String, place: usize },
    /// Delete a perspective; nothing it showed changes.
    Delete { name: String },
}

/// A perspective by its name (any case) or the end of its id.
pub(crate) fn find<'a>(perspectives: &'a [Perspective], name: &str) -> Result<&'a Perspective> {
    let lower = name.to_lowercase();
    if let Some(p) = perspectives
        .iter()
        .find(|p| p.title.to_lowercase() == lower)
    {
        return Ok(p);
    }
    let by_id: Vec<&Perspective> = perspectives
        .iter()
        .filter(|p| p.id.to_string().ends_with(&lower))
        .collect();
    match by_id.as_slice() {
        [one] => Ok(one),
        [] => Err(Failure::NotFound(format!("no perspective {name:?}")).into()),
        _ => bail!("{name:?} names several perspectives; take more of the id"),
    }
}

/// The id of the perspective `name` names, for `samu list --perspective`.
pub(crate) fn resolve(client: &Client, name: &str) -> Result<Id> {
    let snapshot = snapshot(client)?;
    Ok(find(&snapshot.perspectives, name)?.id)
}

/// A word the server knows for one of the enums, never a newer one.
fn word<T: DeserializeOwned + serde::Serialize>(text: &str, what: &str, known: &str) -> Result<T> {
    let parsed: Option<T> = serde_json::from_value(json!(text.to_lowercase())).ok();
    // `Unknown` is what serde makes of any other word: compare by name.
    match parsed {
        Some(value) if serde_json::to_value(&value)? == json!(text.to_lowercase()) => Ok(value),
        _ => bail!("{text:?} is no {what}: {known}"),
    }
}

const FILTERS: &str = "available, remaining or all";
const GROUPS: &str = "project, folder, tag, state, priority, date or none";
const SORTS: &str = "position, due, planned, defer, title, created, modified, estimate or priority";
const VIEWS: &str = "list or board";

fn filter_of(text: &str) -> Result<PerspectiveFilter> {
    word(text, "filter", FILTERS)
}

fn group_of(text: &str) -> Result<PerspectiveGroup> {
    word(text, "grouping", GROUPS)
}

fn sort_of(text: &str) -> Result<PerspectiveSort> {
    word(text, "sorting", SORTS)
}

/// Whether a word is meant as a chip: it starts with `#`, `@` or `~`, or
/// with a key the search language knows (ADR 0042 §2).
fn chip_like(word: &str) -> bool {
    let text = word
        .strip_prefix('-')
        .filter(|r| !r.is_empty())
        .unwrap_or(word);
    if text.starts_with(['#', '@', '~']) {
        return true;
    }
    text.split_once(':').is_some_and(|(key, _)| {
        let key = key.to_lowercase();
        DATE_WORDS.iter().any(|(k, _)| *k == key)
            || ["ist", "prio", "ordner", "zustand", "prop"].contains(&key.as_str())
    })
}

/// Words as chips and free text, names resolved to ids.
fn split_search(snapshot: &Snapshot, words: &[String]) -> Result<(Vec<ScopeChip>, String)> {
    let mut chips = Vec::new();
    let mut text = Vec::new();
    for word in words.iter().flat_map(|w| split_words(w)) {
        if chip_like(&word) {
            chips.push(parse_chip(snapshot, &word)?);
        } else {
            text.push(word);
        }
    }
    Ok((chips, text.join(" ")))
}

/// A perspective's search as the words that make it.
fn search_words(snapshot: &Snapshot, p: &Perspective) -> String {
    let mut words: Vec<String> = p.chips.iter().map(|c| chip_word(snapshot, c)).collect();
    if !p.text.is_empty() {
        words.push(p.text.clone());
    }
    words.join(" ")
}

fn name_of<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn arrange_words(arrange: Option<&PerspectiveArrange>) -> String {
    let a = arrange.cloned().unwrap_or_default();
    let arrow = |desc: bool| if desc { "↓" } else { "↑" };
    let mut text = format!(
        "{} · {} {}",
        name_of(&a.group),
        name_of(&a.sort),
        arrow(a.desc)
    );
    if let Some(then) = a.then {
        text.push_str(&format!(", {} {}", name_of(&then), arrow(a.then_desc)));
    }
    text
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

pub(crate) fn run(config: &Config, format: Format, command: PerspectiveCommand) -> Result<()> {
    let client = signed_in(config)?;
    let snapshot = snapshot(&client)?;
    let all = &snapshot.perspectives;
    match command {
        PerspectiveCommand::List => {
            let records: Vec<Value> = all
                .iter()
                .map(|p| {
                    json!({
                        "id": p.id,
                        "short": short(&p.id),
                        "icon": p.icon,
                        "title": p.title,
                        "search": search_words(&snapshot, p),
                        "filter": name_of(&p.filter),
                        "arrange": arrange_words(p.arrange.as_ref()),
                        "view": name_of(&p.view),
                        "perspective": p,
                    })
                })
                .collect();
            emit(
                format,
                &records,
                &[
                    "short", "icon", "title", "search", "filter", "arrange", "view",
                ],
            )?;
        }
        PerspectiveCommand::Add {
            name,
            words,
            icon,
            filter,
            group,
            sort,
            board,
        } => {
            if all
                .iter()
                .any(|p| p.title.to_lowercase() == name.to_lowercase())
            {
                bail!("there is a perspective {name:?} already");
            }
            let (chips, text) = split_search(&snapshot, &words)?;
            let arrange = if group.is_some() || sort.is_some() {
                Some(PerspectiveArrange {
                    group: group
                        .as_deref()
                        .map(group_of)
                        .transpose()?
                        .unwrap_or_default(),
                    sort: sort
                        .as_deref()
                        .map(sort_of)
                        .transpose()?
                        .unwrap_or_default(),
                    ..PerspectiveArrange::default()
                })
            } else {
                None
            };
            let position = all.iter().map(|p| p.position + 1).max().unwrap_or(0);
            let now = jiff::Timestamp::now();
            let perspective = Perspective {
                id: Id::new(),
                created: now,
                modified: now,
                title: name,
                icon,
                text,
                chips,
                filter: filter
                    .as_deref()
                    .map(filter_of)
                    .transpose()?
                    .unwrap_or_default(),
                arrange,
                view: if board {
                    PerspectiveView::Board
                } else {
                    PerspectiveView::List
                },
                board: None,
                position,
            };
            write(
                &client,
                &[Change::Create {
                    object: perspective.id,
                    kind: ObjectKind::Perspective,
                    value: serde_json::to_value(&perspective)?,
                    hlc: placeholder(),
                }],
            )?;
            emit(
                format,
                &[json!({"id": perspective.id, "title": perspective.title})],
                &["id"],
            )?;
        }
        PerspectiveCommand::Search { name, words, clear } => {
            let found = find(all, &name)?;
            if !clear && words.is_empty() {
                emit(
                    format,
                    &[json!({
                        "id": found.id,
                        "search": search_words(&snapshot, found),
                        "chips": found.chips,
                        "text": found.text,
                    })],
                    &["search"],
                )?;
                return Ok(());
            }
            let (chips, text) = split_search(&snapshot, &words)?;
            write(
                &client,
                &[
                    set(found.id, "chips", serde_json::to_value(&chips)?),
                    set(found.id, "text", json!(text)),
                ],
            )?;
        }
        PerspectiveCommand::Chips { name, words, clear } => {
            let found = find(all, &name)?;
            let chips = if clear {
                Vec::new()
            } else {
                words
                    .iter()
                    .flat_map(|w| split_words(w))
                    .map(|w| parse_chip(&snapshot, &w))
                    .collect::<Result<Vec<_>>>()?
            };
            write(
                &client,
                &[set(found.id, "chips", serde_json::to_value(&chips)?)],
            )?;
        }
        PerspectiveCommand::Text { name, words, clear } => {
            let found = find(all, &name)?;
            let text = if clear {
                String::new()
            } else {
                words.join(" ")
            };
            write(&client, &[set(found.id, "text", json!(text))])?;
        }
        PerspectiveCommand::Filter { name, filter } => {
            let found = find(all, &name)?;
            let filter = filter_of(&filter)?;
            write(
                &client,
                &[set(found.id, "filter", serde_json::to_value(filter)?)],
            )?;
        }
        PerspectiveCommand::Arrange {
            name,
            group,
            sort,
            desc,
            then,
            then_desc,
            clear,
        } => {
            let found = find(all, &name)?;
            let value = if clear {
                Value::Null
            } else {
                let sort = sort
                    .as_deref()
                    .map(sort_of)
                    .transpose()?
                    .unwrap_or_default();
                let then = then.as_deref().map(sort_of).transpose()?;
                serde_json::to_value(PerspectiveArrange {
                    group: group
                        .as_deref()
                        .map(group_of)
                        .transpose()?
                        .unwrap_or_default(),
                    sort,
                    desc,
                    // Ordnung is unique; a key twice says nothing (ADR 0064 §2).
                    then: then.filter(|t| sort != PerspectiveSort::Position && *t != sort),
                    then_desc,
                })?
            };
            write(&client, &[set(found.id, "arrange", value)])?;
        }
        PerspectiveCommand::View { name, view } => {
            let found = find(all, &name)?;
            let view: PerspectiveView = word(&view, "presentation", VIEWS)?;
            write(
                &client,
                &[set(found.id, "view", serde_json::to_value(view)?)],
            )?;
        }
        PerspectiveCommand::Rename { name, title } => {
            let found = find(all, &name)?;
            if title.trim().is_empty() {
                bail!("a perspective needs a name");
            }
            write(&client, &[set(found.id, "title", json!(title))])?;
        }
        PerspectiveCommand::Icon { name, icon, none } => {
            let found = find(all, &name)?;
            let value = icon.filter(|_| !none).map_or(Value::Null, |i| json!(i));
            write(&client, &[set(found.id, "icon", value)])?;
        }
        PerspectiveCommand::Move { name, place } => {
            let found = find(all, &name)?.id;
            if place == 0 {
                bail!("places count from 1");
            }
            let mut order: Vec<&Perspective> = all.iter().filter(|p| p.id != found).collect();
            let at = (place - 1).min(order.len());
            let moved = all.iter().find(|p| p.id == found);
            if let Some(moved) = moved {
                order.insert(at, moved);
            }
            // Only what changes is written; each position is its own field.
            let changes: Vec<Change> = order
                .iter()
                .zip(0_i64..)
                .filter(|(p, position)| p.position != *position)
                .map(|(p, position)| set(p.id, "position", json!(position)))
                .collect();
            if !changes.is_empty() {
                write(&client, &changes)?;
            }
        }
        PerspectiveCommand::Delete { name } => {
            let found = find(all, &name)?;
            write(
                &client,
                &[Change::Delete {
                    object: found.id,
                    hlc: placeholder(),
                }],
            )?;
        }
    }
    Ok(())
}
