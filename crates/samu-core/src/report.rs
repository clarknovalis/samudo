//! The report (ADR 0080): what was done, what was worked on and the time,
//! over days in the account's zone.
//!
//! [`compute`] is a pure function over the objects, the logbook entries,
//! comments and field stamps of the span; the store only reads them. Which
//! tasks count at all (scope and filter) the caller decides, as for the
//! clock report.

use std::collections::{BTreeMap, HashMap, HashSet};

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};

use samu_api::{
    AuthorKind, ClockReportBy, Comment, Id, LogEntry, LogKind, ObjectKind, Project, Report,
    ReportGroup, ReportList, ReportReason, ReportReasonCount, ReportRow, Task,
};

use crate::availability::Snapshot;

/// The last change of one field, as the store keeps it (ADR 0080 §1): only
/// the last one per field, so a field changed again later does not count
/// for an earlier span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldStamp {
    pub object: Id,
    pub field: String,
    pub at: Timestamp,
    pub device: String,
}

/// Whose work counts (ADR 0080 §2, `autor:`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Author {
    /// A person's, the default: `autor:<account>`, the account's own name.
    #[default]
    Human,
    Agent,
    All,
}

impl Author {
    fn counts(self, agent: bool) -> bool {
        match self {
            Self::Human => !agent,
            Self::Agent => agent,
            Self::All => true,
        }
    }
}

/// The `autor:` chip (`author:` in English) taken out of a filter text
/// (ADR 0080 §2, ADR 0105 §3), and the rest of the text for the search
/// language. Without one, [`Author::Human`]. A person's work is named by
/// the `account` (`autor:alice`), or by `ich`, `mensch`, `me` or `human`.
///
/// # Errors
///
/// A text saying what is wrong: a value samu does not know, a negated or a
/// second `autor:`.
pub fn take_author(text: &str, account: &str) -> Result<(Author, String), String> {
    let mut author = None;
    let mut rest = Vec::new();
    for word in text.split_whitespace() {
        let lower = word.to_lowercase();
        if lower.starts_with("-autor:") || lower.starts_with("-author:") {
            return Err(format!("{word}: autor: cannot be negated; use autor:alle"));
        }
        // `author:` is the English word (ADR 0105 §3).
        let Some(value) = lower
            .strip_prefix("autor:")
            .or_else(|| lower.strip_prefix("author:"))
        else {
            rest.push(word);
            continue;
        };
        let this = match value {
            "agent" | "agenten" => Author::Agent,
            "alle" | "all" => Author::All,
            "ich" | "mensch" | "me" | "human" => Author::Human,
            v if v == account.to_lowercase() => Author::Human,
            _ => {
                return Err(format!(
                    "{word}: autor: is {}, agent or alle",
                    account.to_lowercase()
                ));
            }
        };
        if author.replace(this).is_some_and(|a| a != this) {
            return Err(format!("{word}: a second autor:"));
        }
    }
    Ok((author.unwrap_or_default(), rest.join(" ")))
}

/// What happened in the account besides its objects: read by the store for
/// the span (ADR 0080 §4).
#[derive(Debug, Clone, Copy)]
pub struct Events<'a> {
    /// The logbook from the span's start on; entries after its end only
    /// tell who completed what.
    pub log: &'a [LogEntry],
    pub comments: &'a [Comment],
    /// Only read with [`Ask::fields`].
    pub stamps: &'a [FieldStamp],
    /// The devices of agent sessions: a stamp from one is an agent's.
    pub agent_devices: &'a HashSet<String>,
    /// The account's name, the word for a person's work in `autor:`.
    pub account: &'a str,
}

/// What a report is asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub from: Date,
    /// Included.
    pub to: Date,
    pub by: ClockReportBy,
    pub done: bool,
    pub worked: bool,
    pub time: bool,
    /// Dropped tasks in `done`, marked.
    pub dropped: bool,
    pub author: Author,
    /// `report_worked = fields`: every field change counts as work.
    pub fields: bool,
}

fn day_start(day: Date, zone: &TimeZone) -> Option<Timestamp> {
    day.to_zoned(zone.clone()).ok().map(|z| z.timestamp())
}

/// What each object collects in the span.
#[derive(Default)]
struct Collected {
    reasons: BTreeMap<ReportReason, u32>,
    last: Option<Timestamp>,
}

impl Collected {
    fn add(&mut self, reason: ReportReason, at: Timestamp) {
        *self.reasons.entry(reason).or_default() += 1;
        self.last = Some(self.last.map_or(at, |l| l.max(at)));
    }
}

/// The report over the days `from` to `to` (ADR 0080 §1). `keep_task` and
/// `keep_project` decide which objects count at all (scope and filter);
/// time counts for whoever clocked it, `author` decides the rest.
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "one pass over each kind of event, in order"
)]
pub fn compute(
    snapshot: &Snapshot<'_>,
    events: &Events<'_>,
    ask: &Ask,
    zone: &TimeZone,
    now: Timestamp,
    keep_task: &dyn Fn(&Task) -> bool,
    keep_project: &dyn Fn(&Project) -> bool,
) -> Report {
    let mut report = Report {
        from: ask.from,
        to: ask.to,
        by: ask.by,
        done: None,
        worked: None,
        time: None,
    };
    let tasks: HashMap<Id, &Task> = snapshot.tasks.iter().map(|t| (t.id, t)).collect();
    let projects: HashMap<Id, &Project> = snapshot.projects.iter().map(|p| (p.id, p)).collect();
    let kept = |id: Id| -> bool {
        if let Some(task) = tasks.get(&id) {
            return keep_task(task);
        }
        projects.get(&id).is_some_and(|p| keep_project(p))
    };
    if ask.time {
        report.time = Some(crate::clocking::report(
            snapshot,
            ask.from,
            ask.to,
            ask.by,
            zone,
            now,
            |clock| kept(clock.owner),
        ));
    }
    if !ask.done && !ask.worked {
        return report;
    }
    let (Some(start), Some(end)) = (
        day_start(ask.from, zone),
        ask.to
            .checked_add(1.day())
            .ok()
            .and_then(|d| day_start(d, zone)),
    ) else {
        return report;
    };
    let within = |at: Timestamp| at >= start && at < end;

    // Time and clocks per owner in the span.
    let mut seconds: HashMap<Id, i64> = HashMap::new();
    let mut clocked: HashMap<Id, (u32, Timestamp)> = HashMap::new();
    for clock in snapshot.clocks {
        let from = clock.start.max(start);
        let to = clock.end.unwrap_or(now).min(end);
        if to <= from {
            continue;
        }
        *seconds.entry(clock.owner).or_default() += to.duration_since(from).as_secs();
        let entry = clocked.entry(clock.owner).or_insert((0, to));
        entry.0 += 1;
        entry.1 = entry.1.max(to);
    }

    // Who closed what: the last `completed` or `dropped` entry per object.
    let mut closed_by: HashMap<Id, bool> = HashMap::new();
    for entry in events.log {
        if matches!(entry.kind, LogKind::Completed | LogKind::Dropped) {
            closed_by.insert(entry.object, entry.author_kind == Some(AuthorKind::Agent));
        }
    }
    let closed_counts = |id: Id| {
        ask.author
            .counts(closed_by.get(&id).copied().unwrap_or(false))
    };

    // Completed and dropped in the span, task or project.
    let closed: Vec<(Id, Timestamp, bool)> = snapshot
        .tasks
        .iter()
        .map(|t| (t.id, t.completed_at, t.dropped_at))
        .chain(
            snapshot
                .projects
                .iter()
                .map(|p| (p.id, p.completed_at, p.dropped_at)),
        )
        .filter_map(|(id, completed, dropped)| match (completed, dropped) {
            (Some(at), _) if within(at) => Some((id, at, false)),
            (None, Some(at)) if within(at) => Some((id, at, true)),
            _ => None,
        })
        .filter(|&(id, _, _)| kept(id) && closed_counts(id))
        .collect();

    if ask.done {
        let rows: Vec<ReportRow> = closed
            .iter()
            .filter(|(_, _, dropped)| ask.dropped || !dropped)
            .filter_map(|&(id, at, dropped)| {
                let mut row = row_of(id, at, &tasks, &projects)?;
                row.dropped = dropped;
                row.seconds = seconds.get(&id).copied().unwrap_or(0);
                Some(row)
            })
            .collect();
        report.done = Some(list(rows, ask.by, snapshot, zone));
    }

    if ask.worked {
        let mut collected: HashMap<Id, Collected> = HashMap::new();
        for (&owner, &(count, last)) in &clocked {
            let c = collected.entry(owner).or_default();
            *c.reasons.entry(ReportReason::Clocked).or_default() += count;
            c.last = Some(c.last.map_or(last, |l| l.max(last)));
        }
        for &(id, at, dropped) in &closed {
            let reason = if dropped {
                ReportReason::Dropped
            } else {
                ReportReason::Completed
            };
            collected.entry(id).or_default().add(reason, at);
            if !dropped && let Some(parent) = tasks.get(&id).and_then(|t| t.parent) {
                collected
                    .entry(parent)
                    .or_default()
                    .add(ReportReason::ChildCompleted, at);
            }
        }
        for entry in events.log.iter().filter(|e| within(e.at)) {
            let reason = match entry.kind {
                LogKind::State => ReportReason::State,
                LogKind::Reopened => ReportReason::Reopened,
                _ => continue,
            };
            if ask
                .author
                .counts(entry.author_kind == Some(AuthorKind::Agent))
            {
                collected
                    .entry(entry.object)
                    .or_default()
                    .add(reason, entry.at);
            }
        }
        for comment in events.comments.iter().filter(|c| within(c.created)) {
            let agent = comment
                .created_by
                .as_ref()
                .is_some_and(|a| a.kind == AuthorKind::Agent);
            if ask.author.counts(agent) {
                collected
                    .entry(comment.owner)
                    .or_default()
                    .add(ReportReason::Commented, comment.created);
            }
        }
        if ask.fields {
            for stamp in events.stamps.iter().filter(|s| within(s.at)) {
                // Completion is counted above, with its author.
                if matches!(stamp.field.as_str(), "completed_at" | "dropped_at") {
                    continue;
                }
                if ask
                    .author
                    .counts(events.agent_devices.contains(&stamp.device))
                {
                    collected
                        .entry(stamp.object)
                        .or_default()
                        .add(ReportReason::Changed, stamp.at);
                }
            }
        }
        let rows: Vec<ReportRow> = collected
            .into_iter()
            .filter(|(id, _)| kept(*id))
            .filter_map(|(id, c)| {
                let mut row = row_of(id, c.last?, &tasks, &projects)?;
                row.reasons = c
                    .reasons
                    .into_iter()
                    .map(|(reason, count)| ReportReasonCount { reason, count })
                    .collect();
                row.seconds = seconds.get(&id).copied().unwrap_or(0);
                Some(row)
            })
            .collect();
        report.worked = Some(list(rows, ask.by, snapshot, zone));
    }
    report
}

/// A row for a task or project as it is now; none for what is gone.
fn row_of(
    id: Id,
    at: Timestamp,
    tasks: &HashMap<Id, &Task>,
    projects: &HashMap<Id, &Project>,
) -> Option<ReportRow> {
    let folder_of = |project: Option<Id>| {
        project
            .and_then(|p| projects.get(&p))
            .and_then(|p| p.folder)
    };
    if let Some(task) = tasks.get(&id) {
        let project = crate::availability::home(task, projects);
        return Some(ReportRow {
            id,
            kind: ObjectKind::Task,
            title: task.title.clone(),
            project,
            folder: folder_of(project),
            at,
            reasons: Vec::new(),
            seconds: 0,
            estimate: task.estimate,
            dropped: false,
        });
    }
    let project = projects.get(&id)?;
    Some(ReportRow {
        id,
        kind: ObjectKind::Project,
        title: project.title.clone(),
        project: None,
        folder: project.folder,
        at,
        reasons: Vec::new(),
        seconds: 0,
        estimate: project.estimate,
        dropped: false,
    })
}

/// The rows latest first, and their groups for `by` other than `task`.
fn list(
    mut rows: Vec<ReportRow>,
    by: ClockReportBy,
    snapshot: &Snapshot<'_>,
    zone: &TimeZone,
) -> ReportList {
    rows.sort_by(|a, b| {
        b.at.cmp(&a.at)
            .then_with(|| a.title.cmp(&b.title))
            .then(a.id.cmp(&b.id))
    });
    let keys = |row: &ReportRow| -> Vec<(Option<Id>, Option<Date>)> {
        match by {
            ClockReportBy::Project => vec![(
                if row.kind == ObjectKind::Project {
                    Some(row.id)
                } else {
                    row.project
                },
                None,
            )],
            ClockReportBy::Folder => vec![(row.folder, None)],
            ClockReportBy::Tag => {
                let tags = snapshot
                    .tasks
                    .iter()
                    .find(|t| t.id == row.id)
                    .map(|t| &t.tags)
                    .or_else(|| {
                        snapshot
                            .projects
                            .iter()
                            .find(|p| p.id == row.id)
                            .map(|p| &p.tags)
                    });
                match tags {
                    Some(tags) if !tags.is_empty() => {
                        tags.iter().map(|g| (Some(g.tag), None)).collect()
                    }
                    _ => vec![(None, None)],
                }
            }
            ClockReportBy::Day => vec![(None, Some(row.at.to_zoned(zone.clone()).date()))],
            ClockReportBy::Task | ClockReportBy::Unknown => Vec::new(),
        }
    };
    let mut groups: Vec<ReportGroup> = Vec::new();
    for row in &rows {
        for (id, date) in keys(row) {
            let group = match groups.iter().position(|g| g.id == id && g.date == date) {
                Some(i) => &mut groups[i],
                None => {
                    groups.push(ReportGroup {
                        id,
                        date,
                        title: id.map(|id| title_of(id, snapshot)).unwrap_or_default(),
                        seconds: 0,
                        rows: Vec::new(),
                    });
                    let last = groups.len() - 1;
                    &mut groups[last]
                }
            };
            group.seconds += row.seconds;
            group.rows.push(row.id);
        }
    }
    if by == ClockReportBy::Day {
        groups.sort_by_key(|g| std::cmp::Reverse(g.date));
    } else {
        // Most rows first; the group without an object last among equals.
        groups.sort_by(|a, b| {
            b.rows
                .len()
                .cmp(&a.rows.len())
                .then(a.id.is_none().cmp(&b.id.is_none()))
                .then_with(|| a.title.cmp(&b.title))
        });
    }
    ReportList { rows, groups }
}

fn title_of(id: Id, snapshot: &Snapshot<'_>) -> String {
    snapshot
        .projects
        .iter()
        .find(|p| p.id == id)
        .map(|p| p.title.clone())
        .or_else(|| {
            snapshot
                .folders
                .iter()
                .find(|f| f.id == id)
                .map(|f| f.title.clone())
        })
        .or_else(|| {
            snapshot
                .tags
                .iter()
                .find(|t| t.id == id)
                .map(|t| t.title.clone())
        })
        .unwrap_or_default()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests may unwrap")]
mod tests {
    use super::*;
    use crate::availability::Settings;
    use crate::overlay::Objects;
    use crate::views::{ReportQuery, report};
    use samu_api::{Author as Who, Clock, ReportPart};
    use serde_json::json;

    fn id(n: u8) -> Id {
        format!("01936f5e-8a00-7c3e-9b1a-3f2d4c5b6c{n:02x}")
            .parse()
            .unwrap()
    }

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn day(text: &str) -> Date {
        text.parse().unwrap()
    }

    fn berlin() -> TimeZone {
        TimeZone::get("Europe/Berlin").unwrap()
    }

    /// Friday, 13 November 2026, noon in Berlin.
    fn now() -> Timestamp {
        at("2026-11-13T11:00:00Z")
    }

    fn task(n: u8, fields: serde_json::Value) -> Task {
        let mut value = json!({
            "id": id(n), "created": "2026-11-01T00:00:00Z",
            "modified": "2026-11-01T00:00:00Z", "title": format!("task {n}"),
            "flagged": false, "sequential": false,
            "complete_with_last_child": false, "position": n,
        });
        if let (Some(base), Some(extra)) = (value.as_object_mut(), fields.as_object()) {
            base.extend(extra.clone());
        }
        serde_json::from_value(value).unwrap()
    }

    fn project(n: u8, title: &str) -> Project {
        serde_json::from_value(json!({
            "id": id(n), "created": "2026-11-01T00:00:00Z",
            "modified": "2026-11-01T00:00:00Z", "title": title,
            "kind": "parallel", "status": "active", "flagged": false, "position": n,
        }))
        .unwrap()
    }

    fn clock(n: u8, owner: u8, start: &str, end: &str) -> Clock {
        Clock {
            id: id(n),
            created: at(start),
            modified: at(start),
            owner: id(owner),
            start: at(start),
            end: Some(at(end)),
            note: None,
        }
    }

    fn log(object: u8, at_: &str, kind: LogKind, agent: bool) -> LogEntry {
        LogEntry {
            object: id(object),
            at: at(at_),
            kind,
            device: if agent { "claude" } else { "laptop" }.into(),
            author_kind: Some(if agent {
                AuthorKind::Agent
            } else {
                AuthorKind::Human
            }),
            note: None,
            old: None,
            new: None,
            field: None,
            old_value: None,
            new_value: None,
        }
    }

    fn comment(n: u8, owner: u8, created: &str, agent: bool) -> Comment {
        Comment {
            id: id(n),
            created: at(created),
            modified: at(created),
            created_by: Some(Who {
                device: if agent { "claude" } else { "laptop" }.into(),
                kind: if agent {
                    AuthorKind::Agent
                } else {
                    AuthorKind::Human
                },
            }),
            owner: id(owner),
            body: "x".into(),
        }
    }

    /// Two projects, Haus (1) and Garten (2); in Haus task 10 completed by
    /// a person on Wednesday with an hour clocked over Tuesday's midnight,
    /// task 11 completed by an agent, task 12 with a comment of each and a
    /// state change, task 13 a child of 12 completed, 14 dropped.
    fn objects() -> Objects {
        Objects {
            projects: vec![project(1, "Haus"), project(2, "Garten")],
            tasks: vec![
                task(
                    10,
                    json!({"project": id(1), "completed_at": "2026-11-11T09:00:00Z", "estimate": "PT45M"}),
                ),
                task(
                    11,
                    json!({"project": id(1), "completed_at": "2026-11-11T10:00:00Z"}),
                ),
                task(12, json!({"project": id(2)})),
                task(
                    13,
                    json!({"project": id(2), "parent": id(12), "completed_at": "2026-11-12T08:00:00Z"}),
                ),
                task(14, json!({"dropped_at": "2026-11-12T09:00:00Z"})),
                // Completed the week before: in no report of this week.
                task(15, json!({"completed_at": "2026-11-06T09:00:00Z"})),
            ],
            // 23:30 to 00:30 Berlin, Tuesday to Wednesday.
            clocks: vec![clock(
                30,
                10,
                "2026-11-10T22:30:00Z",
                "2026-11-10T23:30:00Z",
            )],
            comments: vec![
                comment(40, 12, "2026-11-12T10:00:00Z", false),
                comment(41, 12, "2026-11-12T11:00:00Z", true),
            ],
            ..Objects::default()
        }
    }

    fn entries() -> Vec<LogEntry> {
        vec![
            log(10, "2026-11-11T09:00:00Z", LogKind::Completed, false),
            log(11, "2026-11-11T10:00:00Z", LogKind::Completed, true),
            log(12, "2026-11-12T07:00:00Z", LogKind::State, false),
            log(13, "2026-11-12T08:00:00Z", LogKind::Completed, false),
            log(14, "2026-11-12T09:00:00Z", LogKind::Dropped, false),
        ]
    }

    fn run(objects: &Objects, query: &ReportQuery, stamps: &[FieldStamp]) -> Report {
        let log = entries();
        let comments = objects.comments.clone();
        let agents: HashSet<String> = ["claude".to_string()].into();
        let events = Events {
            log: &log,
            comments: &comments,
            stamps,
            agent_devices: &agents,
            account: "alice",
        };
        report(objects, &Settings::new(berlin()), now(), query, &events).unwrap()
    }

    fn week(q: &str) -> ReportQuery {
        ReportQuery {
            from: Some(day("2026-11-09")),
            to: Some(day("2026-11-15")),
            q: q.into(),
            ..ReportQuery::default()
        }
    }

    fn ids(list: &ReportList) -> Vec<Id> {
        list.rows.iter().map(|r| r.id).collect()
    }

    fn reasons(list: &ReportList, n: u8) -> Vec<(ReportReason, u32)> {
        list.rows
            .iter()
            .find(|r| r.id == id(n))
            .map(|r| r.reasons.iter().map(|c| (c.reason, c.count)).collect())
            .unwrap_or_default()
    }

    #[test]
    fn without_autor_only_a_persons_work_counts() {
        let report = run(&objects(), &week(""), &[]);
        let done = report.done.unwrap();
        // Latest first; the agent's 11 and the dropped 14 are not there.
        assert_eq!(ids(&done), [id(13), id(10)]);
        assert_eq!(done.rows[1].seconds, 3600);
        assert_eq!(done.rows[1].project, Some(id(1)));
        let worked = report.worked.unwrap();
        assert_eq!(
            reasons(&worked, 10),
            [(ReportReason::Clocked, 1), (ReportReason::Completed, 1)]
        );
        // The person's comment, not the agent's; the state; the child.
        assert_eq!(
            reasons(&worked, 12),
            [
                (ReportReason::State, 1),
                (ReportReason::Commented, 1),
                (ReportReason::ChildCompleted, 1)
            ]
        );
        assert_eq!(
            worked.rows.iter().find(|r| r.id == id(12)).unwrap().at,
            at("2026-11-12T10:00:00Z")
        );
        // Dropping is work, even when done leaves it out.
        assert_eq!(reasons(&worked, 14), [(ReportReason::Dropped, 1)]);
        assert!(reasons(&worked, 11).is_empty());
        assert_eq!(report.time.unwrap().seconds, 3600);
    }

    #[test]
    fn autor_agent_and_alle() {
        let agent = run(&objects(), &week("autor:agent"), &[]);
        assert_eq!(ids(&agent.done.unwrap()), [id(11)]);
        assert_eq!(
            reasons(agent.worked.as_ref().unwrap(), 12),
            [(ReportReason::Commented, 1)]
        );
        // Time has no author: the same either way.
        assert_eq!(agent.time.unwrap().seconds, 3600);
        let all = run(&objects(), &week("autor:alle"), &[]);
        assert_eq!(ids(&all.done.unwrap()), [id(13), id(11), id(10)]);
        assert_eq!(
            reasons(&all.worked.unwrap(), 12),
            [
                (ReportReason::State, 1),
                (ReportReason::Commented, 2),
                (ReportReason::ChildCompleted, 1)
            ]
        );
    }

    #[test]
    fn the_span_is_days_in_the_zone() {
        // Wednesday alone: the clock's half hour after midnight in Berlin.
        let query = ReportQuery {
            from: Some(day("2026-11-11")),
            ..ReportQuery::default()
        };
        let report = run(&objects(), &query, &[]);
        assert_eq!(ids(&report.done.unwrap()), [id(10)]);
        let worked = report.worked.unwrap();
        assert_eq!(worked.rows[0].seconds, 1800);
        assert_eq!(report.time.unwrap().seconds, 1800);
        // Dropped ones only when asked, marked.
        let query = ReportQuery {
            dropped: true,
            ..week("")
        };
        let done = run(&objects(), &query, &[]).done.unwrap();
        assert!(done.rows.iter().any(|r| r.id == id(14) && r.dropped));
    }

    #[test]
    fn the_filter_is_the_search_language() {
        let report = run(&objects(), &week("#Garten"), &[]);
        assert_eq!(ids(&report.done.unwrap()), [id(13)]);
        let worked = report.worked.unwrap();
        assert_eq!(ids(&worked), [id(12), id(13)]);
        assert_eq!(report.time.unwrap().seconds, 0);
        assert!(matches!(
            report_err(&week("Garten")),
            crate::views::ViewError::BadRequest(_)
        ));
        assert!(matches!(
            report_err(&week("autor:jemand")),
            crate::views::ViewError::BadRequest(_)
        ));
        // Only some parts.
        let query = ReportQuery {
            parts: vec![ReportPart::Done],
            ..week("")
        };
        let only = run(&objects(), &query, &[]);
        assert!(only.done.is_some() && only.worked.is_none() && only.time.is_none());
    }

    fn report_err(query: &ReportQuery) -> crate::views::ViewError {
        let agents = HashSet::new();
        let events = Events {
            log: &[],
            comments: &[],
            stamps: &[],
            agent_devices: &agents,
            account: "alice",
        };
        report(&objects(), &Settings::new(berlin()), now(), query, &events).unwrap_err()
    }

    #[test]
    fn groups_by_project_and_by_day() {
        let query = ReportQuery {
            by: ClockReportBy::Project,
            ..week("autor:alle")
        };
        let done = run(&objects(), &query, &[]).done.unwrap();
        let groups: Vec<(Option<Id>, Vec<Id>, i64)> = done
            .groups
            .iter()
            .map(|g| (g.id, g.rows.clone(), g.seconds))
            .collect();
        assert_eq!(
            groups,
            [
                (Some(id(1)), vec![id(11), id(10)], 3600),
                (Some(id(2)), vec![id(13)], 0)
            ]
        );
        assert_eq!(done.groups[0].title, "Haus");
        let query = ReportQuery {
            by: ClockReportBy::Day,
            ..week("")
        };
        let done = run(&objects(), &query, &[]).done.unwrap();
        let days: Vec<Option<Date>> = done.groups.iter().map(|g| g.date).collect();
        assert_eq!(days, [Some(day("2026-11-12")), Some(day("2026-11-11"))]);
        // By task: no groups.
        assert!(
            run(&objects(), &week(""), &[])
                .done
                .unwrap()
                .groups
                .is_empty()
        );
    }

    #[test]
    fn fields_counts_changes_by_the_device() {
        let stamps = vec![
            FieldStamp {
                object: id(12),
                field: "title".into(),
                at: at("2026-11-12T12:00:00Z"),
                device: "laptop".into(),
            },
            FieldStamp {
                object: id(12),
                field: "note".into(),
                at: at("2026-11-12T12:30:00Z"),
                device: "claude".into(),
            },
            // Completion counts once, with its author.
            FieldStamp {
                object: id(10),
                field: "completed_at".into(),
                at: at("2026-11-11T09:00:00Z"),
                device: "laptop".into(),
            },
        ];
        // `events`: the stamps are not read.
        let events = run(&objects(), &week(""), &stamps).worked.unwrap();
        assert!(
            !reasons(&events, 12)
                .iter()
                .any(|(r, _)| *r == ReportReason::Changed)
        );
        let mut fields = objects();
        fields.settings = Some(
            serde_json::from_value(json!({
                "id": id(90), "created": "2026-11-01T00:00:00Z",
                "modified": "2026-11-01T00:00:00Z", "due_soon": "P2D",
                "remind_time": "09:00:00", "report_worked": "fields"
            }))
            .unwrap(),
        );
        let worked = run(&fields, &week(""), &stamps).worked.unwrap();
        assert!(reasons(&worked, 12).contains(&(ReportReason::Changed, 1)));
        assert_eq!(
            reasons(&worked, 10),
            [(ReportReason::Clocked, 1), (ReportReason::Completed, 1)]
        );
        let agent = run(&fields, &week("autor:agent"), &stamps).worked.unwrap();
        assert!(reasons(&agent, 12).contains(&(ReportReason::Changed, 1)));
    }

    #[test]
    fn autor_comes_out_of_the_text() {
        assert_eq!(
            take_author("#Haus autor:alle ist:markiert", "alice").unwrap(),
            (Author::All, "#Haus ist:markiert".into())
        );
        assert_eq!(
            take_author("", "alice").unwrap(),
            (Author::Human, String::new())
        );
        assert!(take_author("-autor:agent", "alice").is_err());
        assert!(take_author("autor:agent autor:alice", "alice").is_err());
    }

    #[test]
    fn author_is_the_english_autor() {
        assert_eq!(
            take_author("#Haus author:all", "alice").unwrap(),
            (Author::All, "#Haus".into())
        );
        let agent = take_author("Author:Agent", "alice").unwrap().0;
        assert_eq!(agent, Author::Agent);
        let all = take_author("autor:alle author:all", "alice").unwrap().0;
        assert_eq!(all, Author::All);
        assert!(take_author("-author:agent", "alice").is_err());
        assert!(take_author("author:agent autor:alice", "alice").is_err());
        assert!(take_author("author:someone", "alice").is_err());
    }

    #[test]
    fn a_persons_work_is_named_by_the_account() {
        for word in [
            "autor:alice",
            "author:Alice",
            "autor:ich",
            "author:me",
            "autor:mensch",
            "author:human",
        ] {
            assert_eq!(
                take_author(word, "Alice").unwrap().0,
                Author::Human,
                "{word}"
            );
        }
        // Another account's name is not a person's work here.
        let wrong = take_author("autor:bob", "alice").unwrap_err();
        assert!(wrong.contains("alice, agent or alle"), "{wrong}");
    }
}
