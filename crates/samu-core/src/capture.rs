// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Quick capture (ADR 0011): one syntax, one parser.
//!
//! [`parse`] turns a line like `Anrufen #haus @telefon d:fr 9:00 ~15m` into a
//! [`Draft`]. It reads nothing: project and tag names stay names, and the
//! caller resolves them with [`match_name`] against what it knows. A project
//! that does not resolve goes back into the title with [`Draft::miss_project`]
//! (ADR 0011 §5).
//!
//! A date is only read after a prefix (`due:`, `d:`, `start:`, `s:`,
//! `plan:`, `p:`), never freely in a sentence. A prefix whose value is no
//! date stays in the title and is reported; a date is never guessed.
//!
//! Warnings are codes, not sentences, so the parser needs no language
//! setting: dates are read in German and English alike (ADR 0011 §3), and
//! each surface words the warnings in its own language.

use jiff::civil::{Date, Time, Weekday};
use jiff::tz::TimeZone;
use jiff::{Span, Timestamp, ToSpan};
use samu_api::{
    Clock, Habit, HabitKind, HabitPer, Id, Period, Priority, Project, ProjectStatus, Reminder,
    Repeat, RepeatMode, Schedule, Scope, Tag, TagStatus, Task, Template, When,
};

/// What a marked stretch of the input means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Meaning {
    Project,
    Tag,
    Flag,
    Priority,
    Defer,
    Due,
    Planned,
    Estimate,
    Repeat,
    Note,
    /// `/name`, a template (ADR 0073 §3).
    Template,
    /// `z:`, time booked (ADR 0081 §2).
    Clock,
    /// `x:`, completed (ADR 0081 §2).
    Completed,
    /// `habit:`, the task is a habit (ADR 0091 §5).
    Habit,
}

/// A recognised stretch of the input, as byte offsets into it, so a surface
/// can highlight it while one types.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mark {
    pub start: usize,
    pub end: usize,
    pub meaning: Meaning,
}

/// Something the parser did not take; the text in question stays in the
/// title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// A date prefix whose value is no date the parser knows.
    UnknownDate { meaning: Meaning, text: String },
    /// A `~` whose value is no length of time.
    UnknownEstimate { text: String },
    /// An `every:` whose value is none of the short forms.
    UnknownRepeat { text: String },
    /// A second project, date, estimate or repetition.
    Twice { meaning: Meaning, text: String },
    /// The `#name` matched no project or several (set by
    /// [`Draft::miss_project`]).
    UnknownProject { name: String },
    /// The `@name` matched several tags, or would be a new tag under a
    /// workflow (set by [`Draft::miss_tag`], ADR 0027 §3).
    UnknownTag { name: String },
    /// The `/name` is no template (set by [`apply_template`], ADR 0073 §3).
    UnknownTemplate { name: String },
    /// Text that is no mark, in a line that edits a task (set by
    /// [`apply_to`], ADR 0078 §2): the title has its own field there.
    FreeText { text: String },
    /// A `z:` whose clock would end in the future (set by
    /// [`place_clocks`], ADR 0081 §2); it is not booked.
    ClockInFuture { text: String },
    /// A `z:` or `x:` whose value is no time the parser reads; it stays in
    /// the title.
    UnknownClock { text: String },
    /// A `habit:` whose value is none of the short forms (ADR 0091 §5).
    UnknownHabit { text: String },
}

/// Time booked with `z:` (ADR 0081 §2), as written; [`place_clocks`]
/// makes clocks of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClockSpec {
    /// `z:1h30`: a length of time that ends now.
    Length(Period),
    /// `z:gestern 2h`: a length on a day, after that day's last clock, or
    /// from 09:00.
    OnDay(Date, Period),
    /// `z:gestern 14:00-15:30`, `z:14-15:30`: from and to on a day.
    Range(Date, Time, Time),
}

/// One `z:` with the text it came from, for a warning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookedClock {
    pub spec: ClockSpec,
    pub text: String,
}

/// One word of the input that the title may hold: plain text, or a `#name`
/// or `@name` taken out of the title unless it goes back in.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Piece {
    raw: String,
    kind: PieceKind,
    /// Where the word starts, to find its mark.
    start: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PieceKind {
    Text,
    Project,
    Tag(String),
    Template,
}

/// The fields read from one line of quick capture.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Draft {
    pub title: String,
    pub project: Option<String>,
    pub tags: Vec<String>,
    pub flagged: bool,
    /// `!A`, `!B`, `!C` (ADR 0057).
    pub priority: Option<Priority>,
    pub defer: Option<When>,
    pub due: Option<When>,
    pub planned: Option<When>,
    pub estimate: Option<Period>,
    pub repeat: Option<Repeat>,
    pub note: Option<String>,
    /// `/name`: the template the line calls, by name (ADR 0073 §3).
    pub template: Option<String>,
    /// `z:`, each its own clock (ADR 0081 §2).
    pub clocks: Vec<BookedClock>,
    /// `x:`: completed now (`Some(None)`) or on a day (ADR 0081 §2).
    pub completed: Option<Option<Date>>,
    /// `habit:`, the task is a habit (ADR 0091 §5).
    pub habit: Option<Habit>,
    pub marks: Vec<Mark>,
    pub warnings: Vec<Warning>,
    /// The words the title is made of, `#name` and `@name` among them.
    pieces: Vec<Piece>,
    /// The free text of the template the line calls, put before the title
    /// (ADR 0120).
    prefix: String,
}

impl Draft {
    /// The project did not resolve: drop it, put `#name` back into the title
    /// and warn (ADR 0011 §5). The task then goes to the inbox.
    pub fn miss_project(&mut self) {
        if let Some(name) = self.project.take() {
            self.restore(|kind| *kind == PieceKind::Project, Meaning::Project);
            self.warnings.push(Warning::UnknownProject { name });
        }
    }

    /// The tag `name` is not taken: drop it, put `@name` back into the title
    /// where it stood and warn (ADR 0027 §3).
    pub fn miss_tag(&mut self, name: &str) {
        let before = self.tags.len();
        self.tags.retain(|tag| tag != name);
        if self.tags.len() == before {
            return;
        }
        self.restore(
            |kind| matches!(kind, PieceKind::Tag(tag) if tag == name),
            Meaning::Tag,
        );
        self.warnings.push(Warning::UnknownTag {
            name: name.to_owned(),
        });
    }

    /// The `/name` is no template: put it back into the title and warn
    /// (ADR 0073 §3).
    fn miss_template(&mut self) {
        if let Some(name) = self.template.take() {
            self.restore(|kind| *kind == PieceKind::Template, Meaning::Template);
            self.warnings.push(Warning::UnknownTemplate { name });
        }
    }

    /// Fill in what this line names not from `other`, field by field
    /// (ADR 0073 §4). Tags are not mixed: a line with a tag keeps its own.
    fn fill_from(&mut self, other: Draft) {
        if self.project.is_none() {
            self.project = other.project;
        }
        if self.tags.is_empty() {
            self.tags = other.tags;
        }
        self.flagged |= other.flagged;
        self.priority = self.priority.or(other.priority);
        if self.defer.is_none() {
            self.defer = other.defer;
        }
        if self.due.is_none() {
            self.due = other.due;
        }
        if self.planned.is_none() {
            self.planned = other.planned;
        }
        self.estimate = self.estimate.or(other.estimate);
        if self.habit.is_none() {
            self.habit = other.habit;
        }
        if self.repeat.is_none() {
            self.repeat = other.repeat;
        }
        if self.note.is_none() {
            self.note = other.note;
        }
        // The template's free text goes before the line's title (ADR 0120).
        self.prefix = other.title;
        self.title = self.titled(title_of(&self.pieces));
    }

    /// Forget the template's free text: a line that edits a task leaves its
    /// title alone (ADR 0120).
    fn drop_prefix(&mut self) {
        self.prefix.clear();
        self.title = title_of(&self.pieces);
    }

    /// `title` with the template's free text before it.
    fn titled(&self, title: String) -> String {
        match (self.prefix.is_empty(), title.is_empty()) {
            (true, _) => title,
            (false, true) => self.prefix.clone(),
            (false, false) => format!("{} {title}", self.prefix),
        }
    }

    fn restore(&mut self, which: impl Fn(&PieceKind) -> bool, meaning: Meaning) {
        let mut starts = Vec::new();
        for piece in &mut self.pieces {
            if which(&piece.kind) {
                piece.kind = PieceKind::Text;
                starts.push(piece.start);
            }
        }
        self.marks
            .retain(|mark| !(mark.meaning == meaning && starts.contains(&mark.start)));
        self.title = self.titled(title_of(&self.pieces));
    }
}

fn title_of(pieces: &[Piece]) -> String {
    pieces
        .iter()
        .filter(|piece| piece.kind == PieceKind::Text)
        .map(|piece| piece.raw.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The result of looking a typed name up among known names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameMatch {
    /// The index of the one name meant.
    One(usize),
    /// The typed name starts several names and is none of them.
    Many,
    None,
}

/// Look a project or tag name up, ignoring case: an exact name wins, else a
/// prefix that starts exactly one name (ADR 0011 §2).
pub fn match_name<'a>(query: &str, names: impl IntoIterator<Item = &'a str>) -> NameMatch {
    let query = query.to_lowercase();
    let mut prefixed = Vec::new();
    for (index, name) in names.into_iter().enumerate() {
        let name = name.to_lowercase();
        if name == query {
            return NameMatch::One(index);
        }
        if name.starts_with(&query) {
            prefixed.push(index);
        }
    }
    match prefixed.as_slice() {
        [] => NameMatch::None,
        [one] => NameMatch::One(*one),
        _ => NameMatch::Many,
    }
}

/// Like [`match_name`], by whole name only (ignoring case).
fn match_exact<'a>(query: &str, names: impl IntoIterator<Item = &'a str>) -> NameMatch {
    let query = query.to_lowercase();
    let hits: Vec<usize> = names
        .into_iter()
        .enumerate()
        .filter(|(_, name)| name.to_lowercase() == query)
        .map(|(index, _)| index)
        .collect();
    match hits.as_slice() {
        [] => NameMatch::None,
        [one] => NameMatch::One(*one),
        _ => NameMatch::Many,
    }
}

impl From<Meaning> for samu_api::CaptureMeaning {
    fn from(meaning: Meaning) -> Self {
        match meaning {
            Meaning::Project => Self::Project,
            Meaning::Tag => Self::Tag,
            Meaning::Flag => Self::Flag,
            Meaning::Priority => Self::Priority,
            Meaning::Defer => Self::Defer,
            Meaning::Due => Self::Due,
            Meaning::Planned => Self::Planned,
            Meaning::Estimate => Self::Estimate,
            Meaning::Repeat => Self::Repeat,
            Meaning::Note => Self::Note,
            Meaning::Template => Self::Template,
            Meaning::Clock => Self::Clock,
            Meaning::Completed => Self::Completed,
            Meaning::Habit => Self::Habit,
        }
    }
}

impl From<Mark> for samu_api::CaptureMark {
    fn from(mark: Mark) -> Self {
        Self {
            start: mark.start,
            end: mark.end,
            meaning: mark.meaning.into(),
        }
    }
}

impl From<&Warning> for samu_api::CaptureWarning {
    fn from(warning: &Warning) -> Self {
        match warning.clone() {
            Warning::UnknownDate { meaning, text } => Self::UnknownDate {
                meaning: meaning.into(),
                text,
            },
            Warning::UnknownEstimate { text } => Self::UnknownEstimate { text },
            Warning::UnknownRepeat { text } => Self::UnknownRepeat { text },
            Warning::Twice { meaning, text } => Self::Twice {
                meaning: meaning.into(),
                text,
            },
            Warning::UnknownProject { name } => Self::UnknownProject { name },
            Warning::UnknownTag { name } => Self::UnknownTag { name },
            Warning::UnknownTemplate { name } => Self::UnknownTemplate { name },
            Warning::FreeText { text } => Self::FreeText { text },
            Warning::ClockInFuture { text } => Self::ClockInFuture { text },
            Warning::UnknownClock { text } => Self::UnknownClock { text },
            Warning::UnknownHabit { text } => Self::UnknownHabit { text },
        }
    }
}

/// What the account holds, to resolve names against (ADR 0027 §3).
#[derive(Debug, Clone, Copy)]
pub struct Known<'a> {
    pub projects: &'a [Project],
    pub tags: &'a [Tag],
    pub tasks: &'a [Task],
}

/// A draft turned into a task: the tags to create first, then the task, then
/// the tags to add to it.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    /// The task to create, without tags (they come as `add`, so the rules
    /// of workflows hold, ADR 0024).
    pub task: Task,
    /// New tags, parents before children.
    pub new_tags: Vec<Tag>,
    /// The tags to add, in the order typed.
    pub tags: Vec<Id>,
    /// Each tag name of the draft that resolved, with the tag it means; a
    /// surface finds a `@name` mark's tag through it.
    pub named: Vec<(String, Id)>,
    /// The draft with what did not resolve put back into the title.
    pub draft: Draft,
}

/// [`resolve`] with the scope chosen on the device (ADR 0043 §6): a line
/// that names no project goes to the view's project, else to the scope's;
/// a line that names no tag gets the scope's tags that are still live.
/// What the line names always wins.
pub fn resolve_in_scope(
    draft: Draft,
    known: Known<'_>,
    view: Option<Id>,
    scope: Option<&Scope>,
    now: Timestamp,
    new_id: &mut impl FnMut() -> Id,
) -> Resolved {
    let capture = scope.and_then(|s| s.capture.as_ref());
    let named_tags = !draft.tags.is_empty();
    let fallback = view.or_else(|| capture.and_then(|c| c.project));
    let mut resolved = resolve(draft, known, fallback, now, new_id);
    if let Some(capture) = capture
        && !named_tags
    {
        for tag in &capture.tags {
            let live = known
                .tags
                .iter()
                .any(|t| t.id == *tag && t.status != TagStatus::Dropped);
            if live && !resolved.tags.contains(tag) {
                resolved.tags.push(*tag);
            }
        }
    }
    resolved
}

/// Put a resolved task under `parent`, as its last child and in its project
/// (ADR 0062 §2, §3), unless the line named a project that resolved: that
/// wins, and the task stays where [`resolve`] put it. `tasks` are the
/// account's tasks, for the position among the parent's children.
pub fn place_under(resolved: &mut Resolved, parent: &Task, tasks: &[Task]) {
    if resolved.draft.project.is_some() {
        return;
    }
    resolved.task.project = parent.project;
    resolved.task.parent = Some(parent.id);
    resolved.task.position = tasks
        .iter()
        .filter(|t| t.parent == Some(parent.id))
        .map(|t| t.position + 1)
        .max()
        .unwrap_or(0);
}

/// Resolve the names of `draft` against `known` and describe the task
/// (ADR 0027): a project by name or a unique prefix among open projects,
/// else `fallback`, else the inbox; a tag by name, prefix or path, else a
/// new one, never under an exclusive group. `new_id` hands out the ids, so
/// nothing here is random.
pub fn resolve(
    mut draft: Draft,
    known: Known<'_>,
    fallback: Option<Id>,
    now: Timestamp,
    new_id: &mut impl FnMut() -> Id,
) -> Resolved {
    let open: Vec<&Project> = known
        .projects
        .iter()
        .filter(|p| matches!(p.status, ProjectStatus::Active | ProjectStatus::OnHold))
        .collect();
    let project = match draft.project.clone() {
        Some(name) => match match_name(&name, open.iter().map(|p| p.title.as_str())) {
            NameMatch::One(index) => Some(open[index].id),
            NameMatch::Many | NameMatch::None => {
                draft.miss_project();
                None
            }
        },
        None => fallback.filter(|id| open.iter().any(|p| p.id == *id)),
    };

    let mut pool: Vec<Tag> = known
        .tags
        .iter()
        .filter(|t| t.status != TagStatus::Dropped)
        .cloned()
        .collect();
    let mut new_tags = Vec::new();
    let mut tags = Vec::new();
    let mut named = Vec::new();
    for name in draft.tags.clone() {
        match tag_for(&name, Lookup::Prefix, &mut pool, &mut new_tags, now, new_id) {
            Some(id) => {
                if !tags.contains(&id) {
                    tags.push(id);
                }
                named.push((name, id));
            }
            None => draft.miss_tag(&name),
        }
    }

    let position = known
        .tasks
        .iter()
        .filter(|t| t.project == project && t.parent.is_none())
        .map(|t| t.position + 1)
        .max()
        .unwrap_or(0);
    let task = Task {
        created_by: None,
        habit: draft.habit.clone(),
        id: new_id(),
        created: now,
        modified: now,
        title: draft.title.clone(),
        note: draft.note.clone(),
        project,
        parent: None,
        schedule: Schedule {
            defer: draft.defer.clone(),
            due: draft.due.clone(),
            planned: draft.planned.clone(),
            repeat: draft.repeat.clone(),
            ..Schedule::default()
        },
        flagged: draft.flagged,
        estimate: draft.estimate,
        priority: draft.priority,
        completed_at: None,
        dropped_at: None,
        repeat_of: None,
        sequential: false,
        complete_with_last_child: false,
        tags: Vec::new(),
        properties: std::collections::BTreeMap::new(),
        position,
    };
    Resolved {
        task,
        new_tags,
        tags,
        named,
        draft,
    }
}

/// Whether `title` may name a tag: a slash would make a path like
/// `Ort/Büro` mean two tags (ADR 0027 §3), so a title never holds one. The
/// server refuses such a title on every way in; a surface may ask first.
pub fn check_tag_title(title: &str) -> Result<(), String> {
    if title.contains('/') {
        return Err(format!(
            "{title:?}: a tag title cannot hold a slash; Ort/Büro is Büro under Ort"
        ));
    }
    Ok(())
}

/// How a step of a tag path finds an existing tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lookup {
    /// As capture types it: the whole title or a unique prefix.
    Prefix,
    /// The whole title only, ignoring case: a tag picker asked for a new
    /// tag, so `Bü` makes one rather than meaning `Büro`.
    Exact,
}

/// The tag a tag picker means by `path` (`a` or `a/b/c`), with the tags to
/// create for it, parent first; steps are matched by whole title, ignoring
/// case, and only missing ones are made. Dropped tags are not looked at.
/// An error when the path is empty, a step names more than one tag, a new
/// tag would sit under an exclusive group, or the path names a workflow or
/// one of its states (those are set as a state, not picked).
pub fn tag_at_path(
    path: &str,
    tags: &[Tag],
    now: Timestamp,
    new_id: &mut impl FnMut() -> Id,
) -> Result<(Id, Vec<Tag>), String> {
    let mut pool: Vec<Tag> = tags
        .iter()
        .filter(|t| t.status != TagStatus::Dropped)
        .cloned()
        .collect();
    let mut new_tags = Vec::new();
    match tag_for(
        path.trim(),
        Lookup::Exact,
        &mut pool,
        &mut new_tags,
        now,
        new_id,
    ) {
        Some(id) => {
            let tag = pool.iter().find(|t| t.id == id);
            let group = tag
                .and_then(|t| t.parent)
                .and_then(|p| pool.iter().find(|t| t.id == p));
            if tag.is_some_and(|t| t.exclusive) || group.is_some_and(|g| g.exclusive) {
                return Err(format!(
                    "{:?} is a workflow or one of its states, not a tag to pick",
                    path.trim()
                ));
            }
            Ok((id, new_tags))
        }
        None if path.split('/').all(|s| s.trim().is_empty()) => {
            Err("a tag needs a name".to_owned())
        }
        None => Err(format!(
            "{:?}: names more than one tag, or would make a tag under an exclusive group",
            path.trim()
        )),
    }
}

/// The tag a typed `name` (`a` or `a/b/c`) means, creating what is missing;
/// none when a step is ambiguous or a new tag would sit under an exclusive
/// group.
fn tag_for(
    name: &str,
    lookup: Lookup,
    pool: &mut Vec<Tag>,
    new_tags: &mut Vec<Tag>,
    now: Timestamp,
    new_id: &mut impl FnMut() -> Id,
) -> Option<Id> {
    let steps: Vec<&str> = name
        .split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if steps.is_empty() {
        return None;
    }
    let mut parent: Option<Id> = None;
    for step in &steps {
        // A single name is looked up among all tags; a path step by step.
        let candidates: Vec<&Tag> = pool
            .iter()
            .filter(|t| steps.len() == 1 || t.parent == parent)
            .collect();
        let titles = candidates.iter().map(|t| t.title.as_str());
        let found = match lookup {
            Lookup::Prefix => match_name(step, titles),
            Lookup::Exact => match_exact(step, titles),
        };
        match found {
            NameMatch::One(at) => parent = Some(candidates[at].id),
            NameMatch::Many => return None,
            NameMatch::None => {
                let under = parent.and_then(|id| pool.iter().find(|t| t.id == id));
                if under.is_some_and(|t| t.exclusive) {
                    return None;
                }
                let position = pool
                    .iter()
                    .filter(|t| t.parent == parent)
                    .map(|t| t.position + 1)
                    .max()
                    .unwrap_or(0);
                let tag = Tag {
                    scope_only: false,
                    own_scope: None,
                    id: new_id(),
                    created: now,
                    modified: now,
                    title: (*step).to_owned(),
                    parent,
                    status: TagStatus::Active,
                    exclusive: false,
                    key: None,
                    ask_note: false,
                    default_workflow: false,
                    position,
                };
                parent = Some(tag.id);
                pool.push(tag.clone());
                new_tags.push(tag);
            }
        }
    }
    parent
}

#[derive(Debug, Clone, Copy)]
struct Token<'a> {
    start: usize,
    end: usize,
    text: &'a str,
}

/// Split at whitespace; `#"…"` and `@"…"` hold spaces up to the closing quote.
fn tokenize(text: &str) -> Vec<Token<'_>> {
    let mut tokens = Vec::new();
    let mut start: Option<usize> = None;
    let mut quoted_until = 0;
    for (at, c) in text.char_indices() {
        if at < quoted_until {
            continue;
        }
        if c.is_whitespace() {
            if let Some(begin) = start.take() {
                tokens.push(Token {
                    start: begin,
                    end: at,
                    text: &text[begin..at],
                });
            }
            continue;
        }
        if start.is_none() {
            start = Some(at);
            let rest = &text[at..];
            if (rest.starts_with("#\"") || rest.starts_with("@\""))
                && let Some(close) = rest[2..].find('"')
            {
                quoted_until = at + 2 + close + 1;
            }
        }
    }
    if let Some(begin) = start {
        tokens.push(Token {
            start: begin,
            end: text.len(),
            text: &text[begin..],
        });
    }
    tokens
}

fn unquote(name: &str) -> String {
    match name.strip_prefix('"') {
        Some(inner) => inner.strip_suffix('"').unwrap_or(inner).to_owned(),
        None => name.to_owned(),
    }
}

fn date_prefix(token: &str) -> Option<(Meaning, &str)> {
    const PREFIXES: [(&str, Meaning); 6] = [
        ("due:", Meaning::Due),
        ("d:", Meaning::Due),
        ("start:", Meaning::Defer),
        ("s:", Meaning::Defer),
        ("plan:", Meaning::Planned),
        ("p:", Meaning::Planned),
    ];
    PREFIXES
        .iter()
        .find_map(|(prefix, meaning)| token.strip_prefix(prefix).map(|value| (*meaning, value)))
}

/// Read the value of a prefix: attached (`d:fr`) or, when nothing is
/// attached (`d: fr`), the next token.
fn value_words(value: &str, end: usize, following: &[Token<'_>]) -> Vec<(String, usize)> {
    let mut words = Vec::new();
    if !value.is_empty() {
        words.push((value.to_lowercase(), end));
    }
    words.extend(
        following
            .iter()
            .take(4)
            .map(|token| (token.text.to_lowercase(), token.end)),
    );
    words
}

/// Whether `name` may name a template: letters, digits, `-` and `_`, at
/// least one (ADR 0073 §1). A `/` alone or `//` is something else.
#[must_use]
pub fn template_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
}

/// Whether `key` may call a template after the capture prefix (ADR 0084
/// §1): one letter or digit; `h` and `H` are two keys.
#[must_use]
pub fn template_key(key: &str) -> bool {
    let mut chars = key.chars();
    chars.next().is_some_and(char::is_alphanumeric) && chars.next().is_none()
}

/// Why a template's text cannot be stored (ADR 0073 §1): it says it in
/// words, for the refusal. Free text is allowed: it goes before the title
/// of what is captured (ADR 0120).
///
/// # Errors
///
/// A `/name` inside, or anything the parser warns about.
pub fn check_template_text(text: &str) -> Result<(), String> {
    let draft = parse(text, Timestamp::UNIX_EPOCH, &TimeZone::UTC);
    if draft.template.is_some() {
        return Err("a template does not call another".into());
    }
    if let Some(warning) = draft.warnings.first() {
        return Err(format!("the template's text does not read: {warning:?}"));
    }
    Ok(())
}

/// The template a line calls, `/name` first, else `chosen` (a button), and
/// the line filled in from its text (ADR 0073 §3, §4). A `/name` that is no
/// template goes back into the title with a warning. Relative dates in the
/// template count from `now`.
pub fn apply_template<'a>(
    draft: &mut Draft,
    templates: &'a [Template],
    chosen: Option<Id>,
    now: Timestamp,
    zone: &TimeZone,
) -> Option<&'a Template> {
    let by_name = draft.template.clone().and_then(|name| {
        let found = templates
            .iter()
            .find(|t| t.title.to_lowercase() == name.to_lowercase());
        if found.is_none() {
            draft.miss_template();
        }
        found
    });
    let template =
        by_name.or_else(|| chosen.and_then(|id| templates.iter().find(|t| t.id == id)))?;
    draft.fill_from(parse(&template.text, now, zone));
    Some(template)
}

/// The template's reminders on a new task `owner` (ADR 0073 §1); none
/// without a template. The server and the app make them here alike.
pub fn template_reminders(
    template: Option<&Template>,
    owner: Id,
    now: Timestamp,
    new_id: &mut impl FnMut() -> Id,
) -> Vec<Reminder> {
    template
        .map(|t| t.reminders.as_slice())
        .unwrap_or_default()
        .iter()
        .map(|r| Reminder {
            id: new_id(),
            created: now,
            modified: now,
            owner,
            anchor: r.anchor,
            at: None,
            before: r.before,
            time: r.time,
        })
        .collect()
}

/// A line applied to an existing task (ADR 0078 §2): the task as it would
/// be, and what makes it so.
#[derive(Debug, Clone, PartialEq)]
pub struct Applied {
    /// The task after the line, tags included.
    pub task: Task,
    /// New tags, parents before children.
    pub new_tags: Vec<Tag>,
    /// The tags to add, with their position on the task, in the order typed.
    pub tags: Vec<(Id, i64)>,
    /// The fields of the task to set, by name as on the wire, each to its
    /// value in wire form (`null` clears).
    pub fields: Vec<(&'static str, serde_json::Value)>,
    /// Descendants that follow the task into its new project (ADR 0040 §3).
    pub moved: Vec<Id>,
    /// The draft with what did not resolve put back into the title, and
    /// [`Warning::FreeText`] when anything is left there.
    pub draft: Draft,
}

impl Applied {
    /// Whether the line changes nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.new_tags.is_empty()
            && self.tags.is_empty()
            && self.fields.is_empty()
            && self.moved.is_empty()
    }
}

/// Apply `draft` to `task` (ADR 0078 §2), the names resolved as on capture
/// (ADR 0027, ADR 0011 §5): tags are added and those present stay, a tag of
/// an exclusive group replaces its siblings; a project, priority, date,
/// estimate, repetition or flag the line names overwrites the task's; a new
/// project moves the task to its top level, last, with its descendants;
/// `// text` is appended to the note. No scope or view fills anything in.
/// Text left over is no title here: it is warned about as free text. A
/// template's free text leaves the title alone (ADR 0120).
pub fn apply_to(
    task: &Task,
    mut draft: Draft,
    known: Known<'_>,
    now: Timestamp,
    new_id: &mut impl FnMut() -> Id,
) -> Applied {
    use serde_json::{Value, to_value};
    let wire = |value: Result<Value, serde_json::Error>| value.unwrap_or(Value::Null);

    draft.drop_prefix();

    let named_project = draft.project.is_some();
    let resolved = resolve(draft, known, None, now, new_id);
    let mut draft = resolved.draft;
    let line = draft.title.trim().to_owned();
    if !line.is_empty() {
        draft.warnings.push(Warning::FreeText { text: line });
    }
    let mut after = task.clone();
    let mut fields: Vec<(&'static str, Value)> = Vec::new();
    let mut moved = Vec::new();

    // A project that resolved and is another one: to its top level, last.
    if named_project && draft.project.is_some() && resolved.task.project != task.project {
        after.project = resolved.task.project;
        after.parent = None;
        after.position = resolved.task.position;
        fields.push(("project", wire(to_value(after.project))));
        fields.push(("parent", Value::Null));
        fields.push(("position", Value::from(after.position)));
        let mut below = vec![task.id];
        while let Some(above) = below.pop() {
            for child in known.tasks.iter().filter(|t| t.parent == Some(above)) {
                below.push(child.id);
                if child.project != after.project {
                    moved.push(child.id);
                }
            }
        }
    }
    if draft.flagged && !task.flagged {
        after.flagged = true;
        fields.push(("flagged", Value::Bool(true)));
    }
    if let Some(priority) = draft.priority
        && task.priority != Some(priority)
    {
        after.priority = Some(priority);
        fields.push(("priority", wire(to_value(priority))));
    }
    if let Some(estimate) = draft.estimate
        && task.estimate != Some(estimate)
    {
        after.estimate = Some(estimate);
        fields.push(("estimate", wire(to_value(estimate))));
    }
    if let Some(habit) = &draft.habit
        && task.habit.as_ref() != Some(habit)
    {
        after.habit = Some(habit.clone());
        fields.push(("habit", wire(to_value(habit))));
    }
    let schedule = [
        ("defer", &draft.defer, &mut after.schedule.defer),
        ("due", &draft.due, &mut after.schedule.due),
        ("planned", &draft.planned, &mut after.schedule.planned),
    ];
    for (field, new, old) in schedule {
        if let Some(new) = new
            && old.as_ref() != Some(new)
        {
            *old = Some(new.clone());
            fields.push((field, wire(to_value(new))));
        }
    }
    if let Some(repeat) = &draft.repeat
        && after.schedule.repeat.as_ref() != Some(repeat)
    {
        after.schedule.repeat = Some(repeat.clone());
        fields.push(("repeat", wire(to_value(repeat))));
    }
    if let Some(text) = draft
        .note
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        let note = match task.note.as_deref().map(str::trim_end) {
            Some(old) if !old.trim().is_empty() => format!("{old}\n\n{text}"),
            _ => text.to_owned(),
        };
        after.note = Some(note.clone());
        fields.push(("note", Value::String(note)));
    }

    // Tags: added after those present; a state of an exclusive group
    // replaces its siblings, as the store does on `add` (ADR 0024 §1).
    let all: Vec<&Tag> = known.tags.iter().chain(&resolved.new_tags).collect();
    let mut tags = Vec::new();
    let mut next = after.tags.iter().map(|t| t.position + 1).max().unwrap_or(0);
    for tag in resolved.tags {
        if after.tags.iter().any(|t| t.tag == tag) {
            continue;
        }
        let group = all
            .iter()
            .find(|t| t.id == tag)
            .and_then(|t| t.parent)
            .filter(|parent| all.iter().any(|t| t.id == *parent && t.exclusive));
        if let Some(group) = group {
            after.tags.retain(|tagging| {
                all.iter()
                    .find(|t| t.id == tagging.tag)
                    .is_none_or(|t| t.parent != Some(group))
            });
        }
        after.tags.push(samu_api::Tagging {
            tag,
            position: next,
        });
        tags.push((tag, next));
        next += 1;
    }

    Applied {
        task: after,
        new_tags: resolved.new_tags,
        tags,
        fields,
        moved,
        draft,
    }
}

/// The template's reminders for a task that has `present` already: only
/// those with an anchor it has none for (ADR 0078 §2).
pub fn missing_reminders(made: Vec<Reminder>, present: &[Reminder]) -> Vec<Reminder> {
    made.into_iter()
        .filter(|r| !present.iter().any(|p| p.anchor == r.anchor))
        .collect()
}

/// Parse one line of quick capture. `now` and `zone` fix "today" and the zone
/// of a time of day (ADR 0007).
pub fn parse(text: &str, now: Timestamp, zone: &TimeZone) -> Draft {
    let today = now.to_zoned(zone.clone()).date();
    let tokens = tokenize(text);
    let mut draft = Draft::default();
    let mut pieces: Vec<Piece> = Vec::new();

    let mut index = 0;
    while index < tokens.len() {
        let token = tokens[index];
        let raw = token.text;
        index += 1;
        let text_piece = |raw: &str| Piece {
            raw: raw.to_owned(),
            kind: PieceKind::Text,
            start: token.start,
        };
        let mark = |draft: &mut Draft, end: usize, meaning: Meaning| {
            draft.marks.push(Mark {
                start: token.start,
                end,
                meaning,
            });
        };

        if raw == "//" {
            let note = text[token.end..].trim();
            if !note.is_empty() {
                draft.note = Some(note.to_owned());
            }
            mark(&mut draft, text.len(), Meaning::Note);
            break;
        }
        if let Some(escaped) = raw.strip_prefix('\\').filter(|rest| !rest.is_empty()) {
            pieces.push(text_piece(escaped));
            continue;
        }
        if raw == "!" {
            draft.flagged = true;
            mark(&mut draft, token.end, Meaning::Flag);
            continue;
        }
        // ADR 0057: `!A` like `!`; anything else after `!` is text.
        let priority = match raw {
            "!A" | "!a" => Some(Priority::A),
            "!B" | "!b" => Some(Priority::B),
            "!C" | "!c" => Some(Priority::C),
            _ => None,
        };
        if let Some(priority) = priority {
            if draft.priority.is_none() {
                draft.priority = Some(priority);
                mark(&mut draft, token.end, Meaning::Priority);
            } else {
                draft.warnings.push(Warning::Twice {
                    meaning: Meaning::Priority,
                    text: raw.to_owned(),
                });
                pieces.push(text_piece(raw));
            }
            continue;
        }
        if let Some(name) = raw.strip_prefix('/').filter(|name| template_name(name)) {
            if draft.template.is_none() {
                draft.template = Some(name.to_owned());
                mark(&mut draft, token.end, Meaning::Template);
                pieces.push(Piece {
                    raw: raw.to_owned(),
                    kind: PieceKind::Template,
                    start: token.start,
                });
            } else {
                draft.warnings.push(Warning::Twice {
                    meaning: Meaning::Template,
                    text: raw.to_owned(),
                });
                pieces.push(text_piece(raw));
            }
            continue;
        }
        if let Some(name) = raw.strip_prefix('#').filter(|name| !name.is_empty()) {
            if draft.project.is_none() {
                draft.project = Some(unquote(name));
                mark(&mut draft, token.end, Meaning::Project);
                pieces.push(Piece {
                    raw: raw.to_owned(),
                    kind: PieceKind::Project,
                    start: token.start,
                });
            } else {
                draft.warnings.push(Warning::Twice {
                    meaning: Meaning::Project,
                    text: raw.to_owned(),
                });
                pieces.push(text_piece(raw));
            }
            continue;
        }
        if let Some(name) = raw.strip_prefix('@').filter(|name| !name.is_empty()) {
            let name = unquote(name);
            if !draft.tags.contains(&name) {
                draft.tags.push(name.clone());
            }
            mark(&mut draft, token.end, Meaning::Tag);
            pieces.push(Piece {
                raw: raw.to_owned(),
                kind: PieceKind::Tag(name),
                start: token.start,
            });
            continue;
        }
        if let Some(value) = raw.strip_prefix("habit:") {
            match parse_habit(value) {
                Some(habit) if draft.habit.is_none() => {
                    draft.habit = Some(habit);
                    mark(&mut draft, token.end, Meaning::Habit);
                    continue;
                }
                Some(_) => draft.warnings.push(Warning::Twice {
                    meaning: Meaning::Habit,
                    text: raw.to_owned(),
                }),
                None => draft.warnings.push(Warning::UnknownHabit {
                    text: raw.to_owned(),
                }),
            }
            pieces.push(text_piece(raw));
            continue;
        }
        if let Some(value) = raw.strip_prefix('~').filter(|value| !value.is_empty()) {
            match parse_estimate(value) {
                Some(estimate) if draft.estimate.is_none() => {
                    draft.estimate = Some(estimate);
                    mark(&mut draft, token.end, Meaning::Estimate);
                    continue;
                }
                Some(_) => draft.warnings.push(Warning::Twice {
                    meaning: Meaning::Estimate,
                    text: raw.to_owned(),
                }),
                None => draft.warnings.push(Warning::UnknownEstimate {
                    text: raw.to_owned(),
                }),
            }
            pieces.push(text_piece(raw));
            continue;
        }
        if let Some(value) = raw.strip_prefix("z:") {
            let words = value_words(value, token.end, &tokens[index..]);
            let attached = usize::from(!value.is_empty());
            if let Some((spec, used)) = parse_booked(&words, today) {
                draft.clocks.push(BookedClock {
                    spec,
                    text: text[token.start..words[used - 1].1].to_owned(),
                });
                mark(&mut draft, words[used - 1].1, Meaning::Clock);
                index += used - attached;
                continue;
            }
            draft.warnings.push(Warning::UnknownClock {
                text: raw.to_owned(),
            });
            pieces.push(text_piece(raw));
            continue;
        }
        if let Some(value) = raw.strip_prefix("x:") {
            let words = value_words(value, token.end, &tokens[index..]);
            let plain: Vec<&str> = words.iter().map(|(w, _)| w.as_str()).collect();
            let day = past_date(&plain, today);
            if value.is_empty() || day.is_some() {
                if draft.completed.is_some() {
                    draft.warnings.push(Warning::Twice {
                        meaning: Meaning::Completed,
                        text: raw.to_owned(),
                    });
                    pieces.push(text_piece(raw));
                    continue;
                }
                match day {
                    Some((date, used)) => {
                        draft.completed = Some(Some(date));
                        mark(&mut draft, words[used - 1].1, Meaning::Completed);
                        index += used - usize::from(!value.is_empty());
                    }
                    None => {
                        draft.completed = Some(None);
                        mark(&mut draft, token.end, Meaning::Completed);
                    }
                }
                continue;
            }
            draft.warnings.push(Warning::UnknownClock {
                text: raw.to_owned(),
            });
            pieces.push(text_piece(raw));
            continue;
        }
        if let Some((meaning, value)) = date_prefix(raw) {
            let words = value_words(value, token.end, &tokens[index..]);
            let attached = usize::from(!value.is_empty());
            match parse_when(&words, today, zone) {
                Some((when, used)) => {
                    let slot = match meaning {
                        Meaning::Defer => &mut draft.defer,
                        Meaning::Due => &mut draft.due,
                        _ => &mut draft.planned,
                    };
                    if slot.is_none() {
                        *slot = Some(when);
                        mark(&mut draft, words[used - 1].1, meaning);
                        index += used - attached;
                        continue;
                    }
                    draft.warnings.push(Warning::Twice {
                        meaning,
                        text: raw.to_owned(),
                    });
                }
                None => draft.warnings.push(Warning::UnknownDate {
                    meaning,
                    text: raw.to_owned(),
                }),
            }
            pieces.push(text_piece(raw));
            continue;
        }
        // `every+:` is `fixed` with catch-up, like Org `++` (ADR 0073 §2).
        let repeat_value = raw
            .strip_prefix("every!:")
            .map(|value| (RepeatMode::DueAfterCompletion, false, value))
            .or_else(|| {
                raw.strip_prefix("every+:")
                    .map(|value| (RepeatMode::Fixed, true, value))
            })
            .or_else(|| {
                raw.strip_prefix("every:")
                    .map(|value| (RepeatMode::Fixed, false, value))
            });
        if let Some((mode, catch_up, value)) = repeat_value {
            let words = value_words(value, token.end, &tokens[index..]);
            let attached = usize::from(!value.is_empty());
            match words.first().and_then(|(word, _)| repeat_rule(word)) {
                Some(rule) if draft.repeat.is_none() => {
                    draft.repeat = Some(Repeat {
                        rule,
                        mode,
                        catch_up,
                    });
                    mark(&mut draft, words[0].1, Meaning::Repeat);
                    index += 1 - attached;
                    continue;
                }
                Some(_) => draft.warnings.push(Warning::Twice {
                    meaning: Meaning::Repeat,
                    text: raw.to_owned(),
                }),
                None => draft.warnings.push(Warning::UnknownRepeat {
                    text: raw.to_owned(),
                }),
            }
            pieces.push(text_piece(raw));
            continue;
        }
        pieces.push(text_piece(raw));
    }

    draft.title = title_of(&pieces);
    draft.pieces = pieces;
    draft
}

/// The value of `habit:` (ADR 0091 §5): empty for once a day, `avoid`,
/// `N/d`, `N/w`, `N/m`, each with an optional `:unit`, like `8/d:Glas`.
fn parse_habit(value: &str) -> Option<Habit> {
    if value.eq_ignore_ascii_case("avoid") {
        return Some(Habit {
            kind: HabitKind::Avoid,
            ..Habit::default()
        });
    }
    if value.is_empty() {
        return Some(Habit::default());
    }
    let (goal, unit) = match value.split_once(':') {
        Some((goal, unit)) if !unit.is_empty() && unit.len() <= 32 => (goal, Some(unit)),
        Some(_) => return None,
        None => (value, None),
    };
    let (times, per) = goal.split_once('/')?;
    let times: u32 = times.parse().ok().filter(|t| *t > 0)?;
    let per = match per.to_ascii_lowercase().as_str() {
        "d" => HabitPer::Day,
        "w" => HabitPer::Week,
        "m" => HabitPer::Month,
        _ => return None,
    };
    Some(Habit {
        kind: HabitKind::Build,
        times,
        per,
        unit: unit.map(str::to_owned),
    })
}

/// A date, optionally followed by a time of day; returns how many words it
/// took.
fn parse_when(words: &[(String, usize)], today: Date, zone: &TimeZone) -> Option<(When, usize)> {
    let plain: Vec<&str> = words.iter().map(|(word, _)| word.as_str()).collect();
    let (date, used) = parse_date(&plain, today)?;
    if let Some(time) = plain.get(used).and_then(|word| parse_time(word)) {
        let at = date.to_datetime(time).to_zoned(zone.clone()).ok()?;
        return Some((When::At(at), used + 1));
    }
    Some((When::Date(date), used))
}

fn weekday(word: &str) -> Option<Weekday> {
    let day = match word {
        "mo" | "montag" | "mon" | "monday" => Weekday::Monday,
        "di" | "dienstag" | "tue" | "tuesday" => Weekday::Tuesday,
        "mi" | "mittwoch" | "wed" | "wednesday" => Weekday::Wednesday,
        "do" | "donnerstag" | "thu" | "thursday" => Weekday::Thursday,
        "fr" | "freitag" | "fri" | "friday" => Weekday::Friday,
        "sa" | "samstag" | "sat" | "saturday" => Weekday::Saturday,
        "so" | "sonntag" | "sun" | "sunday" => Weekday::Sunday,
        _ => return None,
    };
    Some(day)
}

/// A count with a unit `d`, `w`, `m`, `y` as a span.
fn unit_span(count: i64, unit: &str) -> Option<Span> {
    let span = match unit {
        "d" | "tag" | "tage" | "tagen" | "day" | "days" => count.days(),
        "w" | "woche" | "wochen" | "week" | "weeks" => count.weeks(),
        "m" | "monat" | "monate" | "monaten" | "month" | "months" => count.months(),
        "y" | "jahr" | "jahre" | "jahren" | "year" | "years" => count.years(),
        _ => return None,
    };
    Some(span)
}

/// `3d` → (3, "d").
fn count_and_unit(text: &str) -> Option<(i64, &str)> {
    let digits = text.find(|c: char| !c.is_ascii_digit())?;
    let count = text[..digits].parse().ok()?;
    Some((count, &text[digits..]))
}

fn parse_date(words: &[&str], today: Date) -> Option<(Date, usize)> {
    let first = *words.first()?;
    let second = words.get(1).copied();
    match first {
        "heute" | "today" => return Some((today, 1)),
        "morgen" | "tomorrow" => return Some((today.tomorrow().ok()?, 1)),
        "übermorgen" | "uebermorgen" => return Some((today.checked_add(2.days()).ok()?, 1)),
        "monatsende" => return Some((today.last_of_month(), 1)),
        "nächste" | "naechste" | "next" if matches!(second, Some("woche" | "week")) => {
            return Some((today.nth_weekday(1, Weekday::Monday).ok()?, 2));
        }
        "end" if second == Some("of") && words.get(2) == Some(&"month") => {
            return Some((today.last_of_month(), 3));
        }
        "in" => {
            let count: i64 = second?.parse().ok()?;
            let span = unit_span(count, words.get(2)?)?;
            return Some((today.checked_add(span).ok()?, 3));
        }
        _ => {}
    }
    if let Some(day) = weekday(first) {
        // `nth_weekday(1, …)` never returns the day itself: "the next".
        return Some((today.nth_weekday(1, day).ok()?, 1));
    }
    if let Some(relative) = first.strip_prefix('+') {
        let (count, unit) = count_and_unit(relative)?;
        return Some((today.checked_add(unit_span(count, unit)?).ok()?, 1));
    }
    if first.contains('-') {
        return Some((first.parse().ok()?, 1));
    }
    let parts: Vec<&str> = first.split('.').collect();
    let (day, month, year) = match parts.as_slice() {
        [day, month] | [day, month, ""] => (*day, *month, None),
        [day, month, year] if year.len() == 4 => (*day, *month, Some(*year)),
        _ => return None,
    };
    let day: i8 = day.parse().ok()?;
    let month: i8 = month.parse().ok()?;
    let date = match year {
        Some(year) => Date::new(year.parse().ok()?, month, day).ok()?,
        // Without a year: the next such day, today included.
        None => {
            let this_year = Date::new(today.year(), month, day).ok()?;
            if this_year >= today {
                this_year
            } else {
                Date::new(today.year() + 1, month, day).ok()?
            }
        }
    };
    Some((date, 1))
}

/// `9:00`, `14:30`, `9h`.
fn parse_time(word: &str) -> Option<Time> {
    let (hour, minute) = match word.split_once(':') {
        Some((hour, minute)) if minute.len() == 2 => (hour, minute),
        Some(_) => return None,
        None => (word.strip_suffix('h')?, "0"),
    };
    if hour.is_empty() || hour.len() > 2 {
        return None;
    }
    Time::new(hour.parse().ok()?, minute.parse().ok()?, 0, 0).ok()
}

/// A day in the past for `z:` and `x:` (ADR 0081 §2): `heute`, `gestern`,
/// `vorgestern`, a weekday (the last one, today included), `d.m.` (the
/// last one) or a full date. Returns how many words it took.
fn past_date(words: &[&str], today: Date) -> Option<(Date, usize)> {
    let first = *words.first()?;
    let back = |days: i64| today.checked_sub(days.days()).ok();
    match first {
        "heute" | "today" => return Some((today, 1)),
        "gestern" | "yesterday" => return Some((back(1)?, 1)),
        "vorgestern" => return Some((back(2)?, 1)),
        _ => {}
    }
    if let Some(day) = weekday(first) {
        let since =
            (today.weekday().to_monday_zero_offset() - day.to_monday_zero_offset()).rem_euclid(7);
        return Some((back(i64::from(since))?, 1));
    }
    if first.contains('-') && first.len() == 10 {
        return Some((first.parse().ok()?, 1));
    }
    let parts: Vec<&str> = first.split('.').collect();
    let (day, month, year) = match parts.as_slice() {
        [day, month] | [day, month, ""] => (*day, *month, None),
        [day, month, year] if year.len() == 4 => (*day, *month, Some(*year)),
        _ => return None,
    };
    let day: i8 = day.parse().ok()?;
    let month: i8 = month.parse().ok()?;
    let date = match year {
        Some(year) => Date::new(year.parse().ok()?, month, day).ok()?,
        // Without a year: the last such day, today included.
        None => {
            let this_year = Date::new(today.year(), month, day).ok()?;
            if this_year <= today {
                this_year
            } else {
                Date::new(today.year() - 1, month, day).ok()?
            }
        }
    };
    Some((date, 1))
}

/// `14`, `14:30`, `14h` as a time of day, for a range.
fn range_time(word: &str) -> Option<Time> {
    if !word.is_empty() && word.len() <= 2 && word.bytes().all(|b| b.is_ascii_digit()) {
        return Time::new(word.parse().ok()?, 0, 0, 0).ok();
    }
    parse_time(word)
}

/// `14-15:30`, `14:00-15:30`: from and to.
fn time_range(word: &str) -> Option<(Time, Time)> {
    let (from, to) = word.split_once('-')?;
    Some((range_time(from)?, range_time(to)?))
}

/// The value of `z:`: a length, a range today, or a past day and a length
/// or a range (ADR 0081 §2). Returns how many words it took.
fn parse_booked(words: &[(String, usize)], today: Date) -> Option<(ClockSpec, usize)> {
    let plain: Vec<&str> = words.iter().map(|(word, _)| word.as_str()).collect();
    let first = *plain.first()?;
    if let Some(length) = parse_estimate(first) {
        return Some((ClockSpec::Length(length), 1));
    }
    if let Some((from, to)) = time_range(first) {
        return Some((ClockSpec::Range(today, from, to), 1));
    }
    let (date, used) = past_date(&plain, today)?;
    let next = *plain.get(used)?;
    if let Some(length) = parse_estimate(next) {
        return Some((ClockSpec::OnDay(date, length), used + 1));
    }
    let (from, to) = time_range(next)?;
    Some((ClockSpec::Range(date, from, to), used + 1))
}

/// The seconds of a length of time; hours and minutes are all `~` and `z:`
/// make.
fn period_seconds(period: Period) -> i64 {
    period.0.total(jiff::Unit::Second).map_or(0, |s| {
        #[allow(clippy::cast_possible_truncation, reason = "a length booked fits")]
        let s = s as i64;
        s
    })
}

/// The instant a day's time of day is in `zone`.
fn instant(date: Date, time: Time, zone: &TimeZone) -> Option<Timestamp> {
    date.to_datetime(time)
        .to_zoned(zone.clone())
        .ok()
        .map(|z| z.timestamp())
}

/// Where a day's work goes on (ADR 0081 §2): after the last clock that ends
/// on that day, without one at 09:00.
fn after_last_clock(date: Date, clocks: &[Clock], zone: &TimeZone) -> Option<Timestamp> {
    clocks
        .iter()
        .filter_map(|c| c.end)
        .filter(|end| end.to_zoned(zone.clone()).date() == date)
        .max()
        .or_else(|| instant(date, Time::constant(9, 0, 0, 0), zone))
}

/// The clocks of `booked` on `owner` (ADR 0081 §2), placed among `existing`
/// (every clock of the account) and one another; one that would end after
/// `now` or before it starts is left out with a warning.
pub fn place_clocks(
    booked: &[BookedClock],
    existing: &[Clock],
    owner: Id,
    zone: &TimeZone,
    now: Timestamp,
    new_id: &mut impl FnMut() -> Id,
) -> (Vec<Clock>, Vec<Warning>) {
    let mut placed: Vec<Clock> = Vec::new();
    let mut warnings = Vec::new();
    for b in booked {
        let span = match &b.spec {
            ClockSpec::Length(length) => now
                .checked_sub(jiff::SignedDuration::from_secs(period_seconds(*length)))
                .ok()
                .map(|start| (start, now)),
            ClockSpec::OnDay(date, length) => {
                let all: Vec<Clock> = existing.iter().chain(&placed).cloned().collect();
                after_last_clock(*date, &all, zone).and_then(|start| {
                    start
                        .checked_add(jiff::SignedDuration::from_secs(period_seconds(*length)))
                        .ok()
                        .map(|end| (start, end))
                })
            }
            ClockSpec::Range(date, from, to) => {
                instant(*date, *from, zone).zip(instant(*date, *to, zone))
            }
        };
        match span {
            Some((_, end)) if end > now => warnings.push(Warning::ClockInFuture {
                text: b.text.clone(),
            }),
            Some((start, end)) if start < end => placed.push(Clock {
                id: new_id(),
                created: now,
                modified: now,
                owner,
                start,
                end: Some(end),
                note: None,
            }),
            _ => warnings.push(Warning::UnknownClock {
                text: b.text.clone(),
            }),
        }
    }
    (placed, warnings)
}

/// When `x:` completes (ADR 0081 §2): now, or on a day at the end of its
/// last clock, among `clocks`, without one at 09:00; never after `now`.
#[must_use]
pub fn completed_at(
    completed: Option<Date>,
    clocks: &[Clock],
    zone: &TimeZone,
    now: Timestamp,
) -> Timestamp {
    completed
        .and_then(|date| after_last_clock(date, clocks, zone))
        .map_or(now, |at| at.min(now))
}

/// `30m`, `45min`, `2h`, `1h30`, `1h30m`.
fn parse_estimate(text: &str) -> Option<Period> {
    let (count, rest) = count_and_unit(text)?;
    let span = match rest {
        "m" | "min" => count.minutes(),
        "h" => count.hours(),
        _ => {
            let minutes = rest.strip_prefix('h')?;
            let minutes = minutes.strip_suffix('m').unwrap_or(minutes);
            count.hours().minutes(minutes.parse::<i64>().ok()?)
        }
    };
    (!span.is_zero()).then_some(Period(span))
}

/// The short forms of ADR 0011 §5 as an RRULE: `2d`, `1w`, `1m`, `1y`, or a
/// weekday.
fn repeat_rule(word: &str) -> Option<String> {
    if let Some(day) = weekday(word) {
        let byday = match day {
            Weekday::Monday => "MO",
            Weekday::Tuesday => "TU",
            Weekday::Wednesday => "WE",
            Weekday::Thursday => "TH",
            Weekday::Friday => "FR",
            Weekday::Saturday => "SA",
            Weekday::Sunday => "SU",
        };
        return Some(format!("FREQ=WEEKLY;BYDAY={byday}"));
    }
    let (count, unit) = count_and_unit(word)?;
    let freq = match unit {
        "d" => "DAILY",
        "w" => "WEEKLY",
        "m" => "MONTHLY",
        "y" => "YEARLY",
        _ => return None,
    };
    match count {
        0 => None,
        1 => Some(format!("FREQ={freq}")),
        _ => Some(format!("FREQ={freq};INTERVAL={count}")),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::*;
    use jiff::civil::date;

    fn berlin() -> TimeZone {
        TimeZone::get("Europe/Berlin").unwrap()
    }

    /// Saturday, 7 November 2026, 10:00 in Berlin.
    fn saturday() -> Timestamp {
        "2026-11-07T09:00:00Z".parse().unwrap()
    }

    fn draft(text: &str) -> Draft {
        parse(text, saturday(), &berlin())
    }

    fn at(text: &str) -> When {
        When::At(text.parse().unwrap())
    }

    #[test]
    fn dates_after_a_prefix() {
        let table: &[(&str, When)] = &[
            ("x d:heute", When::Date(date(2026, 11, 7))),
            ("x d:today", When::Date(date(2026, 11, 7))),
            ("x d:morgen", When::Date(date(2026, 11, 8))),
            ("x d:tomorrow", When::Date(date(2026, 11, 8))),
            ("x d:übermorgen", When::Date(date(2026, 11, 9))),
            ("x d:mo", When::Date(date(2026, 11, 9))),
            ("x d:Freitag", When::Date(date(2026, 11, 13))),
            ("x d:fri", When::Date(date(2026, 11, 13))),
            // The next Saturday, never today.
            ("x d:sa", When::Date(date(2026, 11, 14))),
            ("x d:+3d", When::Date(date(2026, 11, 10))),
            ("x d:+2w", When::Date(date(2026, 11, 21))),
            ("x d:+1m", When::Date(date(2026, 12, 7))),
            ("x d:in 3 tagen", When::Date(date(2026, 11, 10))),
            ("x d:in 2 wochen", When::Date(date(2026, 11, 21))),
            ("x d:in 3 days", When::Date(date(2026, 11, 10))),
            ("x d:20.11.", When::Date(date(2026, 11, 20))),
            ("x d:7.11.", When::Date(date(2026, 11, 7))),
            ("x d:1.1.", When::Date(date(2027, 1, 1))),
            ("x d:20.11.2027", When::Date(date(2027, 11, 20))),
            ("x d:2026-11-20", When::Date(date(2026, 11, 20))),
            ("x d:nächste woche", When::Date(date(2026, 11, 9))),
            ("x d:next week", When::Date(date(2026, 11, 9))),
            ("x d:monatsende", When::Date(date(2026, 11, 30))),
            ("x d:end of month", When::Date(date(2026, 11, 30))),
            ("x d: fr", When::Date(date(2026, 11, 13))),
            ("x due:fr", When::Date(date(2026, 11, 13))),
            (
                "x d:fr 9:00",
                at("2026-11-13T09:00:00+01:00[Europe/Berlin]"),
            ),
            ("x d:fr 9h", at("2026-11-13T09:00:00+01:00[Europe/Berlin]")),
            (
                "x d:morgen 14:30",
                at("2026-11-08T14:30:00+01:00[Europe/Berlin]"),
            ),
        ];
        for (text, expected) in table {
            let draft = draft(text);
            assert_eq!(draft.due.as_ref(), Some(expected), "{text}");
            assert_eq!(draft.title, "x", "{text}");
            assert!(draft.warnings.is_empty(), "{text}: {:?}", draft.warnings);
        }
    }

    #[test]
    fn a_time_across_the_end_of_summer_time() {
        // Saturday, 24 October 2026, at +02:00; Sunday is at +01:00.
        let now: Timestamp = "2026-10-24T08:00:00Z".parse().unwrap();
        let draft = parse("x p:morgen 9:00", now, &berlin());
        assert_eq!(
            draft.planned,
            Some(at("2026-10-25T09:00:00+01:00[Europe/Berlin]"))
        );
    }

    #[test]
    fn defer_and_planned_have_their_prefixes() {
        let draft = draft("x s:mo start:di p:mi plan:do");
        assert_eq!(draft.defer, Some(When::Date(date(2026, 11, 9))));
        assert_eq!(draft.planned, Some(When::Date(date(2026, 11, 11))));
        assert_eq!(draft.title, "x start:di plan:do");
        assert_eq!(draft.warnings.len(), 2);
    }

    #[test]
    fn no_date_without_a_prefix() {
        let draft = draft("Meeting morgen vorbereiten");
        assert_eq!(draft.title, "Meeting morgen vorbereiten");
        assert_eq!(
            draft,
            Draft {
                title: draft.title.clone(),
                pieces: draft.pieces.clone(),
                ..Draft::default()
            }
        );
    }

    #[test]
    fn an_unknown_date_stays_in_the_title() {
        for text in [
            "x d:bald",
            "x d:32.11.",
            "x d:",
            "x d:9:00",
            "x d:in 3 jahrzehnten",
        ] {
            let draft = draft(text);
            assert_eq!(draft.due, None, "{text}");
            assert!(draft.title.starts_with("x d:"), "{text}: {}", draft.title);
            assert!(
                matches!(
                    draft.warnings.as_slice(),
                    [Warning::UnknownDate {
                        meaning: Meaning::Due,
                        ..
                    }]
                ),
                "{text}: {:?}",
                draft.warnings
            );
        }
    }

    #[test]
    fn a_time_that_is_no_time_stays_text() {
        let draft = draft("d:fr 10 Bücher");
        assert_eq!(draft.due, Some(When::Date(date(2026, 11, 13))));
        assert_eq!(draft.title, "10 Bücher");
    }

    #[test]
    fn project_tags_flag_estimate_note() {
        let draft = draft("Anrufen #haus @telefon @prio/A ! ~1h30 // Nummer steht in der Mail");
        assert_eq!(draft.title, "Anrufen");
        assert_eq!(draft.project.as_deref(), Some("haus"));
        assert_eq!(draft.tags, ["telefon", "prio/A"]);
        assert!(draft.flagged);
        assert_eq!(draft.estimate, Some(Period(1.hours().minutes(30))));
        assert_eq!(draft.note.as_deref(), Some("Nummer steht in der Mail"));
        let meanings: Vec<Meaning> = draft.marks.iter().map(|mark| mark.meaning).collect();
        assert_eq!(
            meanings,
            [
                Meaning::Project,
                Meaning::Tag,
                Meaning::Tag,
                Meaning::Flag,
                Meaning::Estimate,
                Meaning::Note
            ]
        );
    }

    #[test]
    fn marks_are_byte_offsets() {
        let text = "Bäder #haus d:fr 9:00";
        let draft = draft(text);
        let spans: Vec<&str> = draft
            .marks
            .iter()
            .map(|mark| &text[mark.start..mark.end])
            .collect();
        assert_eq!(spans, ["#haus", "d:fr 9:00"]);
    }

    #[test]
    fn estimates() {
        let table: &[(&str, Option<Span>)] = &[
            ("~30m", Some(30.minutes())),
            ("~45min", Some(45.minutes())),
            ("~2h", Some(2.hours())),
            ("~1h30", Some(1.hours().minutes(30))),
            ("~1h30m", Some(1.hours().minutes(30))),
            ("~90", None),
            ("~0m", None),
            ("~bald", None),
        ];
        for (text, expected) in table {
            let draft = draft(&format!("x {text}"));
            assert_eq!(draft.estimate, expected.map(Period), "{text}");
            assert_eq!(draft.warnings.is_empty(), expected.is_some(), "{text}");
        }
    }

    #[test]
    fn repetitions() {
        let table: &[(&str, &str, RepeatMode)] = &[
            ("every:1w", "FREQ=WEEKLY", RepeatMode::Fixed),
            ("every:2d", "FREQ=DAILY;INTERVAL=2", RepeatMode::Fixed),
            ("every:1m", "FREQ=MONTHLY", RepeatMode::Fixed),
            ("every:mo", "FREQ=WEEKLY;BYDAY=MO", RepeatMode::Fixed),
            ("every!:1w", "FREQ=WEEKLY", RepeatMode::DueAfterCompletion),
            ("every: 1y", "FREQ=YEARLY", RepeatMode::Fixed),
        ];
        for (text, rule, mode) in table {
            let draft = draft(&format!("x {text}"));
            let repeat = draft.repeat.expect(text);
            assert_eq!(
                (repeat.rule.as_str(), repeat.mode),
                (*rule, *mode),
                "{text}"
            );
            assert_eq!(draft.title, "x", "{text}");
        }
        let draft = draft("x every:0d every:sometimes");
        assert_eq!(draft.repeat, None);
        assert_eq!(draft.title, "x every:0d every:sometimes");
        assert_eq!(draft.warnings.len(), 2);
    }

    #[test]
    fn quoted_names_hold_spaces() {
        let draft = draft(r#"Fliesen #"Neues Bad" @"bei Gelegenheit" kaufen"#);
        assert_eq!(draft.title, "Fliesen kaufen");
        assert_eq!(draft.project.as_deref(), Some("Neues Bad"));
        assert_eq!(draft.tags, ["bei Gelegenheit"]);
    }

    #[test]
    fn a_backslash_makes_a_shortcut_text() {
        let draft = draft(r"Ticket \#42 \@home \d:fr \! \// zu");
        assert_eq!(draft.title, "Ticket #42 @home d:fr ! // zu");
        assert_eq!(draft.project, None);
        assert!(draft.tags.is_empty() && !draft.flagged && draft.due.is_none());
        assert!(draft.note.is_none());
    }

    #[test]
    fn a_bare_sign_is_text() {
        let draft = draft("# @ ~ Warum!");
        assert_eq!(draft.title, "# @ ~ Warum!");
        assert!(draft.marks.is_empty());
    }

    #[test]
    fn a_second_project_or_date_stays_in_the_title() {
        let draft = draft("x #haus #garten d:mo d:di");
        assert_eq!(draft.project.as_deref(), Some("haus"));
        assert_eq!(draft.due, Some(When::Date(date(2026, 11, 9))));
        assert_eq!(draft.title, "x #garten d:di");
        assert_eq!(draft.warnings.len(), 2);
    }

    #[test]
    fn a_missed_project_goes_back_where_it_stood() {
        let mut draft = draft("Fliesen #bad kaufen @baumarkt");
        draft.miss_project();
        assert_eq!(draft.title, "Fliesen #bad kaufen");
        assert_eq!(draft.project, None);
        assert_eq!(
            draft.warnings,
            [Warning::UnknownProject { name: "bad".into() }]
        );
        assert!(draft.marks.iter().all(|mark| mark.meaning == Meaning::Tag));
    }

    fn id(n: u8) -> Id {
        format!("01936f5e-8a00-7c3e-9b1a-0000000000{n:02x}")
            .parse()
            .unwrap()
    }

    fn project(n: u8, title: &str, status: ProjectStatus) -> Project {
        Project {
            scope_only: false,
            own_scope: None,
            icon: None,
            created_by: None,
            id: id(n),
            created: saturday(),
            modified: saturday(),
            title: title.to_owned(),
            note: None,
            folder: None,
            workflow: None,
            kind: samu_api::ProjectKind::Parallel,
            status,
            schedule: Schedule::default(),
            review_interval: None,
            next_review: None,
            last_reviewed: None,
            flagged: false,
            estimate: None,
            completed_at: None,
            dropped_at: None,
            tags: Vec::new(),
            properties: std::collections::BTreeMap::new(),
            position: i64::from(n),
        }
    }

    fn tag(n: u8, title: &str, parent: Option<u8>, exclusive: bool) -> Tag {
        Tag {
            scope_only: false,
            own_scope: None,
            id: id(n),
            created: saturday(),
            modified: saturday(),
            title: title.to_owned(),
            parent: parent.map(id),
            status: TagStatus::Active,
            exclusive,
            key: None,
            ask_note: false,
            default_workflow: false,
            position: 0,
        }
    }

    /// Projects: Neues Bad, Haus, Haushalt, Alt (dropped). Tags: telefon,
    /// termin, ort/büro, Status (exclusive)/Wartet.
    fn known_world() -> (Vec<Project>, Vec<Tag>) {
        let projects = vec![
            project(1, "Neues Bad", ProjectStatus::Active),
            project(2, "Haus", ProjectStatus::Active),
            project(3, "Haushalt", ProjectStatus::OnHold),
            project(4, "Alt", ProjectStatus::Dropped),
        ];
        let tags = vec![
            tag(0x10, "telefon", None, false),
            tag(0x11, "termin", None, false),
            tag(0x12, "ort", None, false),
            tag(0x13, "büro", Some(0x12), false),
            tag(0x14, "Status", None, true),
            tag(0x15, "Wartet", Some(0x14), false),
        ];
        (projects, tags)
    }

    fn resolved(text: &str, fallback: Option<Id>) -> Resolved {
        let (projects, tags) = known_world();
        let mut next = 0xa0_u8;
        let mut new_id = || {
            next += 1;
            id(next)
        };
        let known = Known {
            projects: &projects,
            tags: &tags,
            tasks: &[],
        };
        resolve(draft(text), known, fallback, saturday(), &mut new_id)
    }

    fn template(title: &str, text: &str) -> Template {
        serde_json::from_value(serde_json::json!({
            "id": id(0x80), "created": "2026-11-07T08:00:00Z",
            "modified": "2026-11-07T08:00:00Z", "title": title, "text": text,
            "reminders": [{"anchor": "planned"}], "position": 0,
        }))
        .unwrap_or_else(|e| panic!("{e}"))
    }

    fn existing(n: u8, title: &str, project: Option<u8>, parent: Option<u8>) -> Task {
        serde_json::from_value(serde_json::json!({
            "id": id(n), "created": "2026-11-07T08:00:00Z",
            "modified": "2026-11-07T08:00:00Z", "title": title,
            "project": project.map(id), "parent": parent.map(id),
            "note": "erste Zeile", "position": 0,
            "tags": [{"tag": id(0x10), "position": 0}],
        }))
        .unwrap_or_else(|e| panic!("{e}"))
    }

    fn applied(task: &Task, tasks: &[Task], tags: &[Tag], text: &str) -> Applied {
        let (projects, _) = known_world();
        let mut next = 0xa0_u8;
        let mut new_id = || {
            next += 1;
            id(next)
        };
        let known = Known {
            projects: &projects,
            tags,
            tasks,
        };
        apply_to(task, draft(text), known, saturday(), &mut new_id)
    }

    /// ADR 0078 §2: tags come on top, fields are overwritten, the note
    /// grows, what is unchanged is not written.
    #[test]
    fn a_line_applied_to_a_task() {
        let (_, tags) = known_world();
        let task = existing(0x40, "Fliesen", Some(2), None);
        let a = applied(
            &task,
            std::slice::from_ref(&task),
            &tags,
            "@termin @telefon !A d:mo ~30m ! // Nummer im Brief",
        );
        let on: Vec<Id> = a.task.tags.iter().map(|t| t.tag).collect();
        assert_eq!(on, [id(0x10), id(0x11)]);
        assert_eq!(a.tags, [(id(0x11), 1)]);
        assert_eq!(a.task.title, "Fliesen");
        assert_eq!(a.task.project, Some(id(2)));
        assert_eq!(a.task.priority, Some(Priority::A));
        assert_eq!(a.task.schedule.due, Some(When::Date(date(2026, 11, 9))));
        assert!(a.task.flagged);
        assert_eq!(
            a.task.note.as_deref(),
            Some("erste Zeile\n\nNummer im Brief")
        );
        let names: Vec<&str> = a.fields.iter().map(|(f, _)| *f).collect();
        assert_eq!(names, ["flagged", "priority", "estimate", "due", "note"]);
        assert!(a.draft.warnings.is_empty());

        // The same project, and only a tag it has: nothing to write.
        let same = applied(&task, &[], &tags, "#Haus @telefon");
        assert!(same.is_empty(), "{same:?}");
    }

    /// ADR 0078 §2: a state of an exclusive group replaces its sibling.
    #[test]
    fn a_state_replaces_its_sibling() {
        let (_, mut tags) = known_world();
        tags.push(tag(0x16, "Erledigt", Some(0x14), false));
        let mut task = existing(0x40, "Fliesen", None, None);
        task.tags.push(samu_api::Tagging {
            tag: id(0x15),
            position: 1,
        });
        let a = applied(&task, &[], &tags, "@Status/Erledigt");
        let on: Vec<Id> = a.task.tags.iter().map(|t| t.tag).collect();
        assert_eq!(on, [id(0x10), id(0x16)]);
        assert_eq!(a.tags, [(id(0x16), 2)]);
    }

    /// ADR 0078 §2: a new project moves the task to its top level, last,
    /// and its descendants follow.
    #[test]
    fn a_new_project_moves_the_task_and_its_children() {
        let (_, tags) = known_world();
        let task = existing(0x40, "Fliesen", Some(2), Some(0x41));
        let child = existing(0x42, "Kleber", Some(2), Some(0x40));
        let grandchild = existing(0x43, "Spachtel", Some(2), Some(0x42));
        let mut there = existing(0x44, "Schon da", Some(1), None);
        there.position = 4;
        let tasks = [task.clone(), child, grandchild, there];
        let a = applied(&task, &tasks, &tags, "#Neues");
        assert_eq!(a.task.project, Some(id(1)));
        assert_eq!(a.task.parent, None);
        assert_eq!(a.task.position, 5);
        assert_eq!(a.moved, [id(0x42), id(0x43)]);
        let names: Vec<&str> = a.fields.iter().map(|(f, _)| *f).collect();
        assert_eq!(names, ["project", "parent", "position"]);
    }

    /// ADR 0078 §2: words that are no mark, and names that do not
    /// resolve, are free text; a new tag is made as on capture.
    #[test]
    fn free_text_is_warned_about() {
        let (_, tags) = known_world();
        let task = existing(0x40, "Fliesen", None, None);
        let a = applied(&task, &[], &tags, "morgen anrufen #gibtsnicht @neu");
        assert_eq!(
            a.draft.warnings,
            [
                Warning::UnknownProject {
                    name: "gibtsnicht".into()
                },
                Warning::FreeText {
                    text: "morgen anrufen #gibtsnicht".into()
                },
            ]
        );
        assert_eq!(a.task.title, "Fliesen");
        assert_eq!(a.new_tags.len(), 1);
        assert_eq!(a.tags, [(a.new_tags[0].id, 1)]);
    }

    /// ADR 0078 §2: a template's reminders only where the task has none
    /// with that anchor.
    #[test]
    fn template_reminders_only_for_new_anchors() {
        let habit = template("Habit", "p:heute 21:00");
        let mut next = 0xc0_u8;
        let mut new_id = || {
            next += 1;
            id(next)
        };
        let made = template_reminders(Some(&habit), id(0x40), saturday(), &mut new_id);
        assert_eq!(made.len(), 1);
        assert_eq!(missing_reminders(made.clone(), &[]).len(), 1);
        assert!(missing_reminders(made.clone(), &made).is_empty());
    }

    /// ADR 0091 §5: `habit:` makes the task a habit, in its short forms; a
    /// value that is none stays in the title with a warning.
    #[test]
    fn habit_short_forms() {
        let cases = [
            ("Dehnen habit:", HabitKind::Build, 1, HabitPer::Day, None),
            ("Dehnen habit:1/d", HabitKind::Build, 1, HabitPer::Day, None),
            ("Kraft habit:3/w", HabitKind::Build, 3, HabitPer::Week, None),
            (
                "Sauna habit:2/M",
                HabitKind::Build,
                2,
                HabitPer::Month,
                None,
            ),
            (
                "Wasser habit:8/d:Glas",
                HabitKind::Build,
                8,
                HabitPer::Day,
                Some("Glas"),
            ),
            (
                "Rauchfrei habit:avoid",
                HabitKind::Avoid,
                1,
                HabitPer::Day,
                None,
            ),
        ];
        for (text, kind, times, per, unit) in cases {
            let d = draft(text);
            let h = d.habit.clone().unwrap_or_else(|| panic!("{text}"));
            assert_eq!((h.kind, h.times, h.per), (kind, times, per), "{text}");
            assert_eq!(h.unit.as_deref(), unit, "{text}");
            assert!(d.marks.iter().any(|m| m.meaning == Meaning::Habit));
            assert!(d.warnings.is_empty(), "{text}");
            assert!(!d.title.contains("habit"), "{text}");
        }
        for bad in [
            "habit:0/d",
            "habit:3/y",
            "habit:3",
            "habit:8/d:",
            "habit:oft",
        ] {
            let d = draft(&format!("X {bad}"));
            assert!(d.habit.is_none(), "{bad}");
            assert_eq!(d.title, format!("X {bad}"));
            assert_eq!(d.warnings, vec![Warning::UnknownHabit { text: bad.into() }]);
        }
        // A template holds it; the line wins over it.
        assert!(check_template_text("#Haus @habit habit:1/d").is_ok());
        let habit = [template("Habit", "#Haus habit:1/d")];
        let mut d = draft("Kraft habit:3/w /habit");
        apply_template(&mut d, &habit, None, saturday(), &berlin());
        assert_eq!(d.habit.unwrap().times, 3);
        let mut d = draft("Dehnen /habit");
        apply_template(&mut d, &habit, None, saturday(), &berlin());
        assert_eq!(d.habit.unwrap(), Habit::default());
    }

    /// ADR 0091 §5: `habit:` in a line applied to a task makes it a habit.
    #[test]
    fn a_line_applied_makes_a_habit() {
        let (_, tags) = known_world();
        let task = existing(0x40, "Dehnen", Some(2), None);
        let a = applied(&task, std::slice::from_ref(&task), &tags, "habit:3/w");
        assert_eq!(a.task.habit.as_ref().unwrap().times, 3);
        let names: Vec<&str> = a.fields.iter().map(|(f, _)| *f).collect();
        assert_eq!(names, ["habit"]);
        let again = applied(&a.task, std::slice::from_ref(&a.task), &tags, "habit:3/w");
        assert!(again.fields.is_empty());
    }

    /// ADR 0073 §2: `every+:` is `fixed` with catch-up.
    #[test]
    fn every_plus_is_fixed_with_catch_up() {
        let d = draft("Dehnen every+:1d");
        let repeat = d.repeat.unwrap();
        assert_eq!(repeat.rule, "FREQ=DAILY");
        assert_eq!(repeat.mode, RepeatMode::Fixed);
        assert!(repeat.catch_up);
        assert_eq!(d.title, "Dehnen");
        assert!(!draft("Dehnen every:1d").repeat.unwrap().catch_up);
    }

    /// ADR 0073 §3: `/name` calls a template; one that is none goes back
    /// into the title with a warning; a second is text.
    #[test]
    fn a_slash_name_calls_a_template() {
        let habit = [template(
            "Habit",
            "#Haus @telefon p:heute 21:00 every+:1d // klein",
        )];
        let mut d = draft("Rauchfrei /habit");
        assert_eq!(d.template.as_deref(), Some("habit"));
        assert!(d.marks.iter().any(|m| m.meaning == Meaning::Template));
        let used = apply_template(&mut d, &habit, None, saturday(), &berlin());
        assert_eq!(used.map(|t| t.title.as_str()), Some("Habit"));
        assert_eq!(d.title, "Rauchfrei");
        assert_eq!(d.project.as_deref(), Some("Haus"));
        assert_eq!(d.tags, vec!["telefon".to_owned()]);
        // Relative dates count from the moment of capture: today 21:00.
        assert_eq!(
            d.planned,
            Some(at("2026-11-07T21:00:00+01:00[Europe/Berlin]"))
        );
        assert!(d.repeat.as_ref().unwrap().catch_up);
        assert_eq!(d.note.as_deref(), Some("klein"));

        let mut unknown = draft("Rauchfrei /gibtsnicht");
        assert!(apply_template(&mut unknown, &habit, None, saturday(), &berlin()).is_none());
        assert_eq!(unknown.title, "Rauchfrei /gibtsnicht");
        assert_eq!(
            unknown.warnings,
            vec![Warning::UnknownTemplate {
                name: "gibtsnicht".into()
            }]
        );
        assert!(!unknown.marks.iter().any(|m| m.meaning == Meaning::Template));

        let twice = draft("x /a /b");
        assert_eq!(twice.title, "x /b");
        assert!(matches!(
            twice.warnings[0],
            Warning::Twice {
                meaning: Meaning::Template,
                ..
            }
        ));
        // `//` is the note, `/` alone and a path are text.
        assert_eq!(draft("a / b").title, "a / b");
        assert_eq!(draft("prüfe /etc/hosts").template, None);
        // A button chooses without `/name`.
        let mut chosen = draft("Rauchfrei");
        assert!(
            apply_template(&mut chosen, &habit, Some(id(0x80)), saturday(), &berlin()).is_some()
        );
        assert_eq!(chosen.project.as_deref(), Some("Haus"));
    }

    /// ADR 0073 §4: the line wins field by field, tags are not mixed, and
    /// the scope fills in only what the template leaves open.
    #[test]
    fn the_line_beats_the_template_beats_the_scope() {
        let habit = [template("habit", "#Haus @telefon p:heute 21:00 ~15m")];
        let mut d = draft("Rauchfrei /habit #neu @termin p:morgen ~5m !");
        apply_template(&mut d, &habit, None, saturday(), &berlin());
        assert_eq!(d.project.as_deref(), Some("neu"));
        assert_eq!(d.tags, vec!["termin".to_owned()]);
        assert_eq!(d.planned, Some(When::Date(date(2026, 11, 8))));
        assert_eq!(d.estimate, Some(Period(5.minutes())));
        assert!(d.flagged);

        let (projects, tags) = known_world();
        let known = Known {
            projects: &projects,
            tags: &tags,
            tasks: &[],
        };
        let scope: Scope = serde_json::from_value(serde_json::json!({
            "id": id(0x70), "created": "2026-11-07T08:00:00Z",
            "modified": "2026-11-07T08:00:00Z", "title": "Arbeit", "mode": "only",
            "capture": {"project": id(1), "tags": [id(0x11)]}, "position": 0,
        }))
        .unwrap_or_else(|e| panic!("{e}"));
        let no_project = [template("t", "@telefon")];
        let mut d = draft("Bericht /t");
        apply_template(&mut d, &no_project, None, saturday(), &berlin());
        let mut new_id = || id(0xb0);
        let r = resolve_in_scope(d, known, None, Some(&scope), saturday(), &mut new_id);
        // The template's tag, not the scope's; the scope's project.
        assert_eq!(r.tags, vec![id(0x10)]);
        assert_eq!(r.task.project, Some(id(1)));
    }

    /// ADR 0120: a template's free text goes before the title, stays there
    /// when a name goes back into the title, and leaves an edited task's
    /// title alone.
    #[test]
    fn a_template_text_prefixes_the_title() {
        let epic = [template("epic", "[EPIC] @telefon")];
        let mut d = draft("Projekt aufwerten /epic");
        apply_template(&mut d, &epic, None, saturday(), &berlin());
        assert_eq!(d.title, "[EPIC] Projekt aufwerten");
        assert_eq!(d.tags, vec!["telefon".to_owned()]);

        let mut alone = draft("/epic");
        apply_template(&mut alone, &epic, None, saturday(), &berlin());
        assert_eq!(alone.title, "[EPIC]");

        let mut missed = draft("Projekt #gibtsnicht /epic");
        apply_template(&mut missed, &epic, None, saturday(), &berlin());
        missed.miss_project();
        assert_eq!(missed.title, "[EPIC] Projekt #gibtsnicht");

        let (projects, tags) = known_world();
        let task = existing(0x40, "Fliesen", Some(2), None);
        let mut line = draft("/epic");
        apply_template(&mut line, &epic, None, saturday(), &berlin());
        let mut new_id = || id(0xb0);
        let known = Known {
            projects: &projects,
            tags: &tags,
            tasks: std::slice::from_ref(&task),
        };
        let a = apply_to(&task, line, known, saturday(), &mut new_id);
        assert_eq!(a.task.title, "Fliesen");
        assert!(a.draft.warnings.is_empty(), "{:?}", a.draft.warnings);
    }

    /// ADR 0073 §1: a template's text holds marks only.
    #[test]
    fn a_template_text_holds_only_marks() {
        assert!(check_template_text("#Haus @habit p:heute 21:00 every+:1d // eine Notiz").is_ok());
        assert!(check_template_text("").is_ok());
        // ADR 0120: free text is the title's prefix.
        assert!(check_template_text("[EPIC] @epic").is_ok());
        assert!(check_template_text("#Haus /andere").is_err());
        assert!(check_template_text("p:irgendwann").is_err());
        assert!(template_name("habit-1_a"));
        assert!(!template_name("zwei worte"));
        assert!(!template_name(""));
        assert!(!template_name("a/b"));
        // ADR 0084 §1: a key is one letter or digit, of any case.
        for key in ["h", "H", "7", "ä"] {
            assert!(template_key(key), "{key}");
        }
        for key in ["", "hh", " ", "-", "/"] {
            assert!(!template_key(key), "{key:?}");
        }
    }

    /// ADR 0043 §6: the scope fills in what the line does not name.
    #[test]
    fn a_scope_gives_its_project_and_tags_when_the_line_names_none() {
        let (projects, tags) = known_world();
        let known = Known {
            projects: &projects,
            tags: &tags,
            tasks: &[],
        };
        let scope: Scope = serde_json::from_value(serde_json::json!({
            "id": id(0x70), "created": "2026-11-07T08:00:00Z",
            "modified": "2026-11-07T08:00:00Z", "title": "Arbeit", "mode": "only",
            "capture": {"project": id(2), "tags": [id(0x10), id(0x99)]}, "position": 0,
        }))
        .unwrap_or_else(|e| panic!("{e}"));
        let run = |text: &str, view: Option<Id>| {
            let mut next = 0xa0_u8;
            let mut new_id = || {
                next += 1;
                id(next)
            };
            resolve_in_scope(
                draft(text),
                known,
                view,
                Some(&scope),
                saturday(),
                &mut new_id,
            )
        };
        let plain = run("Bericht", None);
        assert_eq!(plain.task.project, Some(id(2)));
        // A tag that is gone is left out.
        assert_eq!(plain.tags, vec![id(0x10)]);
        let named = run("Bericht #neu @termin", None);
        assert_eq!(named.task.project, Some(id(1)));
        assert_eq!(named.tags, vec![id(0x11)]);
        // The view's project beats the scope's.
        assert_eq!(run("Bericht", Some(id(1))).task.project, Some(id(1)));
        // Without a scope, the inbox.
        let mut new_id = || id(0xb0);
        let none = resolve_in_scope(draft("Bericht"), known, None, None, saturday(), &mut new_id);
        assert_eq!((none.task.project, none.tags.len()), (None, 0));
    }

    #[test]
    fn projects_resolve_by_name_or_a_unique_prefix() {
        let table: &[(&str, Option<Id>, &str, usize)] = &[
            ("x #\"neues bad\"", Some(id(1)), "x", 0),
            ("x #neu", Some(id(1)), "x", 0),
            // Exact beats prefix: "Haus" is a name and starts "Haushalt".
            ("x #haus", Some(id(2)), "x", 0),
            ("x #haush", Some(id(3)), "x", 0),
            ("x #ha", None, "x #ha", 1),
            ("x #gibtsnicht", None, "x #gibtsnicht", 1),
            // A dropped project is not offered.
            ("x #alt", None, "x #alt", 1),
        ];
        for (text, project, title, warnings) in table {
            let r = resolved(text, None);
            assert_eq!(r.task.project, *project, "{text}");
            assert_eq!(r.task.title, *title, "{text}");
            assert_eq!(
                r.draft.warnings.len(),
                *warnings,
                "{text}: {:?}",
                r.draft.warnings
            );
        }
        let missed = resolved("Anruf #gibtsnicht", None);
        assert_eq!(
            missed.draft.warnings,
            [Warning::UnknownProject {
                name: "gibtsnicht".into()
            }]
        );
        assert!(
            missed
                .draft
                .marks
                .iter()
                .all(|m| m.meaning != Meaning::Project)
        );
    }

    #[test]
    fn the_fallback_project_holds_unless_a_project_is_typed() {
        assert_eq!(resolved("x", Some(id(2))).task.project, Some(id(2)));
        assert_eq!(resolved("x #neu", Some(id(2))).task.project, Some(id(1)));
        // A missed name goes to the inbox, not to the fallback (ADR 0011 §5).
        assert_eq!(resolved("x #nix", Some(id(2))).task.project, None);
        // A fallback that is no open project is ignored.
        assert_eq!(resolved("x", Some(id(4))).task.project, None);
    }

    #[test]
    fn tags_resolve_or_are_made() {
        let r = resolved("x @tel @ort/büro @Wartet", None);
        assert_eq!(r.tags, [id(0x10), id(0x13), id(0x15)]);
        assert!(r.new_tags.is_empty());
        assert_eq!(r.task.title, "x");

        let r = resolved("x @einkauf @ort/garten", None);
        let made: Vec<(&str, Option<Id>)> = r
            .new_tags
            .iter()
            .map(|t| (t.title.as_str(), t.parent))
            .collect();
        assert_eq!(made, [("einkauf", None), ("garten", Some(id(0x12)))]);
        assert_eq!(r.tags, [r.new_tags[0].id, r.new_tags[1].id]);
        assert!(r.draft.warnings.is_empty());
        // A new tag lands after its siblings.
        assert_eq!(r.new_tags[1].position, 1);

        // A path of two new steps makes both, parent first.
        let r = resolved("x @neu/kind", None);
        assert_eq!(r.new_tags.len(), 2);
        assert_eq!(r.new_tags[1].parent, Some(r.new_tags[0].id));
        assert_eq!(r.tags, [r.new_tags[1].id]);
    }

    /// ADR 0062 §2: a capture under a task goes last among its children,
    /// in its project; a project the line names wins.
    #[test]
    fn a_capture_under_a_task_becomes_its_last_child() {
        let task = |n: u8, parent: Option<u8>, position: i64| {
            let mut t = resolved("x", Some(id(1))).task;
            t.id = id(n);
            t.parent = parent.map(id);
            t.position = position;
            t
        };
        let parent = task(0x30, None, 0);
        let tasks = vec![
            parent.clone(),
            task(0x31, Some(0x30), 0),
            task(0x32, Some(0x30), 4),
            task(0x33, None, 9),
        ];

        let mut r = resolved("Fliesen bestellen d:2026-11-20", None);
        place_under(&mut r, &parent, &tasks);
        assert_eq!(
            (r.task.project, r.task.parent, r.task.position),
            (Some(id(1)), Some(id(0x30)), 5)
        );

        let mut r = resolved("Fliesen #haus", None);
        place_under(&mut r, &parent, &tasks);
        assert_eq!((r.task.project, r.task.parent), (Some(id(2)), None));

        // A project that does not resolve is no project: the parent holds.
        let mut r = resolved("Fliesen #gibtsnicht", None);
        place_under(&mut r, &parent, &tasks);
        assert_eq!(
            (r.task.project, r.task.parent, r.task.position),
            (Some(id(1)), Some(id(0x30)), 5)
        );

        // The first child of a task without children.
        let mut r = resolved("Silikon", None);
        place_under(&mut r, &tasks[3], &tasks);
        assert_eq!((r.task.parent, r.task.position), (Some(id(0x33)), 0));
    }

    #[test]
    fn each_typed_tag_name_knows_its_tag() {
        let r = resolved("x @tel @neu/kind @Tel", None);
        let named: Vec<(&str, Id)> = r.named.iter().map(|(n, id)| (n.as_str(), *id)).collect();
        assert_eq!(
            named,
            [
                ("tel", id(0x10)),
                ("neu/kind", r.new_tags[1].id),
                ("Tel", id(0x10))
            ]
        );
        assert_eq!(r.tags, [id(0x10), r.new_tags[1].id]);
    }

    fn at_path(path: &str) -> Result<(Id, Vec<Tag>), String> {
        let (_, tags) = known_world();
        let mut next = 0xa0_u8;
        let mut new_id = || {
            next += 1;
            id(next)
        };
        tag_at_path(path, &tags, saturday(), &mut new_id)
    }

    #[test]
    fn a_picker_finds_a_tag_by_its_whole_title_or_path() {
        assert_eq!(at_path("Telefon"), Ok((id(0x10), vec![])));
        assert_eq!(at_path("ort/Büro"), Ok((id(0x13), vec![])));
        // A single name is looked for at every level, as in capture.
        assert_eq!(at_path(" büro "), Ok((id(0x13), vec![])));
    }

    #[test]
    fn a_picker_makes_what_is_missing_and_no_prefix_matches() {
        // `tel` is a prefix of telefon only; a picker makes it anew.
        let (made, new) = at_path("tel").unwrap();
        assert_eq!(new.len(), 1);
        assert_eq!(
            (new[0].id, new[0].title.as_str(), new[0].parent),
            (made, "tel", None)
        );

        let (made, new) = at_path("Ort/Garten").unwrap();
        assert_eq!(new.len(), 1);
        assert_eq!((new[0].id, new[0].title.as_str()), (made, "Garten"));
        assert_eq!((new[0].parent, new[0].position), (Some(id(0x12)), 1));

        let (made, new) = at_path("neu / kind").unwrap();
        let steps: Vec<(&str, Option<Id>)> =
            new.iter().map(|t| (t.title.as_str(), t.parent)).collect();
        assert_eq!(steps, [("neu", None), ("kind", Some(new[0].id))]);
        assert_eq!(made, new[1].id);
    }

    #[test]
    fn a_picker_makes_nothing_under_a_workflow_or_from_nothing() {
        assert!(at_path("Status/Neu").is_err());
        assert!(at_path("").is_err());
        assert!(at_path(" / ").is_err());
        // A workflow and its states are set as a state, never picked.
        assert!(at_path("Status/Wartet").is_err());
        assert!(at_path("status").is_err());
    }

    #[test]
    fn a_tag_title_holds_no_slash() {
        assert!(check_tag_title("Büro").is_ok());
        assert!(check_tag_title("Ort/Büro").is_err());
        assert!(check_tag_title("/").is_err());
        // Every tag a path makes is a step of it, never the path.
        let r = resolved("x @neu/kind", None);
        assert!(r.new_tags.iter().all(|t| check_tag_title(&t.title).is_ok()));
    }

    #[test]
    fn a_tag_is_not_made_when_ambiguous_or_under_a_workflow() {
        let r = resolved("Anruf @te morgen", None);
        assert!(r.tags.is_empty() && r.new_tags.is_empty());
        assert_eq!(r.task.title, "Anruf @te morgen");
        assert_eq!(
            r.draft.warnings,
            [Warning::UnknownTag { name: "te".into() }]
        );
        assert!(r.draft.marks.iter().all(|m| m.meaning != Meaning::Tag));

        let r = resolved("x @Status/Neu @tel", None);
        assert!(r.new_tags.is_empty());
        assert_eq!(r.tags, [id(0x10)]);
        assert_eq!(r.task.title, "x @Status/Neu");
        assert_eq!(
            r.draft
                .marks
                .iter()
                .filter(|m| m.meaning == Meaning::Tag)
                .count(),
            1
        );
    }

    #[test]
    fn the_task_carries_the_rest_of_the_draft() {
        let r = resolved("Fliesen ! d:fr ~15m every:1w // im Baumarkt", None);
        assert_eq!(r.task.title, "Fliesen");
        assert!(r.task.flagged);
        assert_eq!(r.task.schedule.due, Some(When::Date(date(2026, 11, 13))));
        assert!(r.task.estimate.is_some() && r.task.schedule.repeat.is_some());
        assert_eq!(r.task.note.as_deref(), Some("im Baumarkt"));
        assert!(r.task.tags.is_empty());
    }

    #[test]
    fn a_priority_like_the_flag() {
        let first = draft("Steuer !a ! d:fr");
        assert_eq!(first.title, "Steuer");
        assert_eq!(first.priority, Some(Priority::A));
        assert!(first.flagged);
        let meanings: Vec<Meaning> = first.marks.iter().map(|mark| mark.meaning).collect();
        assert_eq!(meanings, [Meaning::Priority, Meaning::Flag, Meaning::Due]);
        assert_eq!(draft("x !C").priority, Some(Priority::C));

        // A second one stays in the title and says so.
        let twice = draft("x !B !A");
        assert_eq!(twice.priority, Some(Priority::B));
        assert_eq!(twice.title, "x !A");
        assert_eq!(
            twice.warnings,
            [Warning::Twice {
                meaning: Meaning::Priority,
                text: "!A".into()
            }]
        );
        // Anything else after `!` is text, without a warning.
        for text in ["x !D", "x !!", "x !AB"] {
            let other = draft(text);
            assert_eq!(other.priority, None, "{text}");
            assert_eq!(other.title, text, "{text}");
            assert!(other.warnings.is_empty() && !other.flagged, "{text}");
        }

        let r = resolved("Steuer !B", None);
        assert_eq!(r.task.priority, Some(Priority::B));
    }

    #[test]
    fn names_match_exactly_or_by_a_unique_prefix() {
        let names = ["Haus", "Hausboot", "Garten", "Neues Bad"];
        assert_eq!(match_name("haus", names), NameMatch::One(0));
        assert_eq!(match_name("hausb", names), NameMatch::One(1));
        assert_eq!(match_name("ha", names), NameMatch::Many);
        assert_eq!(match_name("neu", names), NameMatch::One(3));
        assert_eq!(match_name("keller", names), NameMatch::None);
    }

    fn spec(text: &str) -> Vec<ClockSpec> {
        draft(text).clocks.into_iter().map(|b| b.spec).collect()
    }

    fn length(text: &str) -> Period {
        parse_estimate(text).unwrap()
    }

    fn hm(h: i8, m: i8) -> Time {
        Time::new(h, m, 0, 0).unwrap()
    }

    /// ADR 0081 §2: the three forms of `z:`, out of the title.
    #[test]
    fn z_books_a_length_a_day_or_a_range() {
        let d = draft("Review Angebot z:1h30");
        assert_eq!(d.title, "Review Angebot");
        assert_eq!(d.clocks[0].spec, ClockSpec::Length(length("1h30")));
        assert_eq!(d.clocks[0].text, "z:1h30");
        assert_eq!(d.marks.last().unwrap().meaning, Meaning::Clock);
        // Saturday 7 November: yesterday is Friday, Monday the 2nd.
        assert_eq!(
            spec("Review z:gestern 2h"),
            [ClockSpec::OnDay(date(2026, 11, 6), length("2h"))]
        );
        assert_eq!(draft("Review z:gestern 2h").title, "Review");
        assert_eq!(
            spec("z:mo 1h"),
            [ClockSpec::OnDay(date(2026, 11, 2), length("1h"))]
        );
        assert_eq!(
            spec("z:sa 1h"),
            [ClockSpec::OnDay(date(2026, 11, 7), length("1h"))]
        );
        assert_eq!(
            spec("z:14-15:30"),
            [ClockSpec::Range(date(2026, 11, 7), hm(14, 0), hm(15, 30))]
        );
        assert_eq!(
            spec("z: gestern 14:00-15:30 Rest"),
            [ClockSpec::Range(date(2026, 11, 6), hm(14, 0), hm(15, 30))]
        );
        assert_eq!(
            spec("z:5.11. 45m"),
            [ClockSpec::OnDay(date(2026, 11, 5), length("45m"))]
        );
        // Several, each its own clock.
        assert_eq!(spec("A z:1h z:gestern 2h").len(), 2);
    }

    #[test]
    fn z_without_a_time_stays_in_the_title() {
        let d = draft("Review z:bald");
        assert!(d.clocks.is_empty());
        assert_eq!(d.title, "Review z:bald");
        assert_eq!(
            d.warnings,
            [Warning::UnknownClock {
                text: "z:bald".into()
            }]
        );
        // A day without a length or range is no time either.
        assert!(draft("z:gestern").clocks.is_empty());
    }

    #[test]
    fn x_completes_now_or_on_a_day() {
        let d = draft("Altpapier x:");
        assert_eq!((d.title.as_str(), d.completed), ("Altpapier", Some(None)));
        assert_eq!(d.marks[0].meaning, Meaning::Completed);
        assert_eq!(
            draft("A x:gestern").completed,
            Some(Some(date(2026, 11, 6)))
        );
        assert_eq!(
            draft("A x: gestern").completed,
            Some(Some(date(2026, 11, 6)))
        );
        assert_eq!(draft("A x: gestern").title, "A");
        let d = draft("A x:bald");
        assert_eq!(d.completed, None);
        assert_eq!(d.title, "A x:bald");
    }

    fn booked(spec: ClockSpec) -> BookedClock {
        BookedClock {
            spec,
            text: "z:…".into(),
        }
    }

    fn ids() -> impl FnMut() -> Id {
        let mut n = 0u8;
        move || {
            n += 1;
            format!("01936f5e-8a00-7c3e-9b1a-3f2d4c5b6e{n:02x}")
                .parse()
                .unwrap()
        }
    }

    fn clock_at(start: &str, end: &str) -> Clock {
        Clock {
            id: Id::new(),
            created: saturday(),
            modified: saturday(),
            owner: Id::new(),
            start: start.parse().unwrap(),
            end: Some(end.parse().unwrap()),
            note: None,
        }
    }

    fn ts(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    /// ADR 0081 §2: a day's length goes after that day's last clock, of any
    /// task, else from 09:00; several follow one another.
    #[test]
    fn a_days_length_follows_its_last_clock() {
        let owner = Id::new();
        let friday = date(2026, 11, 6);
        let (placed, warnings) = place_clocks(
            &[
                booked(ClockSpec::OnDay(friday, length("2h"))),
                booked(ClockSpec::OnDay(friday, length("30m"))),
            ],
            &[],
            owner,
            &berlin(),
            saturday(),
            &mut ids(),
        );
        assert!(warnings.is_empty());
        // 09:00 in Berlin is 08:00Z.
        let spans: Vec<_> = placed.iter().map(|c| (c.start, c.end.unwrap())).collect();
        assert_eq!(
            spans,
            [
                (ts("2026-11-06T08:00:00Z"), ts("2026-11-06T10:00:00Z")),
                (ts("2026-11-06T10:00:00Z"), ts("2026-11-06T10:30:00Z"))
            ]
        );
        assert!(placed.iter().all(|c| c.owner == owner));
        let existing = [
            clock_at("2026-11-06T13:00:00Z", "2026-11-06T15:00:00Z"),
            // Thursday's clock does not count for Friday.
            clock_at("2026-11-05T17:00:00Z", "2026-11-05T19:00:00Z"),
        ];
        let (placed, _) = place_clocks(
            &[booked(ClockSpec::OnDay(friday, length("1h")))],
            &existing,
            owner,
            &berlin(),
            saturday(),
            &mut ids(),
        );
        assert_eq!(placed[0].start, ts("2026-11-06T15:00:00Z"));
    }

    #[test]
    fn a_clock_in_the_future_is_not_booked() {
        let today = date(2026, 11, 7);
        let (placed, warnings) = place_clocks(
            &[
                booked(ClockSpec::Range(today, hm(14, 0), hm(15, 30))),
                booked(ClockSpec::Range(today, hm(8, 0), hm(9, 30))),
                booked(ClockSpec::Range(today, hm(9, 0), hm(8, 0))),
                booked(ClockSpec::Length(length("1h30"))),
            ],
            &[],
            Id::new(),
            &berlin(),
            saturday(),
            &mut ids(),
        );
        assert_eq!(
            warnings,
            [
                Warning::ClockInFuture {
                    text: "z:…".into()
                },
                Warning::UnknownClock {
                    text: "z:…".into()
                }
            ]
        );
        let spans: Vec<_> = placed.iter().map(|c| (c.start, c.end.unwrap())).collect();
        assert_eq!(
            spans,
            [
                (ts("2026-11-07T07:00:00Z"), ts("2026-11-07T08:30:00Z")),
                (ts("2026-11-07T07:30:00Z"), saturday())
            ]
        );
    }

    #[test]
    fn x_on_a_day_is_its_last_clocks_end() {
        let clocks = [clock_at("2026-11-06T13:00:00Z", "2026-11-06T15:00:00Z")];
        let zone = berlin();
        assert_eq!(
            completed_at(Some(date(2026, 11, 6)), &clocks, &zone, saturday()),
            ts("2026-11-06T15:00:00Z")
        );
        assert_eq!(
            completed_at(Some(date(2026, 11, 5)), &clocks, &zone, saturday()),
            ts("2026-11-05T08:00:00Z")
        );
        assert_eq!(completed_at(None, &clocks, &zone, saturday()), saturday());
    }
}
