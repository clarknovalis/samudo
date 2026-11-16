// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Export and restore of an account (ADR 0074 §2, §4, §6).
//!
//! The export reads every live object with what the server keeps beside it
//! (stamps, change numbers, author, removed set elements) and the logbook,
//! in one read transaction. The restore writes all of that back into an
//! empty account, past the rules of `/changes`: only here, and only into an
//! account nothing has been written to.

use std::collections::{BTreeMap, HashMap};

use jiff::Timestamp;
use rusqlite::params;
use serde_json::Value as Json;

use super::{AccountStore, as_seq, author_kind_name, author_kind_of, uuid_blob, write_row};
use crate::core::api::{
    AnyObject, Author, Blob, Export, ExportManifest, ExportedObject, Hlc, Id, LogEntry,
    MemberStamp, ObjectSync, PropertyStamp, SyncStamp, TagStamp, id_of, kind_of,
};
use crate::rows::{self, Object};
use crate::{Error, Result};

fn corrupt(what: impl Into<String>) -> Error {
    Error::Corrupt { what: what.into() }
}

fn id_from(bytes: &[u8]) -> Result<Id> {
    uuid::Uuid::from_slice(bytes)
        .map(Id::from_uuid)
        .map_err(|e| corrupt(format!("uuid: {e}")))
}

fn stamp(wall: i64, counter: i64, device: String, seq: i64) -> Result<SyncStamp> {
    Ok(SyncStamp {
        wall,
        counter: u32::try_from(counter).map_err(|e| corrupt(format!("counter: {e}")))?,
        device,
        seq: as_seq(seq),
    })
}

fn signed(seq: u64) -> Result<i64> {
    i64::try_from(seq).map_err(|e| Error::Unsupported {
        what: format!("change number {seq}: {e}"),
    })
}

/// Why a restore did not write anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreRefused {
    /// The account has objects or a logbook already (ADR 0074 §6).
    NotEmpty,
    /// The export does not fit this samu: a kind it does not know, an
    /// object that does not read, a logbook entry of a missing object.
    Invalid(String),
}

impl std::fmt::Display for RestoreRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotEmpty => f.write_str("the account is not empty"),
            Self::Invalid(why) => write!(f, "the export does not restore: {why}"),
        }
    }
}

impl AccountStore {
    /// The whole account as [`Export`] (ADR 0074 §2): every live object
    /// with its stamps, the logbook and the files attachments point at.
    /// Read in one transaction, so it is one state even while the server
    /// writes. `counts` is filled in by [`Export::to_files`].
    pub fn export(&self, account: &str, now: Timestamp) -> Result<Export> {
        let tx = self.conn.unchecked_transaction()?;
        let (seq, zone, due_soon, remind_time, wall, counter, device): (
            i64,
            String,
            String,
            String,
            i64,
            i64,
            String,
        ) = tx.query_row(
            "SELECT seq, zone, due_soon, remind_time, observed_wall, observed_counter, \
             observed_device FROM account WHERE id = 1",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )?;
        let observed = (wall > 0)
            .then(|| -> Result<Hlc> {
                Ok(Hlc::new(
                    wall,
                    u32::try_from(counter).map_err(|e| corrupt(format!("observed: {e}")))?,
                    device,
                ))
            })
            .transpose()?;

        // What the object table knows, by rowid.
        let mut heads: HashMap<i64, (Id, ObjectSync)> = HashMap::new();
        {
            let mut st = tx.prepare(
                "SELECT id, uuid, seq, author_device, author_kind FROM object \
                 WHERE deleted_at IS NULL",
            )?;
            let rows = st.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<String>>(4)?,
                ))
            })?;
            for row in rows {
                let (rowid, uuid, seq, device, kind) = row?;
                let author = device.zip(kind).map(|(device, kind)| Author {
                    device,
                    kind: author_kind_of(&kind),
                });
                let sync = ObjectSync {
                    seq: as_seq(seq),
                    author,
                    ..ObjectSync::default()
                };
                heads.insert(rowid, (id_from(&uuid)?, sync));
            }
        }
        {
            let mut st = tx.prepare(
                "SELECT object, field, wall, counter, device, seq FROM stamp ORDER BY object, field",
            )?;
            let rows = st.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, i64>(5)?,
                ))
            })?;
            for row in rows {
                let (owner, field, wall, counter, device, seq) = row?;
                if let Some((_, sync)) = heads.get_mut(&owner) {
                    sync.stamps
                        .insert(field, stamp(wall, counter, device, seq)?);
                }
            }
        }
        {
            let mut st = tx.prepare(
                "SELECT owner, tag, position, present, wall, counter, device, seq FROM tagging \
                 ORDER BY owner, position, tag",
            )?;
            let rows = st.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, bool>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, i64>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, i64>(7)?,
                ))
            })?;
            for row in rows {
                let (owner, tag, position, present, wall, counter, device, seq) = row?;
                if let Some((_, sync)) = heads.get_mut(&owner) {
                    sync.tags.push(TagStamp {
                        tag: id_from(&tag)?,
                        position,
                        present,
                        stamp: stamp(wall, counter, device, seq)?,
                    });
                }
            }
        }
        {
            let mut st = tx.prepare(
                "SELECT owner, key, value, wall, counter, device, seq FROM property \
                 ORDER BY owner, key",
            )?;
            let rows = st.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            })?;
            for row in rows {
                let (owner, key, value, wall, counter, device, seq) = row?;
                if let Some((_, sync)) = heads.get_mut(&owner) {
                    sync.properties.push(PropertyStamp {
                        key,
                        value,
                        stamp: stamp(wall, counter, device, seq)?,
                    });
                }
            }
        }
        {
            // In the order the scope shows them (`members` in store.rs).
            let mut st = tx.prepare(
                "SELECT owner, member, present, wall, counter, device, seq FROM scope_member \
                 ORDER BY owner, seq, member",
            )?;
            let rows = st.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Vec<u8>>(1)?,
                    r.get::<_, bool>(2)?,
                    r.get::<_, i64>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            })?;
            for row in rows {
                let (owner, member, present, wall, counter, device, seq) = row?;
                if let Some((_, sync)) = heads.get_mut(&owner) {
                    sync.members.push(MemberStamp {
                        member: id_from(&member)?,
                        present,
                        stamp: stamp(wall, counter, device, seq)?,
                    });
                }
            }
        }

        let mut by_id: HashMap<Id, ObjectSync> =
            heads.into_values().collect::<HashMap<Id, ObjectSync>>();
        let mut objects = Vec::new();
        let mut logbook = Vec::new();
        for object in super::load_after(&tx, 0, true)? {
            let id = object.id();
            let sync = by_id
                .remove(&id)
                .ok_or_else(|| corrupt(format!("{id}: read but not in the object table")))?;
            if let Some(entries) = self.logbook(id)? {
                logbook.extend(entries);
            }
            objects.push(ExportedObject {
                object: AnyObject::from(object),
                sync,
            });
        }

        let mut files = Vec::new();
        {
            let mut st = tx.prepare(
                "SELECT DISTINCT b.hash, b.size, b.media_type FROM blob b \
                 JOIN attachment a ON a.blob = b.hash \
                 JOIN object o ON o.id = a.object AND o.deleted_at IS NULL \
                 ORDER BY b.hash",
            )?;
            let rows = st.query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
            for row in rows {
                let (hash, size, media_type) = row?;
                files.push(Blob {
                    hash,
                    size: u64::try_from(size).unwrap_or(0),
                    media_type,
                });
            }
        }
        tx.finish()?;

        Ok(Export {
            manifest: ExportManifest {
                format: crate::core::api::EXPORT_FORMAT,
                api: crate::core::api::VERSION.to_string(),
                account: account.to_string(),
                exported_at: now,
                seq: as_seq(seq),
                zone,
                due_soon,
                remind_time,
                observed,
                counts: BTreeMap::new(),
            },
            objects,
            logbook,
            files,
        })
    }

    /// Whether nothing has been written to the account: no object but the
    /// settings the server makes on opening, no logbook (ADR 0074 §6).
    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT NOT EXISTS (SELECT 1 FROM object WHERE kind != 'settings') \
             AND NOT EXISTS (SELECT 1 FROM logbook) \
             AND NOT EXISTS (SELECT 1 FROM stamp WHERE device != 'server')",
            [],
            |r| r.get(0),
        )?)
    }

    /// Writes an export into this account as it was: ids, fields, stamps,
    /// change numbers, authors, logbook (ADR 0074 §6). The bytes of the
    /// files must already be in the account's blob directory; this records
    /// them. All or nothing; refuses an account that is not
    /// [empty](Self::is_empty).
    pub fn restore(&mut self, export: &Export) -> Result<Result<(), RestoreRefused>> {
        if !self.is_empty()? {
            return Ok(Err(RestoreRefused::NotEmpty));
        }
        let tx = self.conn.transaction()?;
        match restore_in(&tx, export)? {
            Ok(()) => {
                tx.commit()?;
                Ok(Ok(()))
            }
            Err(refused) => Ok(Err(refused)),
        }
    }
}

fn restore_in(
    tx: &rusqlite::Transaction<'_>,
    export: &Export,
) -> Result<Result<(), RestoreRefused>> {
    let invalid = |why: String| Ok(Err(RestoreRefused::Invalid(why)));
    let m = &export.manifest;
    if jiff::tz::TimeZone::get(&m.zone).is_err() {
        return invalid(format!("unknown time zone {:?}", m.zone));
    }
    // The settings the server made on opening give way to the exported.
    tx.execute("DELETE FROM object", [])?;
    let observed = m.observed.clone().unwrap_or_else(|| Hlc::new(0, 0, ""));
    tx.execute(
        "UPDATE account SET seq = ?1, zone = ?2, due_soon = ?3, remind_time = ?4, \
         observed_wall = ?5, observed_counter = ?6, observed_device = ?7 WHERE id = 1",
        params![
            signed(m.seq)?,
            m.zone,
            m.due_soon,
            m.remind_time,
            observed.wall,
            observed.counter,
            observed.device
        ],
    )?;

    // The files first: an attachment points at its file (ADR 0039 §5).
    let mut hashes = std::collections::HashSet::new();
    for blob in &export.files {
        if !crate::valid_hash(&blob.hash) {
            return invalid(format!("{:?} is not a file hash", blob.hash));
        }
        hashes.insert(blob.hash.as_str());
        tx.execute(
            "INSERT INTO blob (hash, size, media_type, created) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT (hash) DO NOTHING",
            params![
                blob.hash,
                signed(blob.size)?,
                blob.media_type,
                m.exported_at.to_string()
            ],
        )?;
    }
    for exported in &export.objects {
        if let AnyObject::Attachment(a) = &exported.object
            && !hashes.contains(a.blob.as_str())
        {
            return invalid(format!(
                "attachment {} points at {}, which is not in files.json",
                a.id, a.blob
            ));
        }
    }

    let mut rowids: HashMap<Id, i64> = HashMap::new();
    for exported in &export.objects {
        let kind = kind_of(&exported.object);
        let Some(id) = id_of(&exported.object) else {
            return invalid("an object of a kind this samu does not know".into());
        };
        let Some(kind_name) = rows::kind_name(kind) else {
            return invalid(format!("{id}: a kind this samu does not know"));
        };
        let json =
            serde_json::to_value(&exported.object).map_err(|e| corrupt(format!("{id}: {e}")))?;
        let Some(inner) = json.get("object").cloned() else {
            return invalid(format!("{id}: no object"));
        };
        let text = |key: &str| inner.get(key).and_then(Json::as_str).map(str::to_string);
        let (Some(created), Some(modified)) = (text("created"), text("modified")) else {
            return invalid(format!("{id}: no created or modified"));
        };
        let object = match Object::from_json(kind, inner) {
            Ok(object) => object,
            Err(why) => return invalid(format!("{id}: {why}")),
        };
        let sync = &exported.sync;
        let (device, author_kind) = match &sync.author {
            Some(a) => (Some(a.device.clone()), author_kind_name(a.kind)),
            None => (None, None),
        };
        tx.execute(
            "INSERT INTO object (uuid, kind, created, modified, seq, author_device, author_kind) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                uuid_blob(id),
                kind_name,
                created,
                modified,
                signed(sync.seq)?,
                device,
                author_kind
            ],
        )?;
        let rowid = tx.last_insert_rowid();
        if rowids.insert(id, rowid).is_some() {
            return invalid(format!("{id} is in the export twice"));
        }
        if let Err(why) = write_row(tx, rowid, &object, true)? {
            return invalid(format!("{id}: {why}"));
        }
        for (field, s) in &sync.stamps {
            tx.execute(
                "INSERT INTO stamp (object, field, wall, counter, device, seq) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![rowid, field, s.wall, s.counter, s.device, signed(s.seq)?],
            )?;
        }
        for t in &sync.tags {
            let s = &t.stamp;
            tx.execute(
                "INSERT INTO tagging (owner, tag, position, present, wall, counter, device, seq) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    rowid,
                    uuid_blob(t.tag),
                    t.position,
                    t.present,
                    s.wall,
                    s.counter,
                    s.device,
                    signed(s.seq)?
                ],
            )?;
        }
        for p in &sync.properties {
            let s = &p.stamp;
            tx.execute(
                "INSERT INTO property (owner, key, value, wall, counter, device, seq) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    rowid,
                    p.key,
                    p.value,
                    s.wall,
                    s.counter,
                    s.device,
                    signed(s.seq)?
                ],
            )?;
        }
        for member in &sync.members {
            let s = &member.stamp;
            tx.execute(
                "INSERT INTO scope_member (owner, member, present, wall, counter, device, seq) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    rowid,
                    uuid_blob(member.member),
                    member.present,
                    s.wall,
                    s.counter,
                    s.device,
                    signed(s.seq)?
                ],
            )?;
        }
    }

    for entry in &export.logbook {
        let Some(rowid) = rowids.get(&entry.object) else {
            return invalid(format!(
                "a logbook entry of {}, which is not exported",
                entry.object
            ));
        };
        let (old, new) = log_values(entry)?;
        let kind = match serde_json::to_value(entry.kind) {
            Ok(Json::String(kind)) => kind,
            _ => return invalid(format!("a logbook entry of {} of no kind", entry.object)),
        };
        tx.execute(
            "INSERT INTO logbook (object, at, kind, device, note, old_value, new_value, \
             author_kind, field) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                rowid,
                entry.at.to_string(),
                kind,
                entry.device,
                entry.note,
                old,
                new,
                entry.author_kind.and_then(author_kind_name),
                entry.field
            ],
        )?;
    }

    Ok(Ok(()))
}

/// The two value columns of a logbook row as the store writes them: ids for
/// a state, JSON for a field of an appointment (`logbook` in store.rs).
fn log_values(entry: &LogEntry) -> Result<(Option<String>, Option<String>)> {
    if entry.field.is_some() {
        let text = |v: &Option<Json>| v.as_ref().map(Json::to_string);
        Ok((text(&entry.old_value), text(&entry.new_value)))
    } else {
        Ok((
            entry.old.map(|id| id.to_string()),
            entry.new.map(|id| id.to_string()),
        ))
    }
}
