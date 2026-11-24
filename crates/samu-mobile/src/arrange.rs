// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Anordnen in the app (ADR 0125): group, sort and a second sort key as
//! the web does it (ADR 0064), for every list that has the button. A
//! perspective keeps its arrangement on the server; the fixed views and
//! projects keep theirs on this phone, by the keys of the web's
//! `samu.order`. What a list holds samu-core decides (ADR 0006); how it
//! stands is the list's own business (ADR 0090 §3).

use std::cmp::Ordering;
use std::collections::HashMap;

use jiff::Timestamp;
use jiff::civil::{Date, Weekday};
use samu_api::{
    Id, PerspectiveArrange, PerspectiveFilter, PerspectiveGroup, PerspectiveSort, Task,
};
use samu_core::availability::TaskState;
use samu_core::query::Lang;
use samu_core::workflow::placement;
use serde_json::Value;

use crate::{
    MobileError, Names, Samu, Section, TaskRow, input, is_open, parse_id, period_minutes, say,
};

/// A task, its home project (none in the inbox) and its row.
type Hit = (Task, Option<Id>, TaskRow);

/// How a list is arranged, by the names on the wire (ADR 0090 §1).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Arrange {
    /// `project`, `folder`, `tag`, `state`, `priority`, `date` or `none`.
    pub group: String,
    /// `position`, `due`, `planned`, `defer`, `title`, `created`,
    /// `modified`, `estimate` or `priority`.
    pub sort: String,
    pub desc: bool,
    /// The second key; none when there is none, or when the first is
    /// `position`, which is unique (ADR 0064 §2).
    pub then: Option<String>,
    pub then_desc: bool,
}

/// What the meta of this phone keeps a view's arrangement under.
const META: &str = "arrange:";

/// What the meta of this phone keeps a view's filter under.
const SHOW: &str = "show:";

/// Which tasks a list shows (ADR 0020 §1): `available`, `remaining` or
/// `all`, and whether that is what the list shows when nobody chose.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct Show {
    pub value: String,
    pub standard: bool,
}

/// A filter by its name on the wire; none for a name this samu does not
/// know.
fn show_named(name: &str) -> Option<PerspectiveFilter> {
    serde_json::from_value(Value::String(name.to_owned()))
        .ok()
        .filter(|f| *f != PerspectiveFilter::Unknown)
}

/// What a view shows until someone chooses: the inbox, a project and the
/// search everything open, the others what is available. None for a view
/// whose name is its filter (Verfügbar, Heute).
fn show_default(key: &str) -> Option<PerspectiveFilter> {
    match key {
        "available" | "today" => None,
        "inbox" | "search" => Some(PerspectiveFilter::Remaining),
        _ if key.starts_with("project:") => Some(PerspectiveFilter::Remaining),
        _ => Some(PerspectiveFilter::Available),
    }
}

/// Whether a task passes a filter, as the web's `passes`.
pub(crate) fn passes(show: PerspectiveFilter, task: &Task, state: &TaskState) -> bool {
    match show {
        PerspectiveFilter::All => true,
        PerspectiveFilter::Remaining => is_open(task),
        PerspectiveFilter::Available | PerspectiveFilter::Unknown => {
            is_open(task) && state.available
        }
    }
}

/// The name on the wire of an enum `serde_json::to_value` made.
pub(crate) fn wire_name(value: serde_json::Result<Value>) -> String {
    match value {
        Ok(Value::String(name)) => name,
        _ => "unknown".to_owned(),
    }
}

fn group_named(name: &str) -> Option<PerspectiveGroup> {
    serde_json::from_value(Value::String(name.to_owned())).ok()
}

fn sort_named(name: &str) -> Option<PerspectiveSort> {
    serde_json::from_value(Value::String(name.to_owned())).ok()
}

/// The arrangement as it counts: what a reader does not know falls back
/// to the default (ADR 0090 §1), and a second key that says nothing more
/// is none (ADR 0064 §2).
pub(crate) fn settled(mut arrange: PerspectiveArrange) -> PerspectiveArrange {
    if arrange.group == PerspectiveGroup::Unknown {
        arrange.group = PerspectiveGroup::Project;
    }
    if arrange.sort == PerspectiveSort::Unknown {
        arrange.sort = PerspectiveSort::Position;
    }
    if arrange.sort == PerspectiveSort::Position
        || arrange.then == Some(arrange.sort)
        || arrange.then == Some(PerspectiveSort::Unknown)
        || arrange.then == Some(PerspectiveSort::Position)
    {
        arrange.then = None;
    }
    if arrange.then.is_none() {
        arrange.then_desc = false;
    }
    arrange
}

impl From<PerspectiveArrange> for Arrange {
    fn from(arrange: PerspectiveArrange) -> Self {
        let a = settled(arrange);
        Arrange {
            group: wire_name(serde_json::to_value(a.group)),
            sort: wire_name(serde_json::to_value(a.sort)),
            desc: a.desc,
            then: a.then.map(|t| wire_name(serde_json::to_value(t))),
            then_desc: a.then_desc,
        }
    }
}

impl TryFrom<Arrange> for PerspectiveArrange {
    type Error = MobileError;

    fn try_from(a: Arrange) -> Result<Self, MobileError> {
        let group = group_named(&a.group)
            .filter(|g| *g != PerspectiveGroup::Unknown)
            .ok_or_else(|| input(format!("no grouping {:?}", a.group)))?;
        let sort = |name: &str| {
            sort_named(name)
                .filter(|s| *s != PerspectiveSort::Unknown)
                .ok_or_else(|| input(format!("no sort key {name:?}")))
        };
        Ok(settled(PerspectiveArrange {
            group,
            sort: sort(&a.sort)?,
            desc: a.desc,
            then: a.then.as_deref().map(sort).transpose()?,
            then_desc: a.then_desc,
        }))
    }
}

/// Projekt, Ordnung ↑: the list in its own order, as a tree.
fn plain(a: &PerspectiveArrange) -> bool {
    a.group == PerspectiveGroup::Project && a.sort == PerspectiveSort::Position && !a.desc
}

/// The perspective a key names, or none for a view of this phone; a key
/// it does not know is refused.
fn perspective_of(key: &str) -> Result<Option<Id>, MobileError> {
    if let Some(id) = key.strip_prefix("perspective:") {
        return parse_id(id).map(Some);
    }
    match key.split_once(':') {
        Some(("project" | "tag", id)) => parse_id(id).map(|_| None),
        None if matches!(key, "inbox" | "available" | "today" | "flagged" | "search") => Ok(None),
        _ => Err(input(format!("no list {key:?} to arrange"))),
    }
}

/// The value a task is sorted by, as the web's `sortValue`: a text or a
/// number; none goes last either way.
#[derive(PartialEq, PartialOrd)]
enum Key {
    Number(i64),
    Text(String),
}

fn sort_key(task: &Task, row: &TaskRow, sort: PerspectiveSort) -> Option<Key> {
    use PerspectiveSort as S;
    match sort {
        S::Due => row.due.clone().map(Key::Text),
        S::Planned => row.planned.clone().map(Key::Text),
        S::Defer => row.defer.clone().map(Key::Text),
        S::Title => Some(Key::Text(task.title.to_lowercase())),
        S::Created => Some(Key::Text(task.created.to_string())),
        S::Modified => Some(Key::Text(task.modified.to_string())),
        S::Estimate => task
            .estimate
            .as_ref()
            .and_then(period_minutes)
            .map(Key::Number),
        S::Priority => row.priority.clone().map(Key::Text),
        S::Position | S::Unknown => Some(Key::Number(task.position)),
    }
}

/// One level of the order; what lacks the value goes last, also when
/// descending.
fn by(a: Option<Key>, b: Option<Key>, desc: bool) -> Ordering {
    match (a, b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(a), Some(b)) => {
            let order = a.partial_cmp(&b).unwrap_or(Ordering::Equal);
            if desc { order.reverse() } else { order }
        }
    }
}

/// The first key, on a tie the second, then the order of the project and
/// the title (ADR 0064 §3).
fn order(a: &PerspectiveArrange) -> impl Fn(&Hit, &Hit) -> Ordering + '_ {
    move |x, y| {
        by(
            sort_key(&x.0, &x.2, a.sort),
            sort_key(&y.0, &y.2, a.sort),
            a.desc,
        )
        .then_with(|| match a.then {
            Some(then) => by(
                sort_key(&x.0, &x.2, then),
                sort_key(&y.0, &y.2, then),
                a.then_desc,
            ),
            None => Ordering::Equal,
        })
        .then_with(|| x.0.position.cmp(&y.0.position))
        .then_with(|| x.0.title.cmp(&y.0.title))
    }
}

/// `Fr, 20.11.`, in English `Fri 20 Nov`.
fn short_day(day: Date, lang: Lang) -> String {
    let name = match day.weekday() {
        Weekday::Monday => say(lang, "Mo", "Mon"),
        Weekday::Tuesday => say(lang, "Di", "Tue"),
        Weekday::Wednesday => say(lang, "Mi", "Wed"),
        Weekday::Thursday => say(lang, "Do", "Thu"),
        Weekday::Friday => say(lang, "Fr", "Fri"),
        Weekday::Saturday => say(lang, "Sa", "Sat"),
        Weekday::Sunday => say(lang, "So", "Sun"),
    };
    if lang == Lang::En {
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let month = MONTHS[usize::try_from(day.month() - 1).unwrap_or(0)];
        return format!("{name} {} {month}", day.day());
    }
    format!("{name}, {:02}.{:02}.", day.day(), day.month())
}

/// A heading of its own, not a project.
fn headed(title: String, rows: Vec<TaskRow>) -> Section {
    Section {
        project_id: None,
        title: Some(title),
        folder: None,
        rows,
    }
}

/// Groups of hits under a key each, ranked by `rank`; a hit may stand in
/// more than one (a task with two tags). Each group in the order.
fn flat_groups<K: Eq + std::hash::Hash + Clone, R: Ord>(
    hits: Vec<Hit>,
    keys_of: impl Fn(&Hit) -> Vec<K>,
    rank: impl Fn(&K) -> R,
    heading: impl Fn(&K) -> String,
    order: &impl Fn(&Hit, &Hit) -> Ordering,
) -> Vec<Section> {
    let mut map: HashMap<K, Vec<Hit>> = HashMap::new();
    for hit in hits {
        for key in keys_of(&hit) {
            map.entry(key).or_default().push(hit.clone());
        }
    }
    let mut groups: Vec<(K, Vec<Hit>)> = map.into_iter().collect();
    groups.sort_by_key(|(k, _)| rank(k));
    groups
        .into_iter()
        .map(|(key, mut list)| {
            list.sort_by(order);
            let rows = list
                .into_iter()
                .map(|(_, _, mut r)| {
                    // Flat: a child stands on its own.
                    r.depth = 0;
                    r
                })
                .collect();
            headed(heading(&key), rows)
        })
        .collect()
}

impl Samu {
    /// A list's arrangement: a perspective's from the perspective, a
    /// view's as this phone keeps it, else Projekt, Ordnung ↑.
    pub(crate) fn arrangement(&self, key: &str) -> Result<PerspectiveArrange, MobileError> {
        let cache = self.cache()?;
        let found = match perspective_of(key)? {
            Some(id) => {
                let (objects, _) = Self::view(&cache)?;
                objects
                    .perspectives
                    .iter()
                    .find(|p| p.id == id)
                    .ok_or_else(|| input(format!("no perspective {id}")))?
                    .arrange
                    .clone()
            }
            None => cache
                .meta(&format!("{META}{key}"))?
                .and_then(|text| serde_json::from_str(&text).ok()),
        };
        Ok(settled(found.unwrap_or_default()))
    }

    /// What the view `key` shows: its choice on this phone, else its
    /// default; none for a view without a choice.
    pub(crate) fn shown(&self, key: &str) -> Result<Option<PerspectiveFilter>, MobileError> {
        let Some(default) = show_default(key) else {
            return Ok(None);
        };
        let chosen = self.cache()?.meta(&format!("{SHOW}{key}"))?;
        Ok(Some(
            chosen.as_deref().and_then(show_named).unwrap_or(default),
        ))
    }

    /// The sections of a list arranged as `key` says: `pick` takes the
    /// tasks, `home` the projects whose tasks count (none is the inbox),
    /// `show` the children of a group among them (ADR 0021 §6).
    pub(crate) fn listed(
        &self,
        key: &str,
        tag: Option<&str>,
        pick: impl Fn(&Task, &TaskState) -> bool,
        home: impl Fn(Option<Id>) -> bool,
        show: PerspectiveFilter,
    ) -> Result<Vec<Section>, MobileError> {
        let arrange = self.arrangement(key)?;
        self.listed_as(&arrange, tag, pick, home, show)
    }

    /// [`Self::listed`] with the arrangement given.
    pub(crate) fn listed_as(
        &self,
        arrange: &PerspectiveArrange,
        tag: Option<&str>,
        pick: impl Fn(&Task, &TaskState) -> bool,
        home: impl Fn(Option<Id>) -> bool,
        show: PerspectiveFilter,
    ) -> Result<Vec<Section>, MobileError> {
        if plain(arrange) {
            // The project's own order, as a tree.
            return Ok(self
                .rows(tag, pick, show)?
                .into_iter()
                .filter(|s| home(s.project_id.as_deref().and_then(|p| p.parse().ok())))
                .collect());
        }
        let mut hits = self.picked(tag, pick)?;
        hits.retain(|h| home(h.1));
        self.arranged(hits, arrange.group, &order(arrange))
    }

    /// Sections of hits in groups as the web draws them
    /// (`web/lib/list.js`, `groups`).
    fn arranged(
        &self,
        mut hits: Vec<Hit>,
        group: PerspectiveGroup,
        order: &impl Fn(&Hit, &Hit) -> Ordering,
    ) -> Result<Vec<Section>, MobileError> {
        let lang = self.ui_lang();
        let (settings, objects) = {
            let cache = self.cache()?;
            (Self::settings(&cache)?.0, Self::view(&cache)?.0)
        };
        let names = Names::new(&objects);
        let project_of =
            |id: Option<Id>| id.and_then(|p| objects.projects.iter().find(|q| q.id == p));
        let last = |known: bool| if known { 0 } else { 1 };
        Ok(match group {
            PerspectiveGroup::None => {
                hits.sort_by(order);
                if hits.is_empty() {
                    Vec::new()
                } else {
                    vec![Section {
                        project_id: None,
                        title: None,
                        folder: None,
                        rows: hits
                            .into_iter()
                            .map(|(_, _, mut r)| {
                                r.depth = 0;
                                r
                            })
                            .collect(),
                    }]
                }
            }
            // The inbox first, then the folders by path, „Ohne Ordner“ last.
            PerspectiveGroup::Folder => flat_groups(
                hits,
                |h| match h.1 {
                    None => vec![Some(None)],
                    Some(_) => vec![project_of(h.1).and_then(|p| p.folder).map(Some)],
                },
                |k| match k {
                    Some(None) => (0, String::new()),
                    Some(Some(f)) => (1, names.folder_path(*f).to_lowercase()),
                    None => (2, String::new()),
                },
                |k| match k {
                    Some(None) => "Inbox".to_owned(),
                    Some(Some(f)) => names.folder_path(*f),
                    None => say(lang, "Ohne Ordner", "No folder").to_owned(),
                },
                order,
            ),
            // In each of its tags, „Ohne Tag“ last.
            PerspectiveGroup::Tag => flat_groups(
                hits,
                |h| {
                    if h.0.tags.is_empty() {
                        vec![None]
                    } else {
                        h.0.tags.iter().map(|t| Some(t.tag)).collect()
                    }
                },
                |k| {
                    (
                        last(k.is_some()),
                        k.map(|t| names.tag_path(t).to_lowercase()),
                    )
                },
                |k| match k {
                    Some(t) => names.tag_path(*t),
                    None => say(lang, "Ohne Tag", "No tag").to_owned(),
                },
                order,
            ),
            // In the order of the workflow, „Ohne Zustand“ last.
            PerspectiveGroup::State => flat_groups(
                hits,
                |h| vec![placement(&h.0.tags, project_of(h.1), &objects.tags).state],
                |k| {
                    let tag = k.and_then(|s| objects.tags.iter().find(|t| t.id == s));
                    (
                        last(k.is_some()),
                        tag.map_or(0, |t| t.position),
                        tag.map(|t| t.title.to_lowercase()),
                    )
                },
                |k| match k {
                    Some(s) => objects
                        .tags
                        .iter()
                        .find(|t| t.id == *s)
                        .map_or_else(|| "?".to_owned(), |t| t.title.clone()),
                    None => say(lang, "Ohne Zustand", "No state").to_owned(),
                },
                order,
            ),
            // A, B, C, then none (ADR 0064 §4).
            PerspectiveGroup::Priority => flat_groups(
                hits,
                |h| vec![h.2.priority.clone()],
                |k| (last(k.is_some()), k.clone()),
                |k| match k {
                    Some(p) => format!("{} {p}", say(lang, "Priorität", "Priority")),
                    None => say(lang, "Ohne Priorität", "No priority").to_owned(),
                },
                order,
            ),
            // Overdue, then each day by due, else planned, then what has
            // no date.
            PerspectiveGroup::Date => {
                let today = Timestamp::now().to_zoned(settings.zone).date();
                let day_of = |(task, _, row): &Hit| -> Option<Date> {
                    row.due
                        .as_deref()
                        .or(row.planned.as_deref())
                        .and_then(|d| d.get(..10))
                        .and_then(|d| d.parse::<Date>().ok())
                        .map(|d| {
                            if d < today && is_open(task) {
                                Date::MIN
                            } else {
                                d
                            }
                        })
                };
                flat_groups(
                    hits,
                    |h| vec![day_of(h)],
                    |k| (last(k.is_some()), *k),
                    |k| match k {
                        None => say(lang, "Ohne Datum", "No date").to_owned(),
                        Some(d) if *d == Date::MIN => say(lang, "Überfällig", "Overdue").to_owned(),
                        Some(d) if *d == today => say(lang, "Heute", "Today").to_owned(),
                        Some(d) => short_day(*d, lang),
                    },
                    order,
                )
            }
            // By project in its order; `hits` already stand so.
            PerspectiveGroup::Project | PerspectiveGroup::Unknown => {
                let mut sections: Vec<(Option<Id>, Vec<Hit>)> = Vec::new();
                for hit in hits {
                    match sections.last_mut() {
                        Some((home, list)) if *home == hit.1 => list.push(hit),
                        _ => sections.push((hit.1, vec![hit])),
                    }
                }
                sections
                    .into_iter()
                    .map(|(home, mut list)| {
                        list.sort_by(order);
                        let first = &list[0].2;
                        Section {
                            project_id: home.map(|h| h.to_string()),
                            title: home.and(first.project.clone()),
                            // `Projekt · Ordner/Unterordner` without the project.
                            folder: home
                                .and(first.place.as_deref().zip(first.project.as_deref()))
                                .and_then(|(place, title)| {
                                    place.strip_prefix(title)?.strip_prefix(" · ")
                                })
                                .map(str::to_owned),
                            rows: list
                                .into_iter()
                                .map(|(_, _, mut r)| {
                                    r.depth = 0;
                                    r
                                })
                                .collect(),
                        }
                    })
                    .collect()
            }
        })
    }
}

#[uniffi::export]
impl Samu {
    /// How a list is arranged (ADR 0125 §4). `key` is `inbox`,
    /// `available`, `today`, `flagged`, `search`, `project:<id>`,
    /// `tag:<id>` or `perspective:<id>`.
    pub fn arrange(&self, key: String) -> Result<Arrange, MobileError> {
        self.arrangement(&key).map(Arrange::from)
    }

    /// Arranges a list: a perspective through the outbox, for every
    /// device (ADR 0090 §6); a view on this phone only. A value it does
    /// not know is refused.
    pub fn set_arrange(&self, key: String, arrange: Arrange) -> Result<(), MobileError> {
        let wire = PerspectiveArrange::try_from(arrange)?;
        match perspective_of(&key)? {
            Some(id) => self.queue(|objects, stamp| {
                if !objects.perspectives.iter().any(|p| p.id == id) {
                    return Err(input(format!("no perspective {id}")));
                }
                let value = serde_json::to_value(&wire).map_err(|e| input(e.to_string()))?;
                Ok(vec![crate::edit::set(id, "arrange", value, stamp())])
            }),
            None => {
                let cache = self.cache()?;
                let text = if wire == PerspectiveArrange::default() {
                    None
                } else {
                    Some(serde_json::to_string(&wire).map_err(|e| input(e.to_string()))?)
                };
                cache.set_meta(&format!("{META}{key}"), text.as_deref())?;
                Ok(())
            }
        }
    }

    /// The tasks with a tag of the bar, or one below it (ADR 0102 §1),
    /// arranged and filtered as `tag:<id>`: the available ones until
    /// another filter is chosen.
    pub fn tagged(&self, id: String) -> Result<Vec<Section>, MobileError> {
        let key = format!("tag:{}", parse_id(&id)?);
        let show = self.shown(&key)?.unwrap_or_default();
        self.listed(&key, Some(&id), |t, s| passes(show, t, s), |_| true, show)
    }

    /// Which tasks a list shows (ADR 0020 §1), by the keys of
    /// [`Samu::arrange`]: a perspective's filter from the perspective, a
    /// view's as this phone keeps it. None for a view without a choice,
    /// `available` and `today`.
    pub fn show(&self, key: String) -> Result<Option<Show>, MobileError> {
        let name = |f: PerspectiveFilter| wire_name(serde_json::to_value(f));
        if let Some(id) = perspective_of(&key)? {
            let cache = self.cache()?;
            let (objects, _) = Self::view(&cache)?;
            let filter = objects
                .perspectives
                .iter()
                .find(|p| p.id == id)
                .ok_or_else(|| input(format!("no perspective {id}")))?
                .filter;
            let filter = match filter {
                PerspectiveFilter::Unknown => PerspectiveFilter::Available,
                f => f,
            };
            // Its filter is part of what the perspective is.
            return Ok(Some(Show {
                value: name(filter),
                standard: true,
            }));
        }
        Ok(self.shown(&key)?.map(|show| Show {
            value: name(show),
            standard: Some(show) == show_default(&key),
        }))
    }

    /// Chooses what a list shows: a perspective through the outbox, for
    /// every device; a view on this phone only. A value it does not
    /// know, or a view without a choice, is refused.
    pub fn set_show(&self, key: String, show: String) -> Result<(), MobileError> {
        let filter = show_named(&show).ok_or_else(|| input(format!("no filter {show:?}")))?;
        match perspective_of(&key)? {
            Some(id) => self.queue(|objects, stamp| {
                if !objects.perspectives.iter().any(|p| p.id == id) {
                    return Err(input(format!("no perspective {id}")));
                }
                Ok(vec![crate::edit::set(
                    id,
                    "filter",
                    Value::String(show.clone()),
                    stamp(),
                )])
            }),
            None => {
                let default = show_default(&key)
                    .ok_or_else(|| input(format!("the list {key:?} has no filter to choose")))?;
                let text = (filter != default).then_some(show.as_str());
                self.cache()?.set_meta(&format!("{SHOW}{key}"), text)?;
                Ok(())
            }
        }
    }
}
