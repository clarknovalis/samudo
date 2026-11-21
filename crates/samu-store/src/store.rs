// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The account database.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use jiff::Timestamp;
use rusqlite::types::Value as Sql;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::Value as Json;

use crate::core::api::{
    AccountLogEntry, Author, AuthorKind, Change, ChangeOutcome, Clock, Hlc, Id, Link, LogEntry,
    LogKind, ObjectKind, Project, Reminder, ReminderAnchor, Scope, Tagging, Task,
};
use crate::core::availability::Settings;
use crate::core::clock::wins;
use crate::core::report::FieldStamp;
use crate::migrations::{self, MigrationReport};
use crate::rows::{self, Object};
use crate::{DatabaseLock, Error, Result, open};

mod calendar;
mod export;
mod transcript;

pub use calendar::{Fetched, KnownResource, Pending, SecretKey, StoredSource, Synced, WriteJob};
pub use export::RestoreRefused;
pub use transcript::{Asked, Transcribed};

/// One account's database, open for writing.
#[derive(Debug)]
pub struct AccountStore {
    conn: Connection,
    /// Held as long as the store lives; `None` in memory.
    _lock: Option<DatabaseLock>,
    migration: MigrationReport,
    /// The last snapshot read and what the database looked like then
    /// ([`Self::written`]): every view asks for all objects, and between two
    /// writes they are the same.
    snapshot: RefCell<Option<(Written, Arc<OwnedSnapshot>)>>,
}

/// What tells one state of the database from the next: the rows this
/// connection has changed so far, and SQLite's count of commits by any
/// other connection.
type Written = (u64, i64);

/// Every live object of an account, owned; borrow it with
/// [`Objects::snapshot`](crate::core::overlay::Objects::snapshot) for
/// `samu_core::availability::evaluate`. The same type a client's mirror
/// is (ADR 0066 §4). Appointments are not in here: there are many, and few
/// views want them; [`AccountStore::appointments`] reads them.
pub type OwnedSnapshot = crate::core::overlay::Objects;

/// One object changed after a given change number: the ground for
/// `GET /changes?after=` (ADR 0004 §5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changed {
    pub id: Id,
    pub seq: u64,
    /// A tombstone: the object is gone.
    pub deleted: bool,
    /// Created after the change number asked about.
    pub created: bool,
}

/// What `GET /logbook` asks for (ADR 0123 §4).
#[derive(Debug, Clone)]
pub struct LogFilter {
    pub since: Timestamp,
    pub until: Timestamp,
    /// None: every kind.
    pub kinds: Vec<LogKind>,
    pub device: Option<String>,
    pub limit: usize,
}

/// What the `object` table knows about one object.
struct Head {
    rowid: i64,
    kind: ObjectKind,
    created: String,
    seq: i64,
    deleted: bool,
    /// Who made it (ADR 0039 §1); none for what was made before.
    author: Option<Author>,
}

/// A stamp as stored, with the change number it was applied at.
struct Stored {
    hlc: Hlc,
    seq: i64,
}

/// A refusal that rolls the transaction back.
type Refused = String;

/// A removal that found nothing is done too.
fn remove_if_there(result: std::io::Result<()>) -> Result<()> {
    match result {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

fn rejected(reason: impl Into<String>) -> ChangeOutcome {
    ChangeOutcome::Rejected {
        reason: reason.into(),
    }
}

fn as_seq(seq: i64) -> u64 {
    u64::try_from(seq).unwrap_or(0)
}

fn uuid_blob(id: Id) -> Vec<u8> {
    id.uuid().as_bytes().to_vec()
}

/// A CHECK or NOT NULL that fails is a value this store does not take, not a
/// broken database: the change is rejected.
fn refused_by_constraint(err: &rusqlite::Error) -> Option<String> {
    match err {
        rusqlite::Error::SqliteFailure(e, message)
            if e.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            Some(
                message
                    .clone()
                    .unwrap_or_else(|| "a constraint failed".into()),
            )
        }
        _ => None,
    }
}

fn head(tx: &Connection, id: Id) -> Result<Option<Head>> {
    Ok(tx
        .query_row(
            "SELECT id, kind, created, seq, deleted_at IS NOT NULL, author_device, author_kind \
             FROM object WHERE uuid = ?1",
            [uuid_blob(id)],
            |row| {
                let device: Option<String> = row.get(5)?;
                let kind: Option<String> = row.get(6)?;
                Ok(Head {
                    rowid: row.get(0)?,
                    kind: rows::kind_of(&row.get::<_, String>(1)?),
                    created: row.get(2)?,
                    seq: row.get(3)?,
                    deleted: row.get(4)?,
                    author: device.zip(kind).map(|(device, kind)| Author {
                        device,
                        kind: author_kind_of(&kind),
                    }),
                })
            },
        )
        .optional()?)
}

/// The kind as stored in `object.author_kind`, `session.kind` and
/// `logbook.author_kind`.
pub(crate) fn author_kind_name(kind: AuthorKind) -> Option<&'static str> {
    match kind {
        AuthorKind::Human => Some("human"),
        AuthorKind::Agent => Some("agent"),
        AuthorKind::Unknown => None,
    }
}

pub(crate) fn author_kind_of(name: &str) -> AuthorKind {
    match name {
        "human" => AuthorKind::Human,
        "agent" => AuthorKind::Agent,
        _ => AuthorKind::Unknown,
    }
}

fn stamp_of(tx: &Connection, rowid: i64, field: &str) -> Result<Option<Stored>> {
    Ok(tx
        .query_row(
            "SELECT wall, counter, device, seq FROM stamp WHERE object = ?1 AND field = ?2",
            params![rowid, field],
            |row| {
                Ok(Stored {
                    hlc: Hlc::new(row.get(0)?, row.get(1)?, row.get::<_, String>(2)?),
                    seq: row.get(3)?,
                })
            },
        )
        .optional()?)
}

fn write_stamp(tx: &Connection, rowid: i64, field: &str, hlc: &Hlc, seq: i64) -> Result<()> {
    tx.execute(
        "INSERT INTO stamp (object, field, wall, counter, device, seq) VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT (object, field) DO UPDATE SET wall = ?3, counter = ?4, device = ?5, seq = ?6",
        params![rowid, field, hlc.wall, hlc.counter, hlc.device, seq],
    )?;
    Ok(())
}

fn next_seq(tx: &Connection) -> Result<i64> {
    Ok(tx.query_row(
        "UPDATE account SET seq = seq + 1 WHERE id = 1 RETURNING seq",
        [],
        |row| row.get(0),
    )?)
}

/// Keeps the highest stamp seen (catastr ADR 0029); read when the server
/// stamps a change of its own.
fn observe(tx: &Connection, hlc: &Hlc) -> Result<()> {
    tx.execute(
        "UPDATE account SET observed_wall = ?1, observed_counter = ?2, observed_device = ?3 \
         WHERE id = 1 AND (observed_wall, observed_counter, observed_device) < (?1, ?2, ?3)",
        params![hlc.wall, hlc.counter, hlc.device],
    )?;
    Ok(())
}

fn observed_in(conn: &Connection) -> Result<Option<Hlc>> {
    let hlc: Hlc = conn.query_row(
        "SELECT observed_wall, observed_counter, observed_device FROM account WHERE id = 1",
        [],
        |row| Ok(Hlc::new(row.get(0)?, row.get(1)?, row.get::<_, String>(2)?)),
    )?;
    Ok((hlc.wall > 0).then_some(hlc))
}

/// Every live clock without an end.
fn running_clocks(conn: &Connection) -> Result<Vec<Clock>> {
    let mut stmt = conn.prepare(
        "SELECT o.uuid, c.owner, c.start_at FROM clock c JOIN object o ON o.id = c.object \
         WHERE o.deleted_at IS NULL AND c.end_at IS NULL",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, Vec<u8>>(0)?,
            row.get::<_, Vec<u8>>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut clocks = Vec::new();
    for row in rows {
        let (id, owner, start) = row?;
        let start: Timestamp = start.parse().map_err(|e| Error::Corrupt {
            what: format!("clock.start_at {start:?}: {e}"),
        })?;
        clocks.push(Clock {
            id: id_of_blob(&id)?,
            created: start,
            modified: start,
            owner: id_of_blob(&owner)?,
            start,
            end: None,
            note: None,
        });
    }
    Ok(clocks)
}

/// Ends the clocks ADR 0046 says must end: all running ones but the newest
/// (§2), and one on `closed`, completed or dropped at that instant (§3).
/// Each end is a change of its own, stamped after the change that caused it
/// so it holds, with that change's device; it goes out like any other.
fn settle_clocks(
    tx: &Connection,
    closed: Option<(Id, Timestamp)>,
    cause: &Hlc,
    now: Timestamp,
) -> Result<()> {
    use crate::core::clock::next_stamp;
    use crate::core::clocking::{one_running, stop_on_close};

    let running = running_clocks(tx)?;
    let mut endings = one_running(&running);
    if let Some((owner, at)) = closed {
        endings.extend(stop_on_close(&running, owner, at));
    }
    // A clock both rules end: the earlier end.
    endings.sort_by_key(|e| (e.clock, e.end));
    endings.dedup_by_key(|e| e.clock);
    for ending in endings {
        let Some(h) = head(tx, ending.clock)? else {
            continue;
        };
        let seen = observed_in(tx)?;
        let stamp = next_stamp(&cause.device, Some(cause), seen.as_ref(), now);
        tx.execute(
            "UPDATE clock SET end_at = ?1 WHERE object = ?2",
            params![ending.end.to_string(), h.rowid],
        )?;
        let seq = next_seq(tx)?;
        touch(tx, h.rowid, seq, now)?;
        write_stamp(tx, h.rowid, "end", &stamp, seq)?;
        observe(tx, &stamp)?;
    }
    Ok(())
}

/// Marks an object changed at `seq`.
fn touch(tx: &Connection, rowid: i64, seq: i64, now: Timestamp) -> Result<()> {
    tx.execute(
        "UPDATE object SET seq = ?2, modified = ?3 WHERE id = ?1",
        params![rowid, seq, now.to_string()],
    )?;
    Ok(())
}

/// The tags of a task or project, present ones, in order.
fn taggings(conn: &Connection, rowid: i64) -> Result<Vec<Tagging>> {
    let mut statement = conn.prepare(
        "SELECT tag, position FROM tagging WHERE owner = ?1 AND present = 1 ORDER BY position, tag",
    )?;
    let rows = statement.query_map([rowid], |row| {
        Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?))
    })?;
    let mut tags = Vec::new();
    for row in rows {
        let (bytes, position) = row?;
        let uuid = uuid::Uuid::from_slice(&bytes).map_err(|e| Error::Corrupt {
            what: format!("tagging.tag: {e}"),
        })?;
        tags.push(Tagging {
            tag: Id::from_uuid(uuid),
            position,
        });
    }
    Ok(tags)
}

/// The present members of a scope (ADR 0035), in the order they came.
fn members(conn: &Connection, rowid: i64) -> Result<Vec<Id>> {
    let mut statement = conn.prepare(
        "SELECT member FROM scope_member WHERE owner = ?1 AND present = 1 ORDER BY seq, member",
    )?;
    let rows = statement.query_map([rowid], |row| row.get::<_, Vec<u8>>(0))?;
    let mut members = Vec::new();
    for row in rows {
        members.push(id_of_blob(&row?)?);
    }
    Ok(members)
}

/// The present properties of a task or project (ADR 0023).
fn properties(conn: &Connection, rowid: i64) -> Result<BTreeMap<String, String>> {
    let mut statement = conn.prepare(
        "SELECT key, value FROM property WHERE owner = ?1 AND value IS NOT NULL ORDER BY key",
    )?;
    let rows = statement.query_map([rowid], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// What hangs on an object besides its kind row; empty where the kind has
/// none.
#[derive(Default)]
struct Extras {
    tags: Vec<Tagging>,
    properties: BTreeMap<String, String>,
    members: Vec<Id>,
}

/// Reads one live object.
fn load(conn: &Connection, id: Id, head: &Head) -> Result<Object> {
    let corrupt = |what: String| Error::Corrupt {
        what: format!("{id}: {what}"),
    };
    let (table, columns) = rows::layout(head.kind).ok_or_else(|| corrupt("unknown kind".into()))?;
    let names = rows::sql_columns(columns);
    let sql = format!("SELECT {} FROM {table} WHERE object = ?1", names.join(", "));
    let values: Vec<Sql> = conn.query_row(&sql, [head.rowid], |row| {
        (0..names.len()).map(|i| row.get::<_, Sql>(i)).collect()
    })?;
    let modified: String = conn.query_row(
        "SELECT modified FROM object WHERE id = ?1",
        [head.rowid],
        |row| row.get(0),
    )?;
    let mut extras = Extras::default();
    if matches!(head.kind, ObjectKind::Task | ObjectKind::Project) {
        extras.tags = taggings(conn, head.rowid)?;
    }
    if matches!(
        head.kind,
        ObjectKind::Task | ObjectKind::Project | ObjectKind::Settings
    ) {
        extras.properties = properties(conn, head.rowid)?;
    }
    if head.kind == ObjectKind::Scope {
        extras.members = members(conn, head.rowid)?;
    }
    assemble(id, head, &values, modified, extras)
}

/// Every live object changed after `after`, by rowid, appointments only if
/// asked. Reads each table once for all of them instead of a few queries per
/// object: with some hundred objects that is the difference between a
/// handful of statements and thousands, on every read of the views.
fn load_after(conn: &Connection, after: u64, appointments: bool) -> Result<Vec<Object>> {
    let after = i64::try_from(after).unwrap_or(i64::MAX);
    // The objects in question, as a condition on `o`.
    let live = "o.deleted_at IS NULL AND o.seq > ?1";
    let mut statement = conn.prepare(&format!(
        "SELECT o.id, o.uuid, o.kind, o.created, o.seq, o.author_device, o.author_kind, \
         o.modified FROM object o WHERE {live} AND (?2 OR o.kind <> 'appointment') ORDER BY o.id"
    ))?;
    let rows = statement.query_map(params![after, appointments], |row| {
        let device: Option<String> = row.get(5)?;
        let kind: Option<String> = row.get(6)?;
        Ok((
            row.get::<_, Vec<u8>>(1)?,
            Head {
                rowid: row.get(0)?,
                kind: rows::kind_of(&row.get::<_, String>(2)?),
                created: row.get(3)?,
                seq: row.get(4)?,
                deleted: false,
                author: device.zip(kind).map(|(device, kind)| Author {
                    device,
                    kind: author_kind_of(&kind),
                }),
            },
            row.get::<_, String>(7)?,
        ))
    })?;
    let mut heads = Vec::new();
    for row in rows {
        let (uuid, head, modified) = row?;
        heads.push((id_of_blob(&uuid)?, head, modified));
    }

    // The kind rows, one query per table that occurs.
    let mut values: HashMap<i64, Vec<Sql>> = HashMap::new();
    let mut tables: Vec<(&str, &[rows::Column])> = Vec::new();
    for (_, head, _) in &heads {
        if let Some(layout) = rows::layout(head.kind)
            && !tables.iter().any(|(table, _)| *table == layout.0)
        {
            tables.push(layout);
        }
    }
    for (table, columns) in tables {
        let names: Vec<String> = rows::sql_columns(columns)
            .iter()
            .map(|name| format!("t.{name}"))
            .collect();
        let mut statement = conn.prepare(&format!(
            "SELECT t.object, {} FROM {table} t JOIN object o ON o.id = t.object WHERE {live}",
            names.join(", ")
        ))?;
        let rows = statement.query_map([after], |row| {
            let values: Vec<Sql> = (1..=names.len())
                .map(|i| row.get::<_, Sql>(i))
                .collect::<rusqlite::Result<_>>()?;
            Ok((row.get::<_, i64>(0)?, values))
        })?;
        for row in rows {
            let (rowid, row) = row?;
            values.insert(rowid, row);
        }
    }

    // Tags, properties and members, in the order `load` reads them.
    let mut tags: HashMap<i64, Vec<Tagging>> = HashMap::new();
    let mut statement = conn.prepare(&format!(
        "SELECT g.owner, g.tag, g.position FROM tagging g JOIN object o ON o.id = g.owner \
         WHERE g.present = 1 AND {live} ORDER BY g.owner, g.position, g.tag"
    ))?;
    let rows = statement.query_map([after], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, Vec<u8>>(1)?,
            row.get::<_, i64>(2)?,
        ))
    })?;
    for row in rows {
        let (owner, tag, position) = row?;
        tags.entry(owner).or_default().push(Tagging {
            tag: id_of_blob(&tag)?,
            position,
        });
    }
    let mut properties: HashMap<i64, BTreeMap<String, String>> = HashMap::new();
    let mut statement = conn.prepare(&format!(
        "SELECT p.owner, p.key, p.value FROM property p JOIN object o ON o.id = p.owner \
         WHERE p.value IS NOT NULL AND {live}"
    ))?;
    let rows = statement.query_map([after], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (owner, key, value) = row?;
        properties.entry(owner).or_default().insert(key, value);
    }
    let mut members: HashMap<i64, Vec<Id>> = HashMap::new();
    let mut statement = conn.prepare(&format!(
        "SELECT m.owner, m.member FROM scope_member m JOIN object o ON o.id = m.owner \
         WHERE m.present = 1 AND {live} ORDER BY m.owner, m.seq, m.member"
    ))?;
    let rows = statement.query_map([after], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
    })?;
    for row in rows {
        let (owner, member) = row?;
        members.entry(owner).or_default().push(id_of_blob(&member)?);
    }

    let mut objects = Vec::with_capacity(heads.len());
    for (id, head, modified) in heads {
        let row = values.remove(&head.rowid).ok_or_else(|| Error::Corrupt {
            what: format!("{id}: no row in its kind's table"),
        })?;
        let mut extras = Extras::default();
        if matches!(head.kind, ObjectKind::Task | ObjectKind::Project) {
            extras.tags = tags.remove(&head.rowid).unwrap_or_default();
        }
        if matches!(
            head.kind,
            ObjectKind::Task | ObjectKind::Project | ObjectKind::Settings
        ) {
            extras.properties = properties.remove(&head.rowid).unwrap_or_default();
        }
        if head.kind == ObjectKind::Scope {
            extras.members = members.remove(&head.rowid).unwrap_or_default();
        }
        objects.push(assemble(id, &head, &row, modified, extras)?);
    }
    Ok(objects)
}

/// Makes the object from its kind row and what hangs on it.
fn assemble(
    id: Id,
    head: &Head,
    values: &[Sql],
    modified: String,
    extras: Extras,
) -> Result<Object> {
    let corrupt = |what: String| Error::Corrupt {
        what: format!("{id}: {what}"),
    };
    let (_, columns) = rows::layout(head.kind).ok_or_else(|| corrupt("unknown kind".into()))?;
    let mut object = rows::from_sql(values, columns).map_err(corrupt)?;
    object.insert("id".into(), Json::String(id.to_string()));
    object.insert("created".into(), Json::String(head.created.clone()));
    object.insert("modified".into(), Json::String(modified));
    // For older readers (ADR 0094 §6): scope_only follows own_scope.
    if matches!(
        head.kind,
        ObjectKind::Folder | ObjectKind::Project | ObjectKind::Tag
    ) && object.get("own_scope").is_some_and(|v| !v.is_null())
    {
        object.insert("scope_only".into(), Json::Bool(true));
    }
    if matches!(head.kind, ObjectKind::Task | ObjectKind::Project) {
        let tags = serde_json::to_value(extras.tags).map_err(|e| corrupt(e.to_string()))?;
        object.insert("tags".into(), tags);
        let properties =
            serde_json::to_value(&extras.properties).map_err(|e| corrupt(e.to_string()))?;
        object.insert("properties".into(), properties);
    }
    if head.kind == ObjectKind::Settings {
        let links = serde_json::to_value(&extras.properties).map_err(|e| corrupt(e.to_string()))?;
        object.insert("property_links".into(), links);
    }
    if let Some(author) = &head.author
        && matches!(
            head.kind,
            ObjectKind::Task
                | ObjectKind::Project
                | ObjectKind::Comment
                | ObjectKind::Attachment
                | ObjectKind::HabitEntry
                | ObjectKind::ChecklistItem
        )
    {
        let author = serde_json::to_value(author).map_err(|e| corrupt(e.to_string()))?;
        object.insert("created_by".into(), author);
    }
    if head.kind == ObjectKind::Scope {
        let members = serde_json::to_value(extras.members).map_err(|e| corrupt(e.to_string()))?;
        object.insert("members".into(), members);
    }
    Object::from_json(head.kind, Json::Object(object)).map_err(corrupt)
}

/// Writes the kind row of an object, inserting or replacing it.
fn write_row(
    tx: &Connection,
    rowid: i64,
    object: &Object,
    insert: bool,
) -> Result<Result<(), Refused>> {
    let Some((table, columns)) = rows::layout(object.kind()) else {
        return Ok(Err("an object kind this server does not know".into()));
    };
    let json = match object.to_json() {
        Ok(json) => json,
        Err(reason) => return Ok(Err(reason)),
    };
    let values = match rows::to_sql(&json, columns) {
        Ok(values) => values,
        Err(reason) => return Ok(Err(reason)),
    };
    let names = rows::sql_columns(columns);
    let sql = if insert {
        let marks: Vec<String> = (2..=names.len() + 1).map(|i| format!("?{i}")).collect();
        format!(
            "INSERT INTO {table} (object, {}) VALUES (?1, {})",
            names.join(", "),
            marks.join(", ")
        )
    } else {
        let sets: Vec<String> = names
            .iter()
            .enumerate()
            .map(|(i, name)| format!("{name} = ?{}", i + 2))
            .collect();
        format!("UPDATE {table} SET {} WHERE object = ?1", sets.join(", "))
    };
    let mut all = vec![Sql::Integer(rowid)];
    all.extend(values);
    match tx.execute(&sql, rusqlite::params_from_iter(all)) {
        Ok(_) => Ok(Ok(())),
        Err(err) => match refused_by_constraint(&err) {
            Some(reason) => Ok(Err(reason)),
            None => Err(err.into()),
        },
    }
}

/// Checks that what an object points at exists, is alive and is of the
/// right kind (ADR 0018 §6).
fn check_references(tx: &Connection, object: &Object) -> Result<Result<(), Refused>> {
    use ObjectKind as K;
    let mut wanted: Vec<(Id, &[ObjectKind], &str)> = Vec::new();
    match object {
        Object::Folder(f) => wanted.extend(f.parent.map(|p| (p, &[K::Folder][..], "parent"))),
        Object::Project(p) => {
            wanted.extend(p.folder.map(|f| (f, &[K::Folder][..], "folder")));
            wanted.extend(p.workflow.map(|w| (w, &[K::Tag][..], "workflow")));
            wanted.extend(p.tags.iter().map(|t| (t.tag, &[K::Tag][..], "tag")));
        }
        Object::Task(t) => {
            wanted.extend(t.project.map(|p| (p, &[K::Project][..], "project")));
            wanted.extend(t.parent.map(|p| (p, &[K::Task][..], "parent")));
            wanted.extend(t.tags.iter().map(|t| (t.tag, &[K::Tag][..], "tag")));
        }
        Object::Tag(t) => wanted.extend(t.parent.map(|p| (p, &[K::Tag][..], "parent"))),
        Object::Link(l) => wanted.push((l.owner, &[K::Task, K::Project][..], "owner")),
        Object::Clock(c) => wanted.push((c.owner, &[K::Task, K::Project][..], "owner")),
        Object::Reminder(r) => wanted.push((r.owner, &[K::Task, K::Project][..], "owner")),
        // Members are checked when they come, in create and add: one that is
        // deleted later counts for nothing and must not block a rename
        // (ADR 0035 §2).
        Object::Scope(_) => {}
        // The forecast tag is checked when it is set, like a member.
        Object::Settings(_) => {}
        Object::Calendar(_) => {}
        // A template names projects and tags by name in its text; they are
        // looked up when it is used, like a typed line (ADR 0073 §1).
        Object::Template(_) => {}
        // Chips are checked when they come, like a scope's filter.
        Object::Perspective(_) => {}
        Object::HabitEntry(e) => wanted.push((e.habit, &[K::Task][..], "habit")),
        Object::ChecklistItem(i) => wanted.push((i.owner, &[K::Task][..], "owner")),
        Object::Appointment(a) => wanted.push((a.calendar, &[K::Calendar][..], "calendar")),
        Object::Comment(c) => wanted.push((c.owner, &[K::Task, K::Project][..], "owner")),
        Object::Attachment(a) => {
            wanted.push((a.owner, &[K::Task, K::Project, K::Comment][..], "owner"));
        }
    }
    for (target, kinds, field) in wanted {
        if target == object.id() {
            return Ok(Err(format!("{field} points at the object itself")));
        }
        match head(tx, target)? {
            Some(h) if !h.deleted && kinds.contains(&h.kind) => {}
            Some(h) if h.deleted => return Ok(Err(format!("{field} {target} is deleted"))),
            Some(_) => return Ok(Err(format!("{field} {target} is not of the right kind"))),
            None => return Ok(Err(format!("{field} {target} does not exist"))),
        }
    }
    // An attachment's file is there before it, and of the size it says
    // (ADR 0039 §5).
    if let Object::Attachment(a) = object {
        let size: Option<i64> = tx
            .query_row("SELECT size FROM blob WHERE hash = ?1", [&a.blob], |row| {
                row.get(0)
            })
            .optional()?;
        match size {
            None => return Ok(Err(format!("blob {} is not on the server", a.blob))),
            Some(size) if u64::try_from(size).ok() != Some(a.size) => {
                return Ok(Err(format!(
                    "size {} is not the size of blob {} ({size})",
                    a.size, a.blob
                )));
            }
            Some(_) => {}
        }
    }
    // A workflow is an exclusive group (ADR 0024 §3).
    if let Object::Project(Project {
        workflow: Some(workflow),
        ..
    }) = object
    {
        let exclusive: bool = tx
            .query_row(
                "SELECT t.exclusive FROM tag t JOIN object o ON o.id = t.object WHERE o.uuid = ?1",
                [uuid_blob(*workflow)],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(false);
        if !exclusive {
            return Ok(Err(format!("workflow {workflow} is not an exclusive tag")));
        }
    }
    Ok(Ok(()))
}

/// A folder or tag never goes below itself (ADR 0037 §2). Asked when
/// `parent` is written, so broken data elsewhere blocks no rename.
fn check_cycle(tx: &Connection, object: &Object) -> Result<Result<(), Refused>> {
    let above = match object {
        Object::Folder(f) => f.parent.map(|p| ("folder", p)),
        Object::Tag(t) => t.parent.map(|p| ("tag", p)),
        _ => None,
    };
    if let Some((table, parent)) = above
        && below_itself(tx, table, object.id(), parent)?
    {
        return Ok(Err(format!(
            "parent {parent} lies below the {table} itself"
        )));
    }
    Ok(Ok(()))
}

/// Whether `id` is `parent` or one of its ancestors in `table` (`folder`
/// or `tag`). Broken data with a cycle above stops after as many steps as
/// there are rows.
fn below_itself(tx: &Connection, table: &str, id: Id, parent: Id) -> Result<bool> {
    let rows: i64 = tx.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
        row.get(0)
    })?;
    let sql = format!(
        "SELECT x.parent FROM {table} x JOIN object o ON o.id = x.object WHERE o.uuid = ?1"
    );
    let mut current = Some(uuid_blob(parent));
    let me = uuid_blob(id);
    for _ in 0..=rows {
        let Some(at) = current else {
            return Ok(false);
        };
        if at == me {
            return Ok(true);
        }
        current = tx
            .query_row(&sql, [&at], |row| row.get::<_, Option<Vec<u8>>>(0))
            .optional()?
            .flatten();
    }
    Ok(false)
}

/// Inserts the properties an object was created with, all with the creation
/// stamp (ADR 0023 §2).
fn insert_properties(
    tx: &Connection,
    rowid: i64,
    object: &Object,
    hlc: &Hlc,
    seq: i64,
) -> Result<Result<(), Refused>> {
    let properties = match object {
        Object::Project(p) => &p.properties,
        Object::Task(t) => &t.properties,
        _ => return Ok(Ok(())),
    };
    for (key, value) in properties {
        if !rows::property_key(key) {
            return Ok(Err(format!("{key:?} is not a property key")));
        }
        tx.execute(
            "INSERT INTO property (owner, key, value, wall, counter, device, seq) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![rowid, key, value, hlc.wall, hlc.counter, hlc.device, seq],
        )?;
    }
    Ok(Ok(()))
}

/// Inserts the tags an object was created with, all with the creation stamp.
fn insert_taggings(
    tx: &Connection,
    rowid: i64,
    object: &Object,
    hlc: &Hlc,
    seq: i64,
) -> Result<()> {
    let tags = match object {
        Object::Project(p) => &p.tags,
        Object::Task(t) => &t.tags,
        _ => return Ok(()),
    };
    for tagging in tags {
        tx.execute(
            "INSERT INTO tagging (owner, tag, position, present, wall, counter, device, seq) \
             VALUES (?1, ?2, ?3, 1, ?4, ?5, ?6, ?7) ON CONFLICT (owner, tag) DO NOTHING",
            params![
                rowid,
                uuid_blob(tagging.tag),
                tagging.position,
                hlc.wall,
                hlc.counter,
                hlc.device,
                seq
            ],
        )?;
    }
    Ok(())
}

/// Inserts the members a scope was created with, all with the creation
/// stamp.
fn insert_members(tx: &Connection, rowid: i64, object: &Object, hlc: &Hlc, seq: i64) -> Result<()> {
    let Object::Scope(scope) = object else {
        return Ok(());
    };
    for member in &scope.members {
        tx.execute(
            "INSERT INTO scope_member (owner, member, present, wall, counter, device, seq) \
             VALUES (?1, ?2, 1, ?3, ?4, ?5, ?6) ON CONFLICT (owner, member) DO NOTHING",
            params![
                rowid,
                uuid_blob(*member),
                hlc.wall,
                hlc.counter,
                hlc.device,
                seq
            ],
        )?;
    }
    Ok(())
}

impl AccountStore {
    /// Opens the account database at `path`, creating and migrating it as
    /// needed. Refuses if another process holds it.
    pub fn open(path: &Path) -> Result<Self> {
        let lock = DatabaseLock::acquire(path)?;
        let conn = open::open(path)?;
        let migration = migrations::migrate(&conn)?;
        ensure_settings(&conn, Timestamp::now())?;
        Ok(Self {
            conn,
            _lock: Some(lock),
            migration,
            snapshot: RefCell::new(None),
        })
    }

    /// Removes the account database at `path` with its WAL, its blobs
    /// (`<name>.blobs`) and its lock file (ADR 0099 §5). Takes the lock
    /// first, so it refuses while a running server holds the account;
    /// whoever holds an [`AccountStore`] of it has to drop it before.
    /// Nothing there is fine: a half-deleted account is finished.
    pub fn remove(path: &Path) -> Result<()> {
        let lock = DatabaseLock::acquire(path)?;
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let mut name = path.as_os_str().to_os_string();
            name.push(suffix);
            remove_if_there(std::fs::remove_file(PathBuf::from(name)))?;
        }
        remove_if_there(std::fs::remove_dir_all(path.with_extension("blobs")))?;
        // Unlike `Drop for DatabaseLock` says, the lock file goes too: the
        // name is free again, and an account made under it at this very
        // moment is not a case worth a file left behind for every deletion.
        let lock_path = DatabaseLock::path_for(path);
        drop(lock);
        remove_if_there(std::fs::remove_file(lock_path))?;
        Ok(())
    }

    /// Opens the account database at `path` for reading only, without the
    /// lock and without migrating: for an export while the server runs
    /// (ADR 0074 §5). Refuses a database of another schema version than
    /// this build's.
    pub fn open_read_only(path: &Path) -> Result<Self> {
        let conn = open::open_read_only(path)?;
        let version = migrations::schema_version(&conn)?;
        if version != migrations::latest_version() {
            return Err(Error::Unsupported {
                what: format!(
                    "{} is at schema {version}, this build at {}",
                    path.display(),
                    migrations::latest_version()
                ),
            });
        }
        Ok(Self {
            conn,
            _lock: None,
            migration: MigrationReport {
                from: version,
                to: version,
                applied: Vec::new(),
                notes: Vec::new(),
            },
            snapshot: RefCell::new(None),
        })
    }

    /// A fresh database in memory, for tests.
    pub fn open_in_memory() -> Result<Self> {
        let conn = open::open_in_memory()?;
        let migration = migrations::migrate(&conn)?;
        ensure_settings(&conn, Timestamp::now())?;
        Ok(Self {
            conn,
            _lock: None,
            migration,
            snapshot: RefCell::new(None),
        })
    }

    /// What opening did to the schema.
    #[must_use]
    pub fn migration(&self) -> &MigrationReport {
        &self.migration
    }

    /// The account's current change number.
    pub fn seq(&self) -> Result<u64> {
        let seq: i64 = self
            .conn
            .query_row("SELECT seq FROM account WHERE id = 1", [], |row| row.get(0))?;
        Ok(as_seq(seq))
    }

    /// The highest stamp seen from any device.
    pub fn observed(&self) -> Result<Option<Hlc>> {
        observed_in(&self.conn)
    }

    /// The account's settings for availability (ADR 0017).
    pub fn settings(&self) -> Result<Settings> {
        settings_in(&self.conn)
    }

    /// One live object; `None` if there is none or it is deleted.
    pub fn get(&self, id: Id) -> Result<Option<Object>> {
        match head(&self.conn, id)? {
            Some(h) if !h.deleted => Ok(Some(load(&self.conn, id, &h)?)),
            _ => Ok(None),
        }
    }

    /// Whether an object is a tombstone; `None` if there never was one.
    pub fn deleted(&self, id: Id) -> Result<Option<bool>> {
        Ok(head(&self.conn, id)?.map(|h| h.deleted))
    }

    /// Records a blob whose file is in place (ADR 0039 §4); `false` if it
    /// was recorded before, which keeps the first media type.
    pub fn add_blob(
        &mut self,
        hash: &str,
        size: u64,
        media_type: &str,
        now: Timestamp,
    ) -> Result<bool> {
        let size = i64::try_from(size).map_err(|_| Error::Unsupported {
            what: format!("a blob of {size} bytes"),
        })?;
        let added = self.conn.execute(
            "INSERT INTO blob (hash, size, media_type, created) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT (hash) DO NOTHING",
            params![hash, size, media_type, now.to_string()],
        )?;
        Ok(added == 1)
    }

    /// The bytes of all recorded blobs of the account (ADR 0097 §2).
    pub fn blob_bytes(&self) -> Result<u64> {
        let sum: i64 =
            self.conn
                .query_row("SELECT COALESCE(SUM(size), 0) FROM blob", [], |row| {
                    row.get(0)
                })?;
        Ok(u64::try_from(sum).unwrap_or(0))
    }

    /// The size and media type of a recorded blob.
    pub fn blob(&self, hash: &str) -> Result<Option<(u64, String)>> {
        let found: Option<(i64, String)> = self
            .conn
            .query_row(
                "SELECT size, media_type FROM blob WHERE hash = ?1",
                [hash],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        Ok(found.map(|(size, media_type)| (u64::try_from(size).unwrap_or(0), media_type)))
    }

    /// The logbook of an object, oldest first (ADR 0024 §7); `None` if
    /// there is no such object.
    pub fn logbook(&self, id: Id) -> Result<Option<Vec<LogEntry>>> {
        let Some(h) = head(&self.conn, id)? else {
            return Ok(None);
        };
        let mut statement = self.conn.prepare(
            "SELECT o.uuid, l.at, l.kind, l.device, l.note, l.old_value, l.new_value, \
             l.author_kind, l.field FROM logbook l JOIN object o ON o.id = l.object \
             WHERE l.object = ?1 ORDER BY l.at, l.id",
        )?;
        let rows = statement.query_map([h.rowid], log_row)?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(log_entry(row?)?);
        }
        Ok(Some(entries))
    }

    /// The logbook entries of every object at or after `from` and before
    /// `to`, oldest first (ADR 0080 §4): what the report reads.
    pub fn logbook_between(&self, from: Timestamp, to: Timestamp) -> Result<Vec<LogEntry>> {
        // `at` is text; its fraction of a second does not sort with the
        // `Z` of a whole second, so read a second more each way and cut in
        // Rust.
        let wide = |t: Timestamp, s: i64| {
            t.checked_add(jiff::SignedDuration::from_secs(s))
                .unwrap_or(t)
                .to_string()
        };
        let mut statement = self.conn.prepare(
            "SELECT o.uuid, l.at, l.kind, l.device, l.note, l.old_value, l.new_value, \
             l.author_kind, l.field FROM logbook l JOIN object o ON o.id = l.object \
             WHERE l.at >= ?1 AND l.at < ?2 ORDER BY l.at, l.id",
        )?;
        let rows = statement.query_map(params![wide(from, -1), wide(to, 1)], log_row)?;
        let mut entries = Vec::new();
        for row in rows {
            let entry = log_entry(row?)?;
            if entry.at >= from && entry.at < to {
                entries.push(entry);
            }
        }
        Ok(entries)
    }

    /// The logbook of the account, newest first (ADR 0123 §4): entries at
    /// or after `filter.since` and before `filter.until`, of the kinds and
    /// the device asked for, at most `filter.limit`; each with what the
    /// object is and its title, also when it is deleted.
    pub fn logbook_all(&self, filter: &LogFilter) -> Result<Vec<AccountLogEntry>> {
        let wide = |t: Timestamp, s: i64| {
            t.checked_add(jiff::SignedDuration::from_secs(s))
                .unwrap_or(t)
                .to_string()
        };
        let mut statement = self.conn.prepare(
            "SELECT o.uuid, l.at, l.kind, l.device, l.note, l.old_value, l.new_value, \
             l.author_kind, l.field FROM logbook l JOIN object o ON o.id = l.object \
             WHERE l.at >= ?1 AND l.at < ?2 AND (?3 IS NULL OR l.device = ?3) \
             ORDER BY l.at DESC, l.id DESC",
        )?;
        let rows = statement.query_map(
            params![wide(filter.since, -1), wide(filter.until, 1), filter.device],
            log_row,
        )?;
        let mut entries = Vec::new();
        for row in rows {
            let entry = log_entry(row?)?;
            if entry.at < filter.since || entry.at >= filter.until {
                continue;
            }
            if !filter.kinds.is_empty() && !filter.kinds.contains(&entry.kind) {
                continue;
            }
            let Some(h) = head(&self.conn, entry.object)? else {
                continue;
            };
            let title = load(&self.conn, entry.object, &h)
                .ok()
                .and_then(|object| object.to_json().ok())
                .and_then(|json| json.get("title").and_then(Json::as_str).map(str::to_string));
            entries.push(AccountLogEntry {
                entry,
                object_kind: h.kind,
                title,
                deleted: h.deleted,
            });
            if entries.len() >= filter.limit {
                break;
            }
        }
        Ok(entries)
    }

    /// Applies what deleting `root` takes with it (ADR 0036) in one
    /// transaction like [`Self::apply_all`], and writes one logbook entry
    /// for all of it: `deleted` on `root` with `counts` (ADR 0123 §2). The
    /// changes write none of their own.
    pub fn apply_delete(
        &mut self,
        changes: &[Change],
        author: &Author,
        now: Timestamp,
        root: Id,
        counts: &Json,
    ) -> Result<std::result::Result<Vec<ChangeOutcome>, (usize, ChangeOutcome)>> {
        let tx = self.conn.transaction()?;
        let mut outcomes = Vec::with_capacity(changes.len());
        for (index, change) in changes.iter().enumerate() {
            match apply_in(&tx, change, author, now, false)? {
                applied @ ChangeOutcome::Applied { .. } => outcomes.push(applied),
                other => return Ok(Err((index, other))),
            }
        }
        let stamp = changes.iter().rev().find_map(|change| match change {
            Change::Delete { object, hlc } if *object == root => Some(hlc),
            _ => None,
        });
        if let (Some(hlc), Some(h)) = (stamp, head(&tx, root)?) {
            let kind = serde_json::to_value(LogKind::Deleted)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_default();
            tx.execute(
                "INSERT INTO logbook (object, at, kind, note, old_value, new_value, device, author_kind) \
                 VALUES (?1, ?2, ?3, NULL, NULL, ?4, ?5, ?6)",
                params![
                    h.rowid,
                    now.to_string(),
                    kind,
                    counts.to_string(),
                    hlc.device,
                    author_kind_name(author.kind)
                ],
            )?;
        }
        tx.commit()?;
        Ok(Ok(outcomes))
    }

    /// The field stamps written at or after `from` and before `to`
    /// (ADR 0080 §1, `report_worked = fields`): the object, the field, when
    /// and from which device. Only the last change of a field has a stamp.
    /// The server's own stamps (`server`) and the stamps of a creation are
    /// left out.
    pub fn stamps_between(&self, from: Timestamp, to: Timestamp) -> Result<Vec<FieldStamp>> {
        let mut statement = self.conn.prepare(
            "SELECT o.uuid, s.field, s.wall, s.device FROM stamp s JOIN object o ON o.id = s.object \
             WHERE s.wall >= ?1 AND s.wall < ?2 AND s.device <> 'server' AND s.field <> '@create' \
             ORDER BY s.wall",
        )?;
        let rows =
            statement.query_map(params![from.as_millisecond(), to.as_millisecond()], |row| {
                Ok((
                    row.get::<_, Vec<u8>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;
        let mut stamps = Vec::new();
        for row in rows {
            let (uuid, field, wall, device) = row?;
            let object = uuid::Uuid::from_slice(&uuid)
                .map(Id::from_uuid)
                .map_err(|e| Error::Corrupt {
                    what: format!("stamp.object: {e}"),
                })?;
            let at = Timestamp::from_millisecond(wall).map_err(|e| Error::Corrupt {
                what: format!("stamp.wall {wall}: {e}"),
            })?;
            stamps.push(FieldStamp {
                object,
                field,
                at,
                device,
            });
        }
        Ok(stamps)
    }

    /// Every live object of the account. Read once and kept until the
    /// database changes: the views ask for it several times per click, and
    /// reading it is most of what they cost.
    pub fn snapshot(&self) -> Result<Arc<OwnedSnapshot>> {
        let written = self.written()?;
        if let Some((at, snapshot)) = self.snapshot.borrow().as_ref()
            && *at == written
        {
            return Ok(Arc::clone(snapshot));
        }
        let snapshot = Arc::new(self.read_snapshot()?);
        *self.snapshot.borrow_mut() = Some((written, Arc::clone(&snapshot)));
        Ok(snapshot)
    }

    /// See [`Written`]. A write that was rolled back counts too, which only
    /// reads once more than needed.
    fn written(&self) -> Result<Written> {
        let others: i64 = self
            .conn
            .query_row("PRAGMA data_version", [], |row| row.get(0))?;
        Ok((self.conn.total_changes(), others))
    }

    fn read_snapshot(&self) -> Result<OwnedSnapshot> {
        let mut snapshot = OwnedSnapshot::default();
        for object in load_after(&self.conn, 0, false)? {
            match object {
                Object::Folder(o) => snapshot.folders.push(o),
                Object::Project(o) => snapshot.projects.push(o),
                Object::Task(o) => snapshot.tasks.push(o),
                Object::Tag(o) => snapshot.tags.push(o),
                Object::Link(o) => snapshot.links.push(o),
                Object::Clock(o) => snapshot.clocks.push(o),
                Object::Reminder(o) => snapshot.reminders.push(o),
                Object::Scope(o) => snapshot.scopes.push(o),
                Object::Comment(o) => snapshot.comments.push(o),
                Object::Attachment(o) => snapshot.attachments.push(o),
                Object::Settings(o) => snapshot.settings = Some(o),
                Object::Calendar(o) => snapshot.calendars.push(o),
                Object::Appointment(_) => {}
                Object::Template(o) => snapshot.templates.push(o),
                Object::Perspective(o) => snapshot.perspectives.push(o),
                Object::HabitEntry(o) => snapshot.habit_entries.push(o),
                Object::ChecklistItem(o) => snapshot.checklist_items.push(o),
            }
        }
        Ok(snapshot)
    }

    /// Every live object changed after `seq`, appointments included, by
    /// rowid: what [`changed_after`](Self::changed_after) names, read at once.
    pub fn live_after(&self, seq: u64) -> Result<Vec<Object>> {
        load_after(&self.conn, seq, true)
    }

    /// Every object changed after `seq`, tombstones included, in order.
    pub fn changed_after(&self, seq: u64) -> Result<Vec<Changed>> {
        let mut statement = self.conn.prepare(
            "SELECT o.uuid, o.seq, o.deleted_at IS NOT NULL, coalesce(s.seq > ?1, 0) \
             FROM object o LEFT JOIN stamp s ON s.object = o.id AND s.field = '@create' \
             WHERE o.seq > ?1 ORDER BY o.seq",
        )?;
        let rows = statement.query_map([i64::try_from(seq).unwrap_or(i64::MAX)], |row| {
            Ok((
                row.get::<_, Vec<u8>>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, bool>(2)?,
                row.get::<_, bool>(3)?,
            ))
        })?;
        let mut changed = Vec::new();
        for row in rows {
            let (bytes, seq, deleted, created) = row?;
            let uuid = uuid::Uuid::from_slice(&bytes).map_err(|e| Error::Corrupt {
                what: format!("object.uuid: {e}"),
            })?;
            changed.push(Changed {
                id: Id::from_uuid(uuid),
                seq: as_seq(seq),
                deleted,
                created,
            });
        }
        Ok(changed)
    }

    /// Applies one change from an outbox (ADR 0016), all or nothing.
    /// `author` is the session's, never the client's word (ADR 0039 §1).
    pub fn apply(
        &mut self,
        change: &Change,
        author: &Author,
        now: Timestamp,
    ) -> Result<ChangeOutcome> {
        let tx = self.conn.transaction()?;
        let outcome = apply_in(&tx, change, author, now, true)?;
        // Only an applied change is kept; a refusal leaves no trace, not even
        // a spent change number.
        if matches!(outcome, ChangeOutcome::Applied { .. }) {
            tx.commit()?;
        }
        Ok(outcome)
    }

    /// Applies a batch in one transaction (ADR 0036 §1): all of it, or,
    /// at the first change that does not apply, none. Returns the outcome of
    /// every change on success, or the index and outcome of the first that
    /// did not apply.
    pub fn apply_all(
        &mut self,
        changes: &[Change],
        author: &Author,
        now: Timestamp,
    ) -> Result<std::result::Result<Vec<ChangeOutcome>, (usize, ChangeOutcome)>> {
        let tx = self.conn.transaction()?;
        let mut outcomes = Vec::with_capacity(changes.len());
        for (index, change) in changes.iter().enumerate() {
            match apply_in(&tx, change, author, now, true)? {
                applied @ ChangeOutcome::Applied { .. } => outcomes.push(applied),
                // Dropping the transaction rolls it back.
                other => return Ok(Err((index, other))),
            }
        }
        tx.commit()?;
        Ok(Ok(outcomes))
    }

    /// Creates the object of a hook's delivery once (ADR 0022 §1): the
    /// object and the delivery id are written in one transaction, and the
    /// same id again finds the object instead of making another.
    pub fn apply_delivered(
        &mut self,
        system: &str,
        delivery_id: &str,
        change: &Change,
        author: &Author,
        now: Timestamp,
    ) -> Result<Delivered> {
        let Change::Create { object, .. } = change else {
            return Ok(Delivered::Rejected("a delivery makes an object".into()));
        };
        let tx = self.conn.transaction()?;
        let found: Option<Vec<u8>> = tx
            .query_row(
                "SELECT object FROM delivery WHERE system = ?1 AND delivery_id = ?2",
                params![system, delivery_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(blob) = found {
            return Ok(Delivered::Found(id_of_blob(&blob)?));
        }
        match apply_in(&tx, change, author, now, true)? {
            ChangeOutcome::Applied { .. } => {
                tx.execute(
                    "INSERT INTO delivery (system, delivery_id, object, received) \
                     VALUES (?1, ?2, ?3, ?4)",
                    params![system, delivery_id, uuid_blob(*object), now.to_string()],
                )?;
                tx.commit()?;
                Ok(Delivered::Created(*object))
            }
            ChangeOutcome::Rejected { reason } => Ok(Delivered::Rejected(reason)),
            other => Ok(Delivered::Rejected(format!("not applied: {other:?}"))),
        }
    }
}

/// What a delivery to a hook came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivered {
    /// The object was made now.
    Created(Id),
    /// The delivery id was known; this is what it made then.
    Found(Id),
    /// The store refused the object; nothing was written.
    Rejected(String),
}

/// The account's settings, read in any connection or transaction: the zone
/// from the account, the rest from the settings object (ADR 0045 §4), or
/// from the account before there is one.
fn settings_in(conn: &Connection) -> Result<Settings> {
    let (zone, due_soon, remind_time): (String, String, String) = conn.query_row(
        "SELECT a.zone, coalesce(s.due_soon, a.due_soon), coalesce(s.remind_time, a.remind_time) \
         FROM account a LEFT JOIN (SELECT st.* FROM settings st JOIN object o ON o.id = st.object \
                                   WHERE o.deleted_at IS NULL) s \
         WHERE a.id = 1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let zone = jiff::tz::TimeZone::get(&zone).map_err(|e| Error::Corrupt {
        what: format!("account.zone {zone:?}: {e}"),
    })?;
    let mut settings = Settings::new(zone);
    settings.due_soon = due_soon.parse().map_err(|e| Error::Corrupt {
        what: format!("account.due_soon {due_soon:?}: {e}"),
    })?;
    settings.remind_time = remind_time.parse().map_err(|e| Error::Corrupt {
        what: format!("account.remind_time {remind_time:?}: {e}"),
    })?;
    Ok(settings)
}

/// Makes the account's one settings object if it has none (ADR 0045 §4),
/// with `due_soon` and `remind_time` as the account had them. Stamped by
/// the server, so any client's later change wins.
fn ensure_settings(conn: &Connection, now: Timestamp) -> Result<()> {
    let has: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM settings s JOIN object o ON o.id = s.object \
                        WHERE o.deleted_at IS NULL)",
        [],
        |row| row.get(0),
    )?;
    if has {
        return Ok(());
    }
    let settings = settings_in(conn)?;
    conn.execute_batch("BEGIN IMMEDIATE")?;
    let made = (|| -> Result<()> {
        let seq = next_seq(conn)?;
        let id = Id::new();
        conn.execute(
            "INSERT INTO object (uuid, kind, created, modified, seq) VALUES (?1, 'settings', ?2, ?2, ?3)",
            params![uuid_blob(id), now.to_string(), seq],
        )?;
        let rowid = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO settings (object, due_soon, remind_time) VALUES (?1, ?2, ?3)",
            params![
                rowid,
                settings.due_soon.to_string(),
                settings.remind_time.to_string()
            ],
        )?;
        let hlc = Hlc::new(now.as_millisecond(), 0, "server");
        write_stamp(conn, rowid, "@create", &hlc, seq)?;
        Ok(())
    })();
    match made {
        Ok(()) => conn.execute_batch("COMMIT")?,
        Err(e) => {
            conn.execute_batch("ROLLBACK")?;
            return Err(e);
        }
    }
    Ok(())
}

type LogRow = (
    Vec<u8>,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

fn log_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LogRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
        row.get(8)?,
    ))
}

/// One row of the logbook as its wire form (ADR 0024 §7).
fn log_entry(row: LogRow) -> Result<LogEntry> {
    let (uuid, at, kind, device, note, old, new, author_kind, field) = row;
    let id = uuid::Uuid::from_slice(&uuid)
        .map(Id::from_uuid)
        .map_err(|e| Error::Corrupt {
            what: format!("logbook.object: {e}"),
        })?;
    let corrupt = |what: String| Error::Corrupt {
        what: format!("logbook of {id}: {what}"),
    };
    let parse_id = |v: Option<String>| -> Result<Option<Id>> {
        v.map(|v| v.parse().map_err(|e| corrupt(format!("{v:?}: {e}"))))
            .transpose()
    };
    let kind: LogKind = serde_json::from_value(Json::String(kind)).unwrap_or(LogKind::Unknown);
    // A change to an appointment keeps values, the rest ids.
    let (old, new, old_value, new_value) = if field.is_some() || kind == LogKind::Deleted {
        let value = |v: Option<String>| -> Result<Option<Json>> {
            v.map(|v| serde_json::from_str(&v).map_err(|e| corrupt(format!("{v:?}: {e}"))))
                .transpose()
        };
        (None, None, value(old)?, value(new)?)
    } else {
        (old, new, None, None)
    };
    Ok(LogEntry {
        object: id,
        at: at.parse().map_err(|e| corrupt(format!("{at:?}: {e}")))?,
        kind,
        device,
        author_kind: author_kind.as_deref().map(author_kind_of),
        note,
        old: parse_id(old)?,
        new: parse_id(new)?,
        field,
        old_value,
        new_value,
    })
}

fn id_of_blob(blob: &[u8]) -> Result<Id> {
    let uuid = uuid::Uuid::from_slice(blob).map_err(|e| Error::Corrupt {
        what: format!("delivery.object: {e}"),
    })?;
    Ok(Id::from_uuid(uuid))
}

fn apply_in(
    tx: &Transaction<'_>,
    change: &Change,
    author: &Author,
    now: Timestamp,
    logged: bool,
) -> Result<ChangeOutcome> {
    if author_kind_name(author.kind).is_none() {
        return Ok(rejected("an author kind this server does not know"));
    }
    // Every change comes through here, so no stamp from a clock far ahead
    // reaches a field or `observe` (ADR 0016 §1): capped, not refused.
    let capped = match change.hlc() {
        Some(hlc) => crate::core::clock::capped(hlc, observed_in(tx)?.as_ref(), now)
            .map(|stamp| crate::core::clock::restamp(change.clone(), stamp)),
        None => None,
    };
    let change = capped.as_ref().unwrap_or(change);
    let outcome = match change {
        Change::Create {
            object,
            kind,
            value,
            hlc,
        } => create(tx, *object, *kind, value, hlc, author, now)?,
        Change::Set {
            object,
            field,
            value,
            note,
            hlc,
        } => match keyed(field) {
            Some((map, key)) => property(tx, *object, map, key, value, hlc, now)?,
            None => {
                let edit = Edit {
                    hlc,
                    note: note.as_deref(),
                    author,
                    now,
                    logged,
                };
                set(tx, *object, field, value, &edit)?
            }
        },
        Change::Add {
            object,
            field,
            element,
            position,
            note,
            hlc,
        } => {
            let edit = Edit {
                hlc,
                note: note.as_deref(),
                author,
                now,
                logged,
            };
            if field == "members" {
                member(tx, *object, *element, true, &edit)?
            } else {
                tagging(tx, *object, field, *element, Some(*position), &edit)?
            }
        }
        Change::Remove {
            object,
            field,
            element,
            note,
            hlc,
        } => {
            let edit = Edit {
                hlc,
                note: note.as_deref(),
                author,
                now,
                logged,
            };
            if field == "members" {
                member(tx, *object, *element, false, &edit)?
            } else {
                tagging(tx, *object, field, *element, None, &edit)?
            }
        }
        Change::Delete { object, hlc } => delete(tx, *object, hlc, author, now, logged)?,
        Change::Unknown => rejected("an operation this server does not know"),
    };
    Ok(outcome)
}

fn create(
    tx: &Connection,
    id: Id,
    kind: ObjectKind,
    value: &Json,
    hlc: &Hlc,
    author: &Author,
    now: Timestamp,
) -> Result<ChangeOutcome> {
    let Some(kind_name) = rows::kind_name(kind) else {
        return Ok(rejected("an object kind this server does not know"));
    };
    if kind == ObjectKind::Settings {
        return Ok(rejected(
            "the settings of an account are made by the server",
        ));
    }
    if kind == ObjectKind::Calendar
        && let Some(field) = rows::CALENDAR_SERVER_FIELDS
            .iter()
            .find(|f| value.get(**f).is_some_and(|v| !v.is_null()))
    {
        return Ok(rejected(format!("{field} of a calendar is the server's")));
    }
    if kind == ObjectKind::Attachment
        && let Err(reason) = transcript::check_create(value)
    {
        return Ok(rejected(reason));
    }
    if let Some(existing) = head(tx, id)? {
        // Idempotent by the id (ADR 0004 §8).
        return Ok(if existing.deleted {
            rejected(format!("{id} is deleted"))
        } else if existing.kind != kind {
            rejected(format!("{id} exists as another kind"))
        } else {
            ChangeOutcome::Applied {
                seq: as_seq(existing.seq),
            }
        });
    }
    if value.get("id").and_then(Json::as_str) != Some(id.to_string().as_str()) {
        return Ok(rejected("value.id is not the object's id"));
    }
    let object = match Object::from_json(kind, value.clone()) {
        Ok(object) => object,
        Err(reason) => return Ok(rejected(reason)),
    };
    if let Err(reason) = check_references(tx, &object)? {
        return Ok(rejected(reason));
    }
    if let Object::Scope(scope) = &object {
        for member in &scope.members {
            if !member_kind(tx, *member)? {
                return Ok(rejected(format!("no folder, project or tag {member}")));
            }
        }
        if let Err(reason) = check_scope(tx, scope, None)? {
            return Ok(rejected(reason));
        }
    }
    if let Object::Template(template) = &object
        && let Err(reason) = check_template(tx, template, None)?
    {
        return Ok(rejected(reason));
    }
    if let Object::Perspective(perspective) = &object
        && let Err(reason) = check_perspective(tx, perspective, None)?
    {
        return Ok(rejected(reason));
    }
    if let Object::HabitEntry(entry) = &object
        && let Err(reason) = check_habit_entry(tx, entry, now)?
    {
        return Ok(rejected(reason));
    }
    if let Err(reason) = check_values(&object, None) {
        return Ok(rejected(reason));
    }
    // ADR 0068 §3.
    if let Object::Appointment(a) = &object
        && let Err(reason) = calendar::writes::check_create(tx, a)?
    {
        return Ok(rejected(reason));
    }
    let created = object
        .to_json()
        .ok()
        .and_then(|json| {
            json.get("created")
                .and_then(Json::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| now.to_string());
    let seq = next_seq(tx)?;
    tx.execute(
        "INSERT INTO object (uuid, kind, created, modified, seq, author_device, author_kind) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            uuid_blob(id),
            kind_name,
            created,
            now.to_string(),
            seq,
            author.device,
            author_kind_name(author.kind)
        ],
    )?;
    let rowid = tx.last_insert_rowid();
    if let Err(reason) = write_row(tx, rowid, &object, true)? {
        return Ok(rejected(reason));
    }
    insert_taggings(tx, rowid, &object, hlc, seq)?;
    insert_members(tx, rowid, &object, hlc, seq)?;
    if let Err(reason) = insert_properties(tx, rowid, &object, hlc, seq)? {
        return Ok(rejected(reason));
    }
    write_stamp(tx, rowid, "@create", hlc, seq)?;
    observe(tx, hlc)?;
    if kind == ObjectKind::Clock {
        settle_clocks(tx, None, hlc, now)?;
    }
    if let Object::Appointment(a) = &object {
        calendar::writes::queued(tx, rowid, a, a.calendar, "@create", None, hlc, now)?;
        let edit = Edit {
            hlc,
            note: None,
            author,
            now,
            logged: true,
        };
        log_appointment(tx, rowid, LogKind::Created, &edit, None)?;
    }
    Ok(ChangeOutcome::Applied { seq: as_seq(seq) })
}

/// A comment or attachment is changed and deleted only from a session of the
/// kind that wrote it (ADR 0039 §3, §5); one from before counts as a
/// person's.
fn same_kind_of_author(h: &Head, author: &Author) -> Result<(), Refused> {
    if !matches!(h.kind, ObjectKind::Comment | ObjectKind::Attachment) {
        return Ok(());
    }
    let wrote = h.author.as_ref().map_or(AuthorKind::Human, |a| a.kind);
    if wrote == author.kind {
        Ok(())
    } else {
        Err(format!(
            "written by a {}, not to be changed by a {}",
            author_kind_name(wrote).unwrap_or("stranger"),
            author_kind_name(author.kind).unwrap_or("stranger")
        ))
    }
}

/// The live object a change is about, or why there is none.
fn live(tx: &Connection, id: Id) -> Result<Result<Head, Refused>> {
    Ok(match head(tx, id)? {
        None => Err(format!("no object {id}")),
        Some(h) if h.deleted => Err(format!("{id} is deleted")),
        Some(h) => Ok(h),
    })
}

/// Decides last-writer-wins against what is stored; `Some` is the answer
/// when the change does not apply.
fn decide(incoming: &Hlc, current: Option<&Stored>) -> Option<ChangeOutcome> {
    match current {
        // The same change again: answered as it was the first time.
        Some(stored) if stored.hlc == *incoming => Some(ChangeOutcome::Applied {
            seq: as_seq(stored.seq),
        }),
        Some(stored) if !wins(incoming, Some(&stored.hlc)) => Some(ChangeOutcome::Superseded {
            by: stored.hlc.clone(),
        }),
        _ => None,
    }
}

/// What a change carries besides its target: its stamp, its note for the
/// logbook, and when the server applies it.
struct Edit<'a> {
    hlc: &'a Hlc,
    note: Option<&'a str>,
    /// The session's (ADR 0039 §1).
    author: &'a Author,
    now: Timestamp,
    /// Whether it writes logbook entries; not for the changes that come
    /// with a delete, which writes one for all (ADR 0123 §2).
    logged: bool,
}

/// A client's change to an appointment in the logbook (ADR 0068 §3):
/// `change` names the field and its values before and after.
fn log_appointment(
    tx: &Connection,
    rowid: i64,
    kind: LogKind,
    edit: &Edit<'_>,
    change: Option<(&str, Option<&Json>, Option<&Json>)>,
) -> Result<()> {
    if !edit.logged {
        return Ok(());
    }
    let kind = serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let (field, old, new) = change.map_or((None, None, None), |(f, o, n)| {
        let text = |v: Option<&Json>| v.filter(|v| !v.is_null()).map(Json::to_string);
        (Some(f), text(o), text(n))
    });
    tx.execute(
        "INSERT INTO logbook (object, at, kind, note, old_value, new_value, device, author_kind, field) \
         VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, ?8)",
        params![
            rowid,
            edit.now.to_string(),
            kind,
            old,
            new,
            edit.hlc.device,
            author_kind_name(edit.author.kind),
            field
        ],
    )?;
    Ok(())
}

/// One logbook entry (ADR 0024 §5), in the transaction of the change.
fn log(
    tx: &Connection,
    rowid: i64,
    kind: LogKind,
    edit: &Edit<'_>,
    old: Option<Id>,
    new: Option<Id>,
) -> Result<()> {
    if !edit.logged {
        return Ok(());
    }
    let kind = serde_json::to_value(kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    tx.execute(
        "INSERT INTO logbook (object, at, kind, note, old_value, new_value, device, author_kind) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            rowid,
            edit.now.to_string(),
            kind,
            edit.note,
            old.map(|id| id.to_string()),
            new.map(|id| id.to_string()),
            edit.hlc.device,
            author_kind_name(edit.author.kind)
        ],
    )?;
    Ok(())
}

fn set(
    tx: &Transaction<'_>,
    id: Id,
    field: &str,
    value: &Json,
    edit: &Edit<'_>,
) -> Result<ChangeOutcome> {
    let (hlc, note, now) = (edit.hlc, edit.note, edit.now);
    let h = match live(tx, id)? {
        Ok(h) => h,
        Err(reason) => return Ok(rejected(reason)),
    };
    if let Err(reason) = same_kind_of_author(&h, edit.author) {
        return Ok(rejected(reason));
    }
    if !rows::settable(h.kind, field) {
        return Ok(rejected(format!("{field} cannot be set on this object")));
    }
    // Whether samu may write it at all, before any stamp counts (ADR 0068).
    if h.kind == ObjectKind::Appointment
        && let Object::Appointment(a) = load(tx, id, &h)?
        && let Err(reason) = calendar::writes::check_change(tx, &a, None)?
    {
        return Ok(rejected(reason));
    }
    let current = match stamp_of(tx, h.rowid, field)? {
        Some(stamp) => Some(stamp),
        None => stamp_of(tx, h.rowid, "@create")?,
    };
    if let Some(answer) = decide(hlc, current.as_ref()) {
        return Ok(answer);
    }
    let mut json = match load(tx, id, &h)?.to_json() {
        Ok(json) => json,
        Err(reason) => return Ok(rejected(reason)),
    };
    let was_set = json.get(field).is_some_and(|v| !v.is_null());
    let was = json.get(field).cloned();
    let before = (h.kind == ObjectKind::Appointment)
        .then(|| Object::from_json(h.kind, Json::Object(json.clone())))
        .transpose()
        .map_err(|what| Error::Corrupt { what })?;
    json.insert(field.into(), value.clone());
    let object = match Object::from_json(h.kind, Json::Object(json)) {
        Ok(object) => object,
        Err(reason) => return Ok(rejected(format!("{field}: {reason}"))),
    };
    if let Err(reason) = check_references(tx, &object)? {
        return Ok(rejected(reason));
    }
    if field == "parent"
        && let Err(reason) = check_cycle(tx, &object)?
    {
        return Ok(rejected(reason));
    }
    if let Object::Scope(scope) = &object
        && let Err(reason) = check_scope(tx, scope, Some(field))?
    {
        return Ok(rejected(format!("{field}: {reason}")));
    }
    if let Object::Template(template) = &object
        && let Err(reason) = check_template(tx, template, Some(field))?
    {
        return Ok(rejected(format!("{field}: {reason}")));
    }
    if let Object::Perspective(perspective) = &object
        && let Err(reason) = check_perspective(tx, perspective, Some(field))?
    {
        return Ok(rejected(format!("{field}: {reason}")));
    }
    // The forecast tag must be a live tag when it is set (ADR 0045 §4).
    if let Object::Settings(settings) = &object
        && field == "forecast_tag"
        && let Some(tag) = settings.forecast_tag
        && !head(tx, tag)?.is_some_and(|h| !h.deleted && h.kind == ObjectKind::Tag)
    {
        return Ok(rejected(format!("forecast_tag: no tag {tag}")));
    }
    if let Err(reason) = check_values(&object, Some(field)) {
        return Ok(rejected(format!("{field}: {reason}")));
    }
    // ADR 0068 §3.
    if let (Some(Object::Appointment(before)), Object::Appointment(after)) = (&before, &object)
        && let Err(reason) = calendar::writes::check_change(tx, before, Some(after))?
    {
        return Ok(rejected(reason));
    }
    if let Object::Settings(settings) = &object
        && field == "default_calendar"
        && let Some(id) = settings.default_calendar
        && !head(tx, id)?.is_some_and(|h| !h.deleted && h.kind == ObjectKind::Calendar)
    {
        return Ok(rejected(format!("default_calendar: no calendar {id}")));
    }
    if let Err(reason) = write_row(tx, h.rowid, &object, false)? {
        return Ok(rejected(format!("{field}: {reason}")));
    }
    let seq = next_seq(tx)?;
    touch(tx, h.rowid, seq, now)?;
    write_stamp(tx, h.rowid, field, hlc, seq)?;
    observe(tx, hlc)?;
    if let (Some(Object::Appointment(before)), Object::Appointment(after)) = (&before, &object) {
        calendar::writes::queued(
            tx,
            h.rowid,
            after,
            before.calendar,
            field,
            was.as_ref(),
            hlc,
            now,
        )?;
        let change = (field, was.as_ref(), Some(value));
        log_appointment(tx, h.rowid, LogKind::Changed, edit, Some(change))?;
    }
    // Where it is changed (ADR 0123 §3); a value written again is no move.
    let place = matches!(
        (h.kind, field),
        (ObjectKind::Task, "project" | "parent")
            | (ObjectKind::Project, "folder")
            | (ObjectKind::Folder, "parent")
    );
    let was_value = was.as_ref().filter(|v| !v.is_null());
    let new_value = Some(value).filter(|v| !v.is_null());
    if place && was_value != new_value {
        let change = (field, was_value, new_value);
        log_appointment(tx, h.rowid, LogKind::Moved, edit, Some(change))?;
    }
    // Completing a repetition makes the next task (ADR 0033 §2); why it
    // made none goes into the logbook entry below.
    let mut why_none = None;
    if let Object::Task(task) = &object
        && field == "completed_at"
        && !value.is_null()
        && !was_set
        && task.schedule.repeat.is_some()
    {
        why_none = follow_up(tx, task, hlc, edit.author, now)?;
    }
    // One running clock, and none on what was just closed (ADR 0046 §2, §3).
    let closed_at = match &object {
        Object::Task(t) if field == "completed_at" => t.completed_at,
        Object::Task(t) if field == "dropped_at" => t.dropped_at,
        Object::Project(p) if field == "completed_at" => p.completed_at,
        Object::Project(p) if field == "dropped_at" => p.dropped_at,
        _ => None,
    };
    if closed_at.is_some() || matches!(object, Object::Clock(_)) {
        settle_clocks(tx, closed_at.map(|at| (id, at)), hlc, now)?;
    }
    // Completing, dropping and opening again go into the logbook
    // (ADR 0024 §5).
    let kind = match (field, value.is_null(), was_set) {
        ("completed_at", false, _) => Some(LogKind::Completed),
        ("dropped_at", false, _) => Some(LogKind::Dropped),
        ("completed_at" | "dropped_at", true, true) => Some(LogKind::Reopened),
        _ => None,
    };
    if let Some(kind) = kind {
        let note = match (note, why_none) {
            (note, None) => note.map(str::to_string),
            (None, Some(why)) => Some(why),
            (Some(note), Some(why)) => Some(format!("{note}\n({why})")),
        };
        let entry = Edit {
            note: note.as_deref(),
            ..*edit
        };
        log(tx, h.rowid, kind, &entry, None, None)?;
    }
    Ok(ChangeOutcome::Applied { seq: as_seq(seq) })
}

/// Checks values beyond their type: a repeat rule must be one samu can
/// compute (ADR 0033 §4), a tag title holds no slash (ADR 0027 §3). The
/// title is checked when it is written (`field` none on create), so an old
/// tag with a slash still takes other changes and a rename.
fn check_values(object: &Object, field: Option<&str>) -> Result<(), Refused> {
    if let Object::Tag(tag) = object
        && field.is_none_or(|f| f == "title")
    {
        crate::core::capture::check_tag_title(&tag.title)?;
    }
    // A value from a newer samu is read, never written (ADR 0015).
    if let Object::Settings(settings) = object
        && field.is_none_or(|f| f == "report_worked")
        && settings.report_worked == Some(crate::core::api::ReportWorked::Unknown)
    {
        return Err("a report_worked this server does not know".into());
    }
    if let Object::Clock(clock) = object
        && clock.end.is_some_and(|end| end < clock.start)
    {
        return Err("a clock cannot end before it starts".into());
    }
    if let Object::Task(task) = object
        && field.is_none_or(|f| f == "habit")
        && let Some(habit) = &task.habit
    {
        check_habit(habit)?;
    }
    let schedule = match object {
        Object::Task(t) => &t.schedule,
        Object::Project(p) => &p.schedule,
        _ => return Ok(()),
    };
    match &schedule.repeat {
        Some(repeat) => crate::core::repeat::check(&repeat.rule).map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

/// Checks the fields of a scope from ADR 0043 that `field` writes, all of
/// them on create. A value from a newer samu is read, never written
/// (ADR 0015); what a chip or the capture names must be there when it
/// comes, and counts for nothing once it is deleted, like a member
/// (ADR 0035 §2), so a rename is never blocked by it.
fn check_scope(tx: &Connection, scope: &Scope, field: Option<&str>) -> Result<Result<(), Refused>> {
    use crate::core::api::Weekday;
    let writes = |name: &str| field.is_none_or(|f| f == name);
    let is = |id: Id, kinds: &[ObjectKind]| -> Result<bool> {
        Ok(head(tx, id)?.is_some_and(|h| !h.deleted && kinds.contains(&h.kind)))
    };
    if writes("filter")
        && let Err(reason) = check_chips(tx, &scope.filter)?
    {
        return Ok(Err(reason));
    }
    if writes("capture")
        && let Some(capture) = &scope.capture
    {
        if let Some(project) = capture.project
            && !is(project, &[ObjectKind::Project])?
        {
            return Ok(Err(format!("no project {project}")));
        }
        for tag in &capture.tags {
            if !is(*tag, &[ObjectKind::Tag])? {
                return Ok(Err(format!("no tag {tag}")));
            }
        }
    }
    if writes("calendars") {
        for calendar in &scope.calendars {
            if !is(*calendar, &[ObjectKind::Calendar])? {
                return Ok(Err(format!("no calendar {calendar}")));
            }
        }
    }
    if writes("layout")
        && let Some(inspector) = scope.layout.as_ref().and_then(|l| l.inspector.as_ref())
    {
        // A part or a mode from a newer samu is read, never written
        // (ADR 0015, ADR 0072 §1).
        if inspector
            .shown
            .contains(&crate::core::api::InspectorPart::Unknown)
        {
            return Ok(Err(
                "a part of the inspector this server does not know".into()
            ));
        }
        if inspector.note == Some(crate::core::api::NoteMode::Unknown) {
            return Ok(Err("a mode of the note this server does not know".into()));
        }
    }
    if writes("schedule") {
        for window in &scope.schedule {
            if window.days.is_empty() {
                return Ok(Err("a window without days".into()));
            }
            if window.days.contains(&Weekday::Unknown) {
                return Ok(Err("a day this server does not know".into()));
            }
        }
    }
    Ok(Ok(()))
}

/// Checks chips of the search language as a scope's filter and a
/// perspective hold them (ADR 0043 §2, ADR 0090 §1): words samu-core reads,
/// and projects, tags, folders and states as live ids of the right kind.
fn check_chips(
    tx: &Connection,
    chips: &[crate::core::api::ScopeChip],
) -> Result<Result<(), Refused>> {
    use crate::core::api::ScopeChipKind as C;
    for chip in chips {
        // The words, as samu-core reads them (ADR 0043 §2).
        if let Err(e) = crate::core::query::check(chip) {
            return Ok(Err(e.to_string()));
        }
        let wants: &[ObjectKind] = match chip.kind {
            C::Project if chip.value == "inbox" => continue,
            C::Project => &[ObjectKind::Project],
            C::Tag | C::State => &[ObjectKind::Tag],
            C::Folder => &[ObjectKind::Folder],
            _ => continue,
        };
        let Ok(id) = chip.value.parse::<Id>() else {
            return Ok(Err(format!("{:?} is not an id", chip.value)));
        };
        if !head(tx, id)?.is_some_and(|h| !h.deleted && wants.contains(&h.kind)) {
            return Ok(Err(format!("no live object of the right kind {id}")));
        }
    }
    Ok(Ok(()))
}

/// Checks what makes a task a habit (ADR 0091 §1): a kind and a period
/// this server knows, a goal of at least 1, a unit of 1-32 bytes.
fn check_habit(habit: &crate::core::api::Habit) -> Result<(), Refused> {
    use crate::core::api::{HabitKind, HabitPer};
    if habit.kind == HabitKind::Unknown {
        return Err("a kind of habit this server does not know".into());
    }
    if habit.per == HabitPer::Unknown {
        return Err("a period this server does not know".into());
    }
    if habit.times == 0 {
        return Err("a habit's goal is at least 1".into());
    }
    if let Some(unit) = &habit.unit
        && (unit.trim().is_empty() || unit.len() > 32)
    {
        return Err("a unit is a word of at most 32 bytes".into());
    }
    Ok(())
}

/// Checks an entry for a habit (ADR 0091 §2): its task is a habit, its
/// day not after today in the account's zone, an amount only on `done`
/// and an end only on `pause`, not before the day.
fn check_habit_entry(
    tx: &Connection,
    entry: &crate::core::api::HabitEntry,
    now: Timestamp,
) -> Result<Result<(), Refused>> {
    use crate::core::api::HabitEntryStatus as S;
    if let Some(h) = head(tx, entry.habit)?
        && let Object::Task(task) = load(tx, entry.habit, &h)?
        && task.habit.is_none()
    {
        return Ok(Err(format!("task {} is no habit", entry.habit)));
    }
    let today = now.to_zoned(settings_in(tx)?.zone).date();
    if entry.day > today {
        return Ok(Err(format!("{} is after today", entry.day)));
    }
    match entry.status {
        S::Unknown => return Ok(Err("a status this server does not know".into())),
        S::Done => {}
        _ if entry.amount.is_some() => {
            return Ok(Err("only a done entry has an amount".into()));
        }
        _ => {}
    }
    if entry.amount == Some(0) {
        return Ok(Err("an amount is at least 1".into()));
    }
    if let Some(until) = entry.until {
        if entry.status != S::Pause {
            return Ok(Err("only a pause has an end".into()));
        }
        if until < entry.day {
            return Ok(Err("a pause cannot end before it begins".into()));
        }
    }
    Ok(Ok(()))
}

/// Checks the fields of a perspective that `field` writes, all of them on
/// create (ADR 0090 §1): a title no other live perspective has, ignoring
/// case; an icon like a project's (ADR 0059); chips as a scope's filter;
/// no value from a newer samu, which is read, never written (ADR 0015);
/// a board's workflow and tags live tags; a table's columns known ones,
/// each once (ADR 0115 §2); blocks that each show one live plain
/// perspective or a known view, on a perspective no block shows
/// (ADR 0116 §2).
fn check_perspective(
    tx: &Connection,
    perspective: &crate::core::api::Perspective,
    field: Option<&str>,
) -> Result<Result<(), Refused>> {
    use crate::core::api::{
        BoardSource, PerspectiveFilter, PerspectiveGroup, PerspectiveSort, PerspectiveView,
    };
    let writes = |name: &str| field.is_none_or(|f| f == name);
    if writes("title") {
        if perspective.title.trim().is_empty() {
            return Ok(Err("a perspective needs a name".into()));
        }
        let mut taken = tx.prepare(
            "SELECT p.title FROM perspective p JOIN object o ON o.id = p.object \
             WHERE o.deleted_at IS NULL AND o.uuid != ?1",
        )?;
        let titles = taken.query_map([uuid_blob(perspective.id)], |row| row.get::<_, String>(0))?;
        for title in titles {
            let title = title?;
            if title.to_lowercase() == perspective.title.to_lowercase() {
                return Ok(Err(format!("there is a perspective {title:?} already")));
            }
        }
    }
    if writes("icon")
        && let Some(icon) = &perspective.icon
        && (icon.is_empty() || icon.len() > 32)
    {
        return Ok(Err("an icon is one emoji, at most 32 bytes".into()));
    }
    if writes("chips")
        && let Err(reason) = check_chips(tx, &perspective.chips)?
    {
        return Ok(Err(reason));
    }
    if writes("filter") && perspective.filter == PerspectiveFilter::Unknown {
        return Ok(Err("a filter this server does not know".into()));
    }
    if writes("view") && perspective.view == PerspectiveView::Unknown {
        return Ok(Err("a presentation this server does not know".into()));
    }
    if writes("arrange")
        && let Some(arrange) = &perspective.arrange
        && (arrange.group == PerspectiveGroup::Unknown
            || arrange.sort == PerspectiveSort::Unknown
            || arrange.then == Some(PerspectiveSort::Unknown))
    {
        return Ok(Err("a grouping or sorting this server does not know".into()));
    }
    if writes("board")
        && let Some(board) = &perspective.board
    {
        if board.source == BoardSource::Unknown {
            return Ok(Err("a source of columns this server does not know".into()));
        }
        for tag in board.workflow.iter().chain(&board.tags) {
            if !head(tx, *tag)?.is_some_and(|h| !h.deleted && h.kind == ObjectKind::Tag) {
                return Ok(Err(format!("no tag {tag}")));
            }
        }
    }
    if writes("table")
        && let Some(table) = &perspective.table
    {
        for (n, column) in table.columns.iter().enumerate() {
            if !crate::core::table::known_column(column) {
                return Ok(Err(format!(
                    "a column this server does not know: {column:?}"
                )));
            }
            if table.columns[..n].contains(column) {
                return Ok(Err(format!("the column {column:?} twice")));
            }
        }
    }
    if writes("blocks")
        && !perspective.blocks.is_empty()
        && let Err(reason) = check_blocks(tx, perspective)?
    {
        return Ok(Err(reason));
    }
    Ok(Ok(()))
}

/// The blocks of a block perspective (ADR 0116 §2): each shows exactly one
/// of a live plain perspective other than this one, or a fixed view this
/// server knows, under a heading that is not blank when given; and no
/// block of another live perspective shows this one, which would make it
/// a block in a block.
fn check_blocks(
    tx: &Connection,
    perspective: &crate::core::api::Perspective,
) -> Result<Result<(), Refused>> {
    for block in &perspective.blocks {
        if block.title.as_ref().is_some_and(|t| t.trim().is_empty()) {
            return Ok(Err("a block's heading is not blank".into()));
        }
        match (&block.perspective, &block.view) {
            (Some(id), None) => {
                if *id == perspective.id {
                    return Ok(Err("a perspective is not a block of itself".into()));
                }
                if !head(tx, *id)?.is_some_and(|h| !h.deleted && h.kind == ObjectKind::Perspective)
                {
                    return Ok(Err(format!("no perspective {id}")));
                }
                let blocks: Option<String> = tx.query_row(
                    "SELECT p.blocks FROM perspective p JOIN object o ON o.id = p.object \
                     WHERE o.uuid = ?1",
                    [uuid_blob(*id)],
                    |row| row.get(0),
                )?;
                if blocks.is_some() {
                    return Ok(Err(format!(
                        "the perspective {id} has blocks itself, and is no block"
                    )));
                }
            }
            (None, Some(view)) => {
                if !crate::core::blocks::known_view(view) {
                    return Ok(Err(format!("a view this server does not know: {view:?}")));
                }
            }
            _ => {
                return Ok(Err("a block shows one perspective or one view".into()));
            }
        }
    }
    let mut others = tx.prepare(
        "SELECT p.blocks FROM perspective p JOIN object o ON o.id = p.object \
         WHERE o.deleted_at IS NULL AND o.uuid != ?1 AND p.blocks IS NOT NULL",
    )?;
    let others = others.query_map([uuid_blob(perspective.id)], |row| row.get::<_, String>(0))?;
    for blocks in others {
        let blocks: Vec<crate::core::api::PerspectiveBlock> =
            serde_json::from_str(&blocks?).unwrap_or_default();
        if blocks.iter().any(|b| b.perspective == Some(perspective.id)) {
            return Ok(Err(
                "this perspective is a block of another, and gets no blocks".into(),
            ));
        }
    }
    Ok(Ok(()))
}

/// Checks the fields of a template that `field` writes, all of them on
/// create (ADR 0073 §1): a title that can be called as `/title` and no other
/// live template has, ignoring case; a text of marks only; reminders on an
/// anchor of the new task, never an instant or a value from a newer samu;
/// a key of one letter or digit no other live template has (ADR 0084 §1).
fn check_template(
    tx: &Connection,
    template: &crate::core::api::Template,
    field: Option<&str>,
) -> Result<Result<(), Refused>> {
    use crate::core::api::ReminderAnchor;
    let writes = |name: &str| field.is_none_or(|f| f == name);
    if writes("title") {
        if !crate::core::capture::template_name(&template.title) {
            return Ok(Err(format!(
                "{:?}: a template's name has letters, digits, - and _ only",
                template.title
            )));
        }
        let mut taken = tx.prepare(
            "SELECT t.title FROM template t JOIN object o ON o.id = t.object \
             WHERE o.deleted_at IS NULL AND o.uuid != ?1",
        )?;
        let titles = taken.query_map([uuid_blob(template.id)], |row| row.get::<_, String>(0))?;
        for title in titles {
            let title = title?;
            if title.to_lowercase() == template.title.to_lowercase() {
                return Ok(Err(format!("there is a template {title:?} already")));
            }
        }
    }
    if writes("key")
        && let Some(key) = &template.key
    {
        if !crate::core::capture::template_key(key) {
            return Ok(Err(format!(
                "{key:?}: a template's key is one letter or digit"
            )));
        }
        let mut taken = tx.prepare(
            "SELECT t.title FROM template t JOIN object o ON o.id = t.object \
             WHERE o.deleted_at IS NULL AND o.uuid != ?1 AND t.key = ?2",
        )?;
        let mut holders = taken
            .query_map(rusqlite::params![uuid_blob(template.id), key], |row| {
                row.get::<_, String>(0)
            })?;
        if let Some(title) = holders.next() {
            let title = title?;
            return Ok(Err(format!(
                "the key {key:?} is the template {title:?}'s already"
            )));
        }
    }
    if writes("text")
        && let Err(reason) = crate::core::capture::check_template_text(&template.text)
    {
        return Ok(Err(reason));
    }
    if writes("reminders") {
        for reminder in &template.reminders {
            match reminder.anchor {
                ReminderAnchor::Due | ReminderAnchor::Planned | ReminderAnchor::Defer
                    if reminder.time.is_none() => {}
                ReminderAnchor::Daily if reminder.time.is_some() && reminder.before.is_none() => {}
                ReminderAnchor::Due | ReminderAnchor::Planned | ReminderAnchor::Defer => {
                    return Ok(Err("only a daily reminder has a time of day".into()));
                }
                ReminderAnchor::Daily => {
                    return Ok(Err(
                        "a daily reminder has a time of day and nothing before it".into(),
                    ));
                }
                ReminderAnchor::At => {
                    return Ok(Err(
                        "a template's reminder is on a date, not an instant".into()
                    ));
                }
                ReminderAnchor::Unknown => {
                    return Ok(Err("an anchor this server does not know".into()));
                }
            }
        }
    }
    Ok(Ok(()))
}

/// How many tasks a repetition chain has up to this one, counted back over
/// `repeat_of` (ADR 0007 §3).
fn chain_length(tx: &Connection, task: &Task) -> Result<u32> {
    let mut length = 1;
    let mut before = task.repeat_of;
    while let Some(id) = before {
        // `repeat_of` can be set, so a chain could loop; stop counting.
        if length >= 100_000 {
            break;
        }
        length += 1;
        let found: Option<Option<Vec<u8>>> = tx
            .query_row(
                "SELECT t.repeat_of FROM task t JOIN object o ON o.id = t.object WHERE o.uuid = ?1",
                [uuid_blob(id)],
                |row| row.get(0),
            )
            .optional()?;
        before = match found.flatten() {
            Some(blob) => Some(id_of_blob(&blob)?),
            None => None,
        };
    }
    Ok(length)
}

/// Makes the next task of a repetition that was just completed, with its
/// relative reminders (ADR 0033). `Some` says why none was made, for the
/// logbook; an earlier next task, even a deleted one, makes none and says
/// nothing.
fn follow_up(
    tx: &Connection,
    task: &Task,
    hlc: &Hlc,
    author: &Author,
    now: Timestamp,
) -> Result<Option<String>> {
    let id = uuid_blob(task.id);
    let made_before: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM task WHERE repeat_of = ?1)",
        [&id],
        |row| row.get(0),
    )?;
    if made_before {
        return Ok(None);
    }
    let has_children: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM task t JOIN object o ON o.id = t.object \
         WHERE t.parent = ?1 AND o.deleted_at IS NULL)",
        [&id],
        |row| row.get(0),
    )?;
    if has_children {
        return Ok(Some(
            "an action group does not repeat yet: no next task".into(),
        ));
    }
    let chain = chain_length(tx, task)?;
    let zone = settings_in(tx)?.zone;
    let completed = task.completed_at.unwrap_or(now);
    let schedule = match crate::core::repeat::next(&task.schedule, completed, now, chain, &zone) {
        Ok(Some(schedule)) => schedule,
        Ok(None) => return Ok(Some("the repetition has ended: no next task".into())),
        Err(e) => return Ok(Some(format!("no next task: {e}"))),
    };
    let mut properties = task.properties.clone();
    properties.remove("org_id");
    let next = Task {
        id: Id::new(),
        created: now,
        modified: now,
        schedule,
        completed_at: None,
        dropped_at: None,
        repeat_of: Some(task.id),
        position: task.position.saturating_add(1),
        properties,
        ..task.clone()
    };
    let mut owned: Vec<Vec<u8>> = Vec::new();
    for table in ["link", "reminder"] {
        let mut stmt = tx.prepare(&format!(
            "SELECT o.uuid FROM {table} x JOIN object o ON o.id = x.object \
             WHERE x.owner = ?1 AND o.deleted_at IS NULL ORDER BY o.id"
        ))?;
        let found = stmt
            .query_map([&id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<Vec<u8>>>>()?;
        owned.extend(found);
    }
    // All or nothing: a refused part leaves no half a task behind, and the
    // completion stands either way.
    tx.execute_batch("SAVEPOINT follow_up")?;
    let made = make_follow_up(tx, &next, &owned, hlc, author, now);
    match made {
        Ok(None) => {
            tx.execute_batch("RELEASE follow_up")?;
            Ok(None)
        }
        Ok(Some(reason)) => {
            tx.execute_batch("ROLLBACK TO follow_up; RELEASE follow_up")?;
            Ok(Some(format!("no next task: {reason}")))
        }
        Err(e) => {
            tx.execute_batch("ROLLBACK TO follow_up; RELEASE follow_up")?;
            Err(e)
        }
    }
}

/// Creates the next task and copies its links and the reminders relative
/// to its dates; `Some` is why the store refused.
fn make_follow_up(
    tx: &Connection,
    next: &Task,
    owned: &[Vec<u8>],
    hlc: &Hlc,
    author: &Author,
    now: Timestamp,
) -> Result<Option<Refused>> {
    let as_json = |value: serde_json::Result<Json>| {
        value.map_err(|e| Error::Corrupt {
            what: format!("the next task: {e}"),
        })
    };
    let value = as_json(serde_json::to_value(next))?;
    if let ChangeOutcome::Rejected { reason } =
        create(tx, next.id, ObjectKind::Task, &value, hlc, author, now)?
    {
        return Ok(Some(reason));
    }
    for blob in owned {
        let old = id_of_blob(blob)?;
        let Some(h) = head(tx, old)? else { continue };
        let (kind, copy) = match load(tx, old, &h)? {
            Object::Link(link) => (
                ObjectKind::Link,
                serde_json::to_value(Link {
                    id: Id::new(),
                    created: now,
                    modified: now,
                    owner: next.id,
                    ..link
                }),
            ),
            Object::Reminder(reminder)
                if matches!(
                    reminder.anchor,
                    ReminderAnchor::Due
                        | ReminderAnchor::Planned
                        | ReminderAnchor::Defer
                        | ReminderAnchor::Daily
                ) =>
            {
                (
                    ObjectKind::Reminder,
                    serde_json::to_value(Reminder {
                        id: Id::new(),
                        created: now,
                        modified: now,
                        owner: next.id,
                        ..reminder
                    }),
                )
            }
            _ => continue,
        };
        let value = as_json(copy)?;
        let Some(copy_id) = value["id"].as_str().and_then(|id| id.parse().ok()) else {
            continue;
        };
        if let ChangeOutcome::Rejected { reason } =
            create(tx, copy_id, kind, &value, hlc, author, now)?
        {
            return Ok(Some(reason));
        }
    }
    Ok(None)
}

/// The exclusive group a tag is a state of, and the other states in it
/// (ADR 0024 §1); none if its parent is not exclusive.
fn group_of(tx: &Connection, tag: Id) -> Result<Option<Vec<Id>>> {
    let parent: Option<Vec<u8>> = tx
        .query_row(
            "SELECT t.parent FROM tag t JOIN object o ON o.id = t.object WHERE o.uuid = ?1",
            [uuid_blob(tag)],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    let Some(parent) = parent else {
        return Ok(None);
    };
    let exclusive: bool = tx
        .query_row(
            "SELECT t.exclusive FROM tag t JOIN object o ON o.id = t.object WHERE o.uuid = ?1",
            [&parent],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(false);
    if !exclusive {
        return Ok(None);
    }
    let mut statement = tx.prepare(
        "SELECT o.uuid FROM tag t JOIN object o ON o.id = t.object \
         WHERE t.parent = ?1 AND o.uuid != ?2",
    )?;
    let rows = statement.query_map(params![parent, uuid_blob(tag)], |row| {
        row.get::<_, Vec<u8>>(0)
    })?;
    let mut siblings = Vec::new();
    for row in rows {
        siblings.push(id_of_blob(&row?)?);
    }
    Ok(Some(siblings))
}

/// A tag's element on an owner: present or not, with its stamp.
fn element_of(tx: &Connection, owner: i64, tag: Id) -> Result<Option<(bool, Stored)>> {
    Ok(tx
        .query_row(
            "SELECT present, wall, counter, device, seq FROM tagging WHERE owner = ?1 AND tag = ?2",
            params![owner, uuid_blob(tag)],
            |row| {
                Ok((
                    row.get::<_, bool>(0)?,
                    Stored {
                        hlc: Hlc::new(row.get(1)?, row.get(2)?, row.get::<_, String>(3)?),
                        seq: row.get(4)?,
                    },
                ))
            },
        )
        .optional()?)
}

/// `add` (with a position) and `remove` (without) on the tags of a task or
/// project.
fn tagging(
    tx: &Transaction<'_>,
    id: Id,
    field: &str,
    element: Id,
    position: Option<i64>,
    edit: &Edit<'_>,
) -> Result<ChangeOutcome> {
    let (hlc, now) = (edit.hlc, edit.now);
    let h = match live(tx, id)? {
        Ok(h) => h,
        Err(reason) => return Ok(rejected(reason)),
    };
    if field != "tags" || !matches!(h.kind, ObjectKind::Task | ObjectKind::Project) {
        return Ok(rejected(format!("{field} is not a set on this object")));
    }
    if position.is_some() {
        match head(tx, element)? {
            Some(tag) if tag.kind == ObjectKind::Tag && !tag.deleted => {}
            _ => return Ok(rejected(format!("no tag {element}"))),
        }
    }
    let current = element_of(tx, h.rowid, element)?;
    let was_present = current.as_ref().is_some_and(|(present, _)| *present);
    if let Some(answer) = decide(hlc, current.as_ref().map(|(_, stored)| stored)) {
        return Ok(answer);
    }
    // A state of a workflow: the group is one register per object. An add
    // sets it, a remove clears it, and either wins only over every stamp of
    // the siblings (ADR 0024 §2).
    let group = group_of(tx, element)?;
    let mut displaced = Vec::new();
    if let Some(siblings) = &group {
        for sibling in siblings {
            let Some((present, stored)) = element_of(tx, h.rowid, *sibling)? else {
                continue;
            };
            if !wins(hlc, Some(&stored.hlc)) {
                return Ok(ChangeOutcome::Superseded { by: stored.hlc });
            }
            if present {
                displaced.push(*sibling);
            }
        }
    }
    let seq = next_seq(tx)?;
    let present = i64::from(position.is_some());
    for sibling in &displaced {
        tx.execute(
            "UPDATE tagging SET present = 0, wall = ?3, counter = ?4, device = ?5, seq = ?6 \
             WHERE owner = ?1 AND tag = ?2",
            params![
                h.rowid,
                uuid_blob(*sibling),
                hlc.wall,
                hlc.counter,
                hlc.device,
                seq
            ],
        )?;
    }
    tx.execute(
        "INSERT INTO tagging (owner, tag, position, present, wall, counter, device, seq) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
         ON CONFLICT (owner, tag) DO UPDATE SET \
           position = coalesce(?9, position), present = ?4, \
           wall = ?5, counter = ?6, device = ?7, seq = ?8",
        params![
            h.rowid,
            uuid_blob(element),
            position.unwrap_or(0),
            present,
            hlc.wall,
            hlc.counter,
            hlc.device,
            seq,
            position
        ],
    )?;
    touch(tx, h.rowid, seq, now)?;
    observe(tx, hlc)?;
    if group.is_some() {
        let before = if was_present {
            Some(element)
        } else {
            displaced.first().copied()
        };
        let entry = match position {
            Some(_) if !was_present => Some((before, Some(element))),
            None if before.is_some() => Some((before, None)),
            _ => None,
        };
        if let Some((old, new)) = entry {
            log(tx, h.rowid, LogKind::State, edit, old, new)?;
        }
    }
    Ok(ChangeOutcome::Applied { seq: as_seq(seq) })
}

/// Whether `id` may become a member of a scope: a live folder, project or
/// tag.
fn member_kind(tx: &Connection, id: Id) -> Result<bool> {
    Ok(head(tx, id)?.is_some_and(|h| {
        !h.deleted
            && matches!(
                h.kind,
                ObjectKind::Folder | ObjectKind::Project | ObjectKind::Tag
            )
    }))
}

/// `add` and `remove` on the members of a scope (ADR 0035 §1): a set like
/// the tags, each element with its own stamp.
fn member(
    tx: &Transaction<'_>,
    id: Id,
    element: Id,
    present: bool,
    edit: &Edit<'_>,
) -> Result<ChangeOutcome> {
    let (hlc, now) = (edit.hlc, edit.now);
    let h = match live(tx, id)? {
        Ok(h) => h,
        Err(reason) => return Ok(rejected(reason)),
    };
    if h.kind != ObjectKind::Scope {
        return Ok(rejected("members is not a set on this object"));
    }
    if present && !member_kind(tx, element)? {
        return Ok(rejected(format!("no folder, project or tag {element}")));
    }
    let current: Option<Stored> = tx
        .query_row(
            "SELECT wall, counter, device, seq FROM scope_member WHERE owner = ?1 AND member = ?2",
            params![h.rowid, uuid_blob(element)],
            |row| {
                Ok(Stored {
                    hlc: Hlc::new(row.get(0)?, row.get(1)?, row.get::<_, String>(2)?),
                    seq: row.get(3)?,
                })
            },
        )
        .optional()?;
    if let Some(answer) = decide(hlc, current.as_ref()) {
        return Ok(answer);
    }
    let seq = next_seq(tx)?;
    tx.execute(
        "INSERT INTO scope_member (owner, member, present, wall, counter, device, seq) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
         ON CONFLICT (owner, member) DO UPDATE SET \
           present = ?3, wall = ?4, counter = ?5, device = ?6, seq = ?7",
        params![
            h.rowid,
            uuid_blob(element),
            i64::from(present),
            hlc.wall,
            hlc.counter,
            hlc.device,
            seq
        ],
    )?;
    touch(tx, h.rowid, seq, now)?;
    observe(tx, hlc)?;
    Ok(ChangeOutcome::Applied { seq: as_seq(seq) })
}

/// A field written per key: `properties.<key>` (ADR 0023) or
/// `property_links.<key>` (ADR 0054 §3), as the map's name and the key.
fn keyed(field: &str) -> Option<(&'static str, &str)> {
    ["properties", "property_links"]
        .into_iter()
        .find_map(|map| {
            field
                .strip_prefix(map)
                .and_then(|rest| rest.strip_prefix('.'))
                .map(|key| (map, key))
        })
}

/// `set` of one key of a map written per key: a string sets it, `null`
/// removes it; each key has its own stamp (ADR 0023 §2). The properties of
/// a task or project and the templates of the settings (ADR 0054 §3) share
/// the table `property`, told apart by the kind of their owner.
fn property(
    tx: &Transaction<'_>,
    id: Id,
    map: &str,
    key: &str,
    value: &Json,
    hlc: &Hlc,
    now: Timestamp,
) -> Result<ChangeOutcome> {
    let h = match live(tx, id)? {
        Ok(h) => h,
        Err(reason) => return Ok(rejected(reason)),
    };
    let fits = if map == "property_links" {
        h.kind == ObjectKind::Settings
    } else {
        matches!(h.kind, ObjectKind::Task | ObjectKind::Project)
    };
    if !fits {
        return Ok(rejected(if map == "property_links" {
            "only the settings have property links"
        } else {
            "only a task or project has properties"
        }));
    }
    if !rows::property_key(key) {
        return Ok(rejected(format!("{key:?} is not a property key")));
    }
    let value = match value {
        Json::String(text) => Some(text.as_str()),
        Json::Null => None,
        _ => return Ok(rejected(format!("{map}.{key}: a string or null"))),
    };
    let current: Option<Stored> = tx
        .query_row(
            "SELECT wall, counter, device, seq FROM property WHERE owner = ?1 AND key = ?2",
            params![h.rowid, key],
            |row| {
                Ok(Stored {
                    hlc: Hlc::new(row.get(0)?, row.get(1)?, row.get::<_, String>(2)?),
                    seq: row.get(3)?,
                })
            },
        )
        .optional()?;
    // A key never written is as old as the object.
    let current = match current {
        Some(stamp) => Some(stamp),
        None => stamp_of(tx, h.rowid, "@create")?,
    };
    if let Some(answer) = decide(hlc, current.as_ref()) {
        return Ok(answer);
    }
    let seq = next_seq(tx)?;
    tx.execute(
        "INSERT INTO property (owner, key, value, wall, counter, device, seq) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
         ON CONFLICT (owner, key) DO UPDATE SET \
           value = ?3, wall = ?4, counter = ?5, device = ?6, seq = ?7",
        params![h.rowid, key, value, hlc.wall, hlc.counter, hlc.device, seq],
    )?;
    touch(tx, h.rowid, seq, now)?;
    observe(tx, hlc)?;
    Ok(ChangeOutcome::Applied { seq: as_seq(seq) })
}

/// ADR 0076 §2: what still points at a folder, project or tag that is to be
/// deleted, as a refusal; none when nothing does. `POST /objects/{id}/delete`
/// clears it first and comes through.
fn still_pointed_at(tx: &Connection, id: Id, kind: ObjectKind) -> Result<Option<String>> {
    const LIVE: &str = "JOIN object o ON o.id = x.object AND o.deleted_at IS NULL";
    let checks: Vec<(String, &str)> = match kind {
        ObjectKind::Project => vec![(format!("SELECT count(*) FROM task x {LIVE} WHERE x.project = ?1"), "tasks")],
        ObjectKind::Folder => vec![
            (format!("SELECT count(*) FROM project x {LIVE} WHERE x.folder = ?1"), "projects"),
            (format!("SELECT count(*) FROM folder x {LIVE} WHERE x.parent = ?1"), "folders"),
        ],
        ObjectKind::Tag => vec![
            (format!("SELECT count(*) FROM tag x {LIVE} WHERE x.parent = ?1"), "tags below it"),
            (format!("SELECT count(*) FROM project x {LIVE} WHERE x.workflow = ?1"), "projects with it as workflow"),
            (
                "SELECT count(*) FROM tagging x JOIN object o ON o.id = x.owner AND o.deleted_at IS NULL \
                 WHERE x.tag = ?1 AND x.present = 1"
                    .to_string(),
                "tasks and projects tagged with it",
            ),
        ],
        _ => return Ok(None),
    };
    let mut found = Vec::new();
    let mut add = |sql: &str, what: &str| -> Result<()> {
        let n: i64 = tx.query_row(sql, [uuid_blob(id)], |row| row.get(0))?;
        if n > 0 {
            found.push(format!("{n} {what}"));
        }
        Ok(())
    };
    for (sql, what) in &checks {
        add(sql, what)?;
    }
    add(
        "SELECT count(*) FROM scope_member x JOIN object o ON o.id = x.owner AND o.deleted_at IS NULL \
         WHERE x.member = ?1 AND x.present = 1",
        "scopes",
    )?;
    Ok((!found.is_empty()).then(|| {
        format!(
            "{id} is still in use ({}): delete it with POST /objects/{{id}}/delete, \
             which moves or takes those off first",
            found.join(", ")
        )
    }))
}

fn delete(
    tx: &Transaction<'_>,
    id: Id,
    hlc: &Hlc,
    author: &Author,
    now: Timestamp,
    logged: bool,
) -> Result<ChangeOutcome> {
    let Some(h) = head(tx, id)? else {
        return Ok(rejected(format!("no object {id}")));
    };
    if h.deleted {
        return Ok(ChangeOutcome::Applied { seq: as_seq(h.seq) });
    }
    if h.kind == ObjectKind::Settings {
        return Ok(rejected("the settings of an account are not deleted"));
    }
    if let Some(reason) = still_pointed_at(tx, id, h.kind)? {
        return Ok(rejected(reason));
    }
    let appointment = match (h.kind == ObjectKind::Appointment).then(|| load(tx, id, &h)) {
        Some(object) => match object? {
            Object::Appointment(a) => Some(a),
            _ => None,
        },
        None => None,
    };
    // ADR 0068 §3.
    if let Some(a) = &appointment
        && let Err(reason) = calendar::writes::check_change(tx, a, None)?
    {
        return Ok(rejected(reason));
    }
    if let Err(reason) = same_kind_of_author(&h, author) {
        return Ok(rejected(reason));
    }
    let seq = next_seq(tx)?;
    tx.execute(
        "UPDATE object SET deleted_at = ?2, seq = ?3, modified = ?2 WHERE id = ?1",
        params![h.rowid, now.to_string(), seq],
    )?;
    write_stamp(tx, h.rowid, "@delete", hlc, seq)?;
    observe(tx, hlc)?;
    if let Some(a) = &appointment {
        calendar::writes::queued(tx, h.rowid, a, a.calendar, "@delete", None, hlc, now)?;
    }
    // Every kind into the logbook (ADR 0068 §3, ADR 0123 §1).
    let edit = Edit {
        hlc,
        note: None,
        author,
        now,
        logged,
    };
    log_appointment(tx, h.rowid, LogKind::Deleted, &edit, None)?;
    // A calendar takes its appointments and its source along (ADR 0047).
    if h.kind == ObjectKind::Calendar {
        calendar::forget(tx, id, h.rowid, hlc, now)?;
    }
    // A habit takes its entries along (ADR 0091).
    if h.kind == ObjectKind::Task {
        forget_entries(tx, id, hlc, now)?;
        forget_items(tx, id, hlc, now)?;
    }
    Ok(ChangeOutcome::Applied { seq: as_seq(seq) })
}

/// Makes tombstones of the checklist items of a deleted task (ADR 0141 §1).
fn forget_items(tx: &Transaction<'_>, task: Id, hlc: &Hlc, now: Timestamp) -> Result<()> {
    let rows: Vec<i64> = tx
        .prepare(
            "SELECT o.id FROM checklist_item i JOIN object o ON o.id = i.object \
             WHERE i.owner = ?1 AND o.deleted_at IS NULL",
        )?
        .query_map([uuid_blob(task)], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for rowid in rows {
        let seq = next_seq(tx)?;
        tx.execute(
            "UPDATE object SET deleted_at = ?2, seq = ?3, modified = ?2 WHERE id = ?1",
            params![rowid, now.to_string(), seq],
        )?;
        write_stamp(tx, rowid, "@delete", hlc, seq)?;
    }
    Ok(())
}

/// Makes tombstones of the entries of a deleted task, with the stamp of the
/// deletion (ADR 0091 §2).
fn forget_entries(tx: &Transaction<'_>, task: Id, hlc: &Hlc, now: Timestamp) -> Result<()> {
    let rows: Vec<i64> = tx
        .prepare(
            "SELECT o.id FROM habit_entry e JOIN object o ON o.id = e.object \
             WHERE e.habit = ?1 AND o.deleted_at IS NULL",
        )?
        .query_map([uuid_blob(task)], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    for rowid in rows {
        let seq = next_seq(tx)?;
        tx.execute(
            "UPDATE object SET deleted_at = ?2, seq = ?3, modified = ?2 WHERE id = ?1",
            params![rowid, now.to_string(), seq],
        )?;
        write_stamp(tx, rowid, "@delete", hlc, seq)?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use std::path::PathBuf;

    use super::*;
    use crate::Error;

    /// A fresh directory per test, removed at the end.
    pub(crate) struct TempDir(PathBuf);

    impl TempDir {
        pub(crate) fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "samu-store-{name}-{}-{}",
                std::process::id(),
                uuid::Uuid::now_v7()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        pub(crate) fn db(&self) -> PathBuf {
            self.0.join("account.db")
        }

        pub(crate) fn dir(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A file from before the delivery table gets it and keeps its rows.
    #[test]
    fn a_database_at_version_1_is_brought_up() {
        let dir = TempDir::new("migrate-1");
        {
            let conn = Connection::open(dir.db()).unwrap();
            conn.execute_batch(include_str!("migrations/0001_initial.sql"))
                .unwrap();
            conn.execute_batch(
                "INSERT INTO schema_history VALUES (1, '0001_initial', '2026-11-07T09:00:00Z'); \
                 PRAGMA user_version = 1;",
            )
            .unwrap();
        }
        let store = AccountStore::open(&dir.db()).unwrap();
        assert_eq!(store.migration().from, 1);
        assert_eq!(
            store.migration().applied,
            vec![
                "0002_delivery",
                "0003_property",
                "0004_workflow",
                "0005_reminder",
                "0006_scope",
                "0007_comment",
                "0008_scope_round_two",
                "0009_settings",
                "0010_calendar",
                "0011_forecast_events",
                "0012_priority",
                "0013_icon_and_scope_color",
                "0014_appointment_write",
                "0015_scope_only",
                "0016_scope_layout",
                "0017_template",
                "0018_settings_keys",
                "0019_report",
                "0020_template_key",
                "0021_perspective",
                "0022_habit",
                "0023_daily_reminder",
                "0024_own_scope",
                "0025_appointment_alarms",
                "0026_perspective_table",
                "0027_perspective_blocks",
                "0028_transcript",
                "0029_checklist"
            ]
        );
        drop(store);
        let conn = Connection::open(dir.db()).unwrap();
        let rows: i64 = conn
            .query_row("SELECT count(*) FROM delivery", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[test]
    fn a_new_database_is_migrated_once() {
        let dir = TempDir::new("migrate");
        {
            let store = AccountStore::open(&dir.db()).unwrap();
            assert_eq!(store.migration().from, 0);
            assert_eq!(store.migration().to, migrations::latest_version());
            assert_eq!(
                store.migration().applied,
                vec![
                    "0001_initial",
                    "0002_delivery",
                    "0003_property",
                    "0004_workflow",
                    "0005_reminder",
                    "0006_scope",
                    "0007_comment",
                    "0008_scope_round_two",
                    "0009_settings",
                    "0010_calendar",
                    "0011_forecast_events",
                    "0012_priority",
                    "0013_icon_and_scope_color",
                    "0014_appointment_write",
                    "0015_scope_only",
                    "0016_scope_layout",
                    "0017_template",
                    "0018_settings_keys",
                    "0019_report",
                    "0020_template_key",
                    "0021_perspective",
                    "0022_habit",
                    "0023_daily_reminder",
                    "0024_own_scope",
                    "0025_appointment_alarms",
                    "0026_perspective_table",
                    "0027_perspective_blocks",
                    "0028_transcript",
                    "0029_checklist"
                ]
            );
            // The settings object, made on opening (ADR 0045 §4).
            assert_eq!(store.seq().unwrap(), 1);
        }
        let again = AccountStore::open(&dir.db()).unwrap();
        assert!(again.migration().applied.is_empty(), "nothing runs twice");

        // Asked of the database, not of the report.
        let conn = Connection::open(dir.db()).unwrap();
        let history: i64 = conn
            .query_row("SELECT count(*) FROM schema_history", [], |r| r.get(0))
            .unwrap();
        assert_eq!(history, 29);
        let journal: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(journal, "wal");
    }

    #[test]
    fn a_newer_schema_is_refused() {
        let dir = TempDir::new("newer");
        {
            let conn = Connection::open(dir.db()).unwrap();
            conn.execute_batch("PRAGMA user_version = 999").unwrap();
        }
        let err = AccountStore::open(&dir.db()).unwrap_err();
        assert!(matches!(err, Error::Unsupported { .. }), "{err}");
    }

    #[test]
    fn a_second_writer_is_refused() {
        let dir = TempDir::new("lock");
        let _first = AccountStore::open(&dir.db()).unwrap();
        let err = AccountStore::open(&dir.db()).unwrap_err();
        assert!(matches!(err, Error::DatabaseInUse { .. }), "{err}");
    }

    /// ADR 0099 §5: removing an account takes its database, WAL, blobs and
    /// lock file, and is refused while someone holds the account.
    #[test]
    fn removing_an_account_takes_its_files_and_waits_for_its_holder() {
        let dir = TempDir::new("remove");
        let db = dir.db();
        let held = AccountStore::open(&db).unwrap();
        let blobs = db.with_extension("blobs");
        std::fs::create_dir_all(blobs.join("ab")).unwrap();
        std::fs::write(blobs.join("ab").join("abcd"), b"x").unwrap();
        let err = AccountStore::remove(&db).unwrap_err();
        assert!(matches!(err, Error::DatabaseInUse { .. }), "{err}");
        assert!(db.exists() && blobs.exists(), "a refusal removes nothing");
        drop(held);
        AccountStore::remove(&db).unwrap();
        let left: Vec<_> = std::fs::read_dir(dir.dir())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert!(left.is_empty(), "left behind: {left:?}");
        // Again, with nothing there: done already.
        AccountStore::remove(&db).unwrap();
    }

    #[test]
    fn the_lock_goes_with_the_store() {
        let dir = TempDir::new("relock");
        drop(AccountStore::open(&dir.db()).unwrap());
        AccountStore::open(&dir.db()).unwrap();
    }

    /// ADR 0030: `object` is rebuilt for the new kind; what hangs below it
    /// stays, and the foreign keys are on again afterwards.
    #[test]
    fn the_rebuild_for_reminders_keeps_every_object() {
        let dir = TempDir::new("migrate-4");
        {
            let conn = Connection::open(dir.db()).unwrap();
            for sql in [
                include_str!("migrations/0001_initial.sql"),
                include_str!("migrations/0002_delivery.sql"),
                include_str!("migrations/0003_property.sql"),
                include_str!("migrations/0004_workflow.sql"),
            ] {
                conn.execute_batch(sql).unwrap();
            }
            conn.execute_batch(
                "PRAGMA user_version = 4; \
                 INSERT INTO object (id, uuid, kind, created, modified, seq) \
                 VALUES (7, X'01936f5e8a007c3e9b1a3f2d4c5b6c01', 'task', \
                         '2026-11-07T08:00:00Z', '2026-11-07T08:00:00Z', 1); \
                 INSERT INTO task (object, title, due_date) VALUES (7, 'Steuer', '2026-11-20'); \
                 INSERT INTO stamp VALUES (7, '@create', 1000, 0, 'pixel', 1); \
                 INSERT INTO property (owner, key, value, wall, counter, device, seq) \
                 VALUES (7, 'source', 'test', 1000, 0, 'pixel', 1); \
                 UPDATE account SET seq = 1;",
            )
            .unwrap();
        }
        let store = AccountStore::open(&dir.db()).unwrap();
        assert_eq!(
            store.migration().applied,
            vec![
                "0005_reminder",
                "0006_scope",
                "0007_comment",
                "0008_scope_round_two",
                "0009_settings",
                "0010_calendar",
                "0011_forecast_events",
                "0012_priority",
                "0013_icon_and_scope_color",
                "0014_appointment_write",
                "0015_scope_only",
                "0016_scope_layout",
                "0017_template",
                "0018_settings_keys",
                "0019_report",
                "0020_template_key",
                "0021_perspective",
                "0022_habit",
                "0023_daily_reminder",
                "0024_own_scope",
                "0025_appointment_alarms",
                "0026_perspective_table",
                "0027_perspective_blocks",
                "0028_transcript",
                "0029_checklist"
            ]
        );
        let id: Id = "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6c01".parse().unwrap();
        let Some(Object::Task(task)) = store.get(id).unwrap() else {
            panic!("the task is gone");
        };
        assert_eq!(task.title, "Steuer");
        assert_eq!(task.properties["source"], "test");
        assert_eq!(
            store.settings().unwrap().remind_time,
            jiff::civil::time(9, 0, 0, 0)
        );

        // Asked of the database: the rows below the object are all there,
        // and a cascade works again.
        let count = |sql: &str| -> i64 { store.conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(
            count(
                "SELECT count(*) FROM stamp WHERE object IN (SELECT id FROM object WHERE kind <> 'settings')"
            ),
            1
        );
        assert_eq!(count("PRAGMA foreign_keys"), 1);
        store
            .conn
            .execute("DELETE FROM object WHERE id = 7", [])
            .unwrap();
        assert_eq!(count("SELECT count(*) FROM task"), 0);
        assert_eq!(
            count(
                "SELECT count(*) FROM stamp WHERE object IN (SELECT id FROM object WHERE kind <> 'settings')"
            ),
            0
        );
        assert_eq!(count("SELECT count(*) FROM property"), 0);
    }

    /// ADR 0035: `object` is rebuilt once more for scopes; a reminder made
    /// at schema 5 keeps its row, and the foreign keys hold afterwards.
    #[test]
    fn the_rebuild_for_scopes_keeps_every_object() {
        let dir = TempDir::new("migrate-5");
        {
            let conn = Connection::open(dir.db()).unwrap();
            for sql in [
                include_str!("migrations/0001_initial.sql"),
                include_str!("migrations/0002_delivery.sql"),
                include_str!("migrations/0003_property.sql"),
                include_str!("migrations/0004_workflow.sql"),
            ] {
                conn.execute_batch(sql).unwrap();
            }
            conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
            conn.execute_batch(include_str!("migrations/0005_reminder.sql"))
                .unwrap();
            conn.pragma_update(None, "foreign_keys", "ON").unwrap();
            conn.execute_batch(
                "PRAGMA user_version = 5; \
                 INSERT INTO object (id, uuid, kind, created, modified, seq) \
                 VALUES (7, X'01936f5e8a007c3e9b1a3f2d4c5b6c01', 'task', \
                         '2026-11-07T08:00:00Z', '2026-11-07T08:00:00Z', 1), \
                        (8, X'01936f5e8a007c3e9b1a3f2d4c5b6e22', 'reminder', \
                         '2026-11-07T08:00:00Z', '2026-11-07T08:00:00Z', 2); \
                 INSERT INTO task (object, title, due_date) VALUES (7, 'Steuer', '2026-11-20'); \
                 INSERT INTO reminder (object, owner, anchor, before) \
                 VALUES (8, X'01936f5e8a007c3e9b1a3f2d4c5b6c01', 'due', 'PT1H'); \
                 INSERT INTO stamp VALUES (7, '@create', 1000, 0, 'pixel', 1); \
                 INSERT INTO stamp VALUES (8, '@create', 1001, 0, 'pixel', 2); \
                 UPDATE account SET seq = 2;",
            )
            .unwrap();
        }
        let store = AccountStore::open(&dir.db()).unwrap();
        assert_eq!(
            store.migration().applied,
            vec![
                "0006_scope",
                "0007_comment",
                "0008_scope_round_two",
                "0009_settings",
                "0010_calendar",
                "0011_forecast_events",
                "0012_priority",
                "0013_icon_and_scope_color",
                "0014_appointment_write",
                "0015_scope_only",
                "0016_scope_layout",
                "0017_template",
                "0018_settings_keys",
                "0019_report",
                "0020_template_key",
                "0021_perspective",
                "0022_habit",
                "0023_daily_reminder",
                "0024_own_scope",
                "0025_appointment_alarms",
                "0026_perspective_table",
                "0027_perspective_blocks",
                "0028_transcript",
                "0029_checklist"
            ]
        );
        let reminder: Id = "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6e22".parse().unwrap();
        assert!(matches!(
            store.get(reminder).unwrap(),
            Some(Object::Reminder(_))
        ));

        let count = |sql: &str| -> i64 { store.conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(
            count("SELECT count(*) FROM object WHERE kind <> 'settings'"),
            2
        );
        assert_eq!(
            count(
                "SELECT count(*) FROM stamp WHERE object IN (SELECT id FROM object WHERE kind <> 'settings')"
            ),
            2
        );
        assert_eq!(count("SELECT count(*) FROM scope"), 0);
        assert_eq!(count("PRAGMA foreign_keys"), 1);
        store
            .conn
            .execute("DELETE FROM object WHERE id = 8", [])
            .unwrap();
        assert_eq!(count("SELECT count(*) FROM reminder"), 0);
        assert_eq!(
            count(
                "SELECT count(*) FROM stamp WHERE object IN (SELECT id FROM object WHERE kind <> 'settings')"
            ),
            1
        );
    }

    /// ADR 0043 §1: a scope with `only` gets the inbox ticked, one with
    /// `except` does not, and each ticked one gets a change number of its own
    /// so clients fetch it again.
    #[test]
    fn the_second_round_ticks_the_inbox_where_only_holds() {
        let dir = TempDir::new("migrate-7");
        {
            let conn = Connection::open(dir.db()).unwrap();
            for sql in [
                include_str!("migrations/0001_initial.sql"),
                include_str!("migrations/0002_delivery.sql"),
                include_str!("migrations/0003_property.sql"),
                include_str!("migrations/0004_workflow.sql"),
            ] {
                conn.execute_batch(sql).unwrap();
            }
            conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
            for sql in [
                include_str!("migrations/0005_reminder.sql"),
                include_str!("migrations/0006_scope.sql"),
                include_str!("migrations/0007_comment.sql"),
            ] {
                conn.execute_batch(sql).unwrap();
            }
            conn.pragma_update(None, "foreign_keys", "ON").unwrap();
            conn.execute_batch(
                "PRAGMA user_version = 7; \
                 INSERT INTO object (id, uuid, kind, created, modified, seq) \
                 VALUES (7, X'01936f5e8a007c3e9b1a3f2d4c5b6f01', 'scope', \
                         '2026-11-07T08:00:00Z', '2026-11-07T08:00:00Z', 1), \
                        (8, X'01936f5e8a007c3e9b1a3f2d4c5b6f02', 'scope', \
                         '2026-11-07T08:00:00Z', '2026-11-07T08:00:00Z', 2), \
                        (9, X'01936f5e8a007c3e9b1a3f2d4c5b6f03', 'scope', \
                         '2026-11-07T08:00:00Z', '2026-11-07T08:00:00Z', 3); \
                 INSERT INTO scope (object, title, mode) \
                 VALUES (7, 'Arbeit', 'only'), (8, 'Ohne IT', 'except'), (9, 'Privat', 'only'); \
                 INSERT INTO stamp VALUES (7, '@create', 1000, 0, 'pixel', 1); \
                 INSERT INTO stamp VALUES (8, '@create', 1001, 0, 'pixel', 2); \
                 INSERT INTO stamp VALUES (9, '@create', 1002, 0, 'pixel', 3); \
                 UPDATE account SET seq = 3;",
            )
            .unwrap();
        }
        let store = AccountStore::open(&dir.db()).unwrap();
        assert_eq!(
            store.migration().applied,
            vec![
                "0008_scope_round_two",
                "0009_settings",
                "0010_calendar",
                "0011_forecast_events",
                "0012_priority",
                "0013_icon_and_scope_color",
                "0014_appointment_write",
                "0015_scope_only",
                "0016_scope_layout",
                "0017_template",
                "0018_settings_keys",
                "0019_report",
                "0020_template_key",
                "0021_perspective",
                "0022_habit",
                "0023_daily_reminder",
                "0024_own_scope",
                "0025_appointment_alarms",
                "0026_perspective_table",
                "0027_perspective_blocks",
                "0028_transcript",
                "0029_checklist"
            ]
        );

        // Asked of the database: which got the tick, and the change numbers.
        let rows: Vec<(String, i64, i64)> = store
            .conn
            .prepare(
                "SELECT s.title, s.inbox, o.seq FROM scope s JOIN object o ON o.id = s.object \
                 ORDER BY o.id",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            rows,
            vec![
                ("Arbeit".into(), 1, 4),
                ("Ohne IT".into(), 0, 2),
                ("Privat".into(), 1, 5),
            ]
        );
        // 5, then the settings object made on opening (ADR 0045 §4).
        assert_eq!(store.seq().unwrap(), 6);
        let changed: Vec<Id> = store
            .changed_after(3)
            .unwrap()
            .iter()
            .map(|c| c.id)
            .collect();
        // The two ticked scopes, and the settings object made on opening.
        assert_eq!(changed.len(), 3);

        // The typed object reads the tick, and an empty filter stays NULL.
        let arbeit: Id = "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6f01".parse().unwrap();
        let Some(Object::Scope(scope)) = store.get(arbeit).unwrap() else {
            panic!("the scope is gone");
        };
        assert!(scope.inbox);
        assert!(scope.filter.is_empty() && scope.schedule.is_empty());
        assert_eq!(scope.capture, None);
    }

    /// ADR 0045 §4: opening makes the one settings object with the
    /// account's values, once; it is set like any object, never created or
    /// deleted by a client, and the forecast tag must be a live tag.
    #[test]
    fn the_settings_are_one_object_made_on_opening() {
        let dir = TempDir::new("settings");
        let id = {
            let store = AccountStore::open(&dir.db()).unwrap();
            let snapshot = store.snapshot().unwrap();
            let settings = snapshot.settings.clone().unwrap();
            assert_eq!(settings.due_soon.0.to_string(), "P2D");
            assert_eq!(settings.remind_time, jiff::civil::time(9, 0, 0, 0));
            assert_eq!(
                (settings.forecast_days, settings.forecast_flagged),
                (7, true)
            );
            settings.id
        };
        let mut store = AccountStore::open(&dir.db()).unwrap();
        let count = |store: &AccountStore| -> i64 {
            store
                .conn
                .query_row(
                    "SELECT count(*) FROM object WHERE kind = 'settings'",
                    [],
                    |r| r.get(0),
                )
                .unwrap()
        };
        assert_eq!(count(&store), 1, "made once");
        let person = Author {
            device: "laptop".into(),
            kind: AuthorKind::Human,
        };
        let set = |field: &str, value: Json, wall: i64| Change::Set {
            object: id,
            field: field.into(),
            value,
            note: None,
            hlc: Hlc::new(wall, 0, "laptop"),
        };
        let now = Timestamp::now();
        let wall = now.as_millisecond() + 1_000;
        let applied = |o: &ChangeOutcome| matches!(o, ChangeOutcome::Applied { .. });
        assert!(applied(
            &store
                .apply(
                    &set("forecast_days", serde_json::json!(14), wall),
                    &person,
                    now
                )
                .unwrap()
        ));
        assert!(applied(
            &store
                .apply(
                    &set("due_soon", serde_json::json!("P3D"), wall + 1),
                    &person,
                    now
                )
                .unwrap()
        ));
        assert_eq!(
            store.settings().unwrap().due_soon.to_string(),
            "P3D",
            "availability reads it"
        );
        let refused = [
            set("forecast_days", serde_json::json!(0), wall + 2),
            set("forecast_tag", serde_json::json!(Id::new()), wall + 3),
            Change::Delete {
                object: id,
                hlc: Hlc::new(wall + 4, 0, "laptop"),
            },
            Change::Create {
                object: Id::new(),
                kind: ObjectKind::Settings,
                value: serde_json::json!({}),
                hlc: Hlc::new(wall + 5, 0, "laptop"),
            },
        ];
        for change in refused {
            let outcome = store.apply(&change, &person, now).unwrap();
            assert!(
                matches!(outcome, ChangeOutcome::Rejected { .. }),
                "{change:?}: {outcome:?}"
            );
        }
        let settings = store.snapshot().unwrap().settings.clone().unwrap();
        assert_eq!(settings.forecast_days, 14);
        assert_eq!(count(&store), 1);
    }

    /// ADR 0039: `object` is rebuilt for comments and attachments and gains
    /// the author; what was there keeps its rows and has no author, the
    /// logbook keeps its entries, and the foreign keys hold afterwards.
    #[test]
    fn the_rebuild_for_comments_keeps_every_object() {
        let dir = TempDir::new("migrate-6");
        {
            let conn = Connection::open(dir.db()).unwrap();
            for sql in [
                include_str!("migrations/0001_initial.sql"),
                include_str!("migrations/0002_delivery.sql"),
                include_str!("migrations/0003_property.sql"),
                include_str!("migrations/0004_workflow.sql"),
            ] {
                conn.execute_batch(sql).unwrap();
            }
            conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
            conn.execute_batch(include_str!("migrations/0005_reminder.sql"))
                .unwrap();
            conn.execute_batch(include_str!("migrations/0006_scope.sql"))
                .unwrap();
            conn.pragma_update(None, "foreign_keys", "ON").unwrap();
            conn.execute_batch(
                "PRAGMA user_version = 6; \
                 INSERT INTO object (id, uuid, kind, created, modified, seq) \
                 VALUES (7, X'01936f5e8a007c3e9b1a3f2d4c5b6c01', 'task', \
                         '2026-11-07T08:00:00Z', '2026-11-07T08:00:00Z', 1), \
                        (9, X'01936f5e8a007c3e9b1a3f2d4c5b6e31', 'scope', \
                         '2026-11-07T08:00:00Z', '2026-11-07T08:00:00Z', 2); \
                 INSERT INTO task (object, title) VALUES (7, 'Steuer'); \
                 INSERT INTO scope (object, title, mode) VALUES (9, 'Arbeit', 'only'); \
                 INSERT INTO stamp VALUES (7, '@create', 1000, 0, 'pixel', 1); \
                 INSERT INTO stamp VALUES (9, '@create', 1001, 0, 'pixel', 2); \
                 INSERT INTO logbook (object, at, kind, device) \
                 VALUES (7, '2026-11-07T09:00:00Z', 'completed', 'pixel'); \
                 UPDATE account SET seq = 2;",
            )
            .unwrap();
        }
        let store = AccountStore::open(&dir.db()).unwrap();
        assert_eq!(
            store.migration().applied,
            vec![
                "0007_comment",
                "0008_scope_round_two",
                "0009_settings",
                "0010_calendar",
                "0011_forecast_events",
                "0012_priority",
                "0013_icon_and_scope_color",
                "0014_appointment_write",
                "0015_scope_only",
                "0016_scope_layout",
                "0017_template",
                "0018_settings_keys",
                "0019_report",
                "0020_template_key",
                "0021_perspective",
                "0022_habit",
                "0023_daily_reminder",
                "0024_own_scope",
                "0025_appointment_alarms",
                "0026_perspective_table",
                "0027_perspective_blocks",
                "0028_transcript",
                "0029_checklist"
            ]
        );
        let task: Id = "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6c01".parse().unwrap();
        match store.get(task).unwrap() {
            Some(Object::Task(t)) => assert_eq!(t.created_by, None),
            other => panic!("not a task: {other:?}"),
        }
        let log = store.logbook(task).unwrap().unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].author_kind, None);

        let count = |sql: &str| -> i64 { store.conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(
            count("SELECT count(*) FROM object WHERE kind <> 'settings'"),
            2
        );
        assert_eq!(
            count("SELECT count(*) FROM object WHERE author_kind IS NULL AND kind <> 'settings'"),
            2
        );
        assert_eq!(
            count(
                "SELECT count(*) FROM stamp WHERE object IN (SELECT id FROM object WHERE kind <> 'settings')"
            ),
            2
        );
        assert_eq!(count("SELECT count(*) FROM scope"), 1);
        assert_eq!(count("SELECT count(*) FROM comment"), 0);
        assert_eq!(count("PRAGMA foreign_keys"), 1);
        store
            .conn
            .execute("DELETE FROM object WHERE id = 7", [])
            .unwrap();
        assert_eq!(count("SELECT count(*) FROM task"), 0);
        assert_eq!(count("SELECT count(*) FROM logbook"), 0);
        assert_eq!(
            count(
                "SELECT count(*) FROM stamp WHERE object IN (SELECT id FROM object WHERE kind <> 'settings')"
            ),
            1
        );
    }

    /// ADR 0047: `object` is rebuilt for calendars and appointments; what
    /// was there keeps its rows, stamps, logbook and author, a scope gets no
    /// calendars, and the foreign keys hold afterwards.
    #[test]
    fn the_rebuild_for_calendars_keeps_every_object() {
        let dir = TempDir::new("migrate-9");
        {
            let conn = Connection::open(dir.db()).unwrap();
            for sql in [
                include_str!("migrations/0001_initial.sql"),
                include_str!("migrations/0002_delivery.sql"),
                include_str!("migrations/0003_property.sql"),
                include_str!("migrations/0004_workflow.sql"),
            ] {
                conn.execute_batch(sql).unwrap();
            }
            conn.pragma_update(None, "foreign_keys", "OFF").unwrap();
            for sql in [
                include_str!("migrations/0005_reminder.sql"),
                include_str!("migrations/0006_scope.sql"),
                include_str!("migrations/0007_comment.sql"),
                include_str!("migrations/0008_scope_round_two.sql"),
                include_str!("migrations/0009_settings.sql"),
            ] {
                conn.execute_batch(sql).unwrap();
            }
            conn.pragma_update(None, "foreign_keys", "ON").unwrap();
            conn.execute_batch(
                "PRAGMA user_version = 9; \
                 INSERT INTO object (id, uuid, kind, created, modified, seq, author_device, author_kind) \
                 VALUES (7, X'01936f5e8a007c3e9b1a3f2d4c5b6c01', 'task', \
                         '2026-11-07T08:00:00Z', '2026-11-07T08:00:00Z', 1, 'pixel', 'human'), \
                        (9, X'01936f5e8a007c3e9b1a3f2d4c5b6e31', 'scope', \
                         '2026-11-07T08:00:00Z', '2026-11-07T08:00:00Z', 2, NULL, NULL); \
                 INSERT INTO task (object, title) VALUES (7, 'Steuer'); \
                 INSERT INTO scope (object, title, mode) VALUES (9, 'Arbeit', 'only'); \
                 INSERT INTO stamp VALUES (7, '@create', 1000, 0, 'pixel', 1); \
                 INSERT INTO stamp VALUES (9, '@create', 1001, 0, 'pixel', 2); \
                 INSERT INTO logbook (object, at, kind, device, author_kind) \
                 VALUES (7, '2026-11-07T09:00:00Z', 'completed', 'pixel', 'human'); \
                 UPDATE account SET seq = 2;",
            )
            .unwrap();
        }
        let store = AccountStore::open(&dir.db()).unwrap();
        assert_eq!(
            store.migration().applied,
            vec![
                "0010_calendar",
                "0011_forecast_events",
                "0012_priority",
                "0013_icon_and_scope_color",
                "0014_appointment_write",
                "0015_scope_only",
                "0016_scope_layout",
                "0017_template",
                "0018_settings_keys",
                "0019_report",
                "0020_template_key",
                "0021_perspective",
                "0022_habit",
                "0023_daily_reminder",
                "0024_own_scope",
                "0025_appointment_alarms",
                "0026_perspective_table",
                "0027_perspective_blocks",
                "0028_transcript",
                "0029_checklist"
            ]
        );
        let task: Id = "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6c01".parse().unwrap();
        match store.get(task).unwrap() {
            Some(Object::Task(t)) => {
                assert_eq!(t.title, "Steuer");
                assert_eq!(t.created_by.map(|a| a.device).as_deref(), Some("pixel"));
            }
            other => panic!("not a task: {other:?}"),
        }
        let scope: Id = "01936f5e-8a00-7c3e-9b1a-3f2d4c5b6e31".parse().unwrap();
        match store.get(scope).unwrap() {
            Some(Object::Scope(s)) => assert!(s.calendars.is_empty()),
            other => panic!("not a scope: {other:?}"),
        }
        assert_eq!(store.logbook(task).unwrap().unwrap().len(), 1);
        let count = |sql: &str| -> i64 { store.conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(
            count("SELECT count(*) FROM object WHERE kind <> 'settings'"),
            2
        );
        assert_eq!(
            count(
                "SELECT count(*) FROM stamp WHERE object IN (SELECT id FROM object WHERE kind <> 'settings')"
            ),
            2
        );
        assert_eq!(count("SELECT count(*) FROM calendar"), 0);
        assert_eq!(count("PRAGMA foreign_keys"), 1);
        assert_eq!(count("SELECT count(*) FROM pragma_foreign_key_check"), 0);
    }

    #[test]
    fn checks_refuse_an_unknown_value() {
        let store = AccountStore::open_in_memory().unwrap();
        let err = store.conn.execute(
            "INSERT INTO object (uuid, kind, created, modified, seq) \
             VALUES (zeroblob(16), 'unknown', 'x', 'x', 1)",
            [],
        );
        assert!(err.is_err(), "an unknown kind must not be stored");
    }
}
