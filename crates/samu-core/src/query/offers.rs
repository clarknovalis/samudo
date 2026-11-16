// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What the list under a search field offers for the word at the cursor
//! (ADR 0103 §1): the same as `offers` in `web/lib/query.js`. The cases in
//! `testdata/query/offers.json` hold the two to the same answers.
//!
//! Names are ranked as in the web: those that begin with the word before
//! those that only contain it, each part by name, case aside. The web sorts
//! with the German collation; here it is code points after lower case, so
//! the two can differ only for names that differ in an umlaut alone.

use std::collections::BTreeSet;

use samu_api::{FolderStatus, ProjectStatus, ScopeChip, ScopeChipKind as Kind, TagStatus};

use super::lang::{self, Lang};
use super::{IS, folder_path, tag_path};
use crate::availability::Snapshot;
use crate::properties::{Inheritance, is_system};

/// At most this many offers, as in the web.
const MAX: usize = 8;

/// What an empty field offers: the kinds themselves, as what to insert
/// and the hint to show.
const KINDS: &[(&str, &str)] = &[
    ("#", "#Projekt"),
    ("@", "@Tag"),
    ("ordner:", "ordner:Ordner"),
    ("fällig:", "fällig:woche"),
    ("geplant:", "geplant:heute"),
    ("start:", "start:morgen"),
    ("erledigt:", "erledigt:-7t"),
    ("erstellt:", "erstellt:-7t"),
    ("geändert:", "geändert:heute"),
    ("ist:", "ist:markiert"),
    ("zustand:", "zustand:Name"),
    ("prop:", "prop:schlüssel"),
    ("prio:", "prio:a"),
    ("~", "~<15m"),
];

/// [`KINDS`] in English (ADR 0105 §3).
const KINDS_EN: &[(&str, &str)] = &[
    ("#", "#Project"),
    ("@", "@Tag"),
    ("folder:", "folder:Folder"),
    ("due:", "due:week"),
    ("planned:", "planned:today"),
    ("defer:", "defer:tomorrow"),
    ("done:", "done:-7d"),
    ("created:", "created:-7d"),
    ("modified:", "modified:today"),
    ("is:", "is:flagged"),
    ("state:", "state:Name"),
    ("prop:", "prop:key"),
    ("prio:", "prio:a"),
    ("~", "~<15m"),
];

const AHEAD: &[&str] = &["heute", "morgen", "woche", "7t", "30t", "mit", "ohne"];
const BEHIND: &[&str] = &["heute", "gestern", "-7t", "-30t", "mit", "ohne"];
const ESTIMATES: &[&str] = &["<15m", "<30m", "<1h", ">1h", "mit", "ohne"];
const PRIORITIES: &[&str] = &["a", "b", "c", "mit", "ohne"];

/// The date word before the colon, its kind and the values it offers.
const DATES: &[(&str, Kind)] = &[
    ("fällig", Kind::Due),
    ("geplant", Kind::Planned),
    ("start", Kind::Defer),
    ("erledigt", Kind::Completed),
    ("erstellt", Kind::Created),
    ("geändert", Kind::Modified),
];

fn ranges(kind: Kind) -> Vec<&'static str> {
    match kind {
        Kind::Due | Kind::Planned => std::iter::once("überfällig")
            .chain(AHEAD.iter().copied())
            .collect(),
        Kind::Defer => AHEAD.to_vec(),
        _ => BEHIND.to_vec(),
    }
}

/// What taking an offer does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OfferAction {
    /// The word becomes this chip.
    Chip(ScopeChip),
    /// The word is replaced by this text, and the list asks again.
    Insert(String),
}

/// One line of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub label: String,
    pub action: OfferAction,
}

/// What the field is for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OfferOptions {
    /// The search can turn to appointments, so `ist:termin` is offered
    /// (ADR 0047 §14); a scope's filter cannot, nor can the phone yet.
    pub appointments: bool,
    /// The language of labels and inserted words; the chips stay German.
    pub lang: Lang,
}

/// A name and the chip it makes.
struct Named {
    name: String,
    chip: ScopeChip,
}

fn ranked(mut names: Vec<Named>, query: &str) -> Vec<Named> {
    let q = query.to_lowercase();
    names.retain(|n| n.name.to_lowercase().contains(&q));
    names.sort_by(|a, b| {
        let (la, lb) = (a.name.to_lowercase(), b.name.to_lowercase());
        (!la.starts_with(&q), &la, &a.name).cmp(&(!lb.starts_with(&q), &lb, &b.name))
    });
    names
}

fn unquote(text: &str) -> &str {
    if text.len() > 1 && text.starts_with('"') && text.ends_with('"') {
        &text[1..text.len() - 1]
    } else {
        text
    }
}

/// The offers for `word`, the word at the cursor; `-` before it negates.
#[must_use]
pub fn offers(word: &str, snapshot: &Snapshot<'_>, options: OfferOptions) -> Vec<Offer> {
    let not = word.len() > 1 && word.starts_with('-');
    let text = if not { &word[1..] } else { word };
    let sign = if not { "-" } else { "" };
    let kinds = match options.lang {
        Lang::De => KINDS,
        Lang::En => KINDS_EN,
    };
    let key_of = |de: &str| format!("{}:", lang::key_in(de, options.lang));
    let chip = |kind: Kind, value: String| ScopeChip { kind, value, not };
    let insert = |(what, hint): &(&str, &str)| Offer {
        label: (*hint).to_owned(),
        action: OfferAction::Insert(format!("{sign}{what}")),
    };
    let labelled = |prefix: &str, list: Vec<Named>| -> Vec<Offer> {
        list.into_iter()
            .map(|n| Offer {
                label: format!("{sign}{prefix}{}", n.name),
                action: OfferAction::Chip(n.chip),
            })
            .collect()
    };
    let words = |prefix: &str, kind: Kind, list: &[&str], start: &str| -> Vec<Offer> {
        list.iter()
            .map(|w| (lang::value_in(w, options.lang), *w))
            .filter(|(shown, _)| shown.starts_with(start))
            .map(|(shown, w)| Offer {
                label: format!("{sign}{prefix}{shown}"),
                action: OfferAction::Chip(chip(kind, w.to_owned())),
            })
            .collect()
    };
    let projects = || -> Vec<Named> {
        snapshot
            .projects
            .iter()
            .filter(|p| p.status != ProjectStatus::Dropped)
            .map(|p| Named {
                name: p.title.clone(),
                chip: chip(Kind::Project, p.id.to_string()),
            })
            .chain(std::iter::once(Named {
                name: "Inbox".to_owned(),
                chip: chip(Kind::Project, "inbox".to_owned()),
            }))
            .collect()
    };
    let tags = || -> Vec<Named> {
        snapshot
            .tags
            .iter()
            .filter(|t| t.status != TagStatus::Dropped)
            .map(|t| Named {
                name: tag_path(snapshot, t.id),
                chip: chip(Kind::Tag, t.id.to_string()),
            })
            .collect()
    };
    let folders = || -> Vec<Named> {
        snapshot
            .folders
            .iter()
            .filter(|f| f.status != FolderStatus::Dropped)
            .map(|f| Named {
                name: folder_path(snapshot, f.id),
                chip: chip(Kind::Folder, f.id.to_string()),
            })
            .collect()
    };

    let mut out: Vec<Offer> = Vec::new();
    if text.is_empty() {
        return kinds.iter().map(insert).collect();
    }
    if let Some(name) = text.strip_prefix('#') {
        out.extend(labelled("#", ranked(projects(), unquote(name))));
    } else if let Some(name) = text.strip_prefix('@') {
        out.extend(labelled("@", ranked(tags(), unquote(name))));
    } else if let Some(start) = text.strip_prefix('~') {
        out.extend(words("~", Kind::Estimate, ESTIMATES, start));
    } else if let Some((key, value)) = text
        .split_once(':')
        .filter(|(k, _)| !k.is_empty() && k.chars().all(char::is_alphabetic))
    {
        let key = lang::canonical_key(&key.to_lowercase()).unwrap_or_default();
        let value = unquote(value);
        let v = value.to_lowercase();
        if let Some((word, kind)) = DATES.iter().find(|(w, _)| *w == key) {
            out.extend(words(&key_of(word), *kind, &ranges(*kind), &v));
        } else {
            match key {
                "ordner" => out.extend(labelled(&key_of("ordner"), ranked(folders(), value))),
                "prio" => out.extend(words("prio:", Kind::Priority, PRIORITIES, &v)),
                "ist" => {
                    let mut is: Vec<&str> = IS.to_vec();
                    if options.appointments {
                        is.push("termin");
                    }
                    out.extend(words(&key_of("ist"), Kind::Is, &is, &v));
                }
                "zustand" => out.extend(labelled(
                    &key_of("zustand"),
                    ranked(states(snapshot, not), value),
                )),
                "prop" => out.extend(labelled("prop:", properties(snapshot, value, not))),
                _ => {}
            }
        }
    } else {
        // A plain word: the kinds it begins, and the places and tags it names.
        let lower = text.to_lowercase();
        out.extend(
            kinds
                .iter()
                .filter(|(what, _)| what.chars().count() > 1 && what.starts_with(&lower))
                .map(insert),
        );
        if text.chars().count() >= 2 {
            out.extend(labelled(
                "#",
                ranked(projects(), text).into_iter().take(3).collect(),
            ));
            out.extend(labelled(
                "@",
                ranked(tags(), text).into_iter().take(3).collect(),
            ));
            out.extend(labelled(
                &key_of("ordner"),
                ranked(folders(), text).into_iter().take(2).collect(),
            ));
        }
    }
    out.truncate(MAX);
    out
}

/// The states of every workflow: the children of an exclusive tag.
fn states(snapshot: &Snapshot<'_>, not: bool) -> Vec<Named> {
    let exclusive: BTreeSet<_> = snapshot
        .tags
        .iter()
        .filter(|t| t.exclusive)
        .map(|t| t.id)
        .collect();
    snapshot
        .tags
        .iter()
        .filter(|t| t.parent.is_some_and(|p| exclusive.contains(&p)))
        .map(|t| Named {
            name: t.title.clone(),
            chip: ScopeChip {
                kind: Kind::State,
                value: t.id.to_string(),
                not,
            },
        })
        .collect()
}

/// The keys of every property, own or inherited; after `key=` the values
/// that key has.
fn properties(snapshot: &Snapshot<'_>, value: &str, not: bool) -> Vec<Named> {
    let inheritance = Inheritance::new(snapshot.tasks, snapshot.projects);
    let named = |name: String| Named {
        chip: ScopeChip {
            kind: Kind::Property,
            value: name.clone(),
            not,
        },
        name,
    };
    let Some((key, _)) = value.split_once('=') else {
        let mut keys = BTreeSet::new();
        for task in snapshot.tasks {
            keys.extend(task.properties.keys().cloned());
            keys.extend(inheritance.of(task).into_keys());
        }
        let list = keys
            .into_iter()
            .filter(|k| !is_system(k))
            .map(named)
            .collect();
        return ranked(list, value);
    };
    let key = key.to_lowercase();
    let values: BTreeSet<String> = snapshot
        .tasks
        .iter()
        .filter_map(|t| inheritance.value(t, &key))
        .collect();
    let list = values
        .into_iter()
        .map(|v| named(format!("{key}={v}")))
        .collect();
    ranked(list, value)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests may unwrap")]
mod tests {
    use super::*;
    use samu_api::{Folder, Project, Tag, Task};

    /// The cases the web's search must give as well (ADR 0103 §2).
    #[test]
    fn the_shared_cases_hold() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testdata/query/offers.json"
        );
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let folders: Vec<Folder> = serde_json::from_value(doc["folders"].clone()).unwrap();
        let projects: Vec<Project> = serde_json::from_value(doc["projects"].clone()).unwrap();
        let tags: Vec<Tag> = serde_json::from_value(doc["tags"].clone()).unwrap();
        let tasks: Vec<Task> = serde_json::from_value(doc["tasks"].clone()).unwrap();
        let snapshot = Snapshot {
            folders: &folders,
            projects: &projects,
            tasks: &tasks,
            tags: &tags,
            links: &[],
            clocks: &[],
            scopes: &[],
        };
        let cases = doc["cases"].as_array().unwrap();
        assert!(cases.len() > 10, "the cases are there");
        for case in cases {
            let word = case["word"].as_str().unwrap();
            let options = OfferOptions {
                appointments: case["appointments"].as_bool().unwrap_or(false),
                lang: match case["lang"].as_str() {
                    Some("en") => Lang::En,
                    _ => Lang::De,
                },
            };
            let got: Vec<serde_json::Value> = offers(word, &snapshot, options)
                .into_iter()
                .map(|o| match o.action {
                    OfferAction::Chip(chip) => {
                        serde_json::json!({ "label": o.label, "chip": chip })
                    }
                    OfferAction::Insert(text) => {
                        serde_json::json!({ "label": o.label, "insert": text })
                    }
                })
                .collect();
            assert_eq!(
                serde_json::Value::Array(got),
                case["offers"],
                "offers for {word:?}"
            );
        }
    }
}
