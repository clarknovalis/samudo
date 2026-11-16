//! Block perspectives (ADR 0116): several perspectives and fixed views one
//! below the other, like org's block agenda.
//!
//! What a single perspective shows is computed elsewhere
//! ([`crate::views::PerspectiveTest`]); here the blocks are put together:
//! headings, blocks that show nothing because what they point at is gone,
//! and a task that several blocks match. The web does the same in
//! `web/lib/blocks.js`; the cases in `testdata/blocks/sections.json` hold
//! the two to the same answers.

use std::collections::HashSet;

use crate::api::{BlockSection, Perspective, PerspectiveBlock, TaskStatus};

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

/// The sections of `perspective` (ADR 0116 §3-4) from `statuses`, every
/// task the scope shows in the order of `/available`; `shows` says whether
/// a plain perspective shows a task, its filter included. A plain
/// perspective is one section; a block that points at a perspective that
/// is not among `perspectives`, at a block perspective or at an unknown
/// view is `missing` and empty. With `dedupe` a task stands only in the
/// first block that shows it.
pub fn assemble(
    perspective: &Perspective,
    perspectives: &[Perspective],
    statuses: &[TaskStatus],
    shows: impl Fn(&Perspective, &TaskStatus) -> bool,
) -> Vec<BlockSection> {
    if perspective.blocks.is_empty() {
        return vec![BlockSection {
            title: perspective.title.clone(),
            icon: perspective.icon.clone(),
            block: None,
            missing: false,
            tasks: statuses
                .iter()
                .filter(|s| shows(perspective, s))
                .cloned()
                .collect(),
        }];
    }
    let mut seen = HashSet::new();
    perspective
        .blocks
        .iter()
        .map(|block| {
            let (name, icon, hits) = hits_of(block, perspectives, statuses, &shows);
            let missing = hits.is_none();
            let tasks: Vec<TaskStatus> = hits
                .unwrap_or_default()
                .into_iter()
                .filter(|s| !perspective.dedupe || seen.insert(s.task.id))
                .collect();
            BlockSection {
                title: block.title.clone().unwrap_or(name),
                icon,
                block: Some(block.clone()),
                missing,
                tasks,
            }
        })
        .collect()
}

/// A block's name, icon and hits; no hits when it shows nothing this samu
/// can show.
fn hits_of(
    block: &PerspectiveBlock,
    perspectives: &[Perspective],
    statuses: &[TaskStatus],
    shows: &impl Fn(&Perspective, &TaskStatus) -> bool,
) -> (String, Option<String>, Option<Vec<TaskStatus>>) {
    match (&block.perspective, &block.view) {
        (Some(id), None) => match perspectives.iter().find(|p| p.id == *id) {
            Some(p) if p.blocks.is_empty() => (
                p.title.clone(),
                p.icon.clone(),
                Some(statuses.iter().filter(|s| shows(p, s)).cloned().collect()),
            ),
            Some(p) => (p.title.clone(), p.icon.clone(), None),
            None => (id.to_string(), None, None),
        },
        (None, Some(view)) => {
            let hits = known_view(view).then(|| {
                statuses
                    .iter()
                    .filter(|s| in_view(view, s) == Some(true))
                    .cloned()
                    .collect()
            });
            (view.clone(), None, hits)
        }
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
        let shows = |p: &Perspective, s: &TaskStatus| {
            shown.get(&p.id).is_some_and(|ids| ids.contains(&s.task.id))
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
            let got = assemble(&perspective, &perspectives, &statuses, shows);
            let want = case["sections"].as_array().unwrap();
            assert_eq!(got.len(), want.len(), "{about}");
            for (got, want) in got.iter().zip(want) {
                assert_eq!(got.title, want["title"].as_str().unwrap(), "{about}");
                assert_eq!(got.missing, want["missing"].as_bool().unwrap(), "{about}");
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
