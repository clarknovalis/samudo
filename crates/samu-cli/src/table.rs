//! `samu list --columns` (ADR 0115 §8): the table of the web as lines,
//! with the same column names. Worked time and estimates come from
//! samu-core; an action group's row counts its subtree, as in the web.

use std::collections::HashMap;

use anyhow::{Result, bail};
use jiff::Timestamp;
use samu_api::{AnyObject, ChangesPage, Clock, Folder, Id, Link, LinkKind, Tag, Task, TaskStatus};
use samu_core::table::{self, Estimated, Worked};
use serde_json::{Map, Value, json};

use crate::clock::length;
use crate::when_text;

/// The columns named in `text`, comma-separated, in order; empty for the
/// default ones. Unknown names and doubles are refused, as the server
/// refuses them in a perspective.
pub(crate) fn parse(text: &str) -> Result<Vec<String>> {
    let mut columns: Vec<String> = Vec::new();
    for name in text.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        if !table::known_column(name) {
            bail!(
                "{name:?} is no column: {} or prop:<key>",
                table::COLUMNS.join(", ")
            );
        }
        if columns.iter().any(|c| c == name) {
            bail!("the column {name:?} is there twice");
        }
        columns.push(name.to_string());
    }
    Ok(columns)
}

/// The columns to show: those asked for, else the default ones.
pub(crate) fn or_default(columns: Vec<String>) -> Vec<String> {
    if columns.is_empty() {
        table::DEFAULT_COLUMNS
            .iter()
            .map(|c| (*c).to_string())
            .collect()
    } else {
        columns
    }
}

/// Everything the cells are read from: the whole account, so a subtree
/// counts its completed tasks even where the list leaves them out.
pub(crate) struct Table {
    columns: Vec<String>,
    worked: HashMap<Id, Worked>,
    estimated: HashMap<Id, Estimated>,
    titles: HashMap<Id, String>,
    folders: HashMap<Id, Folder>,
    projects: HashMap<Id, (String, Option<Id>)>,
    tags: HashMap<Id, Tag>,
    blockers: HashMap<Id, Vec<Link>>,
}

/// One cell: the value for `json` and the text for `plain` and `tsv`.
struct Cell {
    value: Value,
    text: String,
}

impl Cell {
    fn none() -> Self {
        Self {
            value: Value::Null,
            text: String::new(),
        }
    }

    fn text(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            value: json!(text),
            text,
        }
    }

    fn seconds(seconds: Option<i64>) -> Self {
        seconds.map_or_else(Self::none, |s| Self {
            value: json!(s),
            text: signed_length(s),
        })
    }

    fn instant(at: Option<Timestamp>) -> Self {
        at.map_or_else(Self::none, |at| {
            let text = at.to_string();
            Self {
                value: json!(text),
                text: text.get(..16).unwrap_or(&text).replace('T', " "),
            }
        })
    }
}

/// `1:05`, or `-0:20` for time worked beyond the estimate.
fn signed_length(seconds: i64) -> String {
    if seconds < 0 {
        format!("-{}", length(-seconds))
    } else {
        length(seconds)
    }
}

impl Table {
    pub(crate) fn new(columns: Vec<String>, page: &ChangesPage, now: Timestamp) -> Self {
        let mut tasks: Vec<Task> = Vec::new();
        let mut clocks: Vec<Clock> = Vec::new();
        let mut out = Self {
            columns,
            worked: HashMap::new(),
            estimated: HashMap::new(),
            titles: HashMap::new(),
            folders: HashMap::new(),
            projects: HashMap::new(),
            tags: HashMap::new(),
            blockers: HashMap::new(),
        };
        for object in &page.objects {
            match object {
                AnyObject::Task(t) => {
                    out.titles.insert(t.id, t.title.clone());
                    tasks.push(t.clone());
                }
                AnyObject::Clock(c) => clocks.push(c.clone()),
                AnyObject::Folder(f) => {
                    out.folders.insert(f.id, f.clone());
                }
                AnyObject::Project(p) => {
                    out.projects.insert(p.id, (p.title.clone(), p.folder));
                }
                AnyObject::Tag(t) => {
                    out.tags.insert(t.id, t.clone());
                }
                AnyObject::Link(l) if l.kind == LinkKind::BlockedBy => {
                    out.blockers.entry(l.owner).or_default().push(l.clone());
                }
                _ => {}
            }
        }
        out.worked = table::worked(&tasks, &clocks, now);
        out.estimated = table::estimated(&tasks);
        out
    }

    /// `Arbeit/Kunden/Angebot`: the folders above a project and its title.
    fn project_path(&self, id: Id) -> Option<String> {
        let (title, mut folder) = self.projects.get(&id)?.clone();
        let mut parts = vec![title];
        while let Some(f) = folder.and_then(|f| self.folders.get(&f)) {
            parts.push(f.title.clone());
            if parts.len() > 32 {
                break;
            }
            folder = f.parent;
        }
        parts.reverse();
        Some(parts.join("/"))
    }

    fn tag_path(&self, id: Id) -> String {
        let mut parts = Vec::new();
        let mut at = self.tags.get(&id);
        while let Some(tag) = at {
            parts.push(tag.title.clone());
            if parts.len() > 16 {
                break;
            }
            at = tag.parent.and_then(|p| self.tags.get(&p));
        }
        parts.reverse();
        parts.join("/")
    }

    fn cell(&self, column: &str, status: &TaskStatus) -> Cell {
        let task = &status.task;
        let worked = self.worked.get(&task.id).map_or(0, |w| w.subtree);
        let estimate = self.estimated.get(&task.id).and_then(|e| e.subtree);
        if let Some(key) = column.strip_prefix(table::PROPERTY) {
            return task
                .properties
                .get(key)
                .or_else(|| status.inherited.get(key).map(|i| &i.value))
                .map_or_else(Cell::none, Cell::text);
        }
        match column {
            "state" => status
                .state
                .and_then(|s| self.tags.get(&s))
                .map_or_else(Cell::none, |t| Cell::text(t.title.clone())),
            "priority" => task.priority.map_or_else(Cell::none, |p| {
                let value = serde_json::to_value(p).unwrap_or(Value::Null);
                Cell {
                    text: value.as_str().unwrap_or_default().to_string(),
                    value,
                }
            }),
            "project" => task
                .project
                .and_then(|p| self.project_path(p))
                .map_or_else(Cell::none, Cell::text),
            "tags" => {
                let tags: Vec<String> = task
                    .tags
                    .iter()
                    .filter(|t| Some(t.tag) != status.state)
                    .map(|t| format!("@{}", self.tag_path(t.tag)))
                    .collect();
                Cell {
                    text: tags.join(" "),
                    value: json!(tags),
                }
            }
            "defer" | "due" | "planned" => {
                let when = match column {
                    "defer" => &task.schedule.defer,
                    "due" => &task.schedule.due,
                    _ => &task.schedule.planned,
                };
                when.as_ref().map_or_else(Cell::none, |w| Cell {
                    value: serde_json::to_value(w).unwrap_or(Value::Null),
                    text: when_text(w),
                })
            }
            "created" => Cell::instant(Some(task.created)),
            "modified" => Cell::instant(Some(task.modified)),
            "completed" => Cell::instant(task.completed_at),
            "estimate" => Cell::seconds(estimate),
            "worked" => Cell::seconds(Some(worked)),
            "remaining" => Cell::seconds(estimate.map(|e| e - worked)),
            "blocked_by" => {
                let links = self.blockers.get(&task.id).map_or(&[][..], Vec::as_slice);
                if links.is_empty() {
                    return Cell::none();
                }
                let names: Vec<String> = links
                    .iter()
                    .map(|l| {
                        l.uri
                            .strip_prefix("samu:")
                            .and_then(|id| id.parse::<Id>().ok())
                            .and_then(|id| self.titles.get(&id).cloned())
                            .unwrap_or_else(|| l.uri.clone())
                    })
                    .collect();
                let uris: Vec<&str> = links.iter().map(|l| l.uri.as_str()).collect();
                Cell {
                    value: json!(uris),
                    text: names.join(", "),
                }
            }
            // A column from a newer samu: nothing to show (ADR 0015).
            _ => Cell::none(),
        }
    }

    /// The cells of one task, by column name, for `json` and `jsonl`.
    pub(crate) fn values(&self, status: &TaskStatus) -> Value {
        let mut out = Map::new();
        for column in &self.columns {
            out.insert(column.clone(), self.cell(column, status).value);
        }
        Value::Object(out)
    }

    /// The line of one task for `plain` and `tsv`: `short`, the columns
    /// by name, `title`.
    pub(crate) fn line(&self, short: &str, status: &TaskStatus) -> Value {
        let mut out = Map::new();
        out.insert("short".into(), json!(short));
        for column in &self.columns {
            out.insert(column.clone(), json!(self.cell(column, status).text));
        }
        out.insert("title".into(), json!(status.task.title));
        Value::Object(out)
    }

    /// The head of `plain`: the names of the columns.
    pub(crate) fn head(&self) -> Value {
        let mut out = Map::new();
        out.insert("short".into(), json!("id"));
        for column in &self.columns {
            out.insert(column.clone(), json!(column));
        }
        out.insert("title".into(), json!("title"));
        Value::Object(out)
    }

    /// The keys of a line, in order.
    pub(crate) fn keys(&self) -> Vec<&str> {
        let mut keys = vec!["short"];
        keys.extend(self.columns.iter().map(String::as_str));
        keys.push("title");
        keys
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests may unwrap")]
mod tests {
    use super::*;

    #[test]
    fn columns_are_parsed_and_checked() {
        assert_eq!(
            parse("state, due,prop:jira").unwrap(),
            ["state", "due", "prop:jira"]
        );
        assert!(parse("").unwrap().is_empty());
        assert!(parse("ist").is_err());
        assert!(parse("prop:").is_err());
        assert!(parse("due,due").is_err());
        assert_eq!(or_default(Vec::new()), table::DEFAULT_COLUMNS);
    }

    #[test]
    fn remaining_time_can_be_negative() {
        assert_eq!(signed_length(3900), "1:05");
        assert_eq!(signed_length(-1200), "-0:20");
    }
}
