// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The bar at the bottom of the app (ADR 0102 §1, §3): a list of entries,
//! kept per phone in the cache's meta, never synced. A fixed view, a
//! perspective, a project or a tag; what is gone leaves it silently.

use samu_api::{Id, ProjectStatus, TagStatus};
use samu_core::overlay::Objects;
use serde_json::{Value, json};

use samu_core::query::Lang;

use crate::{MobileError, Names, Samu, input, parse_id, say};

/// The cache's key for the bar.
const BAR_KEY: &str = "bar";

/// The fixed views, by key and title, in the order „Hinzufügen“ offers
/// them.
const VIEWS: [(&str, &str, &str); 10] = [
    ("inbox", "Inbox", "Inbox"),
    ("available", "Verfügbar", "Available"),
    ("today", "Heute", "Today"),
    ("forecast", "Forecast", "Forecast"),
    ("flagged", "Markiert", "Flagged"),
    ("stuck", "Stuck", "Stuck"),
    ("review", "Review", "Review"),
    ("projects", "Projekte", "Projects"),
    ("search", "Suche", "Search"),
    ("habits", "Habits", "Habits"),
];

/// Without a setting: the five of ADR 0020, in its order.
const DEFAULT: [&str; 5] = ["inbox", "available", "forecast", "projects", "search"];

/// One entry of the bar as the app draws it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BarEntry {
    /// `view`, `perspective`, `project` or `tag`.
    pub kind: String,
    /// The view's name (`inbox`, `today`, …) or the object's id.
    pub key: String,
    /// The view's title, the perspective's or project's, the tag's path.
    pub title: String,
    /// The emoji of a perspective or project; none otherwise.
    pub icon: Option<String>,
    /// Kept in the list but not in the bar.
    pub hidden: bool,
}

/// One entry as the settings page hands it back.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BarItem {
    pub kind: String,
    pub key: String,
    pub hidden: bool,
}

pub(crate) fn view(key: &str, lang: Lang) -> Option<BarEntry> {
    VIEWS
        .iter()
        .find(|(k, _, _)| *k == key)
        .map(|(k, de, en)| BarEntry {
            kind: "view".into(),
            key: (*k).into(),
            title: say(lang, de, en).into(),
            icon: None,
            hidden: false,
        })
}

fn inbox() -> BarItem {
    BarItem {
        kind: "view".into(),
        key: "inbox".into(),
        hidden: false,
    }
}

/// The entry for an item, with its title from the mirror; none when what
/// it names is gone, dropped or unknown.
fn resolve(item: &BarItem, objects: &Objects, names: &Names, lang: Lang) -> Option<BarEntry> {
    let id = || item.key.parse::<Id>().ok();
    let (title, icon) = match item.kind.as_str() {
        "view" => {
            return view(&item.key, lang).map(|e| BarEntry {
                hidden: item.hidden,
                ..e
            });
        }
        "perspective" => {
            let id = id()?;
            let p = objects.perspectives.iter().find(|p| p.id == id)?;
            (p.title.clone(), p.icon.clone())
        }
        "project" => {
            let id = id()?;
            let p = objects.projects.iter().find(|p| {
                p.id == id && matches!(p.status, ProjectStatus::Active | ProjectStatus::OnHold)
            })?;
            (p.title.clone(), p.icon.clone())
        }
        "tag" => {
            let id = id()?;
            objects
                .tags
                .iter()
                .find(|t| t.id == id && t.status != TagStatus::Dropped)?;
            (names.tag_path(id), None)
        }
        _ => return None,
    };
    Some(BarEntry {
        kind: item.kind.clone(),
        key: item.key.clone(),
        title,
        icon,
        hidden: item.hidden,
    })
}

/// Inbox first when it is missing, always shown; each entry once.
fn settled(items: Vec<BarItem>) -> Vec<BarItem> {
    let mut out: Vec<BarItem> = Vec::new();
    for item in items {
        if !out.iter().any(|o| o.kind == item.kind && o.key == item.key) {
            out.push(item);
        }
    }
    match out
        .iter_mut()
        .find(|o| o.kind == "view" && o.key == "inbox")
    {
        Some(inbox) => inbox.hidden = false,
        None => out.insert(0, inbox()),
    }
    out
}

fn stored(text: &str) -> Vec<BarItem> {
    let Ok(Value::Array(list)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|v| {
            Some(BarItem {
                kind: v["kind"].as_str()?.to_owned(),
                key: v["key"].as_str()?.to_owned(),
                hidden: v["hidden"].as_bool().unwrap_or(false),
            })
        })
        .collect()
}

#[uniffi::export]
impl Samu {
    /// The bar of this phone, hidden entries among them: as set, else the
    /// five of ADR 0020. A project, tag or perspective that is gone or
    /// dropped leaves it silently (ADR 0102 §3).
    pub fn bar(&self) -> Result<Vec<BarEntry>, MobileError> {
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let names = Names::new(&objects);
        let items = match cache.meta(BAR_KEY)? {
            Some(text) => stored(&text),
            None => DEFAULT
                .iter()
                .map(|k| BarItem {
                    kind: "view".into(),
                    key: (*k).into(),
                    hidden: false,
                })
                .collect(),
        };
        Ok(settled(items)
            .iter()
            .filter_map(|item| resolve(item, &objects, &names, self.ui_lang()))
            .collect())
    }

    /// Keeps the bar of this phone in this order. Inbox stays in it and
    /// shown; an entry named twice counts once. An unknown kind or view,
    /// or an object that is not there, is refused.
    pub fn set_bar(&self, entries: Vec<BarItem>) -> Result<(), MobileError> {
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let names = Names::new(&objects);
        for item in &entries {
            if item.kind != "view" {
                parse_id(&item.key)?;
            }
            if resolve(item, &objects, &names, self.ui_lang()).is_none() {
                return Err(input(format!(
                    "no {} {:?} for the bar",
                    item.kind, item.key
                )));
            }
        }
        let list: Vec<Value> = settled(entries)
            .iter()
            .map(|i| json!({"kind": i.kind, "key": i.key, "hidden": i.hidden}))
            .collect();
        cache.set_meta(BAR_KEY, Some(&Value::Array(list).to_string()))?;
        Ok(())
    }

    /// Every fixed view, for „Hinzufügen“ on the settings page; the
    /// perspectives, projects and tags come from their own lists.
    pub fn bar_views(&self) -> Vec<BarEntry> {
        let lang = self.ui_lang();
        VIEWS.iter().filter_map(|(k, _, _)| view(k, lang)).collect()
    }
}
