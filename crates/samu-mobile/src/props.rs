// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Properties on the phone (ADR 0054): the front matter of a task or
//! project, own and inherited, with where each value leads; setting one;
//! the account's templates per key. Inheritance and links come from
//! samu-core, as on the server. The phone runs no local commands.

use std::collections::BTreeSet;

use samu_api::{Change, Id};
use samu_core::properties::{Inheritance, SYSTEM, is_key, is_system, link_for};
use serde_json::Value;

use crate::{MobileError, Samu, input, parse_id};

/// One property of a task or project.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PropertyRow {
    pub key: String,
    pub value: String,
    /// Its own; else inherited from `from`.
    pub own: bool,
    /// Where an inherited one comes from, as a title.
    pub from: Option<String>,
    /// A key samu writes itself: shown last and faint.
    pub system: bool,
    /// Where a tap leads: the value as a URI, or the key's template.
    pub link: Option<String>,
}

/// One template of the account.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PropertyLinkRow {
    pub key: String,
    pub template: String,
}

fn key_of(key: &str) -> Result<String, MobileError> {
    let key = key.trim().to_lowercase();
    if is_key(&key) {
        Ok(key)
    } else {
        Err(input(format!(
            "{key:?} is no property key: lower-case letters, digits, _ and -, a letter first"
        )))
    }
}

#[uniffi::export]
impl Samu {
    /// The properties of a task (own and inherited) or a project (own),
    /// system keys last.
    pub fn properties(&self, id: String) -> Result<Vec<PropertyRow>, MobileError> {
        let id = parse_id(&id)?;
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let links = objects
            .settings
            .as_ref()
            .map(|s| s.property_links.clone())
            .unwrap_or_default();
        let title = |from: Id| {
            objects
                .projects
                .iter()
                .find(|p| p.id == from)
                .map(|p| p.title.clone())
                .or_else(|| {
                    objects
                        .tasks
                        .iter()
                        .find(|t| t.id == from)
                        .map(|t| t.title.clone())
                })
        };
        let row = |key: &str, value: &str, own: bool, from: Option<String>| PropertyRow {
            key: key.to_owned(),
            value: value.to_owned(),
            own,
            from,
            system: is_system(key),
            link: link_for(value, links.get(key).map(String::as_str)),
        };
        let mut rows: Vec<PropertyRow> =
            if let Some(task) = objects.tasks.iter().find(|t| t.id == id) {
                let inheritance = Inheritance::new(&objects.tasks, &objects.projects);
                task.properties
                    .iter()
                    .map(|(k, v)| row(k, v, true, None))
                    .chain(
                        inheritance
                            .of(task)
                            .into_iter()
                            .map(|(k, i)| row(&k, &i.value, false, title(i.from))),
                    )
                    .collect()
            } else if let Some(project) = objects.projects.iter().find(|p| p.id == id) {
                project
                    .properties
                    .iter()
                    .map(|(k, v)| row(k, v, true, None))
                    .collect()
            } else {
                return Err(input(format!("no task or project {id}")));
            };
        rows.sort_by(|a, b| (a.system, &a.key).cmp(&(b.system, &b.key)));
        Ok(rows)
    }

    /// Sets a property of a task or project; an empty value removes it.
    pub fn set_property(
        &self,
        id: String,
        key: String,
        value: Option<String>,
    ) -> Result<(), MobileError> {
        let id = parse_id(&id)?;
        let key = key_of(&key)?;
        let value = value
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
            .map_or(Value::Null, Value::String);
        self.queue(|objects, stamp| {
            let known = objects.tasks.iter().any(|t| t.id == id)
                || objects.projects.iter().any(|p| p.id == id);
            if !known {
                return Err(input(format!("no task or project {id}")));
            }
            Ok(vec![Change::Set {
                object: id,
                field: format!("properties.{key}"),
                value: value.clone(),
                note: None,
                hlc: stamp(),
            }])
        })
    }

    /// Whether the front matter is unfolded, this phone only (ADR 0054
    /// §2); unfolded until folded once.
    pub fn properties_open(&self) -> Result<bool, MobileError> {
        Ok(self.cache()?.meta("props_folded")?.as_deref() != Some("1"))
    }

    pub fn set_properties_open(&self, open: bool) -> Result<(), MobileError> {
        self.cache()?
            .set_meta("props_folded", (!open).then_some("1"))?;
        Ok(())
    }

    /// Every key in the account, for suggestions; system keys left out.
    pub fn property_keys(&self) -> Result<Vec<String>, MobileError> {
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        let mut keys: BTreeSet<String> = objects
            .settings
            .iter()
            .flat_map(|s| s.property_links.keys().cloned())
            .collect();
        for map in objects
            .tasks
            .iter()
            .map(|t| &t.properties)
            .chain(objects.projects.iter().map(|p| &p.properties))
        {
            keys.extend(map.keys().cloned());
        }
        Ok(keys
            .into_iter()
            .filter(|k| !SYSTEM.contains(&k.as_str()))
            .collect())
    }

    /// The account's templates, by key.
    pub fn property_links(&self) -> Result<Vec<PropertyLinkRow>, MobileError> {
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        Ok(objects
            .settings
            .iter()
            .flat_map(|s| s.property_links.iter())
            .map(|(key, template)| PropertyLinkRow {
                key: key.clone(),
                template: template.clone(),
            })
            .collect())
    }

    /// Sets the template of a key, with `{value}` in it; empty removes it.
    pub fn set_property_link(
        &self,
        key: String,
        template: Option<String>,
    ) -> Result<(), MobileError> {
        let key = key_of(&key)?;
        let template = template
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty());
        if let Some(t) = &template
            && !t.contains("{value}")
        {
            return Err(input(format!("{t:?} has no {{value}} for the value")));
        }
        let value = template.map_or(Value::Null, Value::String);
        self.queue(|objects, stamp| {
            let Some(settings) = &objects.settings else {
                return Err(input("the settings come with the next sync"));
            };
            Ok(vec![Change::Set {
                object: settings.id,
                field: format!("property_links.{key}"),
                value: value.clone(),
                note: None,
                hlc: stamp(),
            }])
        })
    }
}
