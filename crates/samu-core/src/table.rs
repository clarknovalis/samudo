//! The table (ADR 0115): the names of its columns and the time worked on
//! each task and its subtree, and the estimate summed the same way.
//!
//! The web computes the same worked time in the browser from the same
//! clocks; the cases in `testdata/table/worked.json` hold the two to the
//! same answers.

use std::collections::{HashMap, HashSet};

use jiff::Timestamp;

use crate::api::{Clock, Id, Task};

/// The columns a table can show after the title (ADR 0115 §2), besides
/// `prop:<key>`. The same names in the web, the CLI and a perspective.
pub const COLUMNS: &[&str] = &[
    "state",
    "priority",
    "project",
    "tags",
    "defer",
    "due",
    "planned",
    "created",
    "modified",
    "completed",
    "estimate",
    "worked",
    "remaining",
    "blocked_by",
];

/// What a table shows when nothing was chosen (ADR 0115 §2).
pub const DEFAULT_COLUMNS: &[&str] = &["state", "priority", "due", "estimate", "worked"];

/// The prefix of a property's column.
pub const PROPERTY: &str = "prop:";

/// Whether `name` is a column this samu knows: one of [`COLUMNS`] or
/// `prop:` with a key.
#[must_use]
pub fn known_column(name: &str) -> bool {
    COLUMNS.contains(&name)
        || name
            .strip_prefix(PROPERTY)
            .is_some_and(|key| !key.trim().is_empty())
}

/// Seconds worked on one task (ADR 0115 §4).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Worked {
    /// On the task itself.
    pub own: i64,
    /// On the task and every task below it, at any depth: what an action
    /// group's row adds up.
    pub subtree: i64,
}

/// The time worked on each of `tasks`: the sum of its clocks, a running one
/// up to `now` (ADR 0046), booked time included, since that is a clock as
/// well (ADR 0081). So the same number as the report.
///
/// The subtree counts every task below, completed and dropped ones too:
/// the time was spent. Clocks on projects, or on tasks not in `tasks`, add
/// to nothing here. A loop in `parent`, which the server never lets
/// happen, counts each task once.
#[must_use]
pub fn worked(tasks: &[Task], clocks: &[Clock], now: Timestamp) -> HashMap<Id, Worked> {
    let mut own: HashMap<Id, i64> = tasks.iter().map(|t| (t.id, 0)).collect();
    for clock in clocks {
        if let Some(sum) = own.get_mut(&clock.owner) {
            let end = clock.end.unwrap_or(now);
            *sum += (end.as_second() - clock.start.as_second()).max(0);
        }
    }
    let mut children: HashMap<Id, Vec<Id>> = HashMap::new();
    for task in tasks {
        if let Some(parent) = task.parent.filter(|p| own.contains_key(p)) {
            children.entry(parent).or_default().push(task.id);
        }
    }
    tasks
        .iter()
        .map(|task| {
            let mut subtree = 0;
            let mut seen = HashSet::new();
            let mut stack = vec![task.id];
            while let Some(id) = stack.pop() {
                if !seen.insert(id) {
                    continue;
                }
                subtree += own.get(&id).copied().unwrap_or(0);
                if let Some(below) = children.get(&id) {
                    stack.extend(below);
                }
            }
            let mine = own.get(&task.id).copied().unwrap_or(0);
            (task.id, Worked { own: mine, subtree })
        })
        .collect()
}

/// The estimate of one task in seconds (ADR 0115 §4).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Estimated {
    /// Its own estimate, if it has one.
    pub own: Option<i64>,
    /// Its own and those below it: what an action group's row adds up.
    /// None when nothing in the subtree has an estimate.
    pub subtree: Option<i64>,
}

/// The seconds of an estimate; one in days counts them as 24 hours.
#[must_use]
pub fn estimate_seconds(task: &Task) -> Option<i64> {
    let span = task.estimate?.0;
    let total = span.total(jiff::Unit::Second).ok().or_else(|| {
        span.total((jiff::Unit::Second, jiff::civil::date(2026, 1, 1)))
            .ok()
    })?;
    #[allow(
        clippy::cast_possible_truncation,
        reason = "the seconds of an estimate fit"
    )]
    Some(total as i64)
}

/// The estimate of each of `tasks`, its own and its subtree's. A dropped
/// task keeps its own but adds nothing above it: that work will not be
/// done. A completed one adds, so the remaining time (estimate − worked)
/// of an action group compares like with like.
#[must_use]
pub fn estimated(tasks: &[Task]) -> HashMap<Id, Estimated> {
    let own: HashMap<Id, Option<i64>> = tasks.iter().map(|t| (t.id, estimate_seconds(t))).collect();
    let mut children: HashMap<Id, Vec<Id>> = HashMap::new();
    for task in tasks.iter().filter(|t| t.dropped_at.is_none()) {
        if let Some(parent) = task.parent.filter(|p| own.contains_key(p)) {
            children.entry(parent).or_default().push(task.id);
        }
    }
    tasks
        .iter()
        .map(|task| {
            let mut subtree: Option<i64> = None;
            let mut seen = HashSet::new();
            let mut stack = vec![task.id];
            while let Some(id) = stack.pop() {
                if !seen.insert(id) {
                    continue;
                }
                if let Some(seconds) = own.get(&id).copied().flatten() {
                    subtree = Some(subtree.unwrap_or(0) + seconds);
                }
                if let Some(below) = children.get(&id) {
                    stack.extend(below);
                }
            }
            let mine = own.get(&task.id).copied().flatten();
            (task.id, Estimated { own: mine, subtree })
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests may unwrap")]
mod tests {
    use super::*;

    #[test]
    fn known_columns() {
        assert!(known_column("worked"));
        assert!(known_column("prop:jira"));
        assert!(!known_column("prop:"));
        assert!(!known_column("prop: "));
        assert!(!known_column("ist"));
        assert!(!known_column("title"), "the title is always there");
        assert!(DEFAULT_COLUMNS.iter().all(|c| known_column(c)));
    }

    /// The cases the web's table must give as well (ADR 0115, Folgen).
    #[test]
    fn the_shared_cases_hold() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testdata/table/worked.json"
        );
        let doc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let tasks: Vec<Task> = serde_json::from_value(doc["tasks"].clone()).unwrap();
        let clocks: Vec<Clock> = serde_json::from_value(doc["clocks"].clone()).unwrap();
        let cases = doc["cases"].as_array().unwrap();
        assert!(cases.len() >= 3, "the cases are there");
        for case in cases {
            let now: Timestamp = case["now"].as_str().unwrap().parse().unwrap();
            let got = worked(&tasks, &clocks, now);
            let want = case["worked"].as_object().unwrap();
            assert_eq!(got.len(), want.len(), "{}", case["about"]);
            assert!(
                want.values().any(|w| w["subtree"] != 0),
                "a case finds time"
            );
            for (id, w) in want {
                let id: Id = id.parse().unwrap();
                let w = Worked {
                    own: w["own"].as_i64().unwrap(),
                    subtree: w["subtree"].as_i64().unwrap(),
                };
                assert_eq!(got[&id], w, "{} for {id}", case["about"]);
            }
        }
        let got = estimated(&tasks);
        let want = doc["estimated"].as_object().unwrap();
        assert_eq!(got.len(), want.len());
        assert!(
            want.values().any(|e| e["subtree"].is_i64()),
            "a sum is there"
        );
        for (id, e) in want {
            let id: Id = id.parse().unwrap();
            let e = Estimated {
                own: e["own"].as_i64(),
                subtree: e["subtree"].as_i64(),
            };
            assert_eq!(got[&id], e, "estimated for {id}");
        }
    }

    #[test]
    fn an_estimate_in_days_counts_whole_days() {
        let mut task: Task = serde_json::from_value(serde_json::json!({
            "id": "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6f01",
            "created": "2026-11-01T10:00:00Z",
            "modified": "2026-11-01T10:00:00Z",
            "title": "x",
            "estimate": "P1DT2H",
        }))
        .unwrap();
        assert_eq!(estimate_seconds(&task), Some(26 * 3600));
        task.estimate = None;
        assert_eq!(estimate_seconds(&task), None);
    }
}
