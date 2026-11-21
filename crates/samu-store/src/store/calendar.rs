// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Calendars in the account file (ADR 0047): the sealed source, which
//! calendars are due, and laying what was fetched over what is stored.
//!
//! The server is the only writer of a calendar's `fetched_at`, `error`,
//! `host` and `writable`, and writes appointments from what it fetched;
//! clients change appointments of a writable calendar too (ADR 0068, see
//! [`writes`]). The server's changes are stamped with the device `server`,
//! carry no author and go into no logbook; they reach the clients through
//! `/changes` like any other.

use std::collections::BTreeMap;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use jiff::{Timestamp, ToSpan};
use rand::TryRngCore;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde_json::{Map, Value as Json};

use super::{
    Head, head, load, next_seq, observe, observed_in, settings_in, touch, uuid_blob, write_row,
    write_stamp,
};
use crate::core::api::{Appointment, Calendar, CalendarSource, Hlc, Id, ObjectKind};
use crate::core::calendar::{self as cal, Read};
use crate::core::clock::next_stamp;
use crate::rows::Object;
use crate::{AccountStore, Error, Result};

pub(super) mod writes;

pub use writes::{KnownResource, Pending, WriteJob};

/// The device the server's own changes are stamped with.
const SERVER: &str = "server";

/// The key that seals calendar sources: 32 bytes from the server's
/// environment, never in the database (ADR 0047 §3).
#[derive(Clone)]
pub struct SecretKey([u8; 32]);

impl SecretKey {
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretKey(…)")
    }
}

const NONCE: usize = 24;

fn seal(key: &SecretKey, calendar: Id, source: &CalendarSource) -> Result<Vec<u8>> {
    let plain = serde_json::to_vec(source).map_err(|e| Error::Corrupt {
        what: format!("calendar source: {e}"),
    })?;
    let mut nonce = [0u8; NONCE];
    rand::rngs::OsRng
        .try_fill_bytes(&mut nonce)
        .map_err(|e| Error::Unsupported {
            what: format!("no random bytes: {e}"),
        })?;
    let cipher = XChaCha20Poly1305::new((&key.0).into());
    let sealed = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &plain,
                aad: calendar.uuid().as_bytes(),
            },
        )
        .map_err(|_| Error::Unsupported {
            what: "sealing a calendar source failed".into(),
        })?;
    let mut out = nonce.to_vec();
    out.extend(sealed);
    Ok(out)
}

/// The source, or `None` when the key is not the one it was sealed with.
fn open_sealed(key: &SecretKey, calendar: Id, sealed: &[u8]) -> Option<CalendarSource> {
    let (nonce, body) = sealed.split_at_checked(NONCE)?;
    let cipher = XChaCha20Poly1305::new((&key.0).into());
    let plain = cipher
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: body,
                aad: calendar.uuid().as_bytes(),
            },
        )
        .ok()?;
    serde_json::from_slice(&plain).ok()
}

/// A calendar's source as the fetch needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSource {
    pub source: CalendarSource,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

/// What a fetch came to.
#[derive(Debug, Clone)]
pub enum Fetched {
    /// The calendar, read.
    Read {
        read: Read,
        etag: Option<String>,
        last_modified: Option<String>,
    },
    /// `304`: as before.
    NotModified,
    /// Why there is nothing to read: the appointments stay (ADR 0047 §7).
    Failed(String),
}

/// What a sync wrote, for the log and the tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Synced {
    pub created: usize,
    pub changed: usize,
    pub deleted: usize,
}

impl AccountStore {
    /// Seals and keeps where a calendar comes from, and fetches it at once
    /// next. Sets the calendar's `host`. `Err` with a reason when there is
    /// no such calendar or the address is not one.
    pub fn set_calendar_source(
        &mut self,
        id: Id,
        source: &CalendarSource,
        key: &SecretKey,
        now: Timestamp,
    ) -> Result<std::result::Result<(), String>> {
        let Some(host) = host_of(&source.url) else {
            return Ok(Err(
                "the address must start with https://, http:// or webcal://".into(),
            ));
        };
        let tx = self.conn.transaction()?;
        let Some(h) = live_calendar(&tx, id)? else {
            return Ok(Err(format!("no calendar {id}")));
        };
        let sealed = seal(key, id, source)?;
        tx.execute(
            "INSERT INTO calendar_source (calendar, sealed) VALUES (?1, ?2) \
             ON CONFLICT (calendar) DO UPDATE SET sealed = ?2, etag = NULL, \
             last_modified = NULL, next_fetch = NULL",
            params![h.rowid, sealed],
        )?;
        server_set(&tx, id, &h, now, |c| c.host = Some(host.clone()))?;
        tx.commit()?;
        Ok(Ok(()))
    }

    /// A calendar's source, opened with `key`. `None` without a source;
    /// `Err` when the key does not open it.
    pub fn calendar_source(&self, id: Id, key: &SecretKey) -> Result<Option<StoredSource>> {
        let row: Option<(Vec<u8>, Option<String>, Option<String>)> = self
            .conn
            .query_row(
                "SELECT s.sealed, s.etag, s.last_modified FROM calendar_source s \
                 JOIN object o ON o.id = s.calendar WHERE o.uuid = ?1 AND o.deleted_at IS NULL",
                [uuid_blob(id)],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((sealed, etag, last_modified)) = row else {
            return Ok(None);
        };
        let source = open_sealed(key, id, &sealed).ok_or_else(|| Error::Unsupported {
            what: format!("the key does not open the source of calendar {id}"),
        })?;
        Ok(Some(StoredSource {
            source,
            etag,
            last_modified,
        }))
    }

    /// Whether a live calendar has a source, sealed or not openable.
    pub fn has_calendar_source(&self, id: Id) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM calendar_source s JOIN object o ON o.id = s.calendar \
             WHERE o.uuid = ?1 AND o.deleted_at IS NULL)",
            [uuid_blob(id)],
            |row| row.get(0),
        )?)
    }

    /// The enabled calendars with a source whose time to fetch has come.
    pub fn due_calendars(&self, now: Timestamp) -> Result<Vec<Id>> {
        let mut statement = self.conn.prepare(
            "SELECT o.uuid FROM calendar c JOIN object o ON o.id = c.object \
             JOIN calendar_source s ON s.calendar = c.object \
             WHERE o.deleted_at IS NULL AND c.enabled = 1 \
             AND (s.next_fetch IS NULL OR s.next_fetch <= ?1) ORDER BY o.id",
        )?;
        let ids = statement
            .query_map([now.to_string()], |row| row.get::<_, Vec<u8>>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.iter().map(|b| super::id_of_blob(b)).collect()
    }

    /// Whether the server's loops have anything to do in this account: a
    /// live, enabled calendar with a source to fetch, or an appointment
    /// waiting to be written. Without, a closed account stays closed
    /// (ADR 0099, closing idle accounts).
    pub fn has_calendar_work(&self) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM calendar c JOIN object o ON o.id = c.object \
             JOIN calendar_source s ON s.calendar = c.object \
             WHERE o.deleted_at IS NULL AND c.enabled = 1) \
             OR EXISTS (SELECT 1 FROM appointment_write)",
            [],
            |row| row.get(0),
        )?)
    }

    /// Fetch a calendar at once next, whatever its time.
    pub fn fetch_soon(&mut self, id: Id) -> Result<bool> {
        let changed = self.conn.execute(
            "UPDATE calendar_source SET next_fetch = NULL WHERE calendar = \
             (SELECT id FROM object WHERE uuid = ?1 AND deleted_at IS NULL)",
            [uuid_blob(id)],
        )?;
        Ok(changed > 0)
    }

    /// Every live appointment.
    pub fn appointments(&self) -> Result<Vec<Appointment>> {
        let mut statement = self.conn.prepare(
            "SELECT uuid FROM object WHERE kind = 'appointment' AND deleted_at IS NULL ORDER BY id",
        )?;
        let ids = statement
            .query_map([], |row| row.get::<_, Vec<u8>>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut out = Vec::with_capacity(ids.len());
        for bytes in ids {
            if let Some(Object::Appointment(a)) = self.get(super::id_of_blob(&bytes)?)? {
                out.push(a);
            }
        }
        Ok(out)
    }

    /// Lays a fetch over what is stored, in one transaction (ADR 0047 §7):
    /// field by field, so an unchanged calendar writes nothing. What the
    /// window no longer holds or the calendar no longer has is deleted. A
    /// failure keeps the appointments and only sets `error`. `None` when
    /// the calendar is gone.
    pub fn sync_calendar(
        &mut self,
        id: Id,
        fetched: Fetched,
        now: Timestamp,
    ) -> Result<Option<Synced>> {
        let tx = self.conn.transaction()?;
        let Some(h) = live_calendar(&tx, id)? else {
            return Ok(None);
        };
        let calendar = match load(&tx, id, &h)? {
            Object::Calendar(c) => c,
            _ => return Ok(None),
        };
        let next = now
            .checked_add(i64::from(calendar.refresh.max(5)).minutes())
            .unwrap_or(now);
        let mut synced = Synced::default();
        let error = match fetched {
            Fetched::NotModified => calendar.error.clone(),
            Fetched::Failed(reason) => Some(reason),
            Fetched::Read {
                read,
                etag,
                last_modified,
            } => {
                tx.execute(
                    "UPDATE calendar_source SET etag = ?2, last_modified = ?3 WHERE calendar = ?1",
                    params![h.rowid, etag, last_modified],
                )?;
                synced = lay_over(&tx, id, &read, now)?;
                (!read.unknown_zones.is_empty()).then(|| {
                    let names: Vec<&str> = read.unknown_zones.iter().map(String::as_str).collect();
                    format!("unknown zones, read in the account's: {}", names.join(", "))
                })
            }
        };
        tx.execute(
            "UPDATE calendar_source SET next_fetch = ?2 WHERE calendar = ?1",
            params![h.rowid, next.to_string()],
        )?;
        let wrote = synced != Synced::default();
        if wrote || error != calendar.error {
            server_set(&tx, id, &h, now, |c| {
                c.error.clone_from(&error);
                c.fetched_at = Some(now);
            })?;
        }
        tx.commit()?;
        Ok(Some(synced))
    }
}

/// The head of a live calendar.
fn live_calendar(tx: &Connection, id: Id) -> Result<Option<Head>> {
    Ok(head(tx, id)?.filter(|h| !h.deleted && h.kind == ObjectKind::Calendar))
}

/// The host of an address, `webcal://` read as `https://`.
pub(crate) fn host_of(url: &str) -> Option<String> {
    let rest = ["https://", "http://", "webcal://"]
        .iter()
        .find_map(|scheme| url.strip_prefix(scheme))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = host.split(':').next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// One stamp of the server's own, after everything seen.
fn server_stamp(tx: &Connection, now: Timestamp) -> Result<Hlc> {
    let seen = observed_in(tx)?;
    Ok(next_stamp(SERVER, None, seen.as_ref(), now))
}

/// Changes fields of a calendar as the server, each field stamped.
fn server_set(
    tx: &Transaction<'_>,
    id: Id,
    h: &Head,
    now: Timestamp,
    change: impl FnOnce(&mut Calendar),
) -> Result<()> {
    let Object::Calendar(before) = load(tx, id, h)? else {
        return Ok(());
    };
    let mut after = before.clone();
    change(&mut after);
    let fields = differing(
        &Object::Calendar(before).to_json().map_err(corrupt)?,
        &Object::Calendar(after.clone()).to_json().map_err(corrupt)?,
    );
    if fields.is_empty() {
        return Ok(());
    }
    write_fields(tx, h.rowid, &Object::Calendar(after), &fields, now)
}

pub(super) fn corrupt(what: String) -> Error {
    Error::Corrupt { what }
}

/// The fields whose values differ, besides id and times.
pub(super) fn differing(a: &Map<String, Json>, b: &Map<String, Json>) -> Vec<String> {
    let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
    keys.sort();
    keys.dedup();
    keys.into_iter()
        .filter(|k| !matches!(k.as_str(), "id" | "created" | "modified"))
        .filter(|k| a.get(*k) != b.get(*k))
        .cloned()
        .collect()
}

/// Writes the row and one stamp per field, at one change number.
pub(super) fn write_fields(
    tx: &Transaction<'_>,
    rowid: i64,
    object: &Object,
    fields: &[String],
    now: Timestamp,
) -> Result<()> {
    if let Err(reason) = write_row(tx, rowid, object, false)? {
        return Err(corrupt(format!("the server's own change: {reason}")));
    }
    let seq = next_seq(tx)?;
    touch(tx, rowid, seq, now)?;
    let stamp = server_stamp(tx, now)?;
    for field in fields {
        write_stamp(tx, rowid, field, &stamp, seq)?;
    }
    observe(tx, &stamp)?;
    Ok(())
}

/// Creates, changes and deletes the calendar's appointments to match `read`.
fn lay_over(tx: &Transaction<'_>, calendar: Id, read: &Read, now: Timestamp) -> Result<Synced> {
    let today = now.to_zoned(settings_in(tx)?.zone).date();
    let queued = match live_calendar(tx, calendar)? {
        Some(h) => writes::queued_uids(tx, h.rowid)?,
        None => Default::default(),
    };
    let mut stored: BTreeMap<(String, Option<String>), (Head, Appointment)> = BTreeMap::new();
    for id in appointment_ids(tx, calendar)? {
        let Some(h) = head(tx, id)? else { continue };
        if let Object::Appointment(a) = load(tx, id, &h)?
            && !queued.contains(&a.uid)
        {
            stored.insert(cal::key(&a), (h, a));
        }
    }
    let mut synced = Synced::default();
    for incoming in read
        .appointments
        .iter()
        .filter(|a| cal::keep(a, today) && !queued.contains(&a.uid))
    {
        match stored.remove(&incoming.key()) {
            Some((h, old)) => {
                where_it_is(tx, h.rowid, incoming)?;
                let new = incoming
                    .clone()
                    .into_appointment(old.id, calendar, old.created, now);
                let fields = differing(
                    &Object::Appointment(old).to_json().map_err(corrupt)?,
                    &Object::Appointment(new.clone())
                        .to_json()
                        .map_err(corrupt)?,
                );
                if !fields.is_empty() {
                    write_fields(tx, h.rowid, &Object::Appointment(new), &fields, now)?;
                    synced.changed += 1;
                }
            }
            None => {
                let new = incoming
                    .clone()
                    .into_appointment(Id::new(), calendar, now, now);
                let rowid = create_appointment(tx, &new, now)?;
                where_it_is(tx, rowid, incoming)?;
                synced.created += 1;
            }
        }
    }
    for (h, _) in stored.into_values() {
        tombstone(tx, h.rowid, &server_stamp(tx, now)?, now)?;
        synced.deleted += 1;
    }
    Ok(synced)
}

fn appointment_ids(tx: &Connection, calendar: Id) -> Result<Vec<Id>> {
    let mut statement = tx.prepare(
        "SELECT o.uuid FROM appointment a JOIN object o ON o.id = a.object \
         WHERE a.calendar = ?1 AND o.deleted_at IS NULL",
    )?;
    let ids = statement
        .query_map([uuid_blob(calendar)], |row| row.get::<_, Vec<u8>>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    ids.iter().map(|b| super::id_of_blob(b)).collect()
}

/// Keeps where an appointment lives on a CalDAV server (ADR 0068 §6):
/// no change for the clients, so no change number.
fn where_it_is(tx: &Transaction<'_>, rowid: i64, read: &cal::ReadAppointment) -> Result<()> {
    if read.href.is_some() {
        tx.execute(
            "UPDATE appointment SET href = ?2, etag = ?3 WHERE object = ?1 \
             AND (href IS NOT ?2 OR etag IS NOT ?3)",
            params![rowid, read.href, read.etag],
        )?;
    }
    Ok(())
}

fn create_appointment(
    tx: &Transaction<'_>,
    appointment: &Appointment,
    now: Timestamp,
) -> Result<i64> {
    let seq = next_seq(tx)?;
    tx.execute(
        "INSERT INTO object (uuid, kind, created, modified, seq) VALUES (?1, 'appointment', ?2, ?2, ?3)",
        params![uuid_blob(appointment.id), now.to_string(), seq],
    )?;
    let rowid = tx.last_insert_rowid();
    if let Err(reason) = write_row(tx, rowid, &Object::Appointment(appointment.clone()), true)? {
        return Err(corrupt(format!(
            "appointment {}: {reason}",
            appointment.uid
        )));
    }
    let stamp = server_stamp(tx, now)?;
    write_stamp(tx, rowid, "@create", &stamp, seq)?;
    observe(tx, &stamp)?;
    Ok(rowid)
}

fn tombstone(tx: &Connection, rowid: i64, hlc: &Hlc, now: Timestamp) -> Result<()> {
    let seq = next_seq(tx)?;
    tx.execute(
        "UPDATE object SET deleted_at = ?2, seq = ?3, modified = ?2 WHERE id = ?1",
        params![rowid, now.to_string(), seq],
    )?;
    write_stamp(tx, rowid, "@delete", hlc, seq)?;
    observe(tx, hlc)?;
    Ok(())
}

/// A calendar was deleted: its appointments go with it, stamped with the
/// change that deleted it, and its source is dropped.
pub(super) fn forget(
    tx: &Transaction<'_>,
    calendar: Id,
    rowid: i64,
    hlc: &Hlc,
    now: Timestamp,
) -> Result<()> {
    for id in appointment_ids(tx, calendar)? {
        if let Some(h) = head(tx, id)? {
            tombstone(tx, h.rowid, hlc, now)?;
        }
    }
    tx.execute("DELETE FROM calendar_source WHERE calendar = ?1", [rowid])?;
    Ok(())
}
