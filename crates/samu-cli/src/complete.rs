//! Shell completion (zsh first). The shell calls `samu` itself with
//! `COMPLETE=zsh`; clap_complete knows commands and options from the clap
//! definition, and the functions here add what only the server knows:
//! tags and projects inside the text of `samu add`, task ids, states.
//!
//! The candidates are pure functions over a [`World`], so they are tested
//! without a shell; [`load`] fetches the world once per Tab, gives up after
//! a second and then offers nothing rather than an error in the prompt.
//! clap_complete's dynamic engine is behind an "unstable" feature, so its
//! use stays in this file (the `ArgValueCompleter` glue at the bottom).

use std::ffi::OsStr;
use std::time::Duration;

use clap_complete::engine::CompletionCandidate;
use samu_api::{AnyObject, ChangesPage, ProjectStatus, TagStatus};

use crate::config::Config;
use samu_client::Client;

/// What the candidates are drawn from.
#[derive(Debug, Default)]
pub(crate) struct World {
    /// Open projects, by title.
    pub(crate) projects: Vec<String>,
    /// Live tags: (title, path like `ort/büro`, key).
    pub(crate) tags: Vec<(String, String, Option<String>)>,
    /// Workflow states: (title, key).
    pub(crate) states: Vec<(String, Option<String>)>,
    /// Open tasks: (short id, title).
    pub(crate) tasks: Vec<(String, String)>,
}

impl World {
    pub(crate) fn from_page(page: &ChangesPage) -> Self {
        let mut tags = std::collections::HashMap::new();
        let mut world = Self::default();
        for object in &page.objects {
            match object {
                AnyObject::Project(p)
                    if matches!(p.status, ProjectStatus::Active | ProjectStatus::OnHold) =>
                {
                    world.projects.push(p.title.clone());
                }
                AnyObject::Tag(t) if t.status != TagStatus::Dropped => {
                    tags.insert(t.id, t.clone());
                }
                AnyObject::Task(t) if t.completed_at.is_none() && t.dropped_at.is_none() => {
                    let id = t.id.to_string();
                    world.tasks.push((
                        id[id.len().saturating_sub(8)..].to_string(),
                        t.title.clone(),
                    ));
                }
                _ => {}
            }
        }
        for tag in tags.values() {
            let mut parts = vec![tag.title.clone()];
            let mut parent = tag.parent.and_then(|p| tags.get(&p));
            while let Some(up) = parent {
                parts.push(up.title.clone());
                if parts.len() > 16 {
                    break;
                }
                parent = up.parent.and_then(|p| tags.get(&p));
            }
            parts.reverse();
            world
                .tags
                .push((tag.title.clone(), parts.join("/"), tag.key.clone()));
            if tag
                .parent
                .and_then(|p| tags.get(&p))
                .is_some_and(|p| p.exclusive)
            {
                world.states.push((tag.title.clone(), tag.key.clone()));
            }
        }
        world.projects.sort();
        world.tags.sort_by(|a, b| a.1.cmp(&b.1));
        world.states.sort();
        world
    }
}

/// A candidate: what is inserted, and what zsh shows beside it.
pub(crate) type Candidate = (String, Option<String>);

fn starts(name: &str, typed: &str) -> bool {
    name.to_lowercase().starts_with(&typed.to_lowercase())
}

const DATE_PREFIXES: [&str; 6] = ["due:", "d:", "start:", "s:", "plan:", "p:"];
/// Only what the parser reads (ADR 0011 §3) and fits in one word.
const DATES: [&str; 15] = [
    "heute",
    "morgen",
    "übermorgen",
    "mo",
    "di",
    "mi",
    "do",
    "fr",
    "sa",
    "so",
    "+1d",
    "+1w",
    "+2w",
    "+1m",
    "monatsende",
];
const ESTIMATES: [&str; 6] = ["5m", "15m", "30m", "1h", "2h", "4h"];
const REPEATS: [&str; 11] = [
    "1d", "1w", "2w", "1m", "1y", "mo", "di", "mi", "do", "fr", "sa",
];

/// The word being typed in the text of `samu add`: `@tag`, `#project`, a
/// date after `d:`/`s:`/`p:`, `~estimate`, `every:`. The text may be one
/// quoted argument, so only its last word is completed and the rest kept.
pub(crate) fn capture_word(current: &str, world: &World) -> Vec<Candidate> {
    let cut = current.rfind(char::is_whitespace).map_or(0, |i| i + 1);
    let (head, word) = current.split_at(cut);
    let with = |value: String, help: Option<String>| (format!("{head}{value}"), help);
    if let Some(typed) = word.strip_prefix('@') {
        return world
            .tags
            .iter()
            .filter(|(_, path, _)| starts(path, typed))
            .map(|(_, path, _)| with(format!("@{path}"), None))
            .collect();
    }
    if let Some(typed) = word.strip_prefix('#') {
        let typed = typed.trim_start_matches('"');
        return world
            .projects
            .iter()
            .filter(|title| starts(title, typed))
            .map(|title| {
                let value = if title.contains(char::is_whitespace) {
                    format!("#\"{title}\"")
                } else {
                    format!("#{title}")
                };
                with(value, Some("project".into()))
            })
            .collect();
    }
    if let Some(typed) = word.strip_prefix('~') {
        return ESTIMATES
            .iter()
            .filter(|e| starts(e, typed))
            .map(|e| with(format!("~{e}"), None))
            .collect();
    }
    for prefix in ["every!:", "every:"] {
        if let Some(typed) = word.strip_prefix(prefix) {
            return REPEATS
                .iter()
                .filter(|r| starts(r, typed))
                .map(|r| with(format!("{prefix}{r}"), None))
                .collect();
        }
    }
    for prefix in DATE_PREFIXES {
        if let Some(typed) = word.strip_prefix(prefix) {
            return DATES
                .iter()
                .filter(|d| starts(d, typed))
                .map(|d| with(format!("{prefix}{d}"), None))
                .collect();
        }
    }
    Vec::new()
}

/// Open tasks by the end of their id, with the title beside it.
pub(crate) fn task_ids(current: &str, world: &World) -> Vec<Candidate> {
    world
        .tasks
        .iter()
        .filter(|(short, _)| short.starts_with(current))
        .map(|(short, title)| (short.clone(), Some(title.clone())))
        .collect()
}

pub(crate) fn project_titles(current: &str, world: &World) -> Vec<Candidate> {
    world
        .projects
        .iter()
        .filter(|title| starts(title, current))
        .map(|title| (title.clone(), None))
        .collect()
}

/// Tags by path, for `list --tag`.
pub(crate) fn tag_paths(current: &str, world: &World) -> Vec<Candidate> {
    let typed = current.trim_start_matches('@');
    world
        .tags
        .iter()
        .filter(|(_, path, _)| starts(path, typed))
        .map(|(_, path, _)| (path.clone(), None))
        .collect()
}

/// Tags by title, for `samu tag …` (which takes a title or an id end).
pub(crate) fn tag_titles(current: &str, world: &World) -> Vec<Candidate> {
    world
        .tags
        .iter()
        .filter(|(title, _, _)| starts(title, current))
        .map(|(title, path, _)| (title.clone(), Some(path.clone())))
        .collect()
}

/// States of every workflow, by title, the key beside it.
pub(crate) fn states(current: &str, world: &World) -> Vec<Candidate> {
    world
        .states
        .iter()
        .filter(|(title, _)| starts(title, current))
        .map(|(title, key)| (title.clone(), key.as_ref().map(|k| format!("key {k}"))))
        .collect()
}

/// The account's objects, or nothing when there is no session or no
/// answer within a second.
fn load() -> World {
    let attempt = || -> anyhow::Result<World> {
        let config = Config::load(None, None, None)?;
        let Some(token) = config.token()? else {
            anyhow::bail!("not signed in");
        };
        let client =
            Client::new(config.server()?, Some(token))?.with_timeout(Duration::from_secs(1));
        let page: ChangesPage =
            serde_json::from_value(client.call("GET", "/changes?after=0", None)?.json()?)?;
        Ok(World::from_page(&page))
    };
    attempt().unwrap_or_default()
}

fn glue(current: &OsStr, pick: fn(&str, &World) -> Vec<Candidate>) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };
    pick(current, &load())
        .into_iter()
        .map(|(value, help)| CompletionCandidate::new(value).help(help.map(Into::into)))
        .collect()
}

pub(crate) fn capture(current: &OsStr) -> Vec<CompletionCandidate> {
    glue(current, capture_word)
}

pub(crate) fn task(current: &OsStr) -> Vec<CompletionCandidate> {
    glue(current, task_ids)
}

pub(crate) fn project(current: &OsStr) -> Vec<CompletionCandidate> {
    glue(current, project_titles)
}

pub(crate) fn tag_path(current: &OsStr) -> Vec<CompletionCandidate> {
    glue(current, tag_paths)
}

pub(crate) fn tag_title(current: &OsStr) -> Vec<CompletionCandidate> {
    glue(current, tag_titles)
}

pub(crate) fn state(current: &OsStr) -> Vec<CompletionCandidate> {
    glue(current, states)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> World {
        World {
            projects: vec!["Haus".into(), "Neues Bad".into()],
            tags: vec![
                ("ort".into(), "ort".into(), None),
                ("büro".into(), "ort/büro".into(), None),
                ("telefon".into(), "telefon".into(), None),
                ("termin".into(), "termin".into(), None),
            ],
            states: vec![("Wartet".into(), Some("w".into()))],
            tasks: vec![
                ("0000aaaa".into(), "Milch".into()),
                ("0000abcd".into(), "Brot".into()),
            ],
        }
    }

    fn values(candidates: &[Candidate]) -> Vec<&str> {
        candidates.iter().map(|(v, _)| v.as_str()).collect()
    }

    #[test]
    fn the_last_word_of_the_text_is_completed() {
        let w = world();
        assert_eq!(values(&capture_word("@te", &w)), ["@telefon", "@termin"]);
        assert_eq!(
            values(&capture_word("x @Te", &w)),
            ["x @telefon", "x @termin"]
        );
        assert_eq!(values(&capture_word("@ort/b", &w)), ["@ort/büro"]);
        assert_eq!(
            values(&capture_word("Anruf #n", &w)),
            ["Anruf #\"Neues Bad\""]
        );
        assert_eq!(values(&capture_word("#\"neu", &w)), ["#\"Neues Bad\""]);
        assert_eq!(values(&capture_word("#h", &w)), ["#Haus"]);
        assert_eq!(
            values(&capture_word("x d:m", &w)),
            ["x d:morgen", "x d:mo", "x d:mi", "x d:monatsende"]
        );
        assert_eq!(
            values(&capture_word("s:+1", &w)),
            ["s:+1d", "s:+1w", "s:+1m"]
        );
        assert_eq!(values(&capture_word("~1", &w)), ["~15m", "~1h"]);
        assert_eq!(
            values(&capture_word("every!:1", &w)),
            ["every!:1d", "every!:1w", "every!:1m", "every!:1y"]
        );
        assert!(capture_word("Milch kaufen", &w).is_empty());
        assert!(capture_word("@xyz", &w).is_empty());
    }

    #[test]
    fn ids_projects_tags_and_states() {
        let w = world();
        assert_eq!(
            task_ids("0000ab", &w),
            [("0000abcd".to_string(), Some("Brot".to_string()))]
        );
        assert_eq!(values(&task_ids("", &w)).len(), 2);
        assert_eq!(values(&project_titles("neu", &w)), ["Neues Bad"]);
        assert_eq!(values(&tag_paths("@ort", &w)), ["ort", "ort/büro"]);
        assert_eq!(values(&tag_titles("bü", &w)), ["büro"]);
        assert_eq!(
            states("w", &w),
            [("Wartet".to_string(), Some("key w".to_string()))]
        );
    }
}
