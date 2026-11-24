// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Block perspectives (ADR 0116): several perspectives and fixed views one
//! below the other, like org's block agenda.
//!
//! What a single perspective shows is computed elsewhere
//! ([`crate::views::PerspectiveTest`]); here the blocks are put together:
//! headings, blocks that show nothing because what they point at is gone,
//! a task that several blocks match, and a block that shows the note of a
//! project or task instead of tasks (ADR 0146). The web does the same in
//! `web/lib/blocks.js`; the cases in `testdata/blocks/sections.json` hold
//! the two to the same answers.

use std::collections::HashSet;

use crate::api::{BlockQuery, BlockSection, Id, Perspective, PerspectiveBlock, TaskStatus};

/// The fixed views a block may show (ADR 0116 §2): the ones whose hits
/// this crate already computes per task.
pub const VIEWS: &[&str] = &["inbox", "today", "flagged"];

/// Whether `view` names a fixed view this samu can show as a block.
pub fn known_view(view: &str) -> bool {
    VIEWS.contains(&view)
}

/// Whether a fixed view shows a task, as its list in the web does with
/// the default filter: the inbox and Flagged what is open and available,
/// Today, a view of time, what is open. None for a view this samu does not
/// know.
pub fn in_view(view: &str, status: &TaskStatus) -> Option<bool> {
    let open = status.task.completed_at.is_none() && status.task.dropped_at.is_none();
    match view {
        "inbox" => Some(open && status.available && status.inbox),
        "today" => Some(open && status.today),
        "flagged" => Some(open && status.available && status.task.flagged),
        _ => None,
    }
}

/// What a block asks [`assemble`]'s `shows` about: a plain perspective it
/// points at, or the search of its own of the block at this index
/// (ADR 0143 §2), whose plain perspective [`queries`] gives.
#[derive(Debug, Clone, Copy)]
pub enum Shown<'a> {
    Perspective(&'a Perspective),
    Query(usize),
}

/// What a note block shows of its project or task (ADR 0146 §2), as the
/// caller finds it: every live project and task, whatever the scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteSource {
    pub title: String,
    pub icon: Option<String>,
    /// The note as written, empty when there is none.
    pub note: String,
}

/// The plain perspective each block with a search of its own shows as
/// (ADR 0143 §2), by the block's index: the caller builds its test once
/// per block, not once per task. It carries the block perspective's id,
/// so it is no perspective of its own anywhere.
pub fn queries(perspective: &Perspective) -> Vec<(usize, Perspective)> {
    perspective
        .blocks
        .iter()
        .enumerate()
        .filter_map(|(i, block)| match block {
            PerspectiveBlock {
                perspective: None,
                view: None,
                query: Some(query),
                note: None,
                ..
            } => Some((
                i,
                query
                    .clone()
                    .into_perspective(perspective.id, perspective.created, String::new()),
            )),
            _ => None,
        })
        .collect()
}

/// The heading of a search of a block's own without one (ADR 0143 §2):
/// the search as it reads in the search field, chips first, and "All"
/// for an empty one.
pub fn query_name(query: &BlockQuery, snapshot: &crate::availability::Snapshot<'_>) -> String {
    let words = [
        crate::query::words(&query.chips, snapshot),
        query.text.clone(),
    ]
    .into_iter()
    .filter(|w| !w.trim().is_empty())
    .collect::<Vec<_>>()
    .join(" ");
    if words.is_empty() {
        "All".to_owned()
    } else {
        words
    }
}

/// The sections of `perspective` (ADR 0116 §3-4) from `statuses`, every
/// task the scope shows in the order of `/available`; `shows` says whether
/// a plain perspective or a block's own search shows a task, its filter
/// included, and `name` how a search without a heading is called (the
/// search as it reads, ADR 0143 §2). A plain perspective is one section; a
/// block that points at a perspective that is not among `perspectives`, at
/// a block perspective or at an unknown view, and a block of no form, is
/// `missing` and empty. With `dedupe` a task stands only in the first
/// block that shows it. A note block (ADR 0146 §2) has no tasks and the
/// note that `note_of` finds for its object, or is missing without one.
pub fn assemble(
    perspective: &Perspective,
    perspectives: &[Perspective],
    statuses: &[TaskStatus],
    shows: impl Fn(Shown<'_>, &TaskStatus) -> bool,
    name: impl Fn(&BlockQuery) -> String,
    note_of: impl Fn(Id) -> Option<NoteSource>,
) -> Vec<BlockSection> {
    if perspective.blocks.is_empty() {
        return vec![BlockSection {
            title: perspective.title.clone(),
            icon: perspective.icon.clone(),
            block: None,
            missing: false,
            note: None,
            tasks: statuses
                .iter()
                .filter(|s| shows(Shown::Perspective(perspective), s))
                .cloned()
                .collect(),
        }];
    }
    let mut seen = HashSet::new();
    perspective
        .blocks
        .iter()
        .enumerate()
        .map(|(i, block)| {
            if let PerspectiveBlock {
                perspective: None,
                view: None,
                query: None,
                note: Some(id),
                ..
            } = block
            {
                let found = note_of(*id);
                return BlockSection {
                    title: block.title.clone().unwrap_or_else(|| {
                        found
                            .as_ref()
                            .map_or_else(|| id.to_string(), |f| f.title.clone())
                    }),
                    icon: found.as_ref().and_then(|f| f.icon.clone()),
                    block: Some(block.clone()),
                    missing: found.is_none(),
                    note: found.map(|f| f.note),
                    tasks: Vec::new(),
                };
            }
            let (title, icon, hits) = hits_of(i, block, perspectives, statuses, &shows, &name);
            let missing = hits.is_none();
            let tasks: Vec<TaskStatus> = hits
                .unwrap_or_default()
                .into_iter()
                .filter(|s| !perspective.dedupe || seen.insert(s.task.id))
                .collect();
            BlockSection {
                title: block.title.clone().unwrap_or(title),
                icon,
                block: Some(block.clone()),
                missing,
                note: None,
                tasks,
            }
        })
        .collect()
}

/// A block's name, icon and hits; no hits when it shows nothing this samu
/// can show.
fn hits_of(
    index: usize,
    block: &PerspectiveBlock,
    perspectives: &[Perspective],
    statuses: &[TaskStatus],
    shows: &impl Fn(Shown<'_>, &TaskStatus) -> bool,
    name: &impl Fn(&BlockQuery) -> String,
) -> (String, Option<String>, Option<Vec<TaskStatus>>) {
    let hits = |shown: Shown<'_>| -> Vec<TaskStatus> {
        statuses
            .iter()
            .filter(|s| shows(shown, s))
            .cloned()
            .collect()
    };
    if block.note.is_some() {
        return (String::new(), None, None);
    }
    match (&block.perspective, &block.view, &block.query) {
        (Some(id), None, None) => match perspectives.iter().find(|p| p.id == *id) {
            Some(p) if p.blocks.is_empty() => (
                p.title.clone(),
                p.icon.clone(),
                Some(hits(Shown::Perspective(p))),
            ),
            Some(p) => (p.title.clone(), p.icon.clone(), None),
            None => (id.to_string(), None, None),
        },
        (None, Some(view), None) => {
            let hits = known_view(view).then(|| {
                statuses
                    .iter()
                    .filter(|s| in_view(view, s) == Some(true))
                    .cloned()
                    .collect()
            });
            (view.clone(), None, hits)
        }
        (None, None, Some(query)) => (name(query), None, Some(hits(Shown::Query(index)))),
        _ => (String::new(), None, None),
    }
}

/// Every task of `sections` once, in their order: what `/available` answers
/// for a block perspective, so a reader that knows no blocks shows them
/// all (ADR 0116 §4).
pub fn union(sections: Vec<BlockSection>) -> Vec<TaskStatus> {
    let mut seen = HashSet::new();
    sections
        .into_iter()
        .flat_map(|s| s.tasks)
        .filter(|s| seen.insert(s.task.id))
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use std::collections::HashMap;

    use super::*;
    use crate::api::Id;

    #[test]
    fn known_views() {
        assert!(VIEWS.iter().all(|v| known_view(v)));
        assert!(!known_view("forecast"));
        assert!(!known_view(""));
    }

    /// The cases the web's blocks must give as well (ADR 0116 §3).
    #[test]
    fn the_shared_cases_hold() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testdata/blocks/sections.json"
        );
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let perspectives: Vec<Perspective> =
            serde_json::from_value(doc["perspectives"].clone()).unwrap();
        let statuses: Vec<TaskStatus> = serde_json::from_value(doc["statuses"].clone()).unwrap();
        let shown: HashMap<Id, Vec<Id>> = serde_json::from_value(doc["shows"].clone()).unwrap();
        let by_query: HashMap<String, Vec<Id>> =
            serde_json::from_value(doc["queries"].clone()).unwrap();
        let notes: HashMap<Id, serde_json::Value> =
            serde_json::from_value(doc["notes"].clone()).unwrap();
        let note_of = |id: Id| {
            notes.get(&id).map(|n| NoteSource {
                title: n["title"].as_str().unwrap().to_owned(),
                icon: n["icon"].as_str().map(str::to_owned),
                note: n["note"].as_str().unwrap_or_default().to_owned(),
            })
        };
        let cases = doc["cases"].as_array().unwrap();
        assert!(cases.len() >= 4, "the cases are there");
        let mut found = 0;
        for case in cases {
            let about = &case["about"];
            let mut perspective = perspectives
                .iter()
                .find(|p| p.title == case["perspective"].as_str().unwrap())
                .unwrap()
                .clone();
            if let Some(dedupe) = case["dedupe"].as_bool() {
                perspective.dedupe = dedupe;
            }
            // A search of its own stands for its hits by its text.
            let shows = |source: Shown<'_>, s: &TaskStatus| {
                let ids = match source {
                    Shown::Perspective(p) => shown.get(&p.id),
                    Shown::Query(i) => perspective.blocks[i]
                        .query
                        .as_ref()
                        .and_then(|q| by_query.get(&q.text)),
                };
                ids.is_some_and(|ids| ids.contains(&s.task.id))
            };
            let name = |q: &BlockQuery| q.text.clone();
            let got = assemble(&perspective, &perspectives, &statuses, shows, name, note_of);
            let want = case["sections"].as_array().unwrap();
            assert_eq!(got.len(), want.len(), "{about}");
            for (got, want) in got.iter().zip(want) {
                assert_eq!(got.title, want["title"].as_str().unwrap(), "{about}");
                assert_eq!(got.missing, want["missing"].as_bool().unwrap(), "{about}");
                assert_eq!(
                    got.note.as_deref(),
                    want["note"].as_str(),
                    "{about}: {}",
                    got.title
                );
                let ids: Vec<String> = got.tasks.iter().map(|s| s.task.id.to_string()).collect();
                let want_ids: Vec<String> = serde_json::from_value(want["tasks"].clone()).unwrap();
                assert_eq!(ids, want_ids, "{about}: {}", got.title);
                found += ids.len();
            }
            let union: Vec<String> = union(got).iter().map(|s| s.task.id.to_string()).collect();
            let want: Vec<String> = serde_json::from_value(case["union"].clone()).unwrap();
            assert_eq!(union, want, "{about}: union");
        }
        assert!(found > 0, "a case finds tasks");
    }
}
