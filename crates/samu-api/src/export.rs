// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later OR MIT OR Apache-2.0

//! An account as a directory of files, readable without samu (ADR 0074).
//!
//! `GET /export` answers with an [`Export`]; [`Export::to_files`] lays it
//! out as the directory the ADR describes and [`Export::from_files`] reads
//! it back. Both live here, next to the types, so the server, the CLI and a
//! restore all write and read the same bytes. No I/O: the caller writes and
//! reads the files.

use std::collections::BTreeMap;

use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::{AnyObject, Author, Blob, Hlc, Id, LogEntry, ObjectKind};

/// The version of the directory layout. Changes only when a file is
/// renamed or its form changes in a way an older reader would misread; new
/// fields in the objects follow the version of `samu-api`.
pub const EXPORT_FORMAT: u32 = 1;

/// The whole account (ADR 0074 §1, §2): what `GET /export` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Export {
    pub manifest: ExportManifest,
    /// Every live object; deleted ones are not exported (ADR 0074 §4).
    pub objects: Vec<ExportedObject>,
    pub logbook: Vec<LogEntry>,
    /// The files attachments point at; their bytes go by `GET /blobs/{hash}`
    /// and lie in `files/<hash>` of the directory.
    pub files: Vec<Blob>,
}

/// What the export is of and when it was made (`manifest.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportManifest {
    /// [`EXPORT_FORMAT`] of the writer.
    pub format: u32,
    /// The version of `samu-api` the objects are in.
    pub api: String,
    pub account: String,
    /// The only thing that differs between two exports of the same state.
    pub exported_at: Timestamp,
    /// The account's change number the export is current to.
    pub seq: u64,
    /// The account's time zone, IANA name.
    pub zone: String,
    /// What the account row holds besides the settings object; read when
    /// the settings object has none.
    pub due_soon: String,
    pub remind_time: String,
    /// The highest stamp the account has seen (catastr ADR 0029).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed: Option<Hlc>,
    /// Objects in each file, by file name.
    pub counts: BTreeMap<String, u64>,
}

/// One object and what the server keeps beside it for sync (ADR 0074 §2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportedObject {
    #[serde(flatten)]
    pub object: AnyObject,
    pub sync: ObjectSync,
}

/// The stamps of an object, its change number and who made it; with the
/// removed tags, properties and scope members, which the object no longer
/// shows but whose stamps keep an older change from bringing them back
/// (ADR 0004 §1).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectSync {
    /// The change number of the object's last change.
    pub seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<Author>,
    /// By field; `@create` is the stamp of the creation.
    pub stamps: BTreeMap<String, SyncStamp>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<TagStamp>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub properties: Vec<PropertyStamp>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<MemberStamp>,
}

/// A stamp as the server keeps it: the HLC and the change number it was
/// applied at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncStamp {
    pub wall: i64,
    pub counter: u32,
    pub device: String,
    pub seq: u64,
}

/// One element of the tags of a task or project, present or removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagStamp {
    pub tag: Id,
    pub position: i64,
    pub present: bool,
    pub stamp: SyncStamp,
}

/// One property key; `value` none when it was removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropertyStamp {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    pub stamp: SyncStamp,
}

/// One member of a scope, present or removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberStamp {
    pub member: Id,
    pub present: bool,
    pub stamp: SyncStamp,
}

/// A directory that does not read as an export.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportError(pub String);

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ExportError {}

pub const EXPORT_MANIFEST: &str = "manifest.json";
pub const EXPORT_LOGBOOK: &str = "logbook.json";
pub const EXPORT_FILES: &str = "files.json";
pub const EXPORT_README: &str = "README.md";
/// The directory with the bytes of the attachments, one file per hash.
pub const EXPORT_BLOBS: &str = "files";

/// The file that holds the objects of a kind (ADR 0074 §1); none for
/// [`ObjectKind::Unknown`]. Exhaustive on purpose: a new kind does not
/// compile until it has a file.
#[must_use]
pub fn export_file(kind: ObjectKind) -> Option<&'static str> {
    Some(match kind {
        ObjectKind::Folder => "folders.json",
        ObjectKind::Project => "projects.json",
        ObjectKind::Task => "tasks.json",
        ObjectKind::Tag => "tags.json",
        ObjectKind::Link => "links.json",
        ObjectKind::Clock => "clocks.json",
        ObjectKind::Reminder => "reminders.json",
        ObjectKind::Scope => "scopes.json",
        ObjectKind::Comment => "comments.json",
        ObjectKind::Attachment => "attachments.json",
        ObjectKind::Settings => "settings.json",
        ObjectKind::Calendar => "calendars.json",
        ObjectKind::Appointment => "appointments.json",
        ObjectKind::Template => "templates.json",
        ObjectKind::Perspective => "perspectives.json",
        ObjectKind::HabitEntry => "habit_entries.json",
        ObjectKind::ChecklistItem => "checklist_items.json",
        ObjectKind::Unknown => return None,
    })
}

/// Every kind that has a file, in the order of [`ObjectKind`].
pub const EXPORT_KINDS: &[ObjectKind] = &[
    ObjectKind::Folder,
    ObjectKind::Project,
    ObjectKind::Task,
    ObjectKind::Tag,
    ObjectKind::Link,
    ObjectKind::Clock,
    ObjectKind::Reminder,
    ObjectKind::Scope,
    ObjectKind::Comment,
    ObjectKind::Attachment,
    ObjectKind::Settings,
    ObjectKind::Calendar,
    ObjectKind::Appointment,
    ObjectKind::Template,
    ObjectKind::Perspective,
    ObjectKind::HabitEntry,
    ObjectKind::ChecklistItem,
];

/// The kind of an object as the wire form names it.
#[must_use]
pub fn kind_of(object: &AnyObject) -> ObjectKind {
    match object {
        AnyObject::Folder(_) => ObjectKind::Folder,
        AnyObject::Project(_) => ObjectKind::Project,
        AnyObject::Task(_) => ObjectKind::Task,
        AnyObject::Tag(_) => ObjectKind::Tag,
        AnyObject::Link(_) => ObjectKind::Link,
        AnyObject::Clock(_) => ObjectKind::Clock,
        AnyObject::Reminder(_) => ObjectKind::Reminder,
        AnyObject::Scope(_) => ObjectKind::Scope,
        AnyObject::Comment(_) => ObjectKind::Comment,
        AnyObject::Attachment(_) => ObjectKind::Attachment,
        AnyObject::Settings(_) => ObjectKind::Settings,
        AnyObject::Calendar(_) => ObjectKind::Calendar,
        AnyObject::Appointment(_) => ObjectKind::Appointment,
        AnyObject::Template(_) => ObjectKind::Template,
        AnyObject::Perspective(_) => ObjectKind::Perspective,
        AnyObject::HabitEntry(_) => ObjectKind::HabitEntry,
        AnyObject::ChecklistItem(_) => ObjectKind::ChecklistItem,
        AnyObject::Unknown => ObjectKind::Unknown,
    }
}

/// The id of an object; none for one of an unknown kind.
#[must_use]
pub fn id_of(object: &AnyObject) -> Option<Id> {
    Some(match object {
        AnyObject::Folder(o) => o.id,
        AnyObject::Project(o) => o.id,
        AnyObject::Task(o) => o.id,
        AnyObject::Tag(o) => o.id,
        AnyObject::Link(o) => o.id,
        AnyObject::Clock(o) => o.id,
        AnyObject::Reminder(o) => o.id,
        AnyObject::Scope(o) => o.id,
        AnyObject::Comment(o) => o.id,
        AnyObject::Attachment(o) => o.id,
        AnyObject::Settings(o) => o.id,
        AnyObject::Calendar(o) => o.id,
        AnyObject::Appointment(o) => o.id,
        AnyObject::Template(o) => o.id,
        AnyObject::Perspective(o) => o.id,
        AnyObject::HabitEntry(o) => o.id,
        AnyObject::ChecklistItem(o) => o.id,
        AnyObject::Unknown => return None,
    })
}

/// Pretty JSON with two spaces and a newline at the end (ADR 0074 §3).
fn pretty<T: Serialize>(value: &T) -> Result<String, ExportError> {
    let mut text = serde_json::to_string_pretty(value).map_err(|e| ExportError(e.to_string()))?;
    text.push('\n');
    Ok(text)
}

impl Export {
    /// The text files of the directory, by name, each kind in its own file,
    /// sorted so that the same state gives the same bytes (ADR 0074 §3).
    /// The bytes of the attachments are not in here: `files/<hash>` for
    /// each of [`Self::files`].
    pub fn to_files(&self) -> Result<Vec<(String, String)>, ExportError> {
        let mut by_kind: BTreeMap<&'static str, Vec<&ExportedObject>> = BTreeMap::new();
        for kind in EXPORT_KINDS {
            if let Some(name) = export_file(*kind) {
                by_kind.insert(name, Vec::new());
            }
        }
        for object in &self.objects {
            let kind = kind_of(&object.object);
            let name = export_file(kind)
                .ok_or_else(|| ExportError("an object of a kind this samu does not know".into()))?;
            by_kind.entry(name).or_default().push(object);
        }
        let mut manifest = self.manifest.clone();
        manifest.counts.clear();
        let mut files = Vec::new();
        for (name, mut objects) in by_kind {
            objects.sort_by_key(|o| id_of(&o.object));
            manifest
                .counts
                .insert(name.to_string(), objects.len() as u64);
            files.push((name.to_string(), pretty(&objects)?));
        }
        let mut logbook: Vec<&LogEntry> = self.logbook.iter().collect();
        // Stable: entries at the same time stay in the order they came.
        logbook.sort_by_key(|e| (e.object, e.at));
        files.push((EXPORT_LOGBOOK.into(), pretty(&logbook)?));
        let mut blobs: Vec<&Blob> = self.files.iter().collect();
        blobs.sort_by(|a, b| a.hash.cmp(&b.hash));
        blobs.dedup_by(|a, b| a.hash == b.hash);
        files.push((EXPORT_FILES.into(), pretty(&blobs)?));
        files.push((EXPORT_MANIFEST.into(), pretty(&manifest)?));
        files.push((EXPORT_README.into(), README.to_string()));
        files.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(files)
    }

    /// Reads a directory written by [`Self::to_files`]; `read` gives the
    /// text of a file by name, none if it is missing. A missing file of a
    /// kind reads as no objects of it, so an older export reads in a newer
    /// samu; a missing manifest is an error.
    pub fn from_files(mut read: impl FnMut(&str) -> Option<String>) -> Result<Self, ExportError> {
        fn parse<T: serde::de::DeserializeOwned>(name: &str, text: &str) -> Result<T, ExportError> {
            serde_json::from_str(text).map_err(|e| ExportError(format!("{name}: {e}")))
        }
        let manifest: ExportManifest = parse(
            EXPORT_MANIFEST,
            &read(EXPORT_MANIFEST)
                .ok_or_else(|| ExportError(format!("no {EXPORT_MANIFEST}: not an export")))?,
        )?;
        if manifest.format > EXPORT_FORMAT {
            return Err(ExportError(format!(
                "export format {} is newer than this samu reads ({EXPORT_FORMAT})",
                manifest.format
            )));
        }
        let mut objects = Vec::new();
        for kind in EXPORT_KINDS {
            let Some(name) = export_file(*kind) else {
                continue;
            };
            let Some(text) = read(name) else { continue };
            let read: Vec<ExportedObject> = parse(name, &text)?;
            for object in &read {
                if kind_of(&object.object) != *kind {
                    return Err(ExportError(format!(
                        "{name}: holds an object of another kind"
                    )));
                }
            }
            objects.extend(read);
        }
        let logbook = match read(EXPORT_LOGBOOK) {
            Some(text) => parse(EXPORT_LOGBOOK, &text)?,
            None => Vec::new(),
        };
        let files = match read(EXPORT_FILES) {
            Some(text) => parse(EXPORT_FILES, &text)?,
            None => Vec::new(),
        };
        Ok(Self {
            manifest,
            objects,
            logbook,
            files,
        })
    }
}

/// `README.md` of every export (ADR 0074 §1): how to read it without samu.
pub(crate) const README: &str = "\
# samudo export

This directory holds one samudo account, complete, as plain text. You can
read it without samudo: every file is JSON, pretty-printed and sorted, so a
text editor, `jq` or `git diff` is enough.

## Files

- `manifest.json`: which account, when (`exported_at`), the account's time
  zone, the export format and the version of `samu-api` the objects are in,
  and how many objects each file holds.
- One file per kind of object, each a list sorted by id (ids are UUIDv7, so
  this is the order they were made in): `folders.json`, `projects.json`,
  `tasks.json`, `tags.json`, `links.json`, `clocks.json`, `reminders.json`,
  `scopes.json`, `comments.json`, `attachments.json`, `settings.json`,
  `calendars.json`, `appointments.json`, `templates.json`, `perspectives.json`,
  `habit_entries.json`, `checklist_items.json`.
- `logbook.json`: what happened to tasks, projects and appointments, oldest
  first per object, with the device and whether a person or an agent did it.
- `files.json`: the files attachments point at, by SHA-256.
- `files/<sha256>`: the bytes of each such file. `attachments.json` names
  the file (`name`, `media_type`) and its `blob`, the hash.
- `samu.org`, if present: the same tasks as an Org outline, for reading in
  Emacs. It is written from the JSON and never read back.

## Objects

Each entry is `{\"kind\": ..., \"object\": {...}, \"sync\": {...}}`.
`object` is the object as the samudo API sends it: a task has `title`,
`note`, `project`, `parent`, `due`, `defer`, `planned`, `completed_at`,
`tags`, `properties` and so on; references to other objects are their ids.
`sync` is what the server keeps for merging changes from several devices:
the stamp of each field's last change and removed tags, properties and
scope members. You can ignore it when reading.

## Not in here

- Deleted objects.
- Sessions, passwords, hooks and pairing codes (`server.db`).
- Where calendars come from (their URL and credentials): no secrets in an
  export. After a restore, set each calendar's source again.

## Restore

`samu-server restore --account <name> <directory>` makes an account from
this directory again, with the same ids, stamps and logbook. It only
writes into an empty account: create it with `samu-server account add`
first, and stop the server while restoring.
";

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests may unwrap")]

    use super::*;

    fn fixture() -> Export {
        serde_json::from_str(include_str!("../tests/fixtures/export/account.json")).unwrap()
    }

    #[test]
    fn every_known_kind_has_a_file_of_its_own() {
        let mut names: Vec<&str> = EXPORT_KINDS
            .iter()
            .map(|k| export_file(*k).unwrap())
            .collect();
        let all = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all, "two kinds share a file");
        assert_eq!(export_file(ObjectKind::Unknown), None);
    }

    #[test]
    fn the_files_read_back_as_the_same_export() {
        let export = fixture();
        let files = export.to_files().unwrap();
        let read = Export::from_files(|name| {
            files
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, text)| text.clone())
        })
        .unwrap();
        // Read back in the order of the files, not the order they came.
        let mut want = export.clone();
        want.objects
            .sort_by_key(|o| (export_file(kind_of(&o.object)), id_of(&o.object)));
        let mut got = read.clone();
        got.objects
            .sort_by_key(|o| (export_file(kind_of(&o.object)), id_of(&o.object)));
        assert_eq!(got.objects, want.objects);
        assert_eq!(got.logbook, want.logbook);
        assert_eq!(got.files, want.files);
        // Written again, byte for byte the same.
        assert_eq!(read.to_files().unwrap(), files);
    }

    #[test]
    fn every_kind_gets_a_file_even_when_empty_and_the_manifest_counts() {
        let files = fixture().to_files().unwrap();
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        for kind in EXPORT_KINDS {
            assert!(names.contains(&export_file(*kind).unwrap()));
        }
        for name in [EXPORT_MANIFEST, EXPORT_LOGBOOK, EXPORT_FILES, EXPORT_README] {
            assert!(names.contains(&name), "{name}");
        }
        let manifest = &files.iter().find(|(n, _)| n == EXPORT_MANIFEST).unwrap().1;
        let manifest: ExportManifest = serde_json::from_str(manifest).unwrap();
        assert_eq!(manifest.counts["tasks.json"], 1);
        assert_eq!(manifest.counts["folders.json"], 0);
        assert!(files.iter().all(|(_, text)| text.ends_with('\n')));
    }

    #[test]
    fn objects_are_sorted_by_id_whatever_order_they_came_in() {
        let mut export = fixture();
        let sorted = export.to_files().unwrap();
        export.objects.reverse();
        export.logbook.reverse();
        assert_eq!(export.to_files().unwrap(), sorted);
    }

    #[test]
    fn a_directory_without_manifest_or_from_a_newer_samu_is_refused() {
        assert!(Export::from_files(|_| None).is_err());
        let mut export = fixture();
        export.manifest.format = EXPORT_FORMAT + 1;
        let files = export.to_files().unwrap();
        let err = Export::from_files(|name| {
            files
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, t)| t.clone())
        })
        .unwrap_err();
        assert!(err.0.contains("newer"), "{err}");
    }

    #[test]
    fn an_object_in_the_file_of_another_kind_is_refused() {
        let files = fixture().to_files().unwrap();
        let tasks = files
            .iter()
            .find(|(n, _)| n == "tasks.json")
            .unwrap()
            .1
            .clone();
        let err = Export::from_files(|name| match name {
            "tags.json" => Some(tasks.clone()),
            _ => files
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, t)| t.clone()),
        })
        .unwrap_err();
        assert!(err.0.contains("tags.json"), "{err}");
    }
}
