//! The chips of the search language as a filter (ADR 0042 §2, ADR 0043 §2).
//!
//! The web UI's search evaluates the same language in the browser
//! (`web/lib/query.js`); this is the evaluation for a scope's filter, used by
//! the server and the phone. The tests below and the ones of the web search
//! hold the two to the same meaning: chips of one kind OR, kinds AND, a
//! negated chip must not hold. There is no free text here.
//!
//! Relative ranges (`heute`, `7t`, `woche` up to Sunday) are days in the
//! account's zone; an instant counts on its day in that zone.

use std::collections::{HashMap, HashSet};

use jiff::Timestamp;
use jiff::ToSpan;
use jiff::civil::{Date, Weekday};
use jiff::tz::TimeZone;

use samu_api::{Id, Priority, Project, ScopeChip, ScopeChipKind as Kind, Tag, Task, When};

use crate::availability::{DueState, Snapshot, TaskState};

mod lang;
mod offers;
pub use lang::Lang;
pub use offers::{Offer, OfferAction, OfferOptions, offers};

/// The words `ist:` takes; `geclockt` since the clocking (ADR 0046 §6),
/// `blocker` for an open task an open one waits for (09cf1e2e),
/// `blockiert` for an open task that waits for an open blocker, its own or
/// its project's (53a0d8e9).
pub const IS: &[&str] = &[
    "markiert",
    "verfügbar",
    "wartet",
    "offen",
    "erledigt",
    "verworfen",
    "gruppe",
    "notiz",
    "wiederholt",
    "geclockt",
    "blocker",
    "blockiert",
];

/// Why a chip cannot be part of a filter.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChipError {
    #[error("a chip of a kind this samu does not know")]
    UnknownKind,
    #[error("{0:?} is not a range")]
    Range(String),
    #[error("ist:{0} is not a word samu knows")]
    Is(String),
    #[error("~{0} is not an estimate")]
    Estimate(String),
    #[error("{0:?} is not an id")]
    Id(String),
    #[error("prop:{0} needs a property key, like prop:jira or prop:jira=ABC-1")]
    Property(String),
    #[error("prio:{0} is not a priority: a, b, c, mit or ohne")]
    Priority(String),
}

/// A day range, both ends inclusive, `None` open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Range {
    Days(Option<Date>, Option<Date>),
    Overdue,
    Any,
    None,
}

fn range_of(word: &str, today: Date) -> Option<Range> {
    let day = |n: i64| today.checked_add(n.days()).ok();
    match word {
        "heute" => return Some(Range::Days(Some(today), Some(today))),
        "morgen" => return Some(Range::Days(day(1), day(1))),
        "gestern" => return Some(Range::Days(day(-1), day(-1))),
        "woche" => {
            let to_sunday = (7 - i64::from(today.weekday().to_sunday_zero_offset())) % 7;
            return Some(Range::Days(Some(today), day(to_sunday)));
        }
        "überfällig" => return Some(Range::Overdue),
        "mit" => return Some(Range::Any),
        "ohne" => return Some(Range::None),
        _ => {}
    }
    if let Some(number) = word.strip_suffix('t') {
        let (back, digits) = match number.strip_prefix('-') {
            Some(rest) => (true, rest),
            None => (false, number),
        };
        if !digits.is_empty() && digits.len() <= 4 && digits.bytes().all(|b| b.is_ascii_digit()) {
            let n: i64 = digits.parse().ok()?;
            return Some(if back {
                Range::Days(day(-n), Some(today))
            } else {
                Range::Days(Some(today), day(n))
            });
        }
        return None;
    }
    let iso = |text: &str| -> Option<Date> {
        (text.len() == 10 && text.as_bytes()[4] == b'-')
            .then(|| text.parse().ok())
            .flatten()
    };
    if let Some(date) = iso(word) {
        return Some(Range::Days(Some(date), Some(date)));
    }
    let (from, to) = word.split_once("..")?;
    let from = if from.is_empty() {
        None
    } else {
        Some(iso(from)?)
    };
    let to = if to.is_empty() { None } else { Some(iso(to)?) };
    (from.is_some() || to.is_some()).then_some(Range::Days(from, to))
}

/// A test on minutes of an estimate; `None` when the word is none.
fn estimate_test(word: &str) -> Option<impl Fn(Option<i64>) -> bool + use<>> {
    #[derive(Clone, Copy)]
    enum Op {
        Lt,
        Le,
        Gt,
        Ge,
        Eq,
        With,
        Without,
    }
    let (op, limit) = match word {
        "mit" => (Op::With, 0),
        "ohne" => (Op::Without, 0),
        _ => {
            let (op, rest) = [
                ("<=", Op::Le),
                (">=", Op::Ge),
                ("<", Op::Lt),
                (">", Op::Gt),
                ("=", Op::Eq),
            ]
            .into_iter()
            .find_map(|(sign, op)| word.strip_prefix(sign).map(|rest| (op, rest)))
            .unwrap_or((Op::Eq, word));
            let (digits, unit) = rest.split_at(rest.len().checked_sub(1)?);
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let n: i64 = digits.parse().ok()?;
            let limit = match unit {
                "m" => n,
                "h" => n.checked_mul(60)?,
                _ => return None,
            };
            (op, limit)
        }
    };
    Some(move |minutes: Option<i64>| match (op, minutes) {
        (Op::With, m) => m.is_some(),
        (Op::Without, m) => m.is_none(),
        (_, None) => false,
        (Op::Lt, Some(m)) => m < limit,
        (Op::Le, Some(m)) => m <= limit,
        (Op::Gt, Some(m)) => m > limit,
        (Op::Ge, Some(m)) => m >= limit,
        (Op::Eq, Some(m)) => m == limit,
    })
}

/// A test on a priority (ADR 0056 §4); `None` when the word is none.
fn priority_test(word: &str) -> Option<impl Fn(Option<Priority>) -> bool + use<>> {
    let want = match word {
        "a" => Some(Some(Priority::A)),
        "b" => Some(Some(Priority::B)),
        "c" => Some(Some(Priority::C)),
        "mit" => None,
        "ohne" => Some(None),
        _ => return None,
    };
    Some(move |have: Option<Priority>| match want {
        // An unknown priority is still one.
        None => have.is_some(),
        Some(want) => have == want,
    })
}

/// Whether a chip can be part of a filter: a kind and a word samu knows, an
/// id where one belongs. Whether the id names something live is the
/// store's question.
///
/// # Errors
///
/// [`ChipError`] says what is wrong with it.
pub fn check(chip: &ScopeChip) -> Result<(), ChipError> {
    // Any day does to check the shape of a range.
    let today = Date::constant(2026, 1, 1);
    match chip.kind {
        Kind::Unknown => Err(ChipError::UnknownKind),
        Kind::Project if chip.value == "inbox" => Ok(()),
        Kind::Project | Kind::Tag | Kind::Folder | Kind::State => chip
            .value
            .parse::<Id>()
            .map(|_| ())
            .map_err(|_| ChipError::Id(chip.value.clone())),
        Kind::Is if IS.contains(&chip.value.as_str()) => Ok(()),
        Kind::Is => Err(ChipError::Is(chip.value.clone())),
        Kind::Property => property_chip(&chip.value)
            .map(|_| ())
            .ok_or_else(|| ChipError::Property(chip.value.clone())),
        Kind::Estimate => estimate_test(&chip.value)
            .map(|_| ())
            .ok_or_else(|| ChipError::Estimate(chip.value.clone())),
        Kind::Priority => priority_test(&chip.value)
            .map(|_| ())
            .ok_or_else(|| ChipError::Priority(chip.value.clone())),
        Kind::Due
        | Kind::Planned
        | Kind::Defer
        | Kind::Completed
        | Kind::Created
        | Kind::Modified => range_of(&chip.value, today)
            .map(|_| ())
            .ok_or_else(|| ChipError::Range(chip.value.clone())),
    }
}

/// A property chip's key and, after `=`, the value it asks for.
fn property_chip(value: &str) -> Option<(&str, Option<&str>)> {
    let (key, want) = match value.split_once('=') {
        Some((key, want)) => (key, Some(want)),
        None => (value, None),
    };
    crate::properties::is_key(key).then_some((key, want))
}

/// Why a text cannot become a filter.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("{0:?} is no chip; a filter has no free text")]
    Free(String),
    #[error("no {kind} {name:?}")]
    NotFound { kind: &'static str, name: String },
    #[error("{name:?} names more than one {kind}")]
    Ambiguous { kind: &'static str, name: String },
    #[error("{0}: {1}")]
    Chip(String, ChipError),
}

/// The date words before the colon and their kinds (ADR 0042 §2).
const DATE_WORDS: &[(&str, Kind)] = &[
    ("fällig", Kind::Due),
    ("geplant", Kind::Planned),
    ("start", Kind::Defer),
    ("erledigt", Kind::Completed),
    ("erstellt", Kind::Created),
    ("geändert", Kind::Modified),
];

/// A folder's or tag's path from the top, `/` between; stops at a cycle.
fn path_of(id: Id, of: &dyn Fn(Id) -> Option<(Option<Id>, String)>) -> String {
    let mut parts = Vec::new();
    let mut seen = HashSet::new();
    let mut at = Some(id);
    while let Some(here) = at {
        if !seen.insert(here) {
            break;
        }
        let Some((parent, title)) = of(here) else {
            break;
        };
        parts.push(title);
        at = parent;
    }
    parts.reverse();
    parts.join("/")
}

fn tag_path(snapshot: &Snapshot<'_>, id: Id) -> String {
    path_of(id, &|i| {
        snapshot
            .tags
            .iter()
            .find(|t| t.id == i)
            .map(|t| (t.parent, t.title.clone()))
    })
}

fn folder_path(snapshot: &Snapshot<'_>, id: Id) -> String {
    path_of(id, &|i| {
        snapshot
            .folders
            .iter()
            .find(|f| f.id == i)
            .map(|f| (f.parent, f.title.clone()))
    })
}

/// One of `candidates` by path, else by title alone, ignoring case.
fn find(
    kind: &'static str,
    name: &str,
    candidates: &[(Id, String, String)],
) -> Result<Id, ParseError> {
    let want = name.to_lowercase();
    for pick in [
        |c: &(Id, String, String)| c.1.to_lowercase(),
        |c: &(Id, String, String)| c.2.to_lowercase(),
    ] {
        let found: Vec<Id> = candidates
            .iter()
            .filter(|c| pick(c) == want)
            .map(|c| c.0)
            .collect();
        match found.as_slice() {
            [one] => return Ok(*one),
            [] => {}
            _ => {
                return Err(ParseError::Ambiguous {
                    kind,
                    name: name.to_owned(),
                });
            }
        }
    }
    Err(ParseError::NotFound {
        kind,
        name: name.to_owned(),
    })
}

/// The words of a text, split at white space; a name in double quotes
/// stays one word, its quotes dropped (`@"Ort mit Raum"`).
fn split_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
            }
            c => word.push(c),
        }
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}

/// A text in the search language as chips, names resolved to ids
/// (ADR 0043 §2): what the phone's settings take.
///
/// # Errors
///
/// [`ParseError`] for the first word that is no chip.
pub fn parse(text: &str, snapshot: &Snapshot<'_>) -> Result<Vec<ScopeChip>, ParseError> {
    let mut chips: Vec<ScopeChip> = Vec::new();
    for word in split_words(text) {
        let chip = parse_word(&word, snapshot)?;
        check(&chip).map_err(|e| ParseError::Chip(word.clone(), e))?;
        if !chips
            .iter()
            .any(|c| c.kind == chip.kind && c.value == chip.value)
        {
            chips.push(chip);
        }
    }
    Ok(chips)
}

/// A search as the web's field reads it (ADR 0042 §2): each word that is a
/// chip samu knows becomes one, every other word stays free text, in its
/// order. The phone's search takes this (a1f43aed).
#[must_use]
pub fn split_search(text: &str, snapshot: &Snapshot<'_>) -> (Vec<ScopeChip>, String) {
    let mut chips: Vec<ScopeChip> = Vec::new();
    let mut free: Vec<String> = Vec::new();
    for word in split_words(text) {
        match parse_word(&word, snapshot) {
            Ok(chip) if check(&chip).is_ok() => {
                if !chips
                    .iter()
                    .any(|c| c.kind == chip.kind && c.value == chip.value && c.not == chip.not)
                {
                    chips.push(chip);
                }
            }
            _ => free.push(word),
        }
    }
    (chips, free.join(" "))
}

fn parse_word(word: &str, snapshot: &Snapshot<'_>) -> Result<ScopeChip, ParseError> {
    let (not, text) = match word.strip_prefix('-') {
        Some(rest) if !rest.is_empty() => (true, rest),
        _ => (false, word),
    };
    let chip = |kind, value: String| ScopeChip { kind, value, not };
    if let Some(name) = text.strip_prefix('#') {
        if name.eq_ignore_ascii_case("inbox") {
            return Ok(chip(Kind::Project, "inbox".into()));
        }
        let all: Vec<_> = snapshot
            .projects
            .iter()
            .map(|p| (p.id, p.title.clone(), p.title.clone()))
            .collect();
        return Ok(chip(
            Kind::Project,
            find("project", name, &all)?.to_string(),
        ));
    }
    if let Some(name) = text.strip_prefix('@') {
        let all: Vec<_> = snapshot
            .tags
            .iter()
            .map(|t| (t.id, tag_path(snapshot, t.id), t.title.clone()))
            .collect();
        return Ok(chip(Kind::Tag, find("tag", name, &all)?.to_string()));
    }
    if let Some(value) = text.strip_prefix('~') {
        return Ok(chip(Kind::Estimate, lang::canonical_value(value)));
    }
    let Some((key, value)) = text.split_once(':') else {
        return Err(ParseError::Free(word.to_owned()));
    };
    // German or English, read as German (ADR 0105 §3).
    let Some(key) = lang::canonical_key(&key.to_lowercase()) else {
        return Err(ParseError::Free(word.to_owned()));
    };
    if let Some((_, kind)) = DATE_WORDS.iter().find(|(k, _)| *k == key) {
        return Ok(chip(*kind, lang::canonical_value(&value.to_lowercase())));
    }
    match key {
        "ist" => Ok(chip(Kind::Is, lang::canonical_value(&value.to_lowercase()))),
        "prio" => Ok(chip(
            Kind::Priority,
            if value == "-" {
                "ohne".to_owned()
            } else {
                lang::canonical_value(&value.to_lowercase())
            },
        )),
        "prop" => Ok(chip(
            Kind::Property,
            match value.split_once('=') {
                Some((key, want)) => format!("{}={want}", key.to_lowercase()),
                None => value.to_lowercase(),
            },
        )),
        "ordner" => {
            let all: Vec<_> = snapshot
                .folders
                .iter()
                .map(|f| (f.id, folder_path(snapshot, f.id), f.title.clone()))
                .collect();
            Ok(chip(Kind::Folder, find("folder", value, &all)?.to_string()))
        }
        "zustand" => {
            let exclusive: HashSet<Id> = snapshot
                .tags
                .iter()
                .filter(|t| t.exclusive)
                .map(|t| t.id)
                .collect();
            let all: Vec<_> = snapshot
                .tags
                .iter()
                .filter(|t| t.parent.is_some_and(|p| exclusive.contains(&p)))
                .map(|t| (t.id, t.title.clone(), t.title.clone()))
                .collect();
            Ok(chip(Kind::State, find("state", value, &all)?.to_string()))
        }
        _ => Err(ParseError::Free(word.to_owned())),
    }
}

/// Chips as the words that make them, names from the snapshot; `?` for
/// what is gone. In German, the form `samu` writes (ADR 0105 §3).
#[must_use]
pub fn words(chips: &[ScopeChip], snapshot: &Snapshot<'_>) -> String {
    words_in(chips, snapshot, Lang::De)
}

/// [`words`] in the language of the interface.
#[must_use]
pub fn words_in(chips: &[ScopeChip], snapshot: &Snapshot<'_>, lang: Lang) -> String {
    let key = |word: &str| lang::key_in(word, lang).to_owned();
    let value = |word: &str| lang::value_in(word, lang);
    let quote = |name: String| {
        if name.contains(' ') {
            format!("\"{name}\"")
        } else {
            name
        }
    };
    chips
        .iter()
        .map(|chip| {
            let id = chip.value.parse::<Id>().ok();
            let known =
                |exists: bool, name: String| if exists { quote(name) } else { "?".to_owned() };
            let body = match chip.kind {
                Kind::Project if chip.value == "inbox" => "#Inbox".to_owned(),
                Kind::Project => {
                    let p = id.and_then(|i| snapshot.projects.iter().find(|p| p.id == i));
                    format!(
                        "#{}",
                        known(p.is_some(), p.map(|p| p.title.clone()).unwrap_or_default())
                    )
                }
                Kind::Tag => {
                    let t = id.filter(|i| snapshot.tags.iter().any(|t| t.id == *i));
                    format!(
                        "@{}",
                        known(
                            t.is_some(),
                            t.map(|i| tag_path(snapshot, i)).unwrap_or_default()
                        )
                    )
                }
                Kind::Folder => {
                    let f = id.filter(|i| snapshot.folders.iter().any(|f| f.id == *i));
                    format!(
                        "{}:{}",
                        key("ordner"),
                        known(
                            f.is_some(),
                            f.map(|i| folder_path(snapshot, i)).unwrap_or_default()
                        )
                    )
                }
                Kind::State => {
                    let t = id.and_then(|i| snapshot.tags.iter().find(|t| t.id == i));
                    format!(
                        "{}:{}",
                        key("zustand"),
                        known(t.is_some(), t.map(|t| t.title.clone()).unwrap_or_default())
                    )
                }
                Kind::Is => format!("{}:{}", key("ist"), value(&chip.value)),
                Kind::Property => format!("prop:{}", quote(chip.value.clone())),
                Kind::Estimate => format!("~{}", value(&chip.value)),
                Kind::Priority => format!("prio:{}", value(&chip.value)),
                Kind::Unknown => format!("?:{}", chip.value),
                kind => {
                    let word = DATE_WORDS
                        .iter()
                        .find(|(_, k)| *k == kind)
                        .map_or("?", |(w, _)| w);
                    format!("{}:{}", key(word), value(&chip.value))
                }
            };
            format!("{}{body}", if chip.not { "-" } else { "" })
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// What a test looks at besides the task.
struct Facts<'a> {
    state: &'a TaskState,
    /// Its state in its project's workflow (ADR 0024).
    workflow_state: Option<Id>,
    /// An action group: it has children.
    group: bool,
}

type Test<'a> = Box<dyn Fn(&Task, &Facts<'_>) -> bool + 'a>;

/// A filter read against one snapshot and one day.
pub struct Filter<'a> {
    /// Per kind: the plain tests (one must hold) and the negated ones (none
    /// may hold).
    kinds: Vec<(Vec<Test<'a>>, Vec<Test<'a>>)>,
    projects: HashMap<Id, &'a Project>,
    tags: &'a [Tag],
    parents: HashSet<Id>,
}

impl std::fmt::Debug for Filter<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Filter")
            .field("kinds", &self.kinds.len())
            .finish_non_exhaustive()
    }
}

/// `root` and everything below it, by `parent`, stopping at a cycle.
fn subtree(root: Id, parents: impl Iterator<Item = (Id, Option<Id>)>) -> HashSet<Id> {
    let pairs: Vec<(Id, Option<Id>)> = parents.collect();
    let mut found = HashSet::from([root]);
    loop {
        let before = found.len();
        for (id, parent) in &pairs {
            if parent.is_some_and(|p| found.contains(&p)) {
                found.insert(*id);
            }
        }
        if found.len() == before {
            return found;
        }
    }
}

fn day_of_when(when: Option<&When>, zone: &TimeZone) -> Option<Date> {
    match when? {
        When::Date(date) => Some(*date),
        When::At(at) => Some(at.timestamp().to_zoned(zone.clone()).date()),
        When::Unknown => None,
    }
}

fn day_of_instant(at: Option<Timestamp>, zone: &TimeZone) -> Option<Date> {
    at.map(|at| at.to_zoned(zone.clone()).date())
}

fn minutes(task: &Task) -> Option<i64> {
    let span = task.estimate?.0;
    let total = span.total(jiff::Unit::Minute).ok().or_else(|| {
        span.total((jiff::Unit::Minute, jiff::civil::date(2026, 1, 1)))
            .ok()
    })?;
    // An estimate is whole minutes; a fraction is cut.
    #[allow(
        clippy::cast_possible_truncation,
        reason = "minutes of an estimate fit"
    )]
    Some(total as i64)
}

fn open(task: &Task) -> bool {
    task.completed_at.is_none() && task.dropped_at.is_none()
}

impl<'a> Filter<'a> {
    /// The filter of `chips` on the day `now` is in `zone`. A chip that
    /// [`check`] refuses meets every task (ADR 0043 §2), so a value from a
    /// newer samu hides nothing.
    #[must_use]
    pub fn new(
        chips: &[ScopeChip],
        snapshot: &Snapshot<'a>,
        now: Timestamp,
        zone: &TimeZone,
    ) -> Self {
        let today = now.to_zoned(zone.clone()).date();
        let mut kinds: Vec<(Kind, Vec<Test<'a>>, Vec<Test<'a>>)> = Vec::new();
        for chip in chips {
            let Some(test) = Self::test(chip, snapshot, today, zone) else {
                continue;
            };
            let slot = match kinds.iter_mut().position(|(k, _, _)| *k == chip.kind) {
                Some(i) => &mut kinds[i],
                None => {
                    kinds.push((chip.kind, Vec::new(), Vec::new()));
                    let last = kinds.len() - 1;
                    &mut kinds[last]
                }
            };
            if chip.not {
                slot.2.push(test);
            } else {
                slot.1.push(test);
            }
        }
        Self {
            kinds: kinds
                .into_iter()
                .map(|(_, any, none)| (any, none))
                .collect(),
            projects: snapshot.projects.iter().map(|p| (p.id, p)).collect(),
            tags: snapshot.tags,
            parents: snapshot.tasks.iter().filter_map(|t| t.parent).collect(),
        }
    }

    /// No chip that tests anything.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }

    fn test(
        chip: &ScopeChip,
        snapshot: &Snapshot<'a>,
        today: Date,
        zone: &TimeZone,
    ) -> Option<Test<'a>> {
        check(chip).ok()?;
        let zone = zone.clone();
        let value = chip.value.clone();
        Some(match chip.kind {
            Kind::Project if value == "inbox" => Box::new(|_, f| f.state.inbox),
            Kind::Project => {
                let id: Id = value.parse().ok()?;
                Box::new(move |t, f| !f.state.inbox && t.project == Some(id))
            }
            Kind::Folder => {
                let root: Id = value.parse().ok()?;
                let folders = subtree(root, snapshot.folders.iter().map(|f| (f.id, f.parent)));
                let projects: HashMap<Id, Option<Id>> =
                    snapshot.projects.iter().map(|p| (p.id, p.folder)).collect();
                Box::new(move |t, f| {
                    !f.state.inbox
                        && t.project
                            .and_then(|p| projects.get(&p).copied().flatten())
                            .is_some_and(|folder| folders.contains(&folder))
                })
            }
            Kind::Tag => {
                let root: Id = value.parse().ok()?;
                let tags = subtree(root, snapshot.tags.iter().map(|t| (t.id, t.parent)));
                Box::new(move |t, _| t.tags.iter().any(|g| tags.contains(&g.tag)))
            }
            Kind::State => {
                let id: Id = value.parse().ok()?;
                Box::new(move |_, f| f.workflow_state == Some(id))
            }
            Kind::Property => {
                let (key, want) = property_chip(&value)?;
                let key = key.to_owned();
                let want = want.map(str::to_lowercase);
                let inheritance =
                    crate::properties::Inheritance::new(snapshot.tasks, snapshot.projects);
                Box::new(move |t, _| match inheritance.value(t, &key) {
                    None => false,
                    Some(have) => want.as_ref().is_none_or(|w| have.to_lowercase() == *w),
                })
            }
            Kind::Estimate => {
                let fits = estimate_test(&value)?;
                Box::new(move |t, _| fits(minutes(t)))
            }
            Kind::Priority => {
                let fits = priority_test(&value)?;
                Box::new(move |t, _| fits(t.priority))
            }
            Kind::Is => {
                // A task with a clock on it, not deleted (ADR 0046 §6);
                // the blockers the same way, as a set taken once.
                let among: HashSet<Id> = if value == "geclockt" {
                    snapshot.clocks.iter().map(|c| c.owner).collect()
                } else if value == "blocker" {
                    snapshot
                        .tasks
                        .iter()
                        .filter(|t| crate::blockers::holds_up(snapshot.tasks, snapshot.links, t.id))
                        .map(|t| t.id)
                        .collect()
                } else {
                    HashSet::new()
                };
                Box::new(move |t, f| match value.as_str() {
                    "markiert" => t.flagged,
                    "verfügbar" => open(t) && f.state.available,
                    "wartet" => open(t) && !f.state.available,
                    "offen" => open(t),
                    "erledigt" => t.completed_at.is_some(),
                    "verworfen" => t.dropped_at.is_some(),
                    "gruppe" => f.group,
                    "notiz" => t.note.as_deref().is_some_and(|n| !n.trim().is_empty()),
                    "wiederholt" => t.schedule.repeat.is_some() || t.repeat_of.is_some(),
                    "geclockt" | "blocker" => among.contains(&t.id),
                    // The reason the availability found, not a second
                    // reading of the links (ADR 0006).
                    "blockiert" => {
                        open(t)
                            && f.state
                                .reasons
                                .iter()
                                .any(|r| matches!(r, crate::availability::Reason::BlockedBy { .. }))
                    }
                    _ => false,
                })
            }
            Kind::Due
            | Kind::Planned
            | Kind::Defer
            | Kind::Completed
            | Kind::Created
            | Kind::Modified => {
                let range = range_of(&value, today)?;
                let kind = chip.kind;
                Box::new(move |t, f| {
                    let day = match kind {
                        Kind::Due => day_of_when(t.schedule.due.as_ref(), &zone),
                        Kind::Planned => day_of_when(t.schedule.planned.as_ref(), &zone),
                        Kind::Defer => day_of_when(t.schedule.defer.as_ref(), &zone),
                        Kind::Completed => day_of_instant(t.completed_at, &zone),
                        Kind::Created => day_of_instant(Some(t.created), &zone),
                        _ => day_of_instant(Some(t.modified), &zone),
                    };
                    match (range, day) {
                        (Range::None, day) => day.is_none(),
                        (_, None) => false,
                        (Range::Any, Some(_)) => true,
                        (Range::Overdue, Some(day)) => {
                            if kind == Kind::Due {
                                f.state.due == DueState::Overdue
                            } else {
                                open(t) && day < today
                            }
                        }
                        (Range::Days(from, to), Some(day)) => {
                            from.is_none_or(|from| day >= from) && to.is_none_or(|to| day <= to)
                        }
                    }
                })
            }
            Kind::Unknown => return None,
        })
    }

    /// Whether the task meets the filter. An empty filter meets every task.
    #[must_use]
    pub fn matches(&self, task: &Task, state: &TaskState) -> bool {
        if self.kinds.is_empty() {
            return true;
        }
        let project = crate::availability::home(task, &self.projects)
            .and_then(|p| self.projects.get(&p).copied());
        let facts = Facts {
            state,
            workflow_state: crate::workflow::placement(&task.tags, project, self.tags).state,
            group: self.parents.contains(&task.id),
        };
        self.kinds.iter().all(|(any, none)| {
            (any.is_empty() || any.iter().any(|t| t(task, &facts)))
                && !none.iter().any(|t| t(task, &facts))
        })
    }
}

/// Whether every free word of `text` stands in the task's title, note or
/// id, ignoring case, as the search does (ADR 0042 §2, ADR 0090 §2): the
/// end of an id finds its task. No word meets every task.
#[must_use]
pub fn text_matches(text: &str, task: &Task) -> bool {
    let words: Vec<String> = text.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return true;
    }
    let hay = format!(
        "{}\n{}\n{}",
        task.title,
        task.note.as_deref().unwrap_or_default(),
        task.id
    )
    .to_lowercase();
    words.iter().all(|w| hay.contains(w.as_str()))
}

/// A chip that asks for what is closed (`ist:erledigt`, `ist:verworfen`,
/// `erledigt:`): the filter then acts as „all“ (ADR 0042 §4).
#[must_use]
pub fn asks_for_closed(chips: &[ScopeChip]) -> bool {
    chips.iter().any(|c| {
        !c.not
            && ((c.kind == Kind::Is && (c.value == "erledigt" || c.value == "verworfen"))
                || (c.kind == Kind::Completed && c.value != "ohne"))
    })
}

/// A chip that asks for what waits (`ist:wartet`, `start:`): „available“
/// then acts as „remaining“ (ADR 0042 §4). A blocked task never is
/// available, so `ist:blockiert` asks for what waits too (53a0d8e9).
#[must_use]
pub fn asks_for_waiting(chips: &[ScopeChip]) -> bool {
    chips.iter().any(|c| {
        !c.not
            && ((c.kind == Kind::Is && (c.value == "wartet" || c.value == "blockiert"))
                || (c.kind == Kind::Defer && c.value != "ohne"))
    })
}

/// `Weekday` of the wire as jiff's.
#[must_use]
pub fn weekday(day: samu_api::Weekday) -> Option<Weekday> {
    use samu_api::Weekday as W;
    Some(match day {
        W::Mo => Weekday::Monday,
        W::Tu => Weekday::Tuesday,
        W::We => Weekday::Wednesday,
        W::Th => Weekday::Thursday,
        W::Fr => Weekday::Friday,
        W::Sa => Weekday::Saturday,
        W::Su => Weekday::Sunday,
        W::Unknown => return None,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests may unwrap")]
mod tests {
    use super::*;
    use samu_api::{Period, Schedule, Tagging};

    fn id(n: u8) -> Id {
        format!("01936f5e-8a00-7c3e-9b1a-3f2d4c5b6e{n:02x}")
            .parse()
            .unwrap()
    }

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    fn task(n: u8) -> Task {
        serde_json::from_value(serde_json::json!({
            "id": id(n),
            "created": "2026-11-01T10:00:00Z",
            "modified": "2026-11-05T10:00:00Z",
            "title": format!("t{n}"),
            "flagged": false,
            "sequential": false,
            "complete_with_last_child": false,
            "position": 0,
        }))
        .unwrap()
    }

    fn state(available: bool, due: DueState) -> TaskState {
        TaskState {
            available,
            reasons: Vec::new(),
            due,
            today: false,
            inbox: false,
        }
    }

    fn chip(kind: Kind, value: &str, not: bool) -> ScopeChip {
        ScopeChip {
            kind,
            value: value.into(),
            not,
        }
    }

    fn berlin() -> TimeZone {
        TimeZone::get("Europe/Berlin").unwrap()
    }

    // Monday 09.11.2026, 12:00 in Berlin.
    fn now() -> Timestamp {
        at("2026-11-09T11:00:00Z")
    }

    fn tag(n: u8, parent: Option<u8>) -> Tag {
        serde_json::from_value(serde_json::json!({
            "id": id(n),
            "created": "2026-11-01T10:00:00Z",
            "modified": "2026-11-01T10:00:00Z",
            "title": format!("tag{n}"),
            "parent": parent.map(id),
            "status": "active",
            "exclusive": false,
            "position": 0,
        }))
        .unwrap()
    }

    fn filter<'a>(chips: &[ScopeChip], snapshot: &Snapshot<'a>) -> Filter<'a> {
        Filter::new(chips, snapshot, now(), &berlin())
    }

    #[test]
    fn ranges_count_days_in_the_zone() {
        let today = Date::constant(2026, 11, 9);
        assert_eq!(
            range_of("woche", today),
            Some(Range::Days(Some(today), Some(Date::constant(2026, 11, 15))))
        );
        // On a Sunday the week is that day only.
        let sunday = Date::constant(2026, 11, 15);
        assert_eq!(
            range_of("woche", sunday),
            Some(Range::Days(Some(sunday), Some(sunday)))
        );
        assert_eq!(
            range_of("7t", today),
            Some(Range::Days(Some(today), Some(Date::constant(2026, 11, 16))))
        );
        assert_eq!(
            range_of("-7t", today),
            Some(Range::Days(Some(Date::constant(2026, 11, 2)), Some(today)))
        );
        assert_eq!(
            range_of("..2026-11-20", today),
            Some(Range::Days(None, Some(Date::constant(2026, 11, 20))))
        );
        for bad in ["", "t", "12345t", "2026-13-01", "..", "bald"] {
            assert_eq!(range_of(bad, today), None, "{bad}");
        }
    }

    #[test]
    fn a_property_chip_sees_its_own_and_inherited() {
        let mut group = task(1);
        group.properties.insert("jira".into(), "HAUS-12".into());
        let mut child = task(2);
        child.parent = Some(group.id);
        let mut other = task(3);
        other.properties.insert("ticket".into(), "T".into());
        let tasks = [group, child, other];
        let snapshot = Snapshot {
            tasks: &tasks,
            ..Snapshot::default()
        };
        let s = state(true, DueState::None);
        let hits = |chips: &[ScopeChip]| -> Vec<u8> {
            let f = filter(chips, &snapshot);
            (1..=3)
                .filter(|n| f.matches(&tasks[usize::from(*n) - 1], &s))
                .collect()
        };
        assert_eq!(hits(&[chip(Kind::Property, "jira", false)]), [1, 2]);
        assert_eq!(hits(&[chip(Kind::Property, "jira=haus-12", false)]), [1, 2]);
        assert_eq!(
            hits(&[chip(Kind::Property, "jira=HAUS-13", false)]),
            Vec::<u8>::new()
        );
        assert_eq!(hits(&[chip(Kind::Property, "jira", true)]), [3]);
        assert_eq!(
            hits(&[
                chip(Kind::Property, "jira", false),
                chip(Kind::Property, "ticket", false)
            ]),
            [1, 2, 3]
        );
        assert_eq!(
            check(&chip(Kind::Property, "Jira", false)),
            Err(ChipError::Property("Jira".into()))
        );
        let parsed = parse("prop:Jira=HAUS-12 -prop:ticket", &snapshot).unwrap();
        assert_eq!(
            parsed,
            [
                chip(Kind::Property, "jira=HAUS-12", false),
                chip(Kind::Property, "ticket", true)
            ]
        );
        assert_eq!(words(&parsed, &snapshot), "prop:jira=HAUS-12 -prop:ticket");
    }

    #[test]
    fn a_search_splits_into_chips_and_free_text() {
        let tags = [tag(1, None)];
        let snapshot = Snapshot {
            tags: &tags,
            ..Snapshot::default()
        };
        let (chips, text) = split_search(
            "Milch erstellt:gestern @tag1 -ist:markiert kaufen",
            &snapshot,
        );
        assert_eq!(
            chips,
            [
                chip(Kind::Created, "gestern", false),
                chip(Kind::Tag, &id(1).to_string(), false),
                chip(Kind::Is, "markiert", true),
            ]
        );
        assert_eq!(text, "Milch kaufen");
        // What is no chip samu knows stays text: an unknown tag, a bad range.
        let (chips, text) = split_search("@fehlt erstellt:bald 10:30", &snapshot);
        assert!(chips.is_empty(), "{chips:?}");
        assert_eq!(text, "@fehlt erstellt:bald 10:30");
    }

    #[test]
    fn created_counts_the_day_in_the_zone() {
        // Captured at 23:30 UTC on Sunday 08.11.: in Berlin already Monday.
        let mut late = task(1);
        late.created = at("2026-11-08T23:30:00Z");
        let mut sunday = task(2);
        sunday.created = at("2026-11-08T10:00:00Z");
        let s = state(true, DueState::None);
        let snapshot = Snapshot::default();
        let f = filter(&[chip(Kind::Created, "gestern", false)], &snapshot);
        assert!(!f.matches(&late, &s), "00:30 in Berlin is the 9th");
        assert!(f.matches(&sunday, &s));
        let f = filter(&[chip(Kind::Created, "heute", false)], &snapshot);
        assert!(f.matches(&late, &s));
    }

    #[test]
    fn due_within_seven_days_and_flagged_both_hold() {
        let snapshot = Snapshot::default();
        let f = filter(
            &[
                chip(Kind::Due, "7t", false),
                chip(Kind::Is, "markiert", false),
            ],
            &snapshot,
        );
        let mut soon = task(1);
        soon.schedule = Schedule {
            due: Some(When::Date(Date::constant(2026, 11, 12))),
            ..Schedule::default()
        };
        soon.flagged = true;
        let open = state(true, DueState::None);
        assert!(f.matches(&soon, &open));
        let mut unflagged = soon.clone();
        unflagged.flagged = false;
        assert!(!f.matches(&unflagged, &open), "kinds AND");
        let mut later = soon.clone();
        later.schedule.due = Some(When::Date(Date::constant(2026, 11, 30)));
        assert!(!f.matches(&later, &open));
        assert!(!f.matches(&task(2), &open), "no due date, no match");
    }

    #[test]
    fn chips_of_one_kind_or_and_a_negated_one_must_not_hold() {
        let tags = [tag(30, None), tag(31, Some(30)), tag(32, None)];
        let snapshot = Snapshot {
            tags: &tags,
            ..Snapshot::default()
        };
        let tagged = |n: u8| {
            let mut t = task(n);
            t.tags = vec![Tagging {
                tag: id(n),
                position: 0,
            }];
            t
        };
        let s = state(true, DueState::None);
        // @30 or @32: a child tag of 30 counts.
        let f = filter(
            &[
                chip(Kind::Tag, &id(30).to_string(), false),
                chip(Kind::Tag, &id(32).to_string(), false),
            ],
            &snapshot,
        );
        assert!(f.matches(&tagged(31), &s));
        assert!(f.matches(&tagged(32), &s));
        assert!(!f.matches(&task(1), &s));
        // -@30: everything but 30 and below.
        let f = filter(&[chip(Kind::Tag, &id(30).to_string(), true)], &snapshot);
        assert!(!f.matches(&tagged(31), &s));
        assert!(f.matches(&tagged(32), &s));
        assert!(f.matches(&task(1), &s));
    }

    #[test]
    fn an_instant_counts_on_its_day_in_the_zone() {
        let snapshot = Snapshot::default();
        let f = filter(&[chip(Kind::Due, "heute", false)], &snapshot);
        let mut late = task(1);
        // 23:30 UTC on the 8th is 00:30 on the 9th in Berlin.
        late.schedule.due = Some(When::At(at("2026-11-08T23:30:00Z").to_zoned(TimeZone::UTC)));
        assert!(f.matches(&late, &state(true, DueState::Soon)));
        let mut early = task(2);
        early.schedule.due = Some(When::At(at("2026-11-08T22:30:00Z").to_zoned(TimeZone::UTC)));
        assert!(!f.matches(&early, &state(true, DueState::Overdue)));
    }

    #[test]
    fn overdue_and_waiting_ask_what_samu_core_computed() {
        let snapshot = Snapshot::default();
        let mut t = task(1);
        t.schedule.due = Some(When::Date(Date::constant(2026, 11, 1)));
        let overdue = filter(&[chip(Kind::Due, "überfällig", false)], &snapshot);
        assert!(overdue.matches(&t, &state(true, DueState::Overdue)));
        assert!(!overdue.matches(&t, &state(true, DueState::None)));
        let waiting = filter(&[chip(Kind::Is, "wartet", false)], &snapshot);
        assert!(waiting.matches(&t, &state(false, DueState::None)));
        assert!(!waiting.matches(&t, &state(true, DueState::None)));
        let mut done = t.clone();
        done.completed_at = Some(now());
        assert!(!waiting.matches(&done, &state(false, DueState::None)));
    }

    #[test]
    fn estimates_compare_in_minutes() {
        let snapshot = Snapshot::default();
        let mut quick = task(1);
        quick.estimate = Some(Period("PT10M".parse().unwrap()));
        let mut long = task(2);
        long.estimate = Some(Period("PT1H30M".parse().unwrap()));
        let s = state(true, DueState::None);
        let short = filter(&[chip(Kind::Estimate, "<15m", false)], &snapshot);
        assert!(short.matches(&quick, &s));
        assert!(!short.matches(&long, &s));
        assert!(!short.matches(&task(3), &s), "none is not short");
        let over = filter(&[chip(Kind::Estimate, ">1h", false)], &snapshot);
        assert!(over.matches(&long, &s));
        let without = filter(&[chip(Kind::Estimate, "ohne", false)], &snapshot);
        assert!(without.matches(&task(3), &s));
    }

    #[test]
    fn priorities_or_within_and_none_apart() {
        let snapshot = Snapshot::default();
        let mut a = task(1);
        a.priority = Some(Priority::A);
        let mut c = task(2);
        c.priority = Some(Priority::C);
        let none = task(3);
        let s = state(true, DueState::None);
        let a_or_c = filter(
            &[
                chip(Kind::Priority, "a", false),
                chip(Kind::Priority, "c", false),
            ],
            &snapshot,
        );
        assert!(a_or_c.matches(&a, &s) && a_or_c.matches(&c, &s));
        assert!(!a_or_c.matches(&none, &s));
        let with = filter(&[chip(Kind::Priority, "mit", false)], &snapshot);
        assert!(with.matches(&a, &s) && !with.matches(&none, &s));
        let not_a = filter(&[chip(Kind::Priority, "a", true)], &snapshot);
        assert!(!not_a.matches(&a, &s) && not_a.matches(&none, &s));
        assert!(check(&chip(Kind::Priority, "d", false)).is_err());
        let parsed = parse("prio:A -prio:-", &snapshot).unwrap();
        assert_eq!(
            parsed,
            vec![
                chip(Kind::Priority, "a", false),
                chip(Kind::Priority, "ohne", true)
            ]
        );
        assert_eq!(words(&parsed, &snapshot), "prio:a -prio:ohne");
    }

    #[test]
    fn a_chip_samu_cannot_read_hides_nothing() {
        let snapshot = Snapshot::default();
        let s = state(true, DueState::None);
        for bad in [
            chip(Kind::Unknown, "x", false),
            chip(Kind::Due, "bald", false),
        ] {
            assert!(check(&bad).is_err(), "{bad:?}");
            let f = filter(&[bad], &snapshot);
            assert!(f.is_empty());
            assert!(f.matches(&task(1), &s));
        }
        // The probe that must find something: a readable chip does filter.
        assert!(!filter(&[chip(Kind::Is, "markiert", false)], &snapshot).matches(&task(1), &s));
    }

    #[test]
    fn clocked_meets_a_task_with_a_clock() {
        let clocks = [samu_api::Clock {
            id: id(90),
            created: now(),
            modified: now(),
            owner: id(1),
            start: now(),
            end: None,
            note: None,
        }];
        let snapshot = Snapshot {
            clocks: &clocks,
            ..Snapshot::default()
        };
        let s = state(true, DueState::None);
        let clocked = filter(&[chip(Kind::Is, "geclockt", false)], &snapshot);
        assert!(!clocked.is_empty());
        assert!(clocked.matches(&task(1), &s));
        assert!(!clocked.matches(&task(2), &s));
        let not = filter(&[chip(Kind::Is, "geclockt", true)], &snapshot);
        assert!(!not.matches(&task(1), &s));
        assert!(not.matches(&task(2), &s));
        assert_eq!(
            parse("ist:geclockt", &snapshot).unwrap()[0],
            chip(Kind::Is, "geclockt", false)
        );
    }

    #[test]
    fn a_blocker_is_an_open_task_an_open_one_waits_for() {
        let mut waiting = task(1);
        waiting.title = "Dach decken".into();
        let tasks = [waiting, task(2), task(3)];
        let links = [crate::blockers::link(id(1), id(2), now())];
        let snapshot = Snapshot {
            tasks: &tasks,
            links: &links,
            ..Snapshot::default()
        };
        let s = state(true, DueState::None);
        let blocker = filter(&[chip(Kind::Is, "blocker", false)], &snapshot);
        assert!(blocker.matches(&tasks[1], &s));
        assert!(!blocker.matches(&tasks[0], &s), "the waiting one is none");
        assert!(!blocker.matches(&tasks[2], &s));
        let not = filter(&[chip(Kind::Is, "blocker", true)], &snapshot);
        assert!(!not.matches(&tasks[1], &s));
        assert!(not.matches(&tasks[2], &s));
        assert_eq!(
            parse("ist:blocker", &snapshot).unwrap()[0],
            chip(Kind::Is, "blocker", false)
        );
    }

    #[test]
    fn blocked_is_an_open_task_the_availability_finds_waiting_for_a_blocker() {
        let tasks = [task(1), task(2)];
        let snapshot = Snapshot {
            tasks: &tasks,
            ..Snapshot::default()
        };
        let mut blocked = state(false, DueState::None);
        blocked
            .reasons
            .push(crate::availability::Reason::BlockedBy { task: id(2) });
        let mut deferred = state(false, DueState::None);
        deferred
            .reasons
            .push(crate::availability::Reason::Deferred {
                until: When::Date("2026-12-01".parse().unwrap()),
            });
        let free = state(true, DueState::None);
        let chips = filter(&[chip(Kind::Is, "blockiert", false)], &snapshot);
        assert!(chips.matches(&tasks[0], &blocked));
        assert!(
            !chips.matches(&tasks[0], &deferred),
            "waiting, but not for a blocker"
        );
        assert!(!chips.matches(&tasks[1], &free));
        let mut done = task(1);
        done.completed_at = Some(now());
        assert!(!chips.matches(&done, &blocked), "only open tasks");
        let not = filter(&[chip(Kind::Is, "blockiert", true)], &snapshot);
        assert!(!not.matches(&tasks[0], &blocked));
        assert!(not.matches(&tasks[1], &free));
        assert_eq!(
            parse("ist:blockiert", &snapshot).unwrap()[0],
            chip(Kind::Is, "blockiert", false)
        );
        assert!(asks_for_waiting(&[chip(Kind::Is, "blockiert", false)]));
        assert!(!asks_for_waiting(&[chip(Kind::Is, "blockiert", true)]));
    }

    #[test]
    fn a_text_becomes_chips_and_back() {
        let tags = [tag(30, None), tag(31, Some(30))];
        let snapshot = Snapshot {
            tags: &tags,
            ..Snapshot::default()
        };
        let chips = parse(
            "fällig:7T -@tag30/tag31 ist:markiert #inbox ~<15m",
            &snapshot,
        )
        .unwrap();
        assert_eq!(chips.len(), 5);
        assert_eq!(chips[0], chip(Kind::Due, "7t", false));
        assert_eq!(chips[1], chip(Kind::Tag, &id(31).to_string(), true));
        assert_eq!(
            words(&chips, &snapshot),
            "fällig:7t -@tag30/tag31 ist:markiert #Inbox ~<15m"
        );
        // By title alone when that is unique.
        assert_eq!(
            parse("@tag31", &snapshot).unwrap()[0].value,
            id(31).to_string()
        );
        assert!(matches!(
            parse("Milch", &snapshot),
            Err(ParseError::Free(_))
        ));
        assert!(matches!(
            parse("@Nichts", &snapshot),
            Err(ParseError::NotFound { .. })
        ));
        assert!(matches!(
            parse("fällig:bald", &snapshot),
            Err(ParseError::Chip(..))
        ));
        assert!(parse("", &snapshot).unwrap().is_empty());
    }

    /// ADR 0105 §3: English reads as the German chips and writes back in
    /// either language.
    #[test]
    fn english_words_are_the_same_chips() {
        let tags = [tag(30, None), tag(31, Some(30))];
        let snapshot = Snapshot {
            tags: &tags,
            ..Snapshot::default()
        };
        let english = parse(
            "due:7d -@tag30/tag31 is:flagged done:-30d ~none prio:any defer:today",
            &snapshot,
        )
        .unwrap();
        let german = parse(
            "fällig:7t -@tag30/tag31 ist:markiert erledigt:-30t ~ohne prio:mit start:heute",
            &snapshot,
        )
        .unwrap();
        assert_eq!(english, german);
        assert_eq!(
            words_in(&english, &snapshot, Lang::En),
            "due:7d -@tag30/tag31 is:flagged done:-30d ~none prio:any defer:today"
        );
        assert_eq!(
            words(&english, &snapshot),
            "fällig:7t -@tag30/tag31 ist:markiert erledigt:-30t ~ohne prio:mit start:heute"
        );
        let (chips, free) = split_search("is:overdue Milch due:overdue", &snapshot);
        assert_eq!(chips, [chip(Kind::Due, "überfällig", false)]);
        assert_eq!(free, "is:overdue Milch");
    }

    #[test]
    fn the_inbox_a_folder_and_a_state() {
        let folders: Vec<samu_api::Folder> = [(1, None), (2, Some(1))]
            .into_iter()
            .map(|(n, parent): (u8, Option<u8>)| {
                serde_json::from_value(serde_json::json!({
                    "id": id(n), "created": "2026-11-01T10:00:00Z",
                    "modified": "2026-11-01T10:00:00Z", "title": "f",
                    "parent": parent.map(id), "status": "active", "position": 0,
                }))
                .unwrap()
            })
            .collect();
        let mut workflow = tag(40, None);
        workflow.exclusive = true;
        let tags = vec![workflow, tag(41, Some(40))];
        let projects: Vec<Project> = vec![
            serde_json::from_value(serde_json::json!({
                "id": id(10), "created": "2026-11-01T10:00:00Z",
                "modified": "2026-11-01T10:00:00Z", "title": "p", "folder": id(2),
                "workflow": id(40), "kind": "parallel", "status": "active",
                "flagged": false, "position": 0,
            }))
            .unwrap(),
        ];
        let snapshot = Snapshot {
            folders: &folders,
            projects: &projects,
            tags: &tags,
            ..Snapshot::default()
        };
        let mut placed = task(1);
        placed.project = Some(id(10));
        placed.tags = vec![Tagging {
            tag: id(41),
            position: 0,
        }];
        let inbox = TaskState {
            inbox: true,
            ..state(true, DueState::None)
        };
        let s = state(true, DueState::None);

        let f = filter(&[chip(Kind::Project, "inbox", false)], &snapshot);
        assert!(f.matches(&task(2), &inbox));
        assert!(!f.matches(&placed, &s));
        let f = filter(&[chip(Kind::Folder, &id(1).to_string(), false)], &snapshot);
        assert!(f.matches(&placed, &s), "a folder counts with what is below");
        assert!(!f.matches(&task(2), &inbox));
        let f = filter(&[chip(Kind::State, &id(41).to_string(), false)], &snapshot);
        assert!(f.matches(&placed, &s));
    }
}
