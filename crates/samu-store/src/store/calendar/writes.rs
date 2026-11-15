//! Appointments written back (ADR 0068): a client's change taken in a
//! writable calendar, the queue of resources still to be written, and what
//! the writer learns from the CalDAV server.
//!
//! A change from a client goes through `apply` like any other; here it is
//! checked, queued in the same transaction, and the server's writer picks
//! the queue up. What the writer reports back is written as the server,
//! like a fetch.

use std::collections::{BTreeMap, BTreeSet};

use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::Value as Json;

use super::super::{Head, head, load, settings_in, uuid_blob};
use super::{
    SecretKey, StoredSource, create_appointment, differing, live_calendar, server_set,
    server_stamp, tombstone, write_fields,
};
use crate::core::api::{Appointment, Hlc, Id, ObjectKind};
use crate::core::calendar::edit;
use crate::core::calendar::{self as cal, Read};
use crate::rows::Object;
use crate::{AccountStore, Error, Result};

type Refused = String;

/// A row of `appointment_write`: version, attempts, fields, from where.
type EntryRow = (i64, u32, String, Option<i64>, Option<String>);

/// One pending change in the queue: object, field (`@create` and
/// `@delete` too), stamp, and the field's value before it.
pub type Pending = (Id, String, Hlc, Json);

/// A live calendar samu may write to: CalDAV and `writable`.
fn writable_calendar(tx: &Connection, id: Id) -> Result<Option<i64>> {
    Ok(tx
        .query_row(
            "SELECT c.object FROM calendar c JOIN object o ON o.id = c.object \
             WHERE o.uuid = ?1 AND o.deleted_at IS NULL AND c.writable = 1 AND c.kind = 'caldav'",
            [uuid_blob(id)],
            |row| row.get(0),
        )
        .optional()?)
}

fn calendar_rowid(tx: &Connection, id: Id) -> Result<Option<i64>> {
    Ok(head(tx, id)?
        .filter(|h| h.kind == ObjectKind::Calendar)
        .map(|h| h.rowid))
}

/// The appointments of one UID in a calendar, deleted ones too when asked.
fn of_uid(
    tx: &Connection,
    calendar: Id,
    uid: &str,
    deleted_too: bool,
) -> Result<Vec<(Head, Appointment)>> {
    let mut statement = tx.prepare(
        "SELECT o.uuid FROM appointment a JOIN object o ON o.id = a.object \
         WHERE a.calendar = ?1 AND a.uid = ?2 AND (?3 OR o.deleted_at IS NULL) ORDER BY o.id",
    )?;
    let blobs = statement
        .query_map(params![uuid_blob(calendar), uid, deleted_too], |row| {
            row.get::<_, Vec<u8>>(0)
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::new();
    for blob in blobs {
        let id = super::super::id_of_blob(&blob)?;
        let Some(h) = head(tx, id)? else { continue };
        if h.deleted {
            // A tombstone keeps its row; read it as it was.
            if let Some(a) = load_any(tx, id, &h)? {
                out.push((h, a));
            }
        } else if let Object::Appointment(a) = load(tx, id, &h)? {
            out.push((h, a));
        }
    }
    Ok(out)
}

fn load_any(tx: &Connection, id: Id, h: &Head) -> Result<Option<Appointment>> {
    Ok(match load(tx, id, h)? {
        Object::Appointment(a) => Some(a),
        _ => None,
    })
}

/// Whether a client may make this appointment (ADR 0068 §3): in a
/// writable calendar, no invitation, writable values; a series with a new
/// UID there, an instance of a series that is there.
pub(in crate::store) fn check_create(
    tx: &Connection,
    a: &Appointment,
) -> Result<Result<(), Refused>> {
    if writable_calendar(tx, a.calendar)?.is_none() {
        return Ok(Err(format!("calendar {} is not writable", a.calendar)));
    }
    if a.meeting {
        return Ok(Err("meeting is the server's".into()));
    }
    if let Err(e) = edit::check(a) {
        return Ok(Err(e.to_string()));
    }
    let same_uid = of_uid(tx, a.calendar, &a.uid, false)?;
    match &a.recurrence_id {
        None if !same_uid.is_empty() => Ok(Err(format!("UID {} is taken in this calendar", a.uid))),
        None => Ok(Ok(())),
        Some(rid) => {
            let key = cal::key_of(rid);
            if !same_uid.iter().any(|(_, m)| m.recurrence_id.is_none()) {
                return Ok(Err(format!("no series {} to change an instance of", a.uid)));
            }
            if same_uid
                .iter()
                .any(|(_, o)| o.recurrence_id.as_ref().map(cal::key_of).as_ref() == Some(&key))
            {
                return Ok(Err("that instance is changed already".into()));
            }
            Ok(Ok(()))
        }
    }
}

/// Whether a client may change (`after` some) or delete this appointment.
pub(in crate::store) fn check_change(
    tx: &Connection,
    before: &Appointment,
    after: Option<&Appointment>,
) -> Result<Result<(), Refused>> {
    if writable_calendar(tx, before.calendar)?.is_none() {
        return Ok(Err(format!("calendar {} is not writable", before.calendar)));
    }
    if before.meeting {
        return Ok(Err("an invitation; change it in its calendar".into()));
    }
    if let Some(after) = after {
        if after.calendar != before.calendar && writable_calendar(tx, after.calendar)?.is_none() {
            return Ok(Err(format!("calendar {} is not writable", after.calendar)));
        }
        if let Err(e) = edit::check(after) {
            return Ok(Err(e.to_string()));
        }
    }
    Ok(Ok(()))
}

/// Queues the resource of `a` for the writer, `field` being what changed
/// (`@create`, `@delete` or a field) and `base` its value before. A move
/// to another calendar takes the queue entry of the old one along and
/// keeps where the resource was.
#[expect(
    clippy::too_many_arguments,
    reason = "all of one change, from three places"
)]
pub(in crate::store) fn queued(
    tx: &Connection,
    rowid: i64,
    a: &Appointment,
    before_calendar: Id,
    field: &str,
    base: Option<&Json>,
    hlc: &Hlc,
    now: Timestamp,
) -> Result<()> {
    let Some(calendar) = calendar_rowid(tx, a.calendar)? else {
        return Ok(());
    };
    let base = base.cloned().unwrap_or(Json::Null);
    let entry = serde_json::to_string(&(a.id, field, hlc, base)).map_err(|e| Error::Corrupt {
        what: format!("queue entry: {e}"),
    })?;
    let mut from: (Option<i64>, Option<String>) = (None, None);
    let mut carried = String::from("[]");
    if before_calendar != a.calendar
        && let Some(old) = calendar_rowid(tx, before_calendar)?
    {
        let href: Option<String> = tx.query_row(
            "SELECT href FROM appointment WHERE object = ?1",
            [rowid],
            |row| row.get(0),
        )?;
        let old_entry: Option<(String, Option<i64>, Option<String>)> = tx
            .query_row(
                "SELECT fields, from_calendar, from_href FROM appointment_write \
                 WHERE calendar = ?1 AND uid = ?2",
                params![old, a.uid],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        tx.execute(
            "DELETE FROM appointment_write WHERE calendar = ?1 AND uid = ?2",
            params![old, a.uid],
        )?;
        from = match old_entry {
            Some((fields, Some(c), h)) => {
                carried = fields;
                (Some(c), h)
            }
            Some((fields, None, _)) => {
                carried = fields;
                (Some(old), href.clone())
            }
            None => (Some(old), href.clone()),
        };
        if from.1.is_none() {
            // Never written over there: nothing to delete.
            from = (None, None);
        }
        // In the new calendar it is a new resource.
        tx.execute(
            "UPDATE appointment SET href = NULL, etag = NULL WHERE object = ?1",
            [rowid],
        )?;
    }
    tx.execute(
        "INSERT INTO appointment_write (calendar, uid, from_calendar, from_href, fields, since, next_try) \
         VALUES (?1, ?2, ?3, ?4, json_insert(?5, '$[#]', json(?6)), ?7, ?7) \
         ON CONFLICT (calendar, uid) DO UPDATE SET \
           fields = json_insert(fields, '$[#]', json(?6)), \
           version = version + 1, attempts = 0, last_error = NULL, next_try = ?7, \
           from_calendar = COALESCE(from_calendar, excluded.from_calendar), \
           from_href = COALESCE(from_href, excluded.from_href)",
        params![calendar, a.uid, from.0, from.1, carried, entry, now.to_string()],
    )?;
    Ok(())
}

/// One resource for the writer (ADR 0068 §6).
#[derive(Debug, Clone)]
pub struct WriteJob {
    pub calendar: Id,
    pub uid: String,
    /// To tell whether a change came while the writer was at it.
    pub version: i64,
    pub attempts: u32,
    /// What it should hold now; empty when it is to be deleted.
    pub appointments: Vec<Appointment>,
    /// Where it is on the server, none when it was never written there.
    pub href: Option<String>,
    pub etag: Option<String>,
    /// The collection, opened.
    pub source: StoredSource,
    /// Where it was before a move, to delete it there: that collection and
    /// the resource.
    pub from: Option<(StoredSource, String)>,
    /// What clients changed and the server has not written.
    pub fields: Vec<Pending>,
    /// The account's zone, for floating times.
    pub zone: TimeZone,
}

/// What a calendar holds of one UID, for an import (ADR 0100 §4).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KnownResource {
    /// Where it is on the server, none when samu never learnt it.
    pub href: Option<String>,
    pub etag: Option<String>,
    /// A change from samu waits to be written for it.
    pub pending: bool,
}

impl AccountStore {
    /// The UIDs a calendar holds or is about to write, with where each is
    /// on the server (ADR 0100 §4).
    pub fn resources_by_uid(&self, calendar: Id) -> Result<BTreeMap<String, KnownResource>> {
        let mut out: BTreeMap<String, KnownResource> = BTreeMap::new();
        let mut statement = self.conn.prepare(
            "SELECT a.uid, a.href, a.etag FROM appointment a JOIN object o ON o.id = a.object \
             WHERE a.calendar = ?1 AND o.deleted_at IS NULL ORDER BY a.href IS NULL",
        )?;
        let rows = statement
            .query_map([uuid_blob(calendar)], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for (uid, href, etag) in rows {
            let known = out.entry(uid).or_default();
            if known.href.is_none() && href.is_some() {
                known.href = href;
                known.etag = etag;
            }
        }
        let mut statement = self.conn.prepare(
            "SELECT w.uid FROM appointment_write w JOIN object o ON o.id = w.calendar \
             WHERE o.uuid = ?1",
        )?;
        let pending = statement
            .query_map([uuid_blob(calendar)], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for uid in pending {
            out.entry(uid).or_default().pending = true;
        }
        Ok(out)
    }

    /// One logbook entry `imported` on a calendar, with what came of it
    /// (ADR 0100 §4).
    pub fn log_import(
        &mut self,
        calendar: Id,
        note: &str,
        device: &str,
        author: crate::core::api::AuthorKind,
        now: Timestamp,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        if let Some(h) = live_calendar(&tx, calendar)? {
            tx.execute(
                "INSERT INTO logbook (object, at, kind, note, device, author_kind) \
                 VALUES (?1, ?2, 'imported', ?3, ?4, ?5)",
                params![
                    h.rowid,
                    now.to_string(),
                    note,
                    device,
                    super::super::author_kind_name(author)
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Sets whether samu may write to a calendar (ADR 0068 §1).
    pub fn set_writable(&mut self, id: Id, writable: bool, now: Timestamp) -> Result<()> {
        let tx = self.conn.transaction()?;
        if let Some(h) = live_calendar(&tx, id)? {
            server_set(&tx, id, &h, now, |c| c.writable = writable)?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The resources whose time to be written has come.
    pub fn due_writes(&self, now: Timestamp) -> Result<Vec<(Id, String)>> {
        let mut statement = self.conn.prepare(
            "SELECT o.uuid, w.uid FROM appointment_write w JOIN object o ON o.id = w.calendar \
             WHERE o.deleted_at IS NULL AND w.next_try <= ?1 ORDER BY w.since",
        )?;
        let rows = statement
            .query_map([now.to_string()], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(blob, uid)| Ok((super::super::id_of_blob(&blob)?, uid)))
            .collect()
    }

    /// What the writer needs for one resource. `None` when there is no such
    /// entry; `Err` inside when the source cannot be opened.
    pub fn write_job(
        &self,
        calendar: Id,
        uid: &str,
        key: &SecretKey,
    ) -> Result<Option<std::result::Result<WriteJob, String>>> {
        let Some(rowid) = calendar_rowid(&self.conn, calendar)? else {
            return Ok(None);
        };
        let entry: Option<EntryRow> = self
            .conn
            .query_row(
                "SELECT version, attempts, fields, from_calendar, from_href FROM appointment_write \
                 WHERE calendar = ?1 AND uid = ?2",
                params![rowid, uid],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .optional()?;
        let Some((version, attempts, fields, from_calendar, from_href)) = entry else {
            return Ok(None);
        };
        let fields: Vec<Pending> = serde_json::from_str(&fields).map_err(|e| Error::Corrupt {
            what: format!("appointment_write.fields: {e}"),
        })?;
        let rows = of_uid(&self.conn, calendar, uid, true)?;
        let (href, etag) = self
            .conn
            .query_row(
                "SELECT a.href, a.etag FROM appointment a WHERE a.calendar = ?1 AND a.uid = ?2 \
                 AND a.href IS NOT NULL LIMIT 1",
                params![uuid_blob(calendar), uid],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .unwrap_or((None, None));
        let appointments = rows
            .into_iter()
            .filter(|(h, _)| !h.deleted)
            .map(|(_, a)| a)
            .collect();
        let Some(source) = self.calendar_source(calendar, key).ok().flatten() else {
            return Ok(Some(Err(
                "the source of the calendar cannot be opened".into()
            )));
        };
        let from = match (from_calendar, from_href) {
            (Some(c), Some(href)) => {
                let id: Option<Vec<u8>> = self
                    .conn
                    .query_row("SELECT uuid FROM object WHERE id = ?1", [c], |row| {
                        row.get(0)
                    })
                    .optional()?;
                let source = match id {
                    Some(blob) => {
                        let id = super::super::id_of_blob(&blob)?;
                        self.calendar_source(id, key).ok().flatten()
                    }
                    None => None,
                };
                source.map(|s| (s, href))
            }
            _ => None,
        };
        Ok(Some(Ok(WriteJob {
            calendar,
            uid: uid.to_string(),
            version,
            attempts,
            appointments,
            href,
            etag,
            source,
            from,
            fields,
            zone: settings_in(&self.conn)?.zone,
        })))
    }

    /// The writer wrote the resource: at `href` with `etag`, or deleted it
    /// (`href` none). The entry goes unless a change came meanwhile.
    pub fn wrote(
        &mut self,
        calendar: Id,
        uid: &str,
        version: i64,
        href: Option<&str>,
        etag: Option<&str>,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        let Some(rowid) = calendar_rowid(&tx, calendar)? else {
            return Ok(());
        };
        if let Some(href) = href {
            tx.execute(
                "UPDATE appointment SET href = ?3, etag = ?4 WHERE calendar = ?1 AND uid = ?2",
                params![uuid_blob(calendar), uid, href, etag],
            )?;
        }
        tx.execute(
            "DELETE FROM appointment_write WHERE calendar = ?1 AND uid = ?2 AND version = ?3",
            params![rowid, uid, version],
        )?;
        tx.execute(
            "UPDATE appointment_write SET from_calendar = NULL, from_href = NULL, attempts = 0 \
             WHERE calendar = ?1 AND uid = ?2",
            params![rowid, uid],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The writer failed. `retry`: again after a while (1, 5, 15 minutes,
    /// then the calendar's `refresh`). Else the entry is dropped, `error`
    /// of the calendar says why, and the next fetch reads everything again,
    /// which brings back the server's state (ADR 0068 §9).
    pub fn write_failed(
        &mut self,
        calendar: Id,
        uid: &str,
        reason: &str,
        retry: bool,
        now: Timestamp,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        let Some(h) = live_calendar(&tx, calendar)? else {
            return Ok(());
        };
        if retry {
            let attempts: u32 = tx
                .query_row(
                    "SELECT attempts FROM appointment_write WHERE calendar = ?1 AND uid = ?2",
                    params![h.rowid, uid],
                    |row| row.get(0),
                )
                .optional()?
                .unwrap_or(0);
            let refresh: i64 = tx.query_row(
                "SELECT refresh FROM calendar WHERE object = ?1",
                [h.rowid],
                |row| row.get(0),
            )?;
            let minutes = match attempts {
                0 => 1,
                1 => 5,
                2 => 15,
                _ => refresh.max(5),
            };
            let next = now.checked_add(minutes.minutes()).unwrap_or(now);
            tx.execute(
                "UPDATE appointment_write SET attempts = attempts + 1, last_error = ?3, \
                 next_try = ?4 WHERE calendar = ?1 AND uid = ?2",
                params![h.rowid, uid, reason, next.to_string()],
            )?;
        } else {
            let title = of_uid(&tx, calendar, uid, true)?
                .into_iter()
                .find(|(_, a)| a.recurrence_id.is_none())
                .map_or_else(|| uid.to_string(), |(_, a)| a.title);
            tx.execute(
                "DELETE FROM appointment_write WHERE calendar = ?1 AND uid = ?2",
                params![h.rowid, uid],
            )?;
            tx.execute(
                "UPDATE calendar_source SET etag = NULL, last_modified = NULL WHERE calendar = ?1",
                [h.rowid],
            )?;
            let error = format!("could not write „{title}“: {reason}");
            server_set(&tx, calendar, &h, now, |c| c.error = Some(error))?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The resource changed elsewhere since samu read it (`412`): what came
    /// is laid over the appointments, each field samu has not written yet
    /// settled by ADR 0068 §7, and the entry stays to be written again.
    pub fn crossed(
        &mut self,
        calendar: Id,
        uid: &str,
        remote: &Read,
        now: Timestamp,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        let Some(rowid) = calendar_rowid(&tx, calendar)? else {
            return Ok(());
        };
        let fields: Vec<Pending> = tx
            .query_row(
                "SELECT fields FROM appointment_write WHERE calendar = ?1 AND uid = ?2",
                params![rowid, uid],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(|f| serde_json::from_str(&f))
            .transpose()
            .map_err(|e| Error::Corrupt {
                what: format!("appointment_write.fields: {e}"),
            })?
            .unwrap_or_default();
        let made: BTreeSet<Id> = fields
            .iter()
            .filter(|(_, f, _, _)| f == "@create")
            .map(|(id, _, _, _)| *id)
            .collect();
        let rows = of_uid(&tx, calendar, uid, true)?;
        // Deleted by samu and not written yet: not to be made again.
        let unmade: BTreeSet<(String, Option<String>)> = rows
            .iter()
            .filter(|(h, a)| {
                h.deleted
                    && fields
                        .iter()
                        .any(|(id, f, _, _)| *id == a.id && f == "@delete")
            })
            .map(|(_, a)| cal::key(a))
            .collect();
        let mut live: BTreeMap<(String, Option<String>), (Head, Appointment)> = rows
            .into_iter()
            .filter(|(h, _)| !h.deleted)
            .map(|(h, a)| (cal::key(&a), (h, a)))
            .collect();
        for incoming in remote.appointments.iter().filter(|a| a.uid == uid) {
            let key = incoming.key();
            match live.remove(&key) {
                Some((h, local)) => {
                    let theirs =
                        incoming
                            .clone()
                            .into_appointment(local.id, calendar, local.created, now);
                    // Per field: the latest stamp, the value before the first.
                    let mut pending: Vec<edit::Pending> = Vec::new();
                    for (_, field, hlc, base) in fields.iter().filter(|(id, ..)| *id == local.id) {
                        match pending.iter_mut().find(|p| &p.field == field) {
                            Some(p) => p.hlc = hlc.clone(),
                            None => pending.push(edit::Pending {
                                field: field.clone(),
                                hlc: hlc.clone(),
                                base: base.clone(),
                            }),
                        }
                    }
                    let merged =
                        edit::merge_remote(&local, &pending, &theirs, incoming.last_modified)
                            .map_err(|e| Error::Corrupt {
                                what: format!("merging {}: {e}", local.id),
                            })?;
                    let changed = differing(
                        &Object::Appointment(local).to_json().map_err(corrupt)?,
                        &Object::Appointment(merged.clone())
                            .to_json()
                            .map_err(corrupt)?,
                    );
                    if !changed.is_empty() {
                        write_fields(&tx, h.rowid, &Object::Appointment(merged), &changed, now)?;
                    }
                }
                None if unmade.contains(&key) => {}
                None => {
                    let new = incoming
                        .clone()
                        .into_appointment(Id::new(), calendar, now, now);
                    create_appointment(&tx, &new, now)?;
                }
            }
        }
        // What is gone over there and samu did not make goes here too.
        for (h, a) in live.into_values() {
            if !made.contains(&a.id) {
                tombstone(&tx, h.rowid, &server_stamp(&tx, now)?, now)?;
            }
        }
        let (href, etag) = remote
            .appointments
            .iter()
            .find(|a| a.uid == uid)
            .map(|a| (a.href.clone(), a.etag.clone()))
            .unwrap_or((None, None));
        tx.execute(
            "UPDATE appointment SET href = COALESCE(?3, href), etag = ?4 WHERE calendar = ?1 AND uid = ?2",
            params![uuid_blob(calendar), uid, href, etag],
        )?;
        tx.execute(
            "UPDATE appointment_write SET next_try = ?3 WHERE calendar = ?1 AND uid = ?2",
            params![rowid, uid, now.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The resource is gone on the server while samu changed it: deleting
    /// wins (ADR 0068 §7). Its appointments go and the entry with them.
    pub fn gone(&mut self, calendar: Id, uid: &str, now: Timestamp) -> Result<()> {
        let tx = self.conn.transaction()?;
        let Some(rowid) = calendar_rowid(&tx, calendar)? else {
            return Ok(());
        };
        for (h, _) in of_uid(&tx, calendar, uid, false)? {
            tombstone(&tx, h.rowid, &server_stamp(&tx, now)?, now)?;
        }
        tx.execute(
            "DELETE FROM appointment_write WHERE calendar = ?1 AND uid = ?2",
            params![rowid, uid],
        )?;
        tx.commit()?;
        Ok(())
    }
}

/// The UIDs of a calendar still to be written: a fetch leaves them alone
/// (ADR 0068 §8).
pub(super) fn queued_uids(tx: &Transaction<'_>, calendar: i64) -> Result<BTreeSet<String>> {
    let mut statement = tx.prepare("SELECT uid FROM appointment_write WHERE calendar = ?1")?;
    let uids = statement
        .query_map([calendar], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<BTreeSet<_>>>()?;
    Ok(uids)
}

fn corrupt(what: String) -> Error {
    Error::Corrupt { what }
}
