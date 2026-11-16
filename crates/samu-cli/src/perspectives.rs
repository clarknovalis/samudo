//! `samu perspective` (ADR 0090): saved views of the search, made and
//! changed over `/changes` like a scope. `samu list --perspective name`
//! shows what one holds, computed by samu-core on the server.

use anyhow::{Result, bail};
use clap::Subcommand;
use samu_api::{
    Change, Id, ObjectKind, Perspective, PerspectiveArrange, PerspectiveBlock, PerspectiveFilter,
    PerspectiveGroup, PerspectiveSort, PerspectiveTable, PerspectiveView, ScopeChip,
};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::config::Config;
use crate::output::{Format, emit};
use crate::scopes::{Snapshot, chip_word, parse_chips, refused, snapshot};
use crate::{placeholder, short, signed_in, write};
use samu_client::{Client, Error as Failure};
use samu_core::query::{self, ParseError};

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
        #[arg(long, conflicts_with = "table")]
        board: bool,
        /// Show it as a table (ADR 0115).
        #[arg(long)]
        table: bool,
        /// The table's columns, like `state,due,worked,prop:jira`; the
        /// default ones without.
        #[arg(long, requires = "table")]
        columns: Option<String>,
        /// Make a block perspective (ADR 0116) of these blocks, as in
        /// `samu perspective blocks`.
        #[arg(
            long,
            num_args = 1..,
            conflicts_with_all = ["words", "filter", "group", "sort", "board", "table"]
        )]
        blocks: Vec<String>,
    },
    /// Set the blocks of a block perspective (ADR 0116): each a
    /// perspective's name, or a fixed view as `:inbox`, `:today` or
    /// `:flagged`; `=Heading` after it gives a heading of its own, like
    /// `Büro=Kurz im Büro`. Without blocks it prints them.
    Blocks {
        name: String,
        #[arg(conflicts_with = "clear")]
        blocks: Vec<String>,
        /// No blocks: a plain perspective again.
        #[arg(long)]
        clear: bool,
    },
    /// on: a task several blocks show stands only in the first; off: in
    /// every one, as in org.
    Dedupe { name: String, value: String },
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
    /// list, board or table; `--columns` sets the table's columns, which
    /// stay while it shows a list or board. Without a value the default
    /// ones.
    View {
        name: String,
        view: String,
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        columns: Option<String>,
    },
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

/// The perspective `name` names, for `samu list --perspective`.
pub(crate) fn resolve(client: &Client, name: &str) -> Result<Perspective> {
    let snapshot = snapshot(client)?;
    Ok(find(&snapshot.perspectives, name)?.clone())
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
const VIEWS: &str = "list, board or table";

fn filter_of(text: &str) -> Result<PerspectiveFilter> {
    word(text, "filter", FILTERS)
}

fn group_of(text: &str) -> Result<PerspectiveGroup> {
    word(text, "grouping", GROUPS)
}

fn sort_of(text: &str) -> Result<PerspectiveSort> {
    word(text, "sorting", SORTS)
}

/// Words as chips and free text, names resolved to ids: a word samu-core
/// takes for no chip at all stays text, a chip that does not hold (an
/// unknown tag, a wrong value) is refused.
fn split_search(snapshot: &Snapshot, words: &[String]) -> Result<(Vec<ScopeChip>, String)> {
    let mut chips: Vec<ScopeChip> = Vec::new();
    let mut text = Vec::new();
    for word in words.iter().flat_map(|w| query::split_words(w)) {
        match query::parse_chip(&word, &snapshot.core()) {
            Err(ParseError::Free(_)) => text.push(word),
            Err(e) => return Err(refused(&word, e)),
            Ok(chip) if !chips.contains(&chip) => chips.push(chip),
            Ok(_) => {}
        }
    }
    Ok((chips, text.join(" ")))
}

/// A block in the words of `samu perspective blocks`.
fn block_word(perspectives: &[Perspective], block: &PerspectiveBlock) -> String {
    let name = match (&block.perspective, &block.view) {
        (Some(id), _) => perspectives
            .iter()
            .find(|p| p.id == *id)
            .map_or_else(|| short(id), |p| p.title.clone()),
        (None, Some(view)) => format!(":{view}"),
        (None, None) => "?".into(),
    };
    match &block.title {
        Some(title) => format!("{name}={title}"),
        None => name,
    }
}

/// A block perspective's blocks as words, comma-separated; empty for a
/// plain one.
fn blocks_words(perspectives: &[Perspective], p: &Perspective) -> String {
    p.blocks
        .iter()
        .map(|b| block_word(perspectives, b))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Blocks from words: `:view` or a plain perspective's name, each with
/// an optional `=Heading` (ADR 0116 §2, §7). `of` is the perspective that
/// gets them, which is no block of itself.
fn parse_blocks(
    perspectives: &[Perspective],
    of: Option<Id>,
    words: &[String],
) -> Result<Vec<PerspectiveBlock>> {
    words
        .iter()
        .map(|word| {
            let (name, title) = match word.split_once('=') {
                Some((name, title)) if !title.trim().is_empty() => {
                    (name.trim(), Some(title.trim().to_owned()))
                }
                Some(_) => bail!("{word:?}: a heading after = is not blank"),
                None => (word.trim(), None),
            };
            if let Some(view) = name.strip_prefix(':') {
                if !samu_core::blocks::known_view(view) {
                    bail!(
                        "{view:?} is no view a block shows: {}",
                        samu_core::blocks::VIEWS.join(", ")
                    );
                }
                return Ok(PerspectiveBlock {
                    perspective: None,
                    view: Some(view.to_owned()),
                    title,
                });
            }
            let found = find(perspectives, name)?;
            if Some(found.id) == of {
                bail!("a perspective is no block of itself");
            }
            if !found.blocks.is_empty() {
                bail!("{:?} has blocks itself and is no block", found.title);
            }
            Ok(PerspectiveBlock {
                perspective: Some(found.id),
                view: None,
                title,
            })
        })
        .collect()
}

/// A plain perspective, for the commands that change what only a plain
/// one has (ADR 0116 §7).
fn plain<'a>(perspectives: &'a [Perspective], name: &str) -> Result<&'a Perspective> {
    let found = find(perspectives, name)?;
    if !found.blocks.is_empty() {
        bail!(
            "{:?} is a block perspective: change its blocks with `samu perspective blocks`",
            found.title
        );
    }
    Ok(found)
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

/// The columns of a table perspective, comma-separated; empty for the
/// others.
fn columns_words(p: &Perspective) -> String {
    if p.view != PerspectiveView::Table {
        return String::new();
    }
    crate::table::or_default(
        p.table
            .as_ref()
            .map(|t| t.columns.clone())
            .unwrap_or_default(),
    )
    .join(",")
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
                        "columns": columns_words(p),
                        "blocks": blocks_words(all, p),
                        "perspective": p,
                    })
                })
                .collect();
            emit(
                format,
                &records,
                &[
                    "short", "icon", "title", "search", "filter", "arrange", "view", "columns",
                    "blocks",
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
            table,
            columns,
            blocks,
        } => {
            let columns = columns.as_deref().map(crate::table::parse).transpose()?;
            let blocks = parse_blocks(all, None, &blocks)?;
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
                } else if table {
                    PerspectiveView::Table
                } else {
                    PerspectiveView::List
                },
                board: None,
                table: columns.map(|columns| PerspectiveTable { columns }),
                blocks,
                dedupe: true,
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
            let found = plain(all, &name)?;
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
            let found = plain(all, &name)?;
            let chips = if clear {
                Vec::new()
            } else {
                parse_chips(&snapshot, &words)?
            };
            write(
                &client,
                &[set(found.id, "chips", serde_json::to_value(&chips)?)],
            )?;
        }
        PerspectiveCommand::Text { name, words, clear } => {
            let found = plain(all, &name)?;
            let text = if clear {
                String::new()
            } else {
                words.join(" ")
            };
            write(&client, &[set(found.id, "text", json!(text))])?;
        }
        PerspectiveCommand::Filter { name, filter } => {
            let found = plain(all, &name)?;
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
            let found = plain(all, &name)?;
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
        PerspectiveCommand::View {
            name,
            view,
            columns,
        } => {
            let found = plain(all, &name)?;
            let view: PerspectiveView = word(&view, "presentation", VIEWS)?;
            let mut changes = vec![set(found.id, "view", serde_json::to_value(view)?)];
            if let Some(columns) = columns {
                let columns = crate::table::parse(&columns)?;
                changes.push(set(
                    found.id,
                    "table",
                    serde_json::to_value(PerspectiveTable { columns })?,
                ));
            }
            write(&client, &changes)?;
        }
        PerspectiveCommand::Blocks {
            name,
            blocks,
            clear,
        } => {
            let found = find(all, &name)?;
            if !clear && blocks.is_empty() {
                let records: Vec<Value> = found
                    .blocks
                    .iter()
                    .map(|b| json!({"block": block_word(all, b), "value": b}))
                    .collect();
                emit(format, &records, &["block"])?;
                return Ok(());
            }
            if !clear
                && all
                    .iter()
                    .any(|p| p.blocks.iter().any(|b| b.perspective == Some(found.id)))
            {
                bail!(
                    "{:?} is a block of another perspective and gets no blocks",
                    found.title
                );
            }
            let blocks = parse_blocks(all, Some(found.id), &blocks)?;
            write(
                &client,
                &[set(found.id, "blocks", serde_json::to_value(&blocks)?)],
            )?;
        }
        PerspectiveCommand::Dedupe { name, value } => {
            let found = find(all, &name)?;
            let dedupe = match value.to_lowercase().as_str() {
                "on" | "true" | "yes" => true,
                "off" | "false" | "no" => false,
                _ => bail!("{value:?} is neither on nor off"),
            };
            write(&client, &[set(found.id, "dedupe", json!(dedupe))])?;
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
