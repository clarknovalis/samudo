// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A client's own changes laid over what the server last said (ADR 0028 §4).
//!
//! A phone mirrors the server's objects and keeps its unconfirmed changes
//! in an outbox. What it shows is the mirror with the outbox applied on
//! top, in order; once the server has the changes, the next pull brings
//! the truth and the outbox is empty again. Only the server decides who
//! wins (ADR 0004 §4) — this is the view until it has.
//!
//! Changes act on the serde form of an object, the same form `create`
//! carries and `set` names fields of, so this follows the wire and not the
//! Rust structs.

use samu_api::{
    AccountSettings, AnyObject, Appointment, Attachment, Calendar, Change, Clock, Comment, Folder,
    HabitEntry, Id, Link, ObjectKind, Perspective, Project, Reminder, Scope, Tag, Tagging, Task,
    Template,
};
use serde_json::{Map, Value};

use crate::availability::Snapshot;

/// Every live object of an account, by kind.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Objects {
    pub folders: Vec<Folder>,
    pub projects: Vec<Project>,
    pub tasks: Vec<Task>,
    pub tags: Vec<Tag>,
    pub links: Vec<Link>,
    /// ADR 0046.
    pub clocks: Vec<Clock>,
    /// ADR 0030.
    pub reminders: Vec<Reminder>,
    /// ADR 0035.
    pub scopes: Vec<Scope>,
    /// ADR 0039 §3.
    pub comments: Vec<Comment>,
    /// ADR 0039 §5.
    pub attachments: Vec<Attachment>,
    /// ADR 0045 §4: the account's one settings object.
    pub settings: Option<AccountSettings>,
    /// ADR 0047.
    pub calendars: Vec<Calendar>,
    /// ADR 0047, 0048: written by the server alone.
    pub appointments: Vec<Appointment>,
    /// ADR 0073.
    pub templates: Vec<Template>,
    /// ADR 0090.
    pub perspectives: Vec<Perspective>,
    /// ADR 0091.
    pub habit_entries: Vec<HabitEntry>,
}

/// Why a change could not be laid over; the change stays in the outbox and
/// the server will say what it makes of it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OverlayError {
    #[error("no object {0}")]
    Missing(Id),
    #[error("{0}")]
    Shape(String),
}

impl Objects {
    /// The objects of a page or a mirror; unknown kinds are left out.
    pub fn from_any(objects: impl IntoIterator<Item = AnyObject>) -> Self {
        let mut out = Self::default();
        for object in objects {
            out.put(object);
        }
        out
    }

    /// Insert or replace one object.
    pub fn put(&mut self, object: AnyObject) {
        fn replace<T>(list: &mut Vec<T>, item: T, id: Id, id_of: impl Fn(&T) -> Id) {
            match list.iter_mut().find(|o| id_of(o) == id) {
                Some(slot) => *slot = item,
                None => list.push(item),
            }
        }
        match object {
            AnyObject::Folder(o) => replace(&mut self.folders, o.clone(), o.id, |x| x.id),
            AnyObject::Project(o) => replace(&mut self.projects, o.clone(), o.id, |x| x.id),
            AnyObject::Task(o) => replace(&mut self.tasks, o.clone(), o.id, |x| x.id),
            AnyObject::Tag(o) => replace(&mut self.tags, o.clone(), o.id, |x| x.id),
            AnyObject::Link(o) => replace(&mut self.links, o.clone(), o.id, |x| x.id),
            AnyObject::Reminder(o) => replace(&mut self.reminders, o.clone(), o.id, |x| x.id),
            AnyObject::Scope(o) => replace(&mut self.scopes, o.clone(), o.id, |x| x.id),
            AnyObject::Comment(o) => replace(&mut self.comments, o.clone(), o.id, |x| x.id),
            AnyObject::Attachment(o) => replace(&mut self.attachments, o.clone(), o.id, |x| x.id),
            AnyObject::Settings(o) => self.settings = Some(o),
            AnyObject::Clock(o) => replace(&mut self.clocks, o.clone(), o.id, |x| x.id),
            AnyObject::Calendar(o) => replace(&mut self.calendars, o.clone(), o.id, |x| x.id),
            AnyObject::Appointment(o) => {
                replace(&mut self.appointments, o.clone(), o.id, |x| x.id);
            }
            AnyObject::Template(o) => replace(&mut self.templates, o.clone(), o.id, |x| x.id),
            AnyObject::Perspective(o) => {
                replace(&mut self.perspectives, o.clone(), o.id, |x| x.id);
            }
            AnyObject::HabitEntry(o) => {
                replace(&mut self.habit_entries, o.clone(), o.id, |x| x.id);
            }
            AnyObject::Unknown => {}
        }
    }

    /// Remove one object of any kind.
    pub fn remove(&mut self, id: Id) {
        self.folders.retain(|o| o.id != id);
        self.projects.retain(|o| o.id != id);
        self.tasks.retain(|o| o.id != id);
        self.tags.retain(|o| o.id != id);
        self.links.retain(|o| o.id != id);
        self.clocks.retain(|o| o.id != id);
        self.reminders.retain(|o| o.id != id);
        self.scopes.retain(|o| o.id != id);
        self.comments.retain(|o| o.id != id);
        self.attachments.retain(|o| o.id != id);
        self.calendars.retain(|o| o.id != id);
        self.appointments.retain(|o| o.id != id);
        self.templates.retain(|o| o.id != id);
        self.perspectives.retain(|o| o.id != id);
        self.habit_entries.retain(|o| o.id != id);
        if self.settings.as_ref().is_some_and(|o| o.id == id) {
            self.settings = None;
        }
    }

    /// Every object, each as its kind.
    #[must_use]
    pub fn all(&self) -> Vec<AnyObject> {
        let mut out: Vec<AnyObject> = Vec::new();
        out.extend(self.folders.iter().cloned().map(AnyObject::Folder));
        out.extend(self.projects.iter().cloned().map(AnyObject::Project));
        out.extend(self.tasks.iter().cloned().map(AnyObject::Task));
        out.extend(self.tags.iter().cloned().map(AnyObject::Tag));
        out.extend(self.links.iter().cloned().map(AnyObject::Link));
        out.extend(self.clocks.iter().cloned().map(AnyObject::Clock));
        out.extend(self.reminders.iter().cloned().map(AnyObject::Reminder));
        out.extend(self.scopes.iter().cloned().map(AnyObject::Scope));
        out.extend(self.comments.iter().cloned().map(AnyObject::Comment));
        out.extend(self.attachments.iter().cloned().map(AnyObject::Attachment));
        out.extend(self.settings.iter().cloned().map(AnyObject::Settings));
        out.extend(self.calendars.iter().cloned().map(AnyObject::Calendar));
        out.extend(
            self.appointments
                .iter()
                .cloned()
                .map(AnyObject::Appointment),
        );
        out.extend(self.templates.iter().cloned().map(AnyObject::Template));
        out.extend(
            self.perspectives
                .iter()
                .cloned()
                .map(AnyObject::Perspective),
        );
        out.extend(
            self.habit_entries
                .iter()
                .cloned()
                .map(AnyObject::HabitEntry),
        );
        out
    }

    /// One object by id, as its kind.
    #[must_use]
    pub fn get(&self, id: Id) -> Option<AnyObject> {
        let find = |object: AnyObject| Some(object);
        if let Some(o) = self.tasks.iter().find(|o| o.id == id) {
            return find(AnyObject::Task(o.clone()));
        }
        if let Some(o) = self.projects.iter().find(|o| o.id == id) {
            return find(AnyObject::Project(o.clone()));
        }
        if let Some(o) = self.tags.iter().find(|o| o.id == id) {
            return find(AnyObject::Tag(o.clone()));
        }
        if let Some(o) = self.folders.iter().find(|o| o.id == id) {
            return find(AnyObject::Folder(o.clone()));
        }
        if let Some(o) = self.clocks.iter().find(|o| o.id == id) {
            return find(AnyObject::Clock(o.clone()));
        }
        if let Some(o) = self.reminders.iter().find(|o| o.id == id) {
            return find(AnyObject::Reminder(o.clone()));
        }
        if let Some(o) = self.scopes.iter().find(|o| o.id == id) {
            return find(AnyObject::Scope(o.clone()));
        }
        if let Some(o) = self.comments.iter().find(|o| o.id == id) {
            return find(AnyObject::Comment(o.clone()));
        }
        if let Some(o) = self.attachments.iter().find(|o| o.id == id) {
            return find(AnyObject::Attachment(o.clone()));
        }
        if let Some(o) = self.settings.as_ref().filter(|o| o.id == id) {
            return find(AnyObject::Settings(o.clone()));
        }
        if let Some(o) = self.calendars.iter().find(|o| o.id == id) {
            return find(AnyObject::Calendar(o.clone()));
        }
        if let Some(o) = self.appointments.iter().find(|o| o.id == id) {
            return find(AnyObject::Appointment(o.clone()));
        }
        if let Some(o) = self.templates.iter().find(|o| o.id == id) {
            return find(AnyObject::Template(o.clone()));
        }
        if let Some(o) = self.perspectives.iter().find(|o| o.id == id) {
            return find(AnyObject::Perspective(o.clone()));
        }
        if let Some(o) = self.habit_entries.iter().find(|o| o.id == id) {
            return find(AnyObject::HabitEntry(o.clone()));
        }
        self.links
            .iter()
            .find(|o| o.id == id)
            .map(|o| AnyObject::Link(o.clone()))
    }

    /// The view availability is computed on (ADR 0006).
    #[must_use]
    pub fn snapshot(&self) -> Snapshot<'_> {
        Snapshot {
            folders: &self.folders,
            projects: &self.projects,
            tasks: &self.tasks,
            tags: &self.tags,
            links: &self.links,
            clocks: &self.clocks,
            scopes: &self.scopes,
        }
    }
}

fn kind_name(kind: ObjectKind) -> Option<&'static str> {
    serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .and_then(|name| {
            [
                "folder",
                "project",
                "task",
                "tag",
                "link",
                "clock",
                "reminder",
                "scope",
                "comment",
                "attachment",
                "settings",
                "template",
                "perspective",
                "habit_entry",
            ]
            .into_iter()
            .find(|known| *known == name)
        })
}

/// An object back from its serde form, as its kind.
fn typed(kind: &str, object: Value) -> Result<AnyObject, OverlayError> {
    serde_json::from_value(serde_json::json!({ "kind": kind, "object": object }))
        .map_err(|e| OverlayError::Shape(e.to_string()))
}

/// The serde form of an object and its kind's name.
fn untyped(object: &AnyObject) -> Result<(String, Map<String, Value>), OverlayError> {
    let value = serde_json::to_value(object).map_err(|e| OverlayError::Shape(e.to_string()))?;
    let kind = value["kind"].as_str().unwrap_or_default().to_owned();
    match value.get("object") {
        Some(Value::Object(map)) => Ok((kind, map.clone())),
        _ => Err(OverlayError::Shape("an object that is not a map".into())),
    }
}

/// Lay one change over `objects`, as the server would apply it if nothing
/// newer came between.
pub fn apply(objects: &mut Objects, change: &Change) -> Result<(), OverlayError> {
    match change {
        Change::Create {
            object,
            kind,
            value,
            ..
        } => {
            let name = kind_name(*kind).ok_or_else(|| {
                OverlayError::Shape(format!("a kind this samu does not know for {object}"))
            })?;
            let made = typed(name, value.clone())?;
            if made.id() != Some(*object) {
                return Err(OverlayError::Shape(format!(
                    "create {object} carries another id"
                )));
            }
            objects.put(made);
            Ok(())
        }
        Change::Set {
            object,
            field,
            value,
            ..
        } => {
            let current = objects.get(*object).ok_or(OverlayError::Missing(*object))?;
            let (kind, mut map) = untyped(&current)?;
            // A map written per key: properties (ADR 0023), the templates
            // of the settings (ADR 0054 §3).
            let keyed = ["properties", "property_links"]
                .into_iter()
                .find_map(|name| {
                    field
                        .strip_prefix(name)
                        .and_then(|rest| rest.strip_prefix('.'))
                        .map(|key| (name, key))
                });
            if let Some((name, key)) = keyed {
                let properties = map.entry(name).or_insert_with(|| Value::Object(Map::new()));
                if let Value::Object(properties) = properties {
                    if value.is_null() {
                        properties.remove(key);
                    } else {
                        properties.insert(key.to_owned(), value.clone());
                    }
                }
            } else if value.is_null() {
                map.remove(field);
            } else {
                map.insert(field.clone(), value.clone());
            }
            objects.put(typed(&kind, Value::Object(map))?);
            Ok(())
        }
        Change::Add {
            object,
            element,
            position,
            ..
        } => {
            if let Some(scope) = objects.scopes.iter_mut().find(|s| s.id == *object) {
                if !scope.members.contains(element) {
                    scope.members.push(*element);
                }
                return Ok(());
            }
            // A state of a workflow replaces its siblings (ADR 0024 §1).
            let siblings: Vec<Id> = objects
                .tags
                .iter()
                .find(|t| t.id == *element)
                .and_then(|t| t.parent)
                .filter(|parent| objects.tags.iter().any(|t| t.id == *parent && t.exclusive))
                .map(|parent| {
                    objects
                        .tags
                        .iter()
                        .filter(|t| t.parent == Some(parent) && t.id != *element)
                        .map(|t| t.id)
                        .collect()
                })
                .unwrap_or_default();
            let tagging = Tagging {
                tag: *element,
                position: *position,
            };
            let add = |tags: &mut Vec<Tagging>| {
                tags.retain(|t| !siblings.contains(&t.tag));
                if !tags.iter().any(|t| t.tag == *element) {
                    tags.push(tagging);
                }
            };
            if let Some(task) = objects.tasks.iter_mut().find(|t| t.id == *object) {
                add(&mut task.tags);
            } else if let Some(project) = objects.projects.iter_mut().find(|p| p.id == *object) {
                add(&mut project.tags);
            } else {
                return Err(OverlayError::Missing(*object));
            }
            Ok(())
        }
        Change::Remove {
            object, element, ..
        } => {
            if let Some(scope) = objects.scopes.iter_mut().find(|s| s.id == *object) {
                scope.members.retain(|m| m != element);
                return Ok(());
            }
            if let Some(task) = objects.tasks.iter_mut().find(|t| t.id == *object) {
                task.tags.retain(|t| t.tag != *element);
            } else if let Some(project) = objects.projects.iter_mut().find(|p| p.id == *object) {
                project.tags.retain(|t| t.tag != *element);
            } else {
                return Err(OverlayError::Missing(*object));
            }
            Ok(())
        }
        Change::Delete { object, .. } => {
            objects.remove(*object);
            Ok(())
        }
        Change::Unknown => Err(OverlayError::Shape(
            "a change this samu does not know".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use samu_api::Hlc;
    use serde_json::json;

    use super::*;

    fn id(n: u8) -> Id {
        format!("01936f5e-8a00-7c3e-9b1a-0000000000{n:02x}")
            .parse()
            .unwrap()
    }

    fn hlc() -> Hlc {
        Hlc::new(1, 0, "pixel")
    }

    fn task_value(n: u8, title: &str) -> Value {
        json!({"id": id(n), "created": "2026-11-08T08:00:00Z", "modified": "2026-11-08T08:00:00Z",
               "title": title, "flagged": false, "sequential": false,
               "complete_with_last_child": false, "position": 0})
    }

    fn tag_value(n: u8, title: &str, parent: Option<u8>, exclusive: bool) -> Value {
        let mut v = json!({"id": id(n), "created": "2026-11-08T08:00:00Z",
            "modified": "2026-11-08T08:00:00Z", "title": title, "status": "active",
            "exclusive": exclusive, "position": 0});
        if let Some(p) = parent {
            v["parent"] = json!(id(p));
        }
        v
    }

    fn create(kind: &str, value: Value) -> Change {
        serde_json::from_value(json!({"op": "create", "object": value["id"], "kind": kind,
                                       "value": value, "hlc": hlc()}))
        .unwrap()
    }

    fn set(n: u8, field: &str, value: Value) -> Change {
        serde_json::from_value(
            json!({"op": "set", "object": id(n), "field": field, "value": value, "hlc": hlc()}),
        )
        .unwrap()
    }

    fn add(n: u8, tag: u8) -> Change {
        serde_json::from_value(json!({"op": "add", "object": id(n), "field": "tags",
                                       "element": id(tag), "position": 0, "hlc": hlc()}))
        .unwrap()
    }

    fn world() -> Objects {
        let mut objects = Objects::default();
        for change in [
            create("task", task_value(1, "Milch")),
            create("tag", tag_value(0x10, "Status", None, true)),
            create("tag", tag_value(0x11, "Wartet", Some(0x10), false)),
            create("tag", tag_value(0x12, "Irgendwann", Some(0x10), false)),
            create("tag", tag_value(0x13, "telefon", None, false)),
        ] {
            apply(&mut objects, &change).unwrap();
        }
        objects
    }

    /// ADR 0030: a reminder made offline shows before the server has it.
    #[test]
    fn a_reminder_is_laid_over_like_the_rest() {
        let mut o = world();
        let value = json!({"id": id(0x20), "created": "2026-11-08T08:00:00Z",
                           "modified": "2026-11-08T08:00:00Z", "owner": id(1),
                           "anchor": "due", "before": "PT1H"});
        apply(&mut o, &create("reminder", value)).unwrap();
        apply(&mut o, &set(0x20, "before", json!("PT30M"))).unwrap();
        assert_eq!(o.reminders.len(), 1);
        assert_eq!(
            o.reminders[0].before.map(|p| p.0.fieldwise()),
            Some(jiff::Span::new().minutes(30).fieldwise())
        );
        let delete =
            serde_json::from_value(json!({"op": "delete", "object": id(0x20), "hlc": hlc()}))
                .unwrap();
        apply(&mut o, &delete).unwrap();
        assert!(o.reminders.is_empty());
    }

    /// ADR 0039 §6: a comment and an attachment made offline show before
    /// the server has them; editing and deleting are laid over too.
    #[test]
    fn a_comment_and_an_attachment_are_laid_over() {
        let mut o = world();
        let comment = json!({"id": id(0x40), "created": "2026-11-08T08:00:00Z",
                             "modified": "2026-11-08T08:00:00Z", "owner": id(1),
                             "body": "Erster Stand"});
        apply(&mut o, &create("comment", comment)).unwrap();
        apply(&mut o, &set(0x40, "body", json!("Zweiter Stand"))).unwrap();
        let attachment = json!({"id": id(0x41), "created": "2026-11-08T08:00:00Z",
                                "modified": "2026-11-08T08:00:00Z", "owner": id(0x40),
                                "blob": "ab".repeat(32), "name": "foto.jpg",
                                "media_type": "image/jpeg", "size": 3});
        apply(&mut o, &create("attachment", attachment)).unwrap();
        assert_eq!(o.comments.len(), 1);
        assert_eq!(o.comments[0].body, "Zweiter Stand");
        assert_eq!(o.attachments.len(), 1);
        assert_eq!(o.attachments[0].owner, id(0x40));
        for n in [0x41, 0x40] {
            let delete =
                serde_json::from_value(json!({"op": "delete", "object": id(n), "hlc": hlc()}))
                    .unwrap();
            apply(&mut o, &delete).unwrap();
        }
        assert!(o.comments.is_empty());
        assert!(o.attachments.is_empty());
    }

    /// ADR 0035: a scope made offline, its members added and removed.
    #[test]
    fn a_scope_and_its_members_are_laid_over() {
        let mut o = world();
        let value = json!({"id": id(0x30), "created": "2026-11-08T08:00:00Z",
                           "modified": "2026-11-08T08:00:00Z", "title": "Privat",
                           "mode": "only", "position": 0});
        apply(&mut o, &create("scope", value)).unwrap();
        let member = |op: &str, n: u8| -> Change {
            let mut change = json!({"op": op, "object": id(0x30), "field": "members",
                                    "element": id(n), "hlc": hlc()});
            if op == "add" {
                change["position"] = json!(0);
            }
            serde_json::from_value(change).unwrap()
        };
        apply(&mut o, &member("add", 1)).unwrap();
        apply(&mut o, &member("add", 2)).unwrap();
        apply(&mut o, &member("add", 1)).unwrap();
        apply(&mut o, &member("remove", 2)).unwrap();
        apply(&mut o, &set(0x30, "mode", json!("except"))).unwrap();
        assert_eq!(o.scopes.len(), 1);
        assert_eq!(o.scopes[0].members, vec![id(1)]);
        assert_eq!(o.scopes[0].mode, samu_api::ScopeMode::Except);
        // The tags of the task with the same id were not touched.
        assert!(
            o.tasks
                .iter()
                .all(|t| t.tags.iter().all(|g| g.tag != id(2)))
        );
    }

    #[test]
    fn create_set_and_clear_fields() {
        let mut o = world();
        assert_eq!(o.tasks.len(), 1);
        apply(
            &mut o,
            &set(1, "completed_at", json!("2026-11-08T10:00:00Z")),
        )
        .unwrap();
        assert!(o.tasks[0].completed_at.is_some());
        apply(&mut o, &set(1, "completed_at", Value::Null)).unwrap();
        assert!(o.tasks[0].completed_at.is_none());
        apply(&mut o, &set(1, "due", json!({"date": "2026-11-20"}))).unwrap();
        assert!(
            o.tasks[0].schedule.due.is_some(),
            "a flattened schedule field"
        );
        apply(&mut o, &set(1, "flagged", json!(true))).unwrap();
        assert!(o.tasks[0].flagged);
        apply(&mut o, &set(1, "properties.ort", json!("Keller"))).unwrap();
        assert_eq!(o.tasks[0].properties["ort"], "Keller");
        apply(&mut o, &set(1, "properties.ort", Value::Null)).unwrap();
        assert!(o.tasks[0].properties.is_empty());
    }

    /// ADR 0054 §3: the templates of the settings are written per key.
    #[test]
    fn a_property_link_is_set_and_cleared_per_key() {
        let mut o = world();
        let settings: AccountSettings = serde_json::from_value(json!({
            "id": id(0x40), "created": "2026-11-09T08:00:00Z",
            "modified": "2026-11-09T08:00:00Z", "due_soon": "P2D",
            "remind_time": "09:00:00"}))
        .unwrap();
        o.put(AnyObject::Settings(settings));
        let jira = "https://jira.example.org/browse/{value}";
        apply(&mut o, &set(0x40, "property_links.jira", json!(jira))).unwrap();
        apply(
            &mut o,
            &set(0x40, "property_links.ticket", json!("x{value}")),
        )
        .unwrap();
        apply(&mut o, &set(0x40, "property_links.ticket", Value::Null)).unwrap();
        let links = &o.settings.as_ref().unwrap().property_links;
        assert_eq!(links.len(), 1);
        assert_eq!(links["jira"], jira);
    }

    #[test]
    fn tags_are_added_and_removed_and_a_state_replaces_its_siblings() {
        let mut o = world();
        apply(&mut o, &add(1, 0x13)).unwrap();
        apply(&mut o, &add(1, 0x11)).unwrap();
        apply(&mut o, &add(1, 0x11)).unwrap();
        let tags = |o: &Objects| o.tasks[0].tags.iter().map(|t| t.tag).collect::<Vec<_>>();
        assert_eq!(tags(&o), [id(0x13), id(0x11)], "no element twice");
        apply(&mut o, &add(1, 0x12)).unwrap();
        assert_eq!(tags(&o), [id(0x13), id(0x12)], "Irgendwann replaces Wartet");
        let remove: Change = serde_json::from_value(json!({"op": "remove", "object": id(1),
            "field": "tags", "element": id(0x13), "hlc": hlc()}))
        .unwrap();
        apply(&mut o, &remove).unwrap();
        assert_eq!(tags(&o), [id(0x12)]);
    }

    #[test]
    fn delete_and_what_cannot_be_laid_over() {
        let mut o = world();
        let delete: Change =
            serde_json::from_value(json!({"op": "delete", "object": id(1), "hlc": hlc()})).unwrap();
        apply(&mut o, &delete).unwrap();
        assert!(o.tasks.is_empty());
        assert_eq!(
            apply(&mut o, &set(1, "title", json!("x"))),
            Err(OverlayError::Missing(id(1)))
        );
        assert!(matches!(
            apply(&mut world(), &set(1, "flagged", json!("ja"))),
            Err(OverlayError::Shape(_))
        ));
        assert!(matches!(
            apply(&mut world(), &Change::Unknown),
            Err(OverlayError::Shape(_))
        ));
    }

    #[test]
    fn a_page_replaces_what_it_names() {
        let mut o = world();
        let newer: AnyObject =
            serde_json::from_value(json!({"kind": "task", "object": task_value(1, "Hafermilch")}))
                .unwrap();
        o.put(newer);
        assert_eq!(o.tasks.len(), 1);
        assert_eq!(o.tasks[0].title, "Hafermilch");
    }
}
