// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What the reading routes answer, computed from the objects of an account
//! (ADR 0066 §4). The server reads them from its store, the replica on a
//! computer from its mirror with the outbox laid over; both call these
//! functions, so availability and the views are computed in one place
//! (ADR 0006).

use std::collections::HashMap;

use jiff::Timestamp;
use jiff::civil::Date;

use crate::api::{
    Appointment, AppointmentInstance, Block, CaptureLinesRequest, CaptureLinesResponse,
    CaptureRequest, CaptureResponse, Change, ClockReport, ClockReportBy, DueState, Folder,
    ForecastKind, ForecastRow, ForecastSection, ForecastWhy, Hlc, Id, Inline, LinkClass,
    LinkedObject, ObjectKind, Project, ProjectAvailability, ProjectStatus, Report, ReportPart,
    ReportWorked, ScheduledScope, Scope, Tagging, Task, TaskStatus,
};
use crate::availability::{Evaluation, Settings, TaskState, evaluate, group_summary};
use crate::overlay::Objects;
use crate::properties::Inheritance;
use crate::render::TaskRef;
use crate::scope::Lens;

/// Why a view could not be computed; each maps to one HTTP status.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ViewError {
    /// A scope the query names is not there (404, ADR 0035 §5).
    #[error("no scope {0}")]
    NoScope(Id),
    /// A perspective the query names is not there (404, ADR 0090 §2).
    #[error("no perspective {0}")]
    NoPerspective(Id),
    /// The task is not there or is no habit (404, ADR 0091 §4).
    #[error("no habit {0}")]
    NoHabit(Id),
    /// The query does not make sense (400).
    #[error("{0}")]
    BadRequest(String),
    /// The query is well formed but names what cannot be (422).
    #[error("{0}")]
    Unprocessable(String),
}

/// The scope a query names, or [`ViewError::NoScope`].
pub fn chosen_scope(objects: &Objects, scope: Option<Id>) -> Result<Option<&Scope>, ViewError> {
    scope
        .map(|id| {
            objects
                .scopes
                .iter()
                .find(|s| s.id == id)
                .ok_or(ViewError::NoScope(id))
        })
        .transpose()
}

/// What `GET /available` filters by.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AvailableFilter {
    /// Only available tasks.
    pub available: bool,
    /// Only tasks planned for today or earlier.
    pub today: bool,
    /// Only tasks of this project.
    pub project: Option<Id>,
    /// Completed and dropped tasks too.
    pub all: bool,
    /// Only what this scope shows, and what is due outside it marked
    /// `outside` (ADR 0035 §5/§6).
    pub scope: Option<Id>,
    /// Only what this perspective shows, inside the scope (ADR 0090 §2).
    pub perspective: Option<Id>,
    /// Also what an own scope other than this one hides, marked
    /// `elsewhere` (ADR 0124 §2).
    pub elsewhere: bool,
}

/// How far back a history may reach: ten years.
const HISTORY_DAYS: i64 = 3660;

/// `GET /habits`: the open habits the scope shows, in the order of
/// `GET /available`, with what samu-core computed for them (ADR 0091 §3, §4).
pub fn habits(
    objects: &Objects,
    settings: &Settings,
    now: Timestamp,
    scope: Option<Id>,
    from: Option<Date>,
) -> Result<Vec<crate::api::HabitStatus>, ViewError> {
    let filter = AvailableFilter {
        scope,
        ..AvailableFilter::default()
    };
    let statuses = available(objects, settings, now, &filter)?;
    Ok(statuses
        .iter()
        .filter(|s| !s.outside)
        .filter_map(|s| habit_of(objects, settings, now, &s.task, from))
        .collect())
}

/// `GET /habits/{id}`: one habit, also one completed or dropped.
pub fn habit(
    objects: &Objects,
    settings: &Settings,
    now: Timestamp,
    id: Id,
    from: Option<Date>,
) -> Result<crate::api::HabitStatus, ViewError> {
    objects
        .tasks
        .iter()
        .find(|t| t.id == id)
        .and_then(|t| habit_of(objects, settings, now, t, from))
        .ok_or(ViewError::NoHabit(id))
}

fn habit_of(
    objects: &Objects,
    settings: &Settings,
    now: Timestamp,
    task: &Task,
    from: Option<Date>,
) -> Option<crate::api::HabitStatus> {
    use jiff::ToSpan;
    let today = now.to_zoned(settings.zone.clone()).date();
    let begin = task.created.to_zoned(settings.zone.clone()).date();
    let earliest = today.checked_sub(HISTORY_DAYS.days()).unwrap_or(today);
    let from = from.map(|f| f.max(earliest));
    crate::habit::status(task, begin, &objects.habit_entries, today, from)
}

/// What a perspective asks of a task (ADR 0090 §2): its chips, its free
/// text and its filter, lifted as the search lifts it (ADR 0042 §4). The
/// app asks the same of its mirror (ADR 0102 §4).
pub struct PerspectiveTest<'a> {
    filter: crate::query::Filter<'a>,
    text: &'a str,
    /// Completed and dropped tasks too.
    pub all: bool,
    /// Only available tasks.
    pub available: bool,
}

impl<'a> PerspectiveTest<'a> {
    pub fn new(
        perspective: &'a crate::api::Perspective,
        view: &crate::availability::Snapshot<'a>,
        now: Timestamp,
        zone: &jiff::tz::TimeZone,
    ) -> Self {
        use crate::api::PerspectiveFilter as F;
        use crate::query::{asks_for_closed, asks_for_waiting};
        let chips = &perspective.chips;
        let mode = if asks_for_closed(chips) {
            F::All
        } else if perspective.filter == F::Remaining
            || (perspective.filter != F::All && asks_for_waiting(chips))
        {
            F::Remaining
        } else if perspective.filter == F::All {
            F::All
        } else {
            // `available`, and a value from a newer samu (ADR 0090 §1).
            F::Available
        };
        Self {
            filter: crate::query::Filter::new(chips, view, now, zone),
            text: &perspective.text,
            all: mode == F::All,
            available: mode == F::Available,
        }
    }

    /// Its chips and free text hold for the task; the filter is
    /// [`Self::all`] and [`Self::available`].
    pub fn task(&self, task: &Task, state: &TaskState) -> bool {
        self.filter.matches(task, state) && crate::query::text_matches(self.text, task)
    }

    /// Whether a word of its free text names the task by its id: then the
    /// filter does not hold for it (6aec8bd3).
    pub fn names(&self, task: &Task) -> bool {
        crate::query::names_task(self.text, task)
    }

    /// Whether the perspective shows a task the scope shows: its chips,
    /// its free text and its filter, which a task named by its id passes.
    pub fn shows(&self, status: &TaskStatus, state: &TaskState) -> bool {
        let open = status.task.completed_at.is_none() && status.task.dropped_at.is_none();
        let named = self.names(&status.task);
        (open || self.all || named)
            && (status.available || !self.available || named)
            && self.task(&status.task, state)
    }
}

/// The perspective a query names, or [`ViewError::NoPerspective`].
pub fn chosen_perspective(
    objects: &Objects,
    perspective: Option<Id>,
) -> Result<Option<&crate::api::Perspective>, ViewError> {
    perspective
        .map(|id| {
            objects
                .perspectives
                .iter()
                .find(|p| p.id == id)
                .ok_or(ViewError::NoPerspective(id))
        })
        .transpose()
}

/// `GET /available`: tasks with their computed status; overdue and due soon
/// first, then by project order (ADR 0006, ADR 0017).
pub fn available(
    objects: &Objects,
    settings: &Settings,
    now: Timestamp,
    filter: &AvailableFilter,
) -> Result<Vec<TaskStatus>, ViewError> {
    let view = objects.snapshot();
    let evaluation = evaluate(&view, now, settings);
    let lens = Lens::new(
        chosen_scope(objects, filter.scope)?,
        &view,
        now,
        &settings.zone,
    );
    let chosen = chosen_perspective(objects, filter.perspective)?;
    if let Some(perspective) = chosen.filter(|p| !p.blocks.is_empty()) {
        // A block perspective: every block's tasks once (ADR 0116 §4).
        let sections = blocks_of(objects, settings, now, filter.scope, perspective)?;
        return Ok(crate::blocks::union(sections));
    }
    let asked = chosen.map(|p| PerspectiveTest::new(p, &view, now, &settings.zone));
    let all = filter.all || asked.as_ref().is_some_and(|a| a.all);
    let only_available = filter.available || asked.as_ref().is_some_and(|a| a.available);
    let inheritance = Inheritance::new(&objects.tasks, &objects.projects);
    let mut statuses = Vec::new();
    for task in &objects.tasks {
        let Some(computed) = evaluation.tasks.get(&task.id) else {
            continue;
        };
        if asked.as_ref().is_some_and(|a| !a.task(task, computed)) {
            continue;
        }
        // Without a scope, too, what is scope_only stays out and counts
        // for the hint when due (ADR 0069 §4, §7); asked for, what another
        // own scope shows is marked with it (ADR 0124 §2).
        let shown = lens.task(task, computed);
        let outside = !shown && lens.due_outside(task, computed);
        let elsewhere = (!shown && filter.elsewhere)
            .then(|| lens.elsewhere_task(task))
            .flatten();
        if !shown && !outside && elsewhere.is_none() {
            continue;
        }
        let open = task.completed_at.is_none() && task.dropped_at.is_none();
        // A task the search names by its id passes the filter (6aec8bd3).
        let named = asked.as_ref().is_some_and(|a| a.names(task));
        if (!open && !all && !named)
            || (only_available && !computed.available && !named)
            || (filter.today && !computed.today)
            || (filter.project.is_some() && task.project != filter.project)
        {
            continue;
        }
        statuses.push(status_of(
            objects,
            &evaluation,
            &inheritance,
            task,
            computed,
            (outside, elsewhere),
            (now, settings),
        ));
    }
    statuses.sort_by_key(|s| {
        (
            match s.due {
                DueState::Overdue => 0,
                DueState::Soon => 1,
                _ => 2,
            },
            s.task.project,
            s.task.position,
            s.task.id,
        )
    });
    Ok(statuses)
}

/// `GET /blocks`: what a perspective shows, block by block (ADR 0116 §3-4);
/// a plain perspective is one block. Only what the scope shows, no task
/// marked `outside`.
pub fn blocks(
    objects: &Objects,
    settings: &Settings,
    now: Timestamp,
    scope: Option<Id>,
    perspective: Id,
) -> Result<Vec<crate::api::BlockSection>, ViewError> {
    let chosen = chosen_perspective(objects, Some(perspective))?
        .ok_or(ViewError::NoPerspective(perspective))?;
    blocks_of(objects, settings, now, scope, chosen)
}

/// What a note block shows of a live project or task (ADR 0146 §2),
/// whatever the scope.
fn note_source(objects: &Objects, id: Id) -> Option<crate::blocks::NoteSource> {
    if let Some(p) = objects.projects.iter().find(|p| p.id == id) {
        return Some(crate::blocks::NoteSource {
            title: p.title.clone(),
            icon: p.icon.clone(),
            note: p.note.clone().unwrap_or_default(),
        });
    }
    objects
        .tasks
        .iter()
        .find(|t| t.id == id)
        .map(|t| crate::blocks::NoteSource {
            title: t.title.clone(),
            icon: None,
            note: t.note.clone().unwrap_or_default(),
        })
}

fn blocks_of(
    objects: &Objects,
    settings: &Settings,
    now: Timestamp,
    scope: Option<Id>,
    perspective: &crate::api::Perspective,
) -> Result<Vec<crate::api::BlockSection>, ViewError> {
    let everything = AvailableFilter {
        all: true,
        scope,
        ..AvailableFilter::default()
    };
    let statuses: Vec<TaskStatus> = available(objects, settings, now, &everything)?
        .into_iter()
        .filter(|s| !s.outside)
        .collect();
    let view = objects.snapshot();
    let evaluation = evaluate(&view, now, settings);
    let tests: HashMap<Id, PerspectiveTest<'_>> = objects
        .perspectives
        .iter()
        .filter(|p| p.blocks.is_empty())
        .map(|p| (p.id, PerspectiveTest::new(p, &view, now, &settings.zone)))
        .collect();
    // The searches of the blocks' own (ADR 0143 §2), each tested once.
    let queries = crate::blocks::queries(perspective);
    let query_tests: HashMap<usize, PerspectiveTest<'_>> = queries
        .iter()
        .map(|(i, p)| (*i, PerspectiveTest::new(p, &view, now, &settings.zone)))
        .collect();
    Ok(crate::blocks::assemble(
        perspective,
        &objects.perspectives,
        &statuses,
        |source, status| {
            let test = match source {
                crate::blocks::Shown::Perspective(p) => tests.get(&p.id),
                crate::blocks::Shown::Query(i) => query_tests.get(&i),
            };
            let state = evaluation.tasks.get(&status.task.id);
            test.zip(state).is_some_and(|(t, s)| t.shows(status, s))
        },
        |query| crate::blocks::query_name(query, &view),
        |id| note_source(objects, id),
    ))
}

/// A task with what was computed for it, as `/available` and `/forecast`
/// answer.
fn status_of(
    objects: &Objects,
    evaluation: &Evaluation,
    inheritance: &Inheritance<'_>,
    task: &Task,
    computed: &TaskState,
    (outside, elsewhere): (bool, Option<Id>),
    // Now and the account's zone, for the next date of a repetition.
    (now, settings): (Timestamp, &Settings),
) -> TaskStatus {
    let project = task
        .project
        .and_then(|p| objects.projects.iter().find(|q| q.id == p));
    let placed = crate::workflow::placement(&task.tags, project, &objects.tags);
    TaskStatus {
        task: task.clone(),
        available: computed.available,
        reasons: computed.reasons.iter().map(Into::into).collect(),
        due: computed.due.into(),
        today: computed.today,
        workflow: placed.workflow,
        state: placed.state,
        next_state: placed.next,
        group: group_summary(&objects.tasks, evaluation, task),
        outside,
        elsewhere,
        inbox: computed.inbox,
        inherited: inheritance.of(task),
        next_repeat: crate::repeat::preview(&objects.tasks, task, now, &settings.zone),
    }
}

/// `GET /forecast`: the sections of Forecast with the account's settings
/// (ADR 0045), and the appointments of the days through the scope's
/// calendars (ADR 0047 §10, §12).
pub fn forecast(
    objects: &Objects,
    appointments: &[Appointment],
    settings: &Settings,
    now: Timestamp,
    scope: Option<Id>,
) -> Result<Vec<ForecastSection>, ViewError> {
    use crate::calendar::occurrences;
    use crate::forecast::{Options, Section, Why, forecast, horizon, with_appointments};

    let view = objects.snapshot();
    let evaluation = evaluate(&view, now, settings);
    let scope = chosen_scope(objects, scope)?;
    let lens = Lens::new(scope, &view, now, &settings.zone);
    let options = objects
        .settings
        .as_ref()
        .map(Options::from)
        .unwrap_or_default();
    let sections = forecast(
        &view,
        &evaluation,
        &options,
        now,
        &settings.zone,
        |task, computed| lens.task(task, computed),
    );
    let today = now.to_zoned(settings.zone.clone()).date();
    let last = horizon(today, &options);
    let instances = if options.events {
        let only = scope.map(|s| s.calendars.clone()).unwrap_or_default();
        occurrences(
            appointments,
            &objects.calendars,
            &only,
            today,
            last,
            &settings.zone,
        )
    } else {
        Vec::new()
    };
    let inheritance = Inheritance::new(&objects.tasks, &objects.projects);
    let parts = with_appointments(sections, &instances, today, last, &settings.zone);
    Ok(parts
        .into_iter()
        .map(|part| {
            let (kind, date) = match part.section {
                Section::Overdue => (ForecastKind::Overdue, None),
                Section::Day(d) => (ForecastKind::Day, Some(d)),
                Section::Flagged => (ForecastKind::Flagged, None),
                Section::Later => (ForecastKind::Later, None),
            };
            let rows = part
                .rows
                .into_iter()
                .filter_map(|row| {
                    let computed = evaluation.tasks.get(&row.task.id)?;
                    Some(ForecastRow {
                        why: match row.why {
                            Why::Due => ForecastWhy::Due,
                            Why::Planned => ForecastWhy::Planned,
                            Why::Defer => ForecastWhy::Defer,
                            Why::Tag => ForecastWhy::Tag,
                            Why::Done => ForecastWhy::Done,
                            Why::Flagged => ForecastWhy::Flagged,
                        },
                        date: row.date,
                        status: status_of(
                            objects,
                            &evaluation,
                            &inheritance,
                            row.task,
                            computed,
                            (false, None),
                            (now, settings),
                        ),
                    })
                })
                .collect();
            ForecastSection {
                section: kind,
                date,
                rows,
                appointments: part.appointments,
            }
        })
        .collect())
}

/// `GET /appointments`: the occurrences of the days from `from` to `to`
/// (today and `from` by default) in the account's zone, through the
/// scope's calendars (ADR 0047 §5, §12, ADR 0048).
pub fn appointments(
    objects: &Objects,
    appointments: &[Appointment],
    settings: &Settings,
    now: Timestamp,
    (from, to): (Option<Date>, Option<Date>),
    scope: Option<Id>,
) -> Result<Vec<AppointmentInstance>, ViewError> {
    let from = from.unwrap_or_else(|| now.to_zoned(settings.zone.clone()).date());
    let to = to.unwrap_or(from);
    if to < from {
        return Err(ViewError::BadRequest("to is before from".into()));
    }
    if to.since(from).map_or(true, |span| span.get_days() > 400) {
        return Err(ViewError::BadRequest("at most 400 days at once".into()));
    }
    let only = chosen_scope(objects, scope)?
        .map(|s| s.calendars.clone())
        .unwrap_or_default();
    Ok(crate::calendar::occurrences(
        appointments,
        &objects.calendars,
        &only,
        from,
        to,
        &settings.zone,
    ))
}

/// What `GET /alarms` asks for (ADR 0101 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlertsQuery {
    /// Reminders after this instant; now without it.
    pub from: Option<Timestamp>,
    /// Up to this instant, included; a day after `from` without it.
    pub to: Option<Timestamp>,
    pub wanted: crate::reminders::Wanted,
}

/// The longest window `GET /alarms` answers for.
pub const ALERTS_DAYS: i64 = 7;

/// `GET /alarms`: the reminders of tasks and of appointments in a window
/// of at most [`ALERTS_DAYS`] days, by time (ADR 0101 §4, §5).
pub fn alerts(
    objects: &Objects,
    appointments: &[Appointment],
    settings: &Settings,
    now: Timestamp,
    query: AlertsQuery,
) -> Result<Vec<crate::api::Alert>, ViewError> {
    let day = jiff::SignedDuration::from_hours(24);
    let from = query.from.unwrap_or(now);
    let to = query
        .to
        .unwrap_or_else(|| from.saturating_add(day).unwrap_or(from));
    if to < from {
        return Err(ViewError::BadRequest("to is before from".into()));
    }
    if to.duration_since(from) > day * i32::try_from(ALERTS_DAYS).unwrap_or(7) {
        return Err(ViewError::BadRequest(format!(
            "at most {ALERTS_DAYS} days at once"
        )));
    }
    Ok(crate::reminders::alerts(
        &objects.snapshot(),
        &objects.reminders,
        &objects.habit_entries,
        appointments,
        &objects.calendars,
        settings,
        (from, to),
        query.wanted,
    ))
}

/// What `GET /clocks/report` asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClockReportQuery {
    /// The first day; today in the account's zone without it.
    pub from: Option<Date>,
    /// The last day, included; `from` without it.
    pub to: Option<Date>,
    pub by: ClockReportBy,
    /// Only clocks on what this scope shows (ADR 0035 §5).
    pub scope: Option<Id>,
}

/// `GET /clocks/report`: the time clocked over days in the account's zone
/// (ADR 0046 §5).
pub fn clock_report(
    objects: &Objects,
    settings: &Settings,
    now: Timestamp,
    query: &ClockReportQuery,
) -> Result<ClockReport, ViewError> {
    if query.by == ClockReportBy::Unknown {
        return Err(ViewError::BadRequest(
            "by: task, project, folder, tag or day".into(),
        ));
    }
    let from = query
        .from
        .unwrap_or_else(|| now.to_zoned(settings.zone.clone()).date());
    let to = query.to.unwrap_or(from);
    if to < from {
        return Err(ViewError::BadRequest("to is before from".into()));
    }
    if to.since(from).map_or(true, |span| span.get_days() > 3660) {
        return Err(ViewError::BadRequest("at most ten years at once".into()));
    }
    let view = objects.snapshot();
    let evaluation = evaluate(&view, now, settings);
    let lens = Lens::new(
        chosen_scope(objects, query.scope)?,
        &view,
        now,
        &settings.zone,
    );
    let tasks: HashMap<Id, &Task> = objects.tasks.iter().map(|t| (t.id, t)).collect();
    let projects: HashMap<Id, &Project> = objects.projects.iter().map(|p| (p.id, p)).collect();
    Ok(crate::clocking::report(
        &view,
        from,
        to,
        query.by,
        &settings.zone,
        now,
        |clock| {
            if let Some(task) = tasks.get(&clock.owner) {
                return evaluation
                    .tasks
                    .get(&task.id)
                    .is_some_and(|computed| lens.task(task, computed));
            }
            projects.get(&clock.owner).is_some_and(|p| lens.project(p))
        },
    ))
}

/// What `GET /report` asks for (ADR 0080 §4).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReportQuery {
    /// The first day; today in the account's zone without it.
    pub from: Option<Date>,
    /// The last day, included; `from` without it.
    pub to: Option<Date>,
    pub by: ClockReportBy,
    /// Only what this scope shows (ADR 0035 §5).
    pub scope: Option<Id>,
    /// A filter in the search language, `autor:` among it (ADR 0080 §2).
    pub q: String,
    /// The parts; all three without any.
    pub parts: Vec<ReportPart>,
    /// Dropped tasks in `done`.
    pub dropped: bool,
}

/// The days of a report: `from`, today without it, to `to`, `from`
/// without it.
fn report_days(
    settings: &Settings,
    now: Timestamp,
    query: &ReportQuery,
) -> Result<(Date, Date), ViewError> {
    let from = query
        .from
        .unwrap_or_else(|| now.to_zoned(settings.zone.clone()).date());
    let to = query.to.unwrap_or(from);
    if to < from {
        return Err(ViewError::BadRequest("to is before from".into()));
    }
    if to.since(from).map_or(true, |span| span.get_days() > 3660) {
        return Err(ViewError::BadRequest("at most ten years at once".into()));
    }
    Ok((from, to))
}

/// The instants a report's days run between in the account's zone: the
/// start of `from` and the start of the day after `to`. The store reads
/// the logbook, comments and stamps of this span (ADR 0080 §4).
pub fn report_span(
    settings: &Settings,
    now: Timestamp,
    query: &ReportQuery,
) -> Result<(Timestamp, Timestamp), ViewError> {
    use jiff::ToSpan;
    let (from, to) = report_days(settings, now, query)?;
    let start = |day: Date| {
        day.to_zoned(settings.zone.clone())
            .map(|z| z.timestamp())
            .map_err(|e| ViewError::BadRequest(e.to_string()))
    };
    let after = to
        .checked_add(1.day())
        .map_err(|e| ViewError::BadRequest(e.to_string()))?;
    Ok((start(from)?, start(after)?))
}

/// `parts=done,worked,time` as the parts; none for all three.
pub fn report_parts(text: Option<&str>) -> Result<Vec<ReportPart>, ViewError> {
    text.unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| match p {
            "done" => Ok(ReportPart::Done),
            "worked" => Ok(ReportPart::Worked),
            "time" => Ok(ReportPart::Time),
            _ => Err(ViewError::BadRequest(format!(
                "parts: {p:?} is none of done, worked, time"
            ))),
        })
        .collect()
}

/// `GET /report`: done, worked on and time over days in the account's zone
/// (ADR 0080). `events` holds the logbook from the span's start, the
/// comments, the field stamps of the span and the agents' devices; the
/// replica, which has no logbook, passes empty ones and answers no
/// `worked`.
pub fn report(
    objects: &Objects,
    settings: &Settings,
    now: Timestamp,
    query: &ReportQuery,
    events: &crate::report::Events<'_>,
) -> Result<Report, ViewError> {
    if query.by == ClockReportBy::Unknown {
        return Err(ViewError::BadRequest(
            "by: task, project, folder, tag or day".into(),
        ));
    }
    if query.parts.contains(&ReportPart::Unknown) {
        return Err(ViewError::BadRequest("parts: done, worked or time".into()));
    }
    let (from, to) = report_days(settings, now, query)?;
    let (author, rest) =
        crate::report::take_author(&query.q, events.account).map_err(ViewError::BadRequest)?;
    let view = objects.snapshot();
    let chips =
        crate::query::parse(&rest, &view).map_err(|e| ViewError::BadRequest(format!("q: {e}")))?;
    let filter = crate::query::Filter::new(&chips, &view, now, &settings.zone);
    let evaluation = evaluate(&view, now, settings);
    let lens = Lens::new(
        chosen_scope(objects, query.scope)?,
        &view,
        now,
        &settings.zone,
    );
    let all = query.parts.is_empty();
    let fields =
        objects.settings.as_ref().and_then(|s| s.report_worked) == Some(ReportWorked::Fields);
    let ask = crate::report::Ask {
        from,
        to,
        by: query.by,
        done: all || query.parts.contains(&ReportPart::Done),
        worked: all || query.parts.contains(&ReportPart::Worked),
        time: all || query.parts.contains(&ReportPart::Time),
        dropped: query.dropped,
        author,
        fields,
    };
    let keep_task = |task: &Task| {
        evaluation
            .tasks
            .get(&task.id)
            .is_some_and(|state| lens.task(task, state) && filter.matches(task, state))
    };
    // A filter tests tasks; a project counts only without one.
    let keep_project = |project: &Project| filter.is_empty() && lens.project(project);
    Ok(crate::report::compute(
        &view,
        events,
        &ask,
        &settings.zone,
        now,
        &keep_task,
        &keep_project,
    ))
}

/// What `GET /available/projects` filters by.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProjectsFilter {
    /// Only stuck projects.
    pub stuck: bool,
    /// Only projects due for review, by review date (ADR 0088 §4).
    pub review: bool,
    /// Completed and dropped projects too.
    pub all: bool,
    /// Only the projects this scope shows (ADR 0035 §5).
    pub scope: Option<Id>,
    /// Also those an own scope other than this one shows, marked
    /// `elsewhere` (ADR 0124 §2).
    pub elsewhere: bool,
}

/// `GET /available/projects`: projects with their computed status, from the
/// same evaluation as [`available`], in project order.
pub fn projects(
    objects: &Objects,
    settings: &Settings,
    now: Timestamp,
    filter: &ProjectsFilter,
) -> Result<Vec<ProjectAvailability>, ViewError> {
    let view = objects.snapshot();
    let evaluation = evaluate(&view, now, settings);
    let lens = Lens::new(
        chosen_scope(objects, filter.scope)?,
        &view,
        now,
        &settings.zone,
    );
    let mut out = Vec::new();
    for project in &objects.projects {
        let elsewhere = (filter.elsewhere && !lens.project(project))
            .then(|| lens.elsewhere_project(project))
            .flatten();
        if !lens.project(project) && elsewhere.is_none() {
            continue;
        }
        let Some(computed) = evaluation.projects.get(&project.id) else {
            continue;
        };
        let closed = matches!(
            project.status,
            ProjectStatus::Completed | ProjectStatus::Dropped
        );
        if (closed && !filter.all)
            || (filter.stuck && !computed.stuck)
            || (filter.review && !computed.review_due)
        {
            continue;
        }
        out.push(ProjectAvailability {
            project: project.clone(),
            elsewhere,
            available: computed.available,
            reasons: computed.reasons.iter().map(Into::into).collect(),
            stuck: computed.stuck,
            review: computed.review,
            review_due: computed.review_due,
        });
    }
    if filter.review {
        out.sort_by_key(|p| (p.review, p.project.position, p.project.id));
    } else {
        out.sort_by_key(|p| (p.project.position, p.project.id));
    }
    Ok(out)
}

/// `GET /available/folders`: the folders the views show, by position; what
/// `Lens::folder` keeps, with or without a scope (ADR 0037 §4, ADR 0069 §4).
pub fn folders(
    objects: &Objects,
    settings: &Settings,
    now: Timestamp,
    scope: Option<Id>,
) -> Result<Vec<Folder>, ViewError> {
    let view = objects.snapshot();
    let lens = Lens::new(chosen_scope(objects, scope)?, &view, now, &settings.zone);
    let mut out: Vec<Folder> = objects
        .folders
        .iter()
        .filter(|folder| lens.folder(folder))
        .cloned()
        .collect();
    out.sort_by_key(|f| (f.position, f.id));
    Ok(out)
}

/// `GET /scopes/scheduled`: what the schedules of the scopes say now
/// (ADR 0043 §7).
#[must_use]
pub fn scheduled(objects: &Objects, settings: &Settings, now: Timestamp) -> ScheduledScope {
    let now = crate::scope::schedule_now(&objects.scopes, now, &settings.zone);
    ScheduledScope {
        scope: now.scope,
        since: now.since,
        until: now.until,
    }
}

/// Whether a document links to a task, so the account must be read to
/// resolve it (ADR 0058 §4).
#[must_use]
pub fn has_task_link(blocks: &[Block]) -> bool {
    fn inlines(content: &[Inline]) -> bool {
        content.iter().any(|inline| match inline {
            Inline::Link { class, content, .. } => *class == LinkClass::Task || inlines(content),
            Inline::Strong { content, .. }
            | Inline::Emphasis { content, .. }
            | Inline::Underline { content, .. }
            | Inline::Strike { content, .. } => inlines(content),
            _ => false,
        })
    }
    blocks.iter().any(|block| match block {
        Block::Heading { content, .. }
        | Block::Paragraph { content, .. }
        | Block::Verse { content, .. } => inlines(content),
        Block::List { items, .. } => items
            .iter()
            .any(|item| item.term.as_deref().is_some_and(inlines) || has_task_link(&item.blocks)),
        Block::Table { rows, .. } => rows
            .iter()
            .any(|row| row.cells.iter().any(|cell| inlines(cell))),
        Block::Quote { blocks, .. }
        | Block::Center { blocks, .. }
        | Block::Footnote { blocks, .. } => has_task_link(blocks),
        _ => false,
    })
}

/// Resolve the `task` links of `documents` in the account: `samu:` by id
/// among tasks, projects, folders and tags, `id:` by `org_id` of a task or
/// project (ADR 0058 §4, ADR 0077 §1).
pub fn resolve_task_links(documents: &mut [crate::api::Document], objects: &Objects) {
    let find = linked_objects(objects);
    for document in documents {
        crate::render::resolve_links(document, &find);
    }
}

/// How `task` links are looked up in `objects`, for the server and the app
/// alike (ADR 0077 §5).
pub fn linked_objects(objects: &Objects) -> impl Fn(&TaskRef) -> Option<LinkedObject> + '_ {
    let kinds: HashMap<Id, ObjectKind> = objects
        .tasks
        .iter()
        .map(|t| (t.id, ObjectKind::Task))
        .chain(objects.projects.iter().map(|p| (p.id, ObjectKind::Project)))
        .chain(objects.folders.iter().map(|f| (f.id, ObjectKind::Folder)))
        .chain(objects.tags.iter().map(|t| (t.id, ObjectKind::Tag)))
        .collect();
    let org_ids: HashMap<&str, LinkedObject> = objects
        .tasks
        .iter()
        .filter_map(|t| {
            t.properties.get("org_id").map(|o| {
                let kind = ObjectKind::Task;
                (o.as_str(), LinkedObject { id: t.id, kind })
            })
        })
        .chain(objects.projects.iter().filter_map(|p| {
            p.properties.get("org_id").map(|o| {
                let kind = ObjectKind::Project;
                (o.as_str(), LinkedObject { id: p.id, kind })
            })
        }))
        .collect();
    move |r: &TaskRef| match r {
        TaskRef::Id(id) => kinds.get(id).map(|kind| LinkedObject {
            id: *id,
            kind: *kind,
        }),
        TaskRef::OrgId(org) => org_ids.get(org.as_str()).copied(),
    }
}

/// A line of quick capture made into changes (ADR 0027): the answer, and
/// what to write unless it is a preview: new tags, the task without tags,
/// then each tag as an `add`. Every change carries `placeholder` as its
/// stamp; the writer stamps each before it applies or queues it.
#[derive(Debug, Clone, PartialEq)]
pub struct Capture {
    pub answer: CaptureResponse,
    pub changes: Vec<Change>,
}

/// `POST /capture`: parse the line, resolve its names in the account and
/// describe the task (ADR 0027, ADR 0043 §6, ADR 0062 §3).
pub fn capture(
    objects: &Objects,
    settings: &Settings,
    request: &CaptureRequest,
    now: Timestamp,
    placeholder: &Hlc,
    new_id: &mut impl FnMut() -> Id,
) -> Result<Capture, ViewError> {
    use crate::capture::{
        Known, apply_template, parse, place_under, resolve_in_scope, template_reminders,
    };

    let mut draft = parse(&request.text, now, &settings.zone);
    // The template fills in before the scope does: line, then template,
    // then scope (ADR 0073 §4).
    let template = apply_template(
        &mut draft,
        &objects.templates,
        request.template,
        now,
        &settings.zone,
    );
    let known = Known {
        projects: &objects.projects,
        tags: &objects.tags,
        tasks: &objects.tasks,
    };
    if let Some(id) = request.task {
        return capture_edit(
            objects,
            settings,
            request,
            id,
            draft,
            known,
            template,
            now,
            placeholder,
            new_id,
        );
    }
    // The scope chosen on the device fills in what the line names not
    // (ADR 0043 §6); one that is gone counts as none (ADR 0035 §4).
    let scope = request
        .scope
        .and_then(|id| objects.scopes.iter().find(|s| s.id == id));
    // The task it goes under must be an open task of this account.
    let parent = match request.parent {
        Some(id) => {
            let parent = objects.tasks.iter().find(|t| t.id == id).ok_or_else(|| {
                ViewError::Unprocessable(format!("no task {id} to capture under"))
            })?;
            if parent.completed_at.is_some() || parent.dropped_at.is_some() {
                return Err(ViewError::Unprocessable(format!(
                    "the task {id} is closed; nothing goes under it"
                )));
            }
            Some(parent)
        }
        None => None,
    };
    let mut resolved = resolve_in_scope(draft, known, request.project, scope, now, new_id);
    if let Some(parent) = parent {
        place_under(&mut resolved, parent, &objects.tasks);
    }
    let task = resolved.task.clone();
    // The template's reminders, on the new task (ADR 0073 §1).
    let reminders = template_reminders(template, task.id, now, new_id);
    let booked = booked(&resolved.draft, objects, task.id, settings, now, new_id);
    let mut answer = CaptureResponse {
        task: task.clone(),
        created: false,
        new_tags: resolved.new_tags.iter().map(|t| t.title.clone()).collect(),
        marks: resolved.draft.marks.iter().map(|&m| m.into()).collect(),
        warnings: resolved
            .draft
            .warnings
            .iter()
            .chain(&booked.warnings)
            .map(Into::into)
            .collect(),
        reminders: reminders.clone(),
        clocks: booked.clocks.clone(),
    };
    if booked.completed_at.is_some() {
        answer.task.completed_at = booked.completed_at;
    }
    if request.preview {
        answer.task.tags = resolved
            .tags
            .iter()
            .zip(0..)
            .map(|(&tag, position)| Tagging { tag, position })
            .collect();
        return Ok(Capture {
            answer,
            changes: Vec::new(),
        });
    }
    let to_value = |value: serde_json::Result<serde_json::Value>| {
        value.map_err(|e| ViewError::BadRequest(e.to_string()))
    };
    let mut changes = Vec::new();
    for tag in &resolved.new_tags {
        changes.push(Change::Create {
            object: tag.id,
            kind: ObjectKind::Tag,
            value: to_value(serde_json::to_value(tag))?,
            hlc: placeholder.clone(),
        });
    }
    changes.push(Change::Create {
        object: task.id,
        kind: ObjectKind::Task,
        value: to_value(serde_json::to_value(&task))?,
        hlc: placeholder.clone(),
    });
    for (&tag, position) in resolved.tags.iter().zip(0..) {
        changes.push(Change::Add {
            object: task.id,
            field: "tags".into(),
            element: tag,
            position,
            note: None,
            hlc: placeholder.clone(),
        });
    }
    for reminder in &reminders {
        changes.push(Change::Create {
            object: reminder.id,
            kind: ObjectKind::Reminder,
            value: to_value(serde_json::to_value(reminder))?,
            hlc: placeholder.clone(),
        });
    }
    booked.push_changes(task.id, placeholder, &mut changes)?;
    Ok(Capture { answer, changes })
}

/// Several lines of quick capture made into changes (ADR 0129): an answer
/// per line, and what to write for all of them unless it is a preview.
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureLines {
    pub answer: CaptureLinesResponse,
    pub changes: Vec<Change>,
}

/// `POST /capture/lines`: take the text apart with
/// [`crate::capture::lines`] and capture each line as [`capture`] does,
/// over what the lines before it made, so an indented line goes under the
/// task of its line and a tag new in one line is known to the next. A line
/// that fails fails them all; the error names it.
pub fn capture_lines(
    objects: &Objects,
    settings: &Settings,
    request: &CaptureLinesRequest,
    now: Timestamp,
    placeholder: &Hlc,
    new_id: &mut impl FnMut() -> Id,
) -> Result<CaptureLines, ViewError> {
    let lines = crate::capture::lines(&request.text);
    if lines.is_empty() {
        return Err(ViewError::BadRequest("nothing to capture".into()));
    }
    let mut laid = objects.clone();
    let mut tasks: Vec<CaptureResponse> = Vec::new();
    let mut changes = Vec::new();
    for (number, line) in (1..).zip(&lines) {
        let in_line = |e: ViewError| match e {
            ViewError::BadRequest(m) => ViewError::BadRequest(format!("line {number}: {m}")),
            ViewError::Unprocessable(m) => ViewError::Unprocessable(format!("line {number}: {m}")),
            other => other,
        };
        let one = CaptureRequest {
            text: line.text.to_owned(),
            project: request.project,
            parent: match line.parent {
                Some(index) => tasks.get(index).map(|t| t.task.id),
                None => request.parent,
            },
            scope: request.scope,
            template: request.template,
            task: None,
            preview: false,
        };
        let made = capture(&laid, settings, &one, now, placeholder, new_id).map_err(in_line)?;
        for change in &made.changes {
            crate::overlay::apply(&mut laid, change)
                .map_err(|e| in_line(ViewError::Unprocessable(e.to_string())))?;
        }
        let mut answer = made.answer;
        // The task as the changes lay it, tags and `x:` included.
        if let Some(crate::api::AnyObject::Task(task)) = laid.get(answer.task.id) {
            answer.task = task;
        }
        answer.created = true;
        tasks.push(answer);
        changes.extend(made.changes);
    }
    if request.preview {
        changes.clear();
    }
    Ok(CaptureLines {
        answer: CaptureLinesResponse { tasks },
        changes,
    })
}

/// What `z:` and `x:` of a line make (ADR 0081 §2).
struct Booked {
    clocks: Vec<crate::api::Clock>,
    completed_at: Option<Timestamp>,
    warnings: Vec<crate::capture::Warning>,
}

/// The clocks of the line's `z:` on `owner`, placed among the account's
/// clocks, and when its `x:` completes it.
fn booked(
    draft: &crate::capture::Draft,
    objects: &Objects,
    owner: Id,
    settings: &Settings,
    now: Timestamp,
    new_id: &mut impl FnMut() -> Id,
) -> Booked {
    let (clocks, warnings) = crate::capture::place_clocks(
        &draft.clocks,
        &objects.clocks,
        owner,
        &settings.zone,
        now,
        new_id,
    );
    let completed_at = draft.completed.map(|day| {
        let all: Vec<_> = objects.clocks.iter().chain(&clocks).cloned().collect();
        crate::capture::completed_at(day, &all, &settings.zone, now)
    });
    Booked {
        clocks,
        completed_at,
        warnings,
    }
}

impl Booked {
    /// The clocks, then the completion: last, so the logbook has it and
    /// no clock is left running by it.
    fn push_changes(
        &self,
        task: Id,
        placeholder: &Hlc,
        changes: &mut Vec<Change>,
    ) -> Result<(), ViewError> {
        for clock in &self.clocks {
            changes.push(Change::Create {
                object: clock.id,
                kind: ObjectKind::Clock,
                value: serde_json::to_value(clock)
                    .map_err(|e| ViewError::BadRequest(e.to_string()))?,
                hlc: placeholder.clone(),
            });
        }
        if let Some(at) = self.completed_at {
            changes.push(Change::Set {
                object: task,
                field: "completed_at".into(),
                value: serde_json::to_value(at)
                    .map_err(|e| ViewError::BadRequest(e.to_string()))?,
                note: None,
                hlc: placeholder.clone(),
            });
        }
        Ok(())
    }
}

/// `POST /capture` with `task` (ADR 0078 §2, §3): the line applied to that
/// task. The answer is the task as it would be; the changes are new tags,
/// the fields, the descendants that follow into a new project, the tags
/// added, then the template's reminders the task lacks. Free text refuses
/// all of it, but a preview shows it.
#[allow(clippy::too_many_arguments, reason = "the parts of one capture")]
fn capture_edit(
    objects: &Objects,
    settings: &Settings,
    request: &CaptureRequest,
    id: Id,
    draft: crate::capture::Draft,
    known: crate::capture::Known<'_>,
    template: Option<&crate::api::Template>,
    now: Timestamp,
    placeholder: &Hlc,
    new_id: &mut impl FnMut() -> Id,
) -> Result<Capture, ViewError> {
    use crate::capture::{Warning, apply_to, missing_reminders, template_reminders};

    if request.parent.is_some() {
        return Err(ViewError::BadRequest(
            "a capture edits a task or makes one under a parent, not both".into(),
        ));
    }
    let task = objects
        .tasks
        .iter()
        .find(|t| t.id == id)
        .ok_or_else(|| ViewError::Unprocessable(format!("no task {id} to edit")))?;
    let applied = apply_to(task, draft, known, now, new_id);
    let present: Vec<_> = objects
        .reminders
        .iter()
        .filter(|r| r.owner == id)
        .cloned()
        .collect();
    let reminders = missing_reminders(template_reminders(template, id, now, new_id), &present);
    let booked = booked(&applied.draft, objects, id, settings, now, new_id);
    let mut answer = CaptureResponse {
        task: applied.task.clone(),
        created: false,
        new_tags: applied.new_tags.iter().map(|t| t.title.clone()).collect(),
        marks: applied.draft.marks.iter().map(|&m| m.into()).collect(),
        warnings: applied
            .draft
            .warnings
            .iter()
            .chain(&booked.warnings)
            .map(Into::into)
            .collect(),
        reminders: reminders.clone(),
        clocks: booked.clocks.clone(),
    };
    if booked.completed_at.is_some() {
        answer.task.completed_at = booked.completed_at;
    }
    if request.preview {
        return Ok(Capture {
            answer,
            changes: Vec::new(),
        });
    }
    if let Some(Warning::FreeText { text }) = applied
        .draft
        .warnings
        .iter()
        .find(|w| matches!(w, Warning::FreeText { .. }))
    {
        return Err(ViewError::Unprocessable(format!(
            "{text:?} is no mark; an edit takes only marks, the title has its own field"
        )));
    }
    let to_value = |value: serde_json::Result<serde_json::Value>| {
        value.map_err(|e| ViewError::BadRequest(e.to_string()))
    };
    let mut changes = Vec::new();
    for tag in &applied.new_tags {
        changes.push(Change::Create {
            object: tag.id,
            kind: ObjectKind::Tag,
            value: to_value(serde_json::to_value(tag))?,
            hlc: placeholder.clone(),
        });
    }
    for (field, value) in &applied.fields {
        changes.push(Change::Set {
            object: id,
            field: field.clone(),
            value: value.clone(),
            note: None,
            hlc: placeholder.clone(),
        });
    }
    for &child in &applied.moved {
        changes.push(Change::Set {
            object: child,
            field: "project".into(),
            value: to_value(serde_json::to_value(applied.task.project))?,
            note: None,
            hlc: placeholder.clone(),
        });
    }
    for &(tag, position) in &applied.tags {
        changes.push(Change::Add {
            object: id,
            field: "tags".into(),
            element: tag,
            position,
            note: None,
            hlc: placeholder.clone(),
        });
    }
    for reminder in &reminders {
        changes.push(Change::Create {
            object: reminder.id,
            kind: ObjectKind::Reminder,
            value: to_value(serde_json::to_value(reminder))?,
            hlc: placeholder.clone(),
        });
    }
    booked.push_changes(id, placeholder, &mut changes)?;
    Ok(Capture { answer, changes })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests may unwrap")]
mod tests {
    use super::*;
    use crate::api::{ProjectStatus, Schedule};
    use jiff::tz::TimeZone;

    fn now() -> Timestamp {
        "2026-11-10T12:00:00Z".parse().unwrap()
    }

    fn settings() -> Settings {
        Settings::new(TimeZone::UTC)
    }

    fn task(title: &str, position: i64) -> Task {
        let mut t: Task = serde_json::from_value(serde_json::json!({
            "id": Id::new(),
            "title": title,
            "created": now(),
            "modified": now(),
        }))
        .unwrap();
        t.position = position;
        t
    }

    #[test]
    fn a_scope_that_is_not_there_is_an_error() {
        let objects = Objects::default();
        let missing = Id::new();
        let filter = AvailableFilter {
            scope: Some(missing),
            ..AvailableFilter::default()
        };
        assert_eq!(
            available(&objects, &settings(), now(), &filter),
            Err(ViewError::NoScope(missing))
        );
        assert_eq!(
            folders(&objects, &settings(), now(), Some(missing)),
            Err(ViewError::NoScope(missing))
        );
    }

    fn perspective(value: serde_json::Value) -> crate::api::Perspective {
        let mut base = serde_json::json!({
            "id": Id::new(),
            "title": "p",
            "created": now(),
            "modified": now(),
        });
        if let (Some(base), Some(more)) = (base.as_object_mut(), value.as_object()) {
            base.extend(more.clone());
        }
        serde_json::from_value(base).unwrap()
    }

    fn shown_in(objects: &Objects, filter: &AvailableFilter) -> Vec<String> {
        let mut titles: Vec<String> = available(objects, &settings(), now(), filter)
            .unwrap()
            .into_iter()
            .map(|s| s.task.title)
            .collect();
        titles.sort();
        titles
    }

    /// ADR 0090 §2: chips, free text and filter, each narrowing.
    #[test]
    fn a_perspective_shows_what_its_chips_and_text_meet() {
        let mut objects = Objects::default();
        let mut flagged = task("Angebot schreiben", 0);
        flagged.flagged = true;
        let mut other = task("Angebot lesen", 1);
        other.note = Some("nicht markiert".into());
        let plain = task("Steuer", 2);
        let mut done = task("Angebot alt", 3);
        done.flagged = true;
        done.completed_at = Some(now());
        objects.tasks = vec![flagged, other, plain, done];
        let p = perspective(serde_json::json!({
            "text": "ANGEBOT",
            "chips": [{"kind": "is", "value": "markiert"}],
        }));
        let id = p.id;
        objects.perspectives.push(p);
        let filter = AvailableFilter {
            perspective: Some(id),
            ..AvailableFilter::default()
        };
        assert_eq!(shown_in(&objects, &filter), ["Angebot schreiben"]);

        // A word of the note counts as well; negated chips must not hold.
        objects.perspectives[0] = perspective(serde_json::json!({
            "id": id,
            "text": "markiert",
            "chips": [{"kind": "is", "value": "markiert", "not": true}],
        }));
        assert_eq!(shown_in(&objects, &filter), ["Angebot lesen"]);
    }

    /// ADR 0090 §2 with ADR 0042 §4: a chip after what is closed lifts the
    /// filter to all, the filter `all` shows the closed too.
    #[test]
    fn a_perspective_after_the_closed_sees_them() {
        let mut objects = Objects::default();
        let open = task("open", 0);
        let mut done = task("done", 1);
        done.completed_at = Some(now());
        objects.tasks = vec![open, done];
        let asks = perspective(serde_json::json!({
            "chips": [{"kind": "is", "value": "erledigt"}],
        }));
        let all = perspective(serde_json::json!({ "filter": "all" }));
        let (asks_id, all_id) = (asks.id, all.id);
        objects.perspectives = vec![asks, all];
        let of = |id| AvailableFilter {
            perspective: Some(id),
            ..AvailableFilter::default()
        };
        assert_eq!(shown_in(&objects, &of(asks_id)), ["done"]);
        assert_eq!(shown_in(&objects, &of(all_id)), ["done", "open"]);
    }

    /// 6aec8bd3: a free word that names a completed task by its id shows
    /// it in an `available` perspective; a completed task that only
    /// mentions the id stays out, an open one that does stays in.
    #[test]
    fn a_perspective_shows_a_task_its_text_names_by_id() {
        let mut objects = Objects::default();
        let mut epic = task("epic", 0);
        epic.completed_at = Some(now());
        let short = epic.id.to_string()[24..].to_owned();
        let mut rest = task("rest", 1);
        rest.note = Some(format!("rest of {short}"));
        let mut old = task("old", 2);
        old.note = Some(format!("before {short}"));
        old.completed_at = Some(now());
        objects.tasks = vec![epic.clone(), rest, old];
        for text in [short.clone(), format!("samu:{}", epic.id)] {
            let p = perspective(serde_json::json!({ "text": text }));
            let id = p.id;
            objects.perspectives = vec![p];
            let filter = AvailableFilter {
                perspective: Some(id),
                ..AvailableFilter::default()
            };
            let want: &[&str] = if text.starts_with("samu:") {
                &["epic"]
            } else {
                &["epic", "rest"]
            };
            assert_eq!(shown_in(&objects, &filter), want, "{text:?}");
        }
    }

    /// ADR 0090 §2: `available` leaves out what waits, `remaining` and a
    /// chip after what waits keep it.
    #[test]
    fn a_perspective_filters_by_availability() {
        let mut objects = Objects::default();
        let open = task("open", 0);
        let mut later = task("later", 1);
        later.schedule = serde_json::from_value::<Schedule>(
            serde_json::json!({"defer": {"date": "2026-12-01"}}),
        )
        .unwrap();
        objects.tasks = vec![open, later];
        let available_only = perspective(serde_json::json!({}));
        let remaining = perspective(serde_json::json!({ "filter": "remaining" }));
        let waiting = perspective(serde_json::json!({
            "chips": [{"kind": "is", "value": "wartet"}],
        }));
        let ids = [available_only.id, remaining.id, waiting.id];
        objects.perspectives = vec![available_only, remaining, waiting];
        let of = |id| AvailableFilter {
            perspective: Some(id),
            ..AvailableFilter::default()
        };
        assert_eq!(shown_in(&objects, &of(ids[0])), ["open"]);
        assert_eq!(shown_in(&objects, &of(ids[1])), ["later", "open"]);
        assert_eq!(shown_in(&objects, &of(ids[2])), ["later"]);
    }

    /// ADR 0090 §2: the scope stays a lens over the perspective.
    #[test]
    fn a_perspective_stays_inside_the_scope() {
        let mut objects = Objects::default();
        let project: Project = serde_json::from_value(serde_json::json!({
            "id": Id::new(),
            "title": "Arbeit",
            "kind": "parallel",
            "status": "active",
            "created": now(),
            "modified": now(),
        }))
        .unwrap();
        let mut inside = task("Angebot drinnen", 0);
        inside.project = Some(project.id);
        let outside = task("Angebot draußen", 1);
        let scope: Scope = serde_json::from_value(serde_json::json!({
            "id": Id::new(),
            "title": "Arbeit",
            "mode": "only",
            "members": [project.id],
            "created": now(),
            "modified": now(),
        }))
        .unwrap();
        let p = perspective(serde_json::json!({ "text": "angebot" }));
        let filter = AvailableFilter {
            scope: Some(scope.id),
            perspective: Some(p.id),
            ..AvailableFilter::default()
        };
        objects.projects.push(project);
        objects.tasks = vec![inside, outside];
        objects.scopes.push(scope);
        objects.perspectives.push(p);
        assert_eq!(shown_in(&objects, &filter), ["Angebot drinnen"]);
    }

    /// ADR 0124 §1-2: asked for, what another own scope alone shows comes
    /// along marked with that scope; in that scope it is shown as usual, and
    /// without asking nothing changes.
    #[test]
    fn what_another_own_scope_hides_is_marked_with_it() {
        let mut objects = Objects::default();
        let exit: Scope = serde_json::from_value(serde_json::json!({
            "id": Id::new(), "title": "Exit", "mode": "except", "members": [],
            "created": now(), "modified": now(),
        }))
        .unwrap();
        let project: Project = serde_json::from_value(serde_json::json!({
            "id": Id::new(), "title": "Steuer", "kind": "parallel", "status": "active",
            "own_scope": exit.id, "created": now(), "modified": now(),
        }))
        .unwrap();
        let mut hidden = task("Belege", 0);
        hidden.project = Some(project.id);
        let plain = task("Einkaufen", 1);
        objects.projects.push(project.clone());
        objects.tasks = vec![hidden, plain];
        objects.scopes.push(exit.clone());

        let marked = |scope: Option<Id>, elsewhere: bool| -> Vec<(String, Option<Id>)> {
            let filter = AvailableFilter {
                scope,
                elsewhere,
                ..AvailableFilter::default()
            };
            let mut seen: Vec<(String, Option<Id>)> =
                available(&objects, &settings(), now(), &filter)
                    .unwrap()
                    .into_iter()
                    .map(|s| (s.task.title, s.elsewhere))
                    .collect();
            seen.sort();
            seen
        };
        // Without a scope, not asked: hidden as before.
        assert_eq!(marked(None, false), [("Einkaufen".into(), None)]);
        // Asked: along, with the scope that shows it.
        assert_eq!(
            marked(None, true),
            [("Belege".into(), Some(exit.id)), ("Einkaufen".into(), None)]
        );
        // In that scope it is simply shown.
        assert!(marked(Some(exit.id), true).contains(&("Belege".into(), None)));

        let projects = |elsewhere: bool| {
            let filter = ProjectsFilter {
                elsewhere,
                ..ProjectsFilter::default()
            };
            projects(&objects, &settings(), now(), &filter)
                .unwrap()
                .into_iter()
                .map(|p| (p.project.title, p.elsewhere))
                .collect::<Vec<_>>()
        };
        assert!(projects(false).is_empty());
        assert_eq!(projects(true), [("Steuer".into(), Some(exit.id))]);
    }

    /// ADR 0116 §3-4: a block perspective shows its blocks one below the
    /// other, each as its perspective or view alone, inside the scope, a
    /// task once; `/available` answers with all of them together.
    #[test]
    fn a_block_perspective_shows_its_blocks() {
        let mut objects = Objects::default();
        let mut flagged = task("Angebot schreiben", 0);
        flagged.flagged = true;
        let offer = task("Angebot lesen", 1);
        let mut done = task("Angebot alt", 2);
        done.completed_at = Some(now());
        let tax = task("Steuer", 3);
        objects.tasks = vec![flagged, offer, done, tax];
        let offers = perspective(serde_json::json!({ "title": "Angebote", "text": "angebot" }));
        let morning = perspective(serde_json::json!({
            "title": "Morgen",
            "blocks": [
                {"view": "flagged"},
                {"perspective": offers.id, "title": "Alle Angebote"},
                {"perspective": Id::new()},
            ],
        }));
        let (offers_id, morning_id) = (offers.id, morning.id);
        objects.perspectives = vec![offers, morning];
        let titles = |s: &crate::api::BlockSection| -> Vec<String> {
            s.tasks.iter().map(|t| t.task.title.clone()).collect()
        };
        let sections = blocks(&objects, &settings(), now(), None, morning_id).unwrap();
        let heads: Vec<(&str, bool)> = sections
            .iter()
            .map(|s| (s.title.as_str(), s.missing))
            .collect();
        assert_eq!(heads[..2], [("flagged", false), ("Alle Angebote", false)]);
        assert!(sections[2].missing);
        assert_eq!(titles(&sections[0]), ["Angebot schreiben"]);
        assert_eq!(
            titles(&sections[1]),
            ["Angebot lesen"],
            "once, not the closed"
        );

        let filter = AvailableFilter {
            perspective: Some(morning_id),
            ..AvailableFilter::default()
        };
        assert_eq!(
            shown_in(&objects, &filter),
            ["Angebot lesen", "Angebot schreiben"]
        );

        // A plain perspective is one section under its name.
        let one = blocks(&objects, &settings(), now(), None, offers_id).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].title, "Angebote");
        assert_eq!(titles(&one[0]).len(), 2);
        let missing = Id::new();
        assert_eq!(
            blocks(&objects, &settings(), now(), None, missing),
            Err(ViewError::NoPerspective(missing))
        );
    }

    /// ADR 0143 §2: a block's own search shows what a plain perspective
    /// with its fields would, named by its search without a heading; the
    /// union of `/available` takes it in.
    #[test]
    fn a_block_shows_a_search_of_its_own() {
        let mut objects = Objects::default();
        let mut flagged = task("Angebot schreiben", 0);
        flagged.flagged = true;
        let offer = task("Angebot lesen", 1);
        let mut done = task("Angebot alt", 2);
        done.completed_at = Some(now());
        let tax = task("Steuer", 3);
        objects.tasks = vec![flagged, offer, done, tax];
        let morning = perspective(serde_json::json!({
            "title": "Morgen",
            "blocks": [
                {"query": {"chips": [{"kind": "is", "value": "markiert"}]}},
                {"query": {"text": "angebot", "filter": "all"}, "title": "Angebote"},
                {"query": {}},
            ],
        }));
        let morning_id = morning.id;
        objects.perspectives = vec![morning];
        let titles = |s: &crate::api::BlockSection| -> Vec<String> {
            s.tasks.iter().map(|t| t.task.title.clone()).collect()
        };
        let sections = blocks(&objects, &settings(), now(), None, morning_id).unwrap();
        let heads: Vec<(&str, bool)> = sections
            .iter()
            .map(|s| (s.title.as_str(), s.missing))
            .collect();
        assert_eq!(
            heads,
            [("ist:markiert", false), ("Angebote", false), ("All", false)]
        );
        assert_eq!(titles(&sections[0]), ["Angebot schreiben"]);
        assert_eq!(
            titles(&sections[1]),
            ["Angebot lesen", "Angebot alt"],
            "its own filter, the flagged one already above"
        );
        assert_eq!(titles(&sections[2]), ["Steuer"]);
        let filter = AvailableFilter {
            perspective: Some(morning_id),
            ..AvailableFilter::default()
        };
        assert_eq!(shown_in(&objects, &filter).len(), 4);
    }

    #[test]
    fn a_perspective_that_is_not_there_is_an_error() {
        let missing = Id::new();
        let filter = AvailableFilter {
            perspective: Some(missing),
            ..AvailableFilter::default()
        };
        assert_eq!(
            available(&Objects::default(), &settings(), now(), &filter),
            Err(ViewError::NoPerspective(missing))
        );
    }

    #[test]
    fn closed_tasks_only_with_all_and_overdue_first() {
        let mut objects = Objects::default();
        let open = task("open", 0);
        let mut done = task("done", 1);
        done.completed_at = Some(now());
        let mut late = task("late", 2);
        late.schedule =
            serde_json::from_value::<Schedule>(serde_json::json!({"due": {"date": "2026-11-01"}}))
                .unwrap();
        objects.tasks = vec![open.clone(), done.clone(), late.clone()];
        let shown = available(&objects, &settings(), now(), &AvailableFilter::default()).unwrap();
        let titles: Vec<&str> = shown.iter().map(|s| s.task.title.as_str()).collect();
        assert_eq!(titles, ["late", "open"]);
        let all = AvailableFilter {
            all: true,
            ..AvailableFilter::default()
        };
        assert_eq!(
            available(&objects, &settings(), now(), &all).unwrap().len(),
            3
        );
    }

    #[test]
    fn a_report_backwards_is_refused() {
        let query = ClockReportQuery {
            from: Some("2026-11-10".parse().unwrap()),
            to: Some("2026-11-09".parse().unwrap()),
            by: ClockReportBy::Day,
            scope: None,
        };
        assert!(matches!(
            clock_report(&Objects::default(), &settings(), now(), &query),
            Err(ViewError::BadRequest(_))
        ));
    }

    #[test]
    fn closed_projects_only_with_all() {
        let mut objects = Objects::default();
        let project: Project = serde_json::from_value(serde_json::json!({
            "id": Id::new(),
            "title": "p",
            "kind": "parallel",
            "status": "dropped",
            "created": now(),
            "modified": now(),
        }))
        .unwrap();
        assert_eq!(project.status, ProjectStatus::Dropped);
        objects.projects = vec![project];
        assert!(
            projects(&objects, &settings(), now(), &ProjectsFilter::default())
                .unwrap()
                .is_empty()
        );
        let all = ProjectsFilter {
            all: true,
            ..ProjectsFilter::default()
        };
        assert_eq!(
            projects(&objects, &settings(), now(), &all).unwrap().len(),
            1
        );
    }

    #[test]
    fn capturing_under_a_missing_task_is_unprocessable() {
        let request: CaptureRequest = serde_json::from_value(serde_json::json!({
            "text": "buy milk",
            "parent": Id::new(),
        }))
        .unwrap();
        let stamp = Hlc {
            wall: 0,
            counter: 0,
            device: "test".into(),
        };
        assert!(matches!(
            capture(
                &Objects::default(),
                &settings(),
                &request,
                now(),
                &stamp,
                &mut Id::new
            ),
            Err(ViewError::Unprocessable(_))
        ));
    }

    /// ADR 0129: an indented line goes under its line, a tag new in one
    /// line is made once, a project named wins, a failing line fails all.
    #[test]
    fn capture_lines_nests_and_shares_new_tags() {
        let stamp = Hlc {
            wall: 0,
            counter: 0,
            device: "test".into(),
        };
        let mut objects = Objects::default();
        let garten: Project = serde_json::from_value(serde_json::json!({
            "id": Id::new(), "title": "Garten", "kind": "parallel", "status": "active",
            "created": now(), "modified": now(),
        }))
        .unwrap();
        objects.projects.push(garten.clone());
        let request: CaptureLinesRequest = serde_json::from_value(serde_json::json!({
            "text": "Einkauf @laden\n  - Milch @laden\n  - Erde #Garten\nAnrufen !A",
        }))
        .unwrap();
        let made =
            capture_lines(&objects, &settings(), &request, now(), &stamp, &mut Id::new).unwrap();
        let tasks = &made.answer.tasks;
        let titles: Vec<&str> = tasks.iter().map(|t| t.task.title.as_str()).collect();
        assert_eq!(titles, ["Einkauf", "Milch", "Erde", "Anrufen"]);
        assert_eq!(tasks[1].task.parent, Some(tasks[0].task.id));
        assert_eq!(tasks[2].task.parent, None);
        assert_eq!(tasks[2].task.project, Some(garten.id));
        assert_eq!(tasks[3].task.parent, None);
        assert!(tasks.iter().all(|t| t.created));
        // One new tag, on both tasks.
        assert_eq!(tasks[0].new_tags, ["laden"]);
        assert!(tasks[1].new_tags.is_empty());
        assert_eq!(tasks[0].task.tags.len(), 1);
        assert_eq!(tasks[0].task.tags[0].tag, tasks[1].task.tags[0].tag);
        let tags_made = made
            .changes
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    Change::Create {
                        kind: ObjectKind::Tag,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(tags_made, 1);
        // Two tasks at the top, one after the other.
        assert!(tasks[3].task.position > tasks[0].task.position);

        let mut preview = request.clone();
        preview.preview = true;
        let seen =
            capture_lines(&objects, &settings(), &preview, now(), &stamp, &mut Id::new).unwrap();
        assert!(seen.changes.is_empty());
        assert_eq!(seen.answer.tasks.len(), 4);
        assert_eq!(
            seen.answer.tasks[1].task.parent,
            Some(seen.answer.tasks[0].task.id)
        );

        let mut closed = task("zu", 0);
        closed.completed_at = Some(now());
        objects.tasks.push(closed.clone());
        let mut under = request;
        under.parent = Some(closed.id);
        assert!(matches!(
            capture_lines(&objects, &settings(), &under, now(), &stamp, &mut Id::new),
            Err(ViewError::Unprocessable(m)) if m.starts_with("line 1:")
        ));
        under.text = " \n".into();
        assert!(matches!(
            capture_lines(&objects, &settings(), &under, now(), &stamp, &mut Id::new),
            Err(ViewError::BadRequest(_))
        ));
    }

    #[test]
    fn a_capture_writes_the_task_and_a_preview_nothing() {
        let stamp = Hlc {
            wall: 0,
            counter: 0,
            device: "test".into(),
        };
        let request: CaptureRequest =
            serde_json::from_value(serde_json::json!({"text": "buy milk @shop"})).unwrap();
        let made = capture(
            &Objects::default(),
            &settings(),
            &request,
            now(),
            &stamp,
            &mut Id::new,
        )
        .unwrap();
        // The new tag, the task, the tag on the task.
        assert_eq!(made.changes.len(), 3);
        assert_eq!(made.answer.new_tags, ["shop"]);
        let mut preview = request.clone();
        preview.preview = true;
        let seen = capture(
            &Objects::default(),
            &settings(),
            &preview,
            now(),
            &stamp,
            &mut Id::new,
        )
        .unwrap();
        assert!(seen.changes.is_empty());
        assert_eq!(seen.answer.task.tags.len(), 1);
    }

    /// ADR 0078 §3: with `task`, the line edits that task; free text is
    /// shown in a preview and refused on writing; with `parent`, refused.
    #[test]
    fn a_capture_with_task_edits_it() {
        let stamp = Hlc {
            wall: 0,
            counter: 0,
            device: "test".into(),
        };
        let mut objects = Objects::default();
        let fliesen = task("Fliesen", 0);
        objects.tasks.push(fliesen.clone());
        let request: CaptureRequest = serde_json::from_value(serde_json::json!({
            "text": "@shop !A // mehr", "task": fliesen.id,
        }))
        .unwrap();
        let made = capture(&objects, &settings(), &request, now(), &stamp, &mut Id::new).unwrap();
        assert!(!made.answer.created);
        assert_eq!(made.answer.task.id, fliesen.id);
        assert_eq!(made.answer.task.title, "Fliesen");
        assert_eq!(made.answer.new_tags, ["shop"]);
        // The new tag, priority, note, the tag on the task.
        let ops: Vec<&str> = made
            .changes
            .iter()
            .map(|c| match c {
                Change::Create { .. } => "create",
                Change::Set { .. } => "set",
                Change::Add { .. } => "add",
                _ => "other",
            })
            .collect();
        assert_eq!(ops, ["create", "set", "set", "add"]);
        assert!(made.changes.iter().all(|c| match c {
            Change::Set { object, .. } | Change::Add { object, .. } => *object == fliesen.id,
            _ => true,
        }));

        let mut free = request.clone();
        free.text = "morgen anrufen !A".into();
        free.preview = true;
        let seen = capture(&objects, &settings(), &free, now(), &stamp, &mut Id::new).unwrap();
        assert!(seen.changes.is_empty());
        assert!(matches!(
            seen.answer.warnings[..],
            [crate::api::CaptureWarning::FreeText { .. }]
        ));
        free.preview = false;
        assert!(matches!(
            capture(&objects, &settings(), &free, now(), &stamp, &mut Id::new),
            Err(ViewError::Unprocessable(_))
        ));

        let mut both = request.clone();
        both.parent = Some(fliesen.id);
        assert!(matches!(
            capture(&objects, &settings(), &both, now(), &stamp, &mut Id::new),
            Err(ViewError::BadRequest(_))
        ));
        let mut gone = request;
        gone.task = Some(Id::new());
        assert!(matches!(
            capture(&objects, &settings(), &gone, now(), &stamp, &mut Id::new),
            Err(ViewError::Unprocessable(_))
        ));
    }

    /// ADR 0073 §5: a template fills in the line and gives the task its
    /// reminders, written after the task; a preview shows them, writes none.
    #[test]
    fn a_template_gives_the_task_its_reminders() {
        let stamp = Hlc {
            wall: 0,
            counter: 0,
            device: "test".into(),
        };
        let mut objects = Objects::default();
        objects.templates.push(
            serde_json::from_value(serde_json::json!({
                "id": "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6e80",
                "created": "2026-11-10T08:00:00Z", "modified": "2026-11-10T08:00:00Z",
                "title": "habit", "text": "p:heute 21:00 every+:1d",
                "reminders": [{"anchor": "planned"}, {"anchor": "planned", "before": "PT15M"}],
                "position": 0,
            }))
            .unwrap(),
        );
        let request: CaptureRequest =
            serde_json::from_value(serde_json::json!({"text": "Rauchfrei /habit"})).unwrap();
        let made = capture(&objects, &settings(), &request, now(), &stamp, &mut Id::new).unwrap();
        let task = &made.answer.task;
        assert_eq!(task.title, "Rauchfrei");
        assert!(task.schedule.planned.is_some());
        assert!(task.schedule.repeat.as_ref().unwrap().catch_up);
        assert_eq!(made.answer.reminders.len(), 2);
        assert!(made.answer.reminders.iter().all(|r| r.owner == task.id));
        // The task, then the two reminders.
        let kinds: Vec<ObjectKind> = made
            .changes
            .iter()
            .filter_map(|c| match c {
                Change::Create { kind, .. } => Some(*kind),
                _ => None,
            })
            .collect();
        assert_eq!(
            kinds,
            [ObjectKind::Task, ObjectKind::Reminder, ObjectKind::Reminder]
        );

        let mut preview = request.clone();
        preview.preview = true;
        let seen = capture(&objects, &settings(), &preview, now(), &stamp, &mut Id::new).unwrap();
        assert!(seen.changes.is_empty());
        assert_eq!(seen.answer.reminders.len(), 2);
        // A button instead of `/name`.
        let mut button: CaptureRequest =
            serde_json::from_value(serde_json::json!({"text": "Dehnen"})).unwrap();
        button.template = Some("01936f5e-8a00-7c3e-9b1a-3f2d4c5b6e80".parse().unwrap());
        let made = capture(&objects, &settings(), &button, now(), &stamp, &mut Id::new).unwrap();
        assert_eq!(made.answer.reminders.len(), 2);
    }
}
