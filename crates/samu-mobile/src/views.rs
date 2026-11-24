// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The views of the app's bar beyond the first five (ADR 0102 §1, §4):
//! Heute, Markiert, Stuck, Review and the perspectives. Each computes
//! through samu-core as the server's routes do (ADR 0006); grouping and
//! sorting a perspective's hits is the list's own business.

use std::collections::HashSet;

use jiff::Timestamp;
use samu_api::{
    BlockSection, Change, Id, ObjectKind, PerspectiveArrange, PerspectiveFilter, PerspectiveGroup,
    PerspectiveSort, Task,
};
use samu_core::views::{PerspectiveTest, ProjectsFilter};
use serde_json::json;

use crate::arrange::{settled, wire_name};
use crate::bar::BarEntry;
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
    /// A block perspective (ADR 0116), shown block by block (ADR 0147).
    pub blocks: bool,
}

/// One block of a block perspective as the app shows it (ADR 0147 §1):
/// its heading and, below, its tasks in its own arrangement.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ShownBlock {
    /// The block's own heading, else the name of its perspective or view,
    /// or the words of its search (ADR 0143 §2).
    pub title: String,
    pub icon: Option<String>,
    /// It points at nothing this phone can show (ADR 0116 §2).
    pub missing: bool,
    /// What its heading opens (ADR 0147 §3): the perspective or view it
    /// refers to; none for a search of its own, a note or a missing block.
    pub opens: Option<BarEntry>,
    /// A note block (ADR 0146): the project or task whose note it shows;
    /// its heading opens that one.
    pub object: Option<String>,
    /// A note block's note as written, empty when there is none; none for
    /// a block of tasks.
    pub note: Option<String>,
    /// Its tasks, each once, after the tag chosen here.
    pub count: u32,
    pub sections: Vec<Section>,
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
            PerspectiveFilter::Remaining,
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
            show,
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
                    blocks: !p.blocks.is_empty(),
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
    /// below; as a list, never a board (ADR 0090 §9, ADR 0102 §4). A block
    /// perspective shows the union of its blocks (ADR 0116 §4).
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
        let arrange = perspective.arrange.clone().unwrap_or_default();
        if !perspective.blocks.is_empty() {
            // A block perspective: every block's tasks once, as
            // `/available?perspective=` answers it (ADR 0116 §4). The
            // scope chosen here applies in `listed_as`, as for the rest.
            let hits: HashSet<Id> = samu_core::blocks::union(
                samu_core::views::blocks(&objects, &settings, Timestamp::now(), None, id)
                    .map_err(|e| input(e.to_string()))?,
            )
            .into_iter()
            .map(|s| s.task.id)
            .collect();
            let pick = |t: &Task, _: &samu_core::availability::TaskState| hits.contains(&t.id);
            return self.listed_as(
                &settled(arrange),
                tag.as_deref(),
                pick,
                |_| true,
                PerspectiveFilter::Remaining,
            );
        }
        let snapshot = objects.snapshot();
        let test = PerspectiveTest::new(perspective, &snapshot, Timestamp::now(), &settings.zone);
        let pick = |t: &Task, s: &samu_core::availability::TaskState| {
            (test.all || is_open(t)) && (!test.available || s.available) && test.task(t, s)
        };
        // What its filter shows, for the children of a group it finds.
        let show = if test.all {
            PerspectiveFilter::All
        } else if test.available {
            PerspectiveFilter::Available
        } else {
            PerspectiveFilter::Remaining
        };
        self.listed_as(&settled(arrange), tag.as_deref(), pick, |_| true, show)
    }

    /// A block perspective block by block, as the web shows it (ADR 0147
    /// §1, ADR 0116 §5): with `dedupe` a task only in the first block that
    /// shows it, each block's tasks in the arrangement of its perspective
    /// or search, a fixed view's as this phone keeps it. Through the scope
    /// chosen here and, if given, only with `tag` or below; a note block
    /// in every scope (ADR 0146 §2). A plain perspective is one block.
    pub fn blocks(&self, id: String, tag: Option<String>) -> Result<Vec<ShownBlock>, MobileError> {
        let id = parse_id(&id)?;
        let scope = self.chosen_id()?;
        let (settings, objects) = {
            let cache = self.cache()?;
            (Self::settings(&cache)?.0, Self::view(&cache)?.0)
        };
        let lang = self.ui_lang();
        let found = samu_core::views::blocks(&objects, &settings, Timestamp::now(), scope, id)
            .map_err(|e| input(e.to_string()))?;
        let own = objects
            .perspectives
            .iter()
            .find(|p| p.id == id)
            .and_then(|p| p.arrange.clone())
            .unwrap_or_default();
        found
            .into_iter()
            .map(|section| self.block(section, &objects.perspectives, &own, tag.as_deref(), lang))
            .collect()
    }
}

impl Samu {
    /// One section of `/blocks` as the app shows it.
    fn block(
        &self,
        section: BlockSection,
        perspectives: &[samu_api::Perspective],
        own: &PerspectiveArrange,
        tag: Option<&str>,
        lang: samu_core::query::Lang,
    ) -> Result<ShownBlock, MobileError> {
        let block = section.block.clone().unwrap_or_default();
        let target = block
            .perspective
            .and_then(|id| perspectives.iter().find(|p| p.id == id));
        let view = block
            .view
            .as_deref()
            .and_then(|v| crate::bar::view(v, lang));
        let title = match (&block.title, &view) {
            (None, Some(v)) => v.title.clone(),
            _ => section.title.clone(),
        };
        let opens = if section.missing {
            None
        } else if let Some(p) = target {
            Some(BarEntry {
                kind: "perspective".into(),
                key: p.id.to_string(),
                title: p.title.clone(),
                icon: p.icon.clone(),
                hidden: false,
            })
        } else {
            view
        };
        if let Some(object) = block.note {
            return Ok(ShownBlock {
                title,
                icon: section.icon,
                missing: section.missing,
                opens: None,
                object: (!section.missing).then(|| object.to_string()),
                note: Some(section.note.unwrap_or_default()),
                count: 0,
                sections: Vec::new(),
            });
        }
        let hits: HashSet<Id> = section.tasks.iter().map(|s| s.task.id).collect();
        let arrange: PerspectiveArrange = match (&block.query, target, &block.view) {
            (Some(query), _, _) => query.arrange.clone().unwrap_or_default(),
            (None, Some(p), _) => p.arrange.clone().unwrap_or_default(),
            (None, None, Some(name)) => self.arrangement(name).unwrap_or_default(),
            // A plain perspective is one block of its own.
            _ => own.clone(),
        };
        let sections = if hits.is_empty() {
            Vec::new()
        } else {
            let pick = |t: &Task, _: &samu_core::availability::TaskState| hits.contains(&t.id);
            self.listed_as(
                &settled(arrange),
                tag,
                pick,
                |_| true,
                PerspectiveFilter::Remaining,
            )?
        };
        let count = sections
            .iter()
            .flat_map(|s| &s.rows)
            .filter_map(|r| r.id.parse::<Id>().ok())
            .filter(|id| hits.contains(id))
            .collect::<HashSet<_>>()
            .len();
        Ok(ShownBlock {
            title,
            icon: section.icon,
            missing: section.missing,
            opens,
            object: None,
            note: None,
            count: u32::try_from(count).unwrap_or(u32::MAX),
            sections,
        })
    }
}
