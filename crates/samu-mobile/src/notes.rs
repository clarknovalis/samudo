// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Comments and attachments on the phone (ADR 0039 §6).
//!
//! A comment is an ordinary object through the outbox. A file taken on the
//! phone is kept under `blobs/<sha256>` next to the database and waits in
//! `blob_outbox`; [`Samu::sync`] sends it up before the outbox, so the
//! attachment that names it finds it on the server. Files fetched from the
//! server land in the same place and stay: a blob never changes.
//!
//! Who may change a comment or an attachment is the store's rule (ADR 0039
//! §3): only a session of the kind that wrote it. The phone asks the same
//! question first, so the app only offers what the server will take.

use std::io::Write;
use std::path::{Path, PathBuf};

use jiff::Timestamp;
use samu_api::{Attachment, Author, AuthorKind, Change, Comment, Id, ObjectKind};
use samu_core::overlay::Objects;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::{MobileError, Samu, input, parse_id};
use samu_replica::Cache;

/// What the server keeps at most per file (ADR 0039 §4).
pub(crate) const MAX_BYTES: u64 = 50 * 1024 * 1024;

/// One comment, oldest first, with its files.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CommentRow {
    pub id: String,
    pub body: String,
    /// Milliseconds since the epoch.
    pub created_ms: i64,
    /// Whether it was changed after it was written.
    pub edited: bool,
    /// The device that wrote it; none from before ADR 0039.
    pub author: Option<String>,
    /// Written by an agent: the app marks it.
    pub agent: bool,
    /// This session may edit and delete it.
    pub changeable: bool,
    /// Still in the outbox.
    pub pending: bool,
    pub attachments: Vec<AttachmentRow>,
}

/// One file on a task, a project or a comment.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AttachmentRow {
    pub id: String,
    pub name: String,
    pub media_type: String,
    pub size: u64,
    /// The SHA-256 of the file, for [`Samu::fetch_blob`].
    pub hash: String,
    /// Where the file lies on the phone, if it is already here.
    pub local: Option<String>,
    pub agent: bool,
    pub changeable: bool,
    /// The attachment or its file has not reached the server yet.
    pub pending: bool,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn store_error(e: impl std::fmt::Display) -> MobileError {
    MobileError::Store {
        reason: e.to_string(),
    }
}

fn kind_of(author: Option<&Author>) -> AuthorKind {
    author.map_or(AuthorKind::Human, |a| a.kind)
}

fn ms(at: Timestamp) -> i64 {
    at.as_millisecond()
}

/// The session's own kind, as the server said; a phone is human unless
/// told otherwise (ADR 0039 §2).
fn own_kind(cache: &Cache) -> Result<AuthorKind, MobileError> {
    Ok(match cache.meta("kind")?.as_deref() {
        Some("agent") => AuthorKind::Agent,
        _ => AuthorKind::Human,
    })
}

/// A file with this hash, written whole or not at all.
fn keep(dir: &Path, hash: &str, bytes: &[u8]) -> Result<PathBuf, MobileError> {
    std::fs::create_dir_all(dir).map_err(store_error)?;
    let path = dir.join(hash);
    let part = dir.join(format!("{hash}.part"));
    let mut file = std::fs::File::create(&part).map_err(store_error)?;
    file.write_all(bytes).map_err(store_error)?;
    file.sync_all().map_err(store_error)?;
    std::fs::rename(&part, &path).map_err(store_error)?;
    Ok(path)
}

impl Samu {
    pub(crate) fn blobs(&self) -> PathBuf {
        self.dir.join("blobs")
    }

    fn local(&self, hash: &str) -> Option<String> {
        let path = self.blobs().join(hash);
        path.exists().then(|| path.display().to_string())
    }

    fn attachment_row(
        &self,
        a: &Attachment,
        own: AuthorKind,
        touched: &[Id],
        waiting: &[String],
    ) -> AttachmentRow {
        let kind = kind_of(a.created_by.as_ref());
        AttachmentRow {
            id: a.id.to_string(),
            name: a.name.clone(),
            media_type: a.media_type.clone(),
            size: a.size,
            hash: a.blob.clone(),
            local: self.local(&a.blob),
            agent: kind == AuthorKind::Agent,
            changeable: kind == own,
            pending: touched.contains(&a.id) || waiting.contains(&a.blob),
        }
    }

    /// The files waiting to go up, by hash.
    fn waiting(cache: &Cache) -> Result<Vec<String>, MobileError> {
        Ok(cache
            .blob_outbox()?
            .into_iter()
            .map(|(h, _, _)| h)
            .collect())
    }

    fn owner_exists(objects: &Objects, owner: Id, comments_too: bool) -> bool {
        objects.tasks.iter().any(|t| t.id == owner)
            || objects.projects.iter().any(|p| p.id == owner)
            || (comments_too && objects.comments.iter().any(|c| c.id == owner))
    }

    /// The change to a comment or attachment, if this session may make it.
    fn may_change(cache: &Cache, objects: &Objects, id: Id) -> Result<(), MobileError> {
        let author = objects
            .comments
            .iter()
            .find(|c| c.id == id)
            .map(|c| c.created_by.clone())
            .or_else(|| {
                objects
                    .attachments
                    .iter()
                    .find(|a| a.id == id)
                    .map(|a| a.created_by.clone())
            })
            .ok_or_else(|| input(format!("no comment or attachment {id}")))?;
        if kind_of(author.as_ref()) == own_kind(cache)? {
            Ok(())
        } else {
            Err(input(
                "only a session of the kind that wrote it may change it",
            ))
        }
    }
}

/// A note or comment as the tree of ADR 0058, in JSON: the app reads it
/// with `org.json`, and a node it does not know it leaves out. `task`
/// links are looked up among the objects here, the cache with what waits
/// in the outbox: `samu:` by id, `id:` by `org_id`, then by id.
fn render_json(text: &str, objects: &Objects) -> String {
    use samu_core::render::{render_text, resolve_links};
    let mut document = render_text(text, None);
    // The same lookup as the server's (ADR 0077 §5).
    resolve_links(&mut document, &samu_core::views::linked_objects(objects));
    // A tree of strings, numbers and lists always serialises.
    serde_json::to_string(&document).unwrap_or_else(|_| "{\"blocks\":[]}".into())
}

/// The text with the checkbox at `start..end` switched (ADR 0058 §6).
fn toggle(text: &str, start: u64, end: u64) -> Option<String> {
    let span = samu_api::Span {
        start: usize::try_from(start).ok()?,
        end: usize::try_from(end).ok()?,
    };
    samu_core::render::toggle_checkbox(text, span)
}

#[uniffi::export]
impl Samu {
    /// A note or comment rendered (ADR 0058 §7), as JSON of the tree, with
    /// task links found in the cache, offline as well.
    pub fn render_note(&self, text: String) -> Result<String, MobileError> {
        let cache = self.cache()?;
        let (objects, _) = Self::view(&cache)?;
        Ok(render_json(&text, &objects))
    }

    /// The text with the checkbox at `start..end` switched, the place in
    /// code points as the tree gives it; none when no checkbox is there.
    pub fn toggle_checkbox(&self, text: String, start: u64, end: u64) -> Option<String> {
        toggle(&text, start, end)
    }

    /// The comments on a task or project, oldest first, each with its
    /// files.
    pub fn comments(&self, owner: String) -> Result<Vec<CommentRow>, MobileError> {
        let owner = parse_id(&owner)?;
        let cache = self.cache()?;
        let own = own_kind(&cache)?;
        let waiting = Self::waiting(&cache)?;
        let (objects, touched) = Self::view(&cache)?;
        let mut found: Vec<&Comment> = objects
            .comments
            .iter()
            .filter(|c| c.owner == owner)
            .collect();
        found.sort_by_key(|c| (c.created, c.id));
        Ok(found
            .into_iter()
            .map(|c| {
                let kind = kind_of(c.created_by.as_ref());
                CommentRow {
                    id: c.id.to_string(),
                    body: c.body.clone(),
                    created_ms: ms(c.created),
                    edited: c.modified > c.created,
                    author: c.created_by.as_ref().map(|a| a.device.clone()),
                    agent: kind == AuthorKind::Agent,
                    changeable: kind == own,
                    pending: touched.contains(&c.id),
                    attachments: self.attachments_of(&objects, c.id, own, &touched, &waiting),
                }
            })
            .collect())
    }

    /// The files on a task or project itself, not those on its comments.
    pub fn attachments(&self, owner: String) -> Result<Vec<AttachmentRow>, MobileError> {
        let owner = parse_id(&owner)?;
        let cache = self.cache()?;
        let own = own_kind(&cache)?;
        let waiting = Self::waiting(&cache)?;
        let (objects, touched) = Self::view(&cache)?;
        Ok(self.attachments_of(&objects, owner, own, &touched, &waiting))
    }

    /// A comment on a task or project, through the outbox.
    pub fn add_comment(&self, owner: String, body: String) -> Result<String, MobileError> {
        let owner = parse_id(&owner)?;
        let body = body.trim().to_string();
        if body.is_empty() {
            return Err(input("a comment needs a text"));
        }
        let now = Timestamp::now();
        let comment = Comment {
            id: Id::new(),
            created: now,
            modified: now,
            // The server sets it from the session (ADR 0039 §1).
            created_by: None,
            owner,
            body,
        };
        let id = comment.id;
        let value = serde_json::to_value(&comment).map_err(|e| input(e.to_string()))?;
        self.queue(|objects, stamp| {
            if !Self::owner_exists(objects, owner, false) {
                return Err(input(format!("no task or project {owner}")));
            }
            Ok(vec![Change::Create {
                object: id,
                kind: ObjectKind::Comment,
                value,
                hlc: stamp(),
            }])
        })?;
        Ok(id.to_string())
    }

    /// A comment's new text.
    pub fn edit_comment(&self, id: String, body: String) -> Result<(), MobileError> {
        let id = parse_id(&id)?;
        let body = body.trim().to_string();
        if body.is_empty() {
            return Err(input("a comment needs a text"));
        }
        {
            let cache = self.cache()?;
            let (objects, _) = Self::view(&cache)?;
            Self::may_change(&cache, &objects, id)?;
        }
        self.queue(|objects, stamp| {
            if !objects.comments.iter().any(|c| c.id == id) {
                return Err(input(format!("no comment {id}")));
            }
            Ok(vec![Change::Set {
                object: id,
                field: "body".into(),
                value: Value::String(body),
                note: None,
                hlc: stamp(),
            }])
        })
    }

    /// A comment or an attachment gone; the file itself stays (ADR 0039 §5).
    pub fn delete_note(&self, id: String) -> Result<(), MobileError> {
        let id = parse_id(&id)?;
        {
            let cache = self.cache()?;
            let (objects, _) = Self::view(&cache)?;
            Self::may_change(&cache, &objects, id)?;
        }
        self.queue(|_, stamp| {
            Ok(vec![Change::Delete {
                object: id,
                hlc: stamp(),
            }])
        })
    }

    /// A file from the phone onto a task, a project or a comment. The file
    /// at `path` is moved into the phone's store of files and waits there
    /// until the next sync sends it up; the attachment goes through the
    /// outbox after it.
    pub fn attach_file(
        &self,
        owner: String,
        path: String,
        name: String,
        media_type: String,
    ) -> Result<String, MobileError> {
        let owner = parse_id(&owner)?;
        let bytes = std::fs::read(&path).map_err(|e| input(format!("{path}: {e}")))?;
        let size = bytes.len() as u64;
        if size > MAX_BYTES {
            return Err(input(format!(
                "{name} has {size} bytes; the server keeps at most {MAX_BYTES}"
            )));
        }
        if size == 0 {
            return Err(input(format!("{name} is empty")));
        }
        let media_type = if media_type.trim().is_empty() {
            "application/octet-stream".to_string()
        } else {
            media_type.trim().to_string()
        };
        let hash = hex(&Sha256::digest(&bytes));
        keep(&self.blobs(), &hash, &bytes)?;
        let _ = std::fs::remove_file(&path);
        let now = Timestamp::now();
        let attachment = Attachment {
            id: Id::new(),
            created: now,
            modified: now,
            created_by: None,
            owner,
            blob: hash.clone(),
            name: if name.trim().is_empty() {
                "file".into()
            } else {
                name.trim().to_string()
            },
            media_type: media_type.clone(),
            size,
        };
        let id = attachment.id;
        let value = serde_json::to_value(&attachment).map_err(|e| input(e.to_string()))?;
        self.cache()?.push_blob(&hash, &media_type, size)?;
        self.queue(|objects, stamp| {
            if !Self::owner_exists(objects, owner, true) {
                return Err(input(format!("no task, project or comment {owner}")));
            }
            Ok(vec![Change::Create {
                object: id,
                kind: ObjectKind::Attachment,
                value,
                hlc: stamp(),
            }])
        })?;
        Ok(id.to_string())
    }

    /// Where the file of a blob lies on the phone; fetched from the server
    /// the first time and kept.
    pub fn fetch_blob(&self, hash: String) -> Result<String, MobileError> {
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(input(format!("{hash:?} is no SHA-256")));
        }
        let hash = hash.to_ascii_lowercase();
        if let Some(path) = self.local(&hash) {
            return Ok(path);
        }
        let client = self.client()?;
        let answer = client.call("GET", &format!("/blobs/{hash}"), None);
        let answer = self.signed_out_on_401(answer)?;
        if hex(&Sha256::digest(&answer.body)) != hash {
            return Err(MobileError::Refused {
                reason: format!("the server sent another file for {hash}"),
            });
        }
        Ok(keep(&self.blobs(), &hash, &answer.body)?
            .display()
            .to_string())
    }
}

impl Samu {
    fn attachments_of(
        &self,
        objects: &Objects,
        owner: Id,
        own: AuthorKind,
        touched: &[Id],
        waiting: &[String],
    ) -> Vec<AttachmentRow> {
        let mut found: Vec<&Attachment> = objects
            .attachments
            .iter()
            .filter(|a| a.owner == owner)
            .collect();
        found.sort_by_key(|a| (a.created, a.id));
        found
            .into_iter()
            .map(|a| self.attachment_row(a, own, touched, waiting))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use samu_api::Task;

    #[test]
    fn a_note_renders_with_its_task_links_from_the_cache() {
        let mut objects = Objects::default();
        let task: Task = serde_json::from_value(serde_json::json!({
            "id": "01a1c1d1-be00-7364-932c-f70b4e8c4b53",
            "created": "2026-11-07T08:00:00Z",
            "modified": "2026-11-07T08:00:00Z",
            "title": "Ziel",
            "flagged": false,
            "sequential": false,
            "complete_with_last_child": false,
            "position": 0,
            "properties": {"org_id": "abc"}
        }))
        .unwrap_or_else(|e| panic!("{e}"));
        objects.tasks.push(task);
        let json = render_json("#+title: x\n- [ ] [[id:abc][a]] [[id:nope][b]]", &objects);
        let value: Value = serde_json::from_str(&json).unwrap_or_default();
        assert_eq!(value["format"], "org");
        let item = &value["blocks"][1]["items"][0];
        assert_eq!(item["box"], serde_json::json!({"start": 13, "end": 16}));
        let links = &item["blocks"][0]["content"];
        assert_eq!(links[0]["task"], "01a1c1d1-be00-7364-932c-f70b4e8c4b53");
        assert!(links[2].get("task").is_none(), "{links}");
    }

    #[test]
    fn a_checkbox_switches_by_its_place() {
        assert_eq!(toggle("- [ ] a", 2, 5).as_deref(), Some("- [x] a"));
        assert_eq!(toggle("- [ ] a", 0, 3), None);
    }
}
