// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The views of the app's bar beyond the first five (ADR 0102 §1, §4):
//! Heute, Markiert, Stuck, Review and the perspectives. Each computes
//! through samu-core as the server's routes do (ADR 0006); grouping and
//! sorting a perspective's hits is the list's own business.

use jiff::Timestamp;
use samu_api::{
    Change, Id, ObjectKind, PerspectiveFilter, PerspectiveGroup, PerspectiveSort, Task,
};
use samu_core::views::{PerspectiveTest, ProjectsFilter};
use serde_json::json;

use crate::arrange::{settled, wire_name};
use crate::search::{self, SearchChip};
use crate::{MobileError, Samu, Section, TaskRow, input, is_open, parse_id};

/// A project in Stuck or Review.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ProjectView {
    pub id: String,
    pub title: String,
    /// An emoji before the title (ADR 0059).
    pub icon: Option<String>,
    /// The folder with its whole path, `Privat/Wohnung`.
    pub folder: Option<String>,
    /// In Review: the date the review is due since, `2026-11-10`.
    pub review: Option<String>,
    /// In Review: the day it was last reviewed; none when never.
    pub last_reviewed: Option<String>,
    /// In Review: its open tasks, as the project's list shows them; empty
    /// in Stuck.
    pub rows: Vec<TaskRow>,
}

/// A perspective as the bar and its head show it (ADR 0090, ADR 0102 §4).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PerspectiveRow {
    pub id: String,
    pub title: String,
    /// One emoji; none shows the funnel.
    pub icon: Option<String>,
    /// Its chips, as the search field shows them.
    pub chips: Vec<SearchChip>,
    /// Its free words.
    pub text: String,
    /// `available`, `remaining` or `all`.
    pub filter: String,
    /// `project`, `folder`, `tag`, `state`, `priority`, `date` or `none`.
    pub group: String,
    /// `position`, `due`, `planned`, …
    pub sort: String,
}

impl Samu {
    /// The chosen scope's id, for the views of samu-core.
    fn chosen_id(&self) -> Result<Option<Id>, MobileError> {
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        Ok(Self::chosen(&cache, &objects)?.map(|s| s.id))
    }

    /// Stuck or Review from `samu_core::views::projects`, through the scope
    /// chosen here.
    fn project_views(&self, review: bool) -> Result<Vec<ProjectView>, MobileError> {
        let scope = self.chosen_id()?;
        let (found, names) = {
            let cache = self.cache()?;
            let (settings, _) = Self::settings(&cache)?;
            let (objects, _) = Self::view(&cache)?;
            let filter = ProjectsFilter {
                stuck: !review,
                review,
                all: false,
                scope,
                elsewhere: false,
            };
            let found = samu_core::views::projects(&objects, &settings, Timestamp::now(), &filter)
                .map_err(|e| input(e.to_string()))?;
            let names = crate::Names::new(&objects);
            let folders: Vec<Option<String>> = found
                .iter()
                .map(|p| p.project.folder.map(|f| names.folder_path(f)))
                .collect();
            let reviewed: Vec<Option<String>> = found
                .iter()
                .map(|p| {
                    p.project
                        .last_reviewed
                        .map(|at| at.to_zoned(settings.zone.clone()).date().to_string())
                })
                .collect();
            (found, folders.into_iter().zip(reviewed).collect::<Vec<_>>())
        };
        found
            .into_iter()
            .zip(names)
            .map(|(p, (folder, last_reviewed))| {
                let rows = if review {
                    self.project(p.project.id.to_string(), None)?
                        .into_iter()
                        .flat_map(|s| s.rows)
                        .collect()
                } else {
                    Vec::new()
                };
                Ok(ProjectView {
                    id: p.project.id.to_string(),
                    title: p.project.title,
                    icon: p.project.icon,
                    folder,
                    review: review.then(|| p.review.map(|d| d.to_string())).flatten(),
                    last_reviewed: review.then_some(last_reviewed).flatten(),
                    rows,
                })
            })
            .collect()
    }
}

#[uniffi::export]
impl Samu {
    /// Heute as in the web: open tasks planned for today or before
    /// (`today` of samu-core), available or not, by project. Unlike
    /// [`Samu::today`], which feeds the widget from Forecast.
    pub fn today_list(&self, tag: Option<String>) -> Result<Vec<Section>, MobileError> {
        self.listed(
            "today",
            tag.as_deref(),
            |t, s| is_open(t) && s.today,
            |_| true,
        )
    }

    /// Markiert: flagged, by project; open and available until another
    /// filter is chosen ([`Samu::set_show`]).
    pub fn flagged(&self, tag: Option<String>) -> Result<Vec<Section>, MobileError> {
        let show = self.shown("flagged")?.unwrap_or_default();
        self.listed(
            "flagged",
            tag.as_deref(),
            |t, s| t.flagged && crate::arrange::passes(show, t, s),
            |_| true,
        )
    }

    /// Stuck: available projects without a next action (ADR 0006), in
    /// project order, as `GET /available/projects?stuck`.
    pub fn stuck(&self) -> Result<Vec<ProjectView>, MobileError> {
        self.project_views(false)
    }

    /// Review: projects whose review is due (ADR 0088 §4), the longest due
    /// first, each with its open tasks.
    pub fn review(&self) -> Result<Vec<ProjectView>, MobileError> {
        self.project_views(true)
    }

    /// The perspectives, in their order (ADR 0090).
    pub fn perspectives(&self) -> Result<Vec<PerspectiveRow>, MobileError> {
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let snapshot = objects.snapshot();
        let mut list: Vec<_> = objects.perspectives.iter().collect();
        list.sort_by_key(|p| (p.position, p.id));
        Ok(list
            .into_iter()
            .map(|p| {
                let arrange = p.arrange.clone().unwrap_or_default();
                PerspectiveRow {
                    id: p.id.to_string(),
                    title: p.title.clone(),
                    icon: p.icon.clone(),
                    chips: search::rows(&p.chips, &snapshot, self.ui_lang()),
                    text: p.text.clone(),
                    filter: wire_name(serde_json::to_value(match p.filter {
                        PerspectiveFilter::Unknown => PerspectiveFilter::Available,
                        f => f,
                    })),
                    group: wire_name(serde_json::to_value(match arrange.group {
                        PerspectiveGroup::Unknown => PerspectiveGroup::Project,
                        g => g,
                    })),
                    sort: wire_name(serde_json::to_value(match arrange.sort {
                        PerspectiveSort::Unknown => PerspectiveSort::Position,
                        s => s,
                    })),
                }
            })
            .collect())
    }

    /// Saves a search of the phone as a perspective through the outbox,
    /// last in the order, and answers its id (ADR 0103 §5): `search` is
    /// the search field's text, its chips become the perspective's chips,
    /// the other words its text. The filter is `remaining`, so it shows
    /// what the search showed: open tasks, available or not; a chip asking
    /// for what is closed widens it as in the search (ADR 0090 §2). The
    /// name is refused when another perspective has it, ignoring case, as
    /// the server would refuse it.
    pub fn create_perspective(
        &self,
        title: String,
        icon: Option<String>,
        search: String,
    ) -> Result<String, MobileError> {
        let title = title.trim().to_owned();
        if title.is_empty() {
            return Err(input("a perspective needs a name"));
        }
        let icon = icon.map(|i| i.trim().to_owned()).filter(|i| !i.is_empty());
        if icon.as_ref().is_some_and(|i| i.len() > 32) {
            return Err(input("an icon is one emoji, at most 32 bytes"));
        }
        let id = Id::new();
        self.queue(|objects, stamp| {
            let lower = title.to_lowercase();
            if let Some(other) = objects
                .perspectives
                .iter()
                .find(|p| p.title.to_lowercase() == lower)
            {
                return Err(input(format!(
                    "there is a perspective {:?} already",
                    other.title
                )));
            }
            let (chips, free) = samu_core::query::split_search(&search, &objects.snapshot());
            let text = free.split_whitespace().collect::<Vec<_>>().join(" ");
            if chips.is_empty() && text.is_empty() {
                return Err(input("a perspective needs a chip or a word"));
            }
            let now = Timestamp::now().to_string();
            let mut value = json!({
                "id": id,
                "created": now,
                "modified": now,
                "title": title,
                "filter": "remaining",
                "view": "list",
                "position": objects
                    .perspectives
                    .iter()
                    .map(|p| p.position + 1)
                    .max()
                    .unwrap_or(0),
            });
            if !chips.is_empty() {
                value["chips"] = json!(chips);
            }
            if !text.is_empty() {
                value["text"] = json!(text);
            }
            if let Some(icon) = &icon {
                value["icon"] = json!(icon);
            }
            Ok(vec![Change::Create {
                object: id,
                kind: ObjectKind::Perspective,
                value,
                hlc: stamp(),
            }])
        })?;
        Ok(id.to_string())
    }

    /// What a perspective shows, as `GET /available?perspective=` decides
    /// it, through the scope chosen here and, if given, only with `tag` or
    /// below; as a list, never a board (ADR 0090 §9, ADR 0102 §4).
    pub fn perspective(
        &self,
        id: String,
        tag: Option<String>,
    ) -> Result<Vec<Section>, MobileError> {
        let id = parse_id(&id)?;
        // Read and let go of the lock: `picked` takes it again.
        let (settings, objects) = {
            let cache = self.cache()?;
            (Self::settings(&cache)?.0, Self::view(&cache)?.0)
        };
        let perspective = objects
            .perspectives
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| input(format!("no perspective {id}")))?;
        let snapshot = objects.snapshot();
        let test = PerspectiveTest::new(perspective, &snapshot, Timestamp::now(), &settings.zone);
        let arrange = perspective.arrange.clone().unwrap_or_default();
        let pick = |t: &Task, s: &samu_core::availability::TaskState| {
            (test.all || is_open(t)) && (!test.available || s.available) && test.task(t, s)
        };
        self.listed_as(&settled(arrange), tag.as_deref(), pick, |_| true)
    }
}
