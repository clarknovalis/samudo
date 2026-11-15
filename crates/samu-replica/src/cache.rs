//! A client's database (ADR 0028 §3, ADR 0066 §2): the mirror of the
//! server's objects with the change number it reaches, the outbox of changes
//! not yet confirmed, and the sign-in. Small on purpose: the truth is on the
//! server. The phone and the replica on a computer share it.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension, Transaction, params};
use samu_api::{AnyObject, Change, Id};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS object (
    id   TEXT PRIMARY KEY,
    json TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS outbox (
    n      INTEGER PRIMARY KEY AUTOINCREMENT,
    change TEXT NOT NULL
);
-- Later and tomorrow from a reminder (ADR 0030 §6): only on the
-- phone, never synced.
CREATE TABLE IF NOT EXISTS snooze (
    owner TEXT PRIMARY KEY,
    until TEXT NOT NULL
);
-- Files taken on the phone, not yet on the server (ADR 0039 §6). The
-- file lies under blobs/<hash> next to this database; it goes up before
-- the outbox, which may hold an attachment that needs it.
CREATE TABLE IF NOT EXISTS blob_outbox (
    hash       TEXT PRIMARY KEY,
    media_type TEXT NOT NULL,
    size       INTEGER NOT NULL
);
-- The replica's own change numbers (ADR 0066 §3): when each object of
-- the view (mirror with the outbox over it) last changed, for its own
-- `GET /changes?after=` and `/events`. The number itself is meta
-- `local_seq`.
CREATE TABLE IF NOT EXISTS local (
    id  TEXT PRIMARY KEY,
    seq INTEGER NOT NULL
);
";

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("database: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("a stored value that does not read back: {0}")]
    Json(#[from] serde_json::Error),
}

pub(crate) type Result<T> = std::result::Result<T, CacheError>;

pub struct Cache {
    conn: Connection,
    /// The highest outbox number a push has carried. Numbers only grow,
    /// so a change up to it is sent or may be, even when the push failed
    /// on the way back; `withdraw` leaves it (ADR 0104 §4).
    sent_upto: i64,
}

impl Cache {
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode = WAL;")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn, sent_upto: 0 })
    }

    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn, sent_upto: 0 })
    }

    pub fn meta(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }

    pub fn set_meta(&self, key: &str, value: Option<&str>) -> Result<()> {
        match value {
            Some(value) => self.conn.execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )?,
            None => self
                .conn
                .execute("DELETE FROM meta WHERE key = ?1", [key])?,
        };
        Ok(())
    }

    /// Every mirrored object.
    pub fn mirror(&self) -> Result<Vec<AnyObject>> {
        let mut statement = self.conn.prepare("SELECT json FROM object")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(serde_json::from_str(&row?)?);
        }
        Ok(out)
    }

    /// A page from the server: objects replace what is there, `gone` ones
    /// leave; with `reset` the mirror starts over. One transaction, so a
    /// failed pull leaves the mirror as it was.
    pub fn apply_page(
        &mut self,
        objects: &[AnyObject],
        gone: &[Id],
        seq: u64,
        reset: bool,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        let mut touched: Vec<String> = Vec::new();
        if reset {
            touched.extend(ids_in(&tx, "SELECT id FROM object")?);
            tx.execute("DELETE FROM object", [])?;
        }
        touched.extend(
            objects
                .iter()
                .filter_map(|o| o.id())
                .map(|id| id.to_string()),
        );
        touched.extend(gone.iter().map(ToString::to_string));
        bump(&tx, &touched)?;
        for object in objects {
            let Some(id) = object.id() else { continue };
            tx.execute(
                "INSERT INTO object (id, json) VALUES (?1, ?2) \
                 ON CONFLICT(id) DO UPDATE SET json = excluded.json",
                params![id.to_string(), serde_json::to_string(object)?],
            )?;
        }
        for id in gone {
            tx.execute("DELETE FROM object WHERE id = ?1", [id.to_string()])?;
        }
        tx.execute(
            "INSERT INTO meta (key, value) VALUES ('seq', ?1) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [seq.to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn seq(&self) -> Result<u64> {
        Ok(self.meta("seq")?.and_then(|s| s.parse().ok()).unwrap_or(0))
    }

    /// The outbox, oldest first, with each change's number.
    pub fn outbox(&self) -> Result<Vec<(i64, Change)>> {
        let mut statement = self
            .conn
            .prepare("SELECT n, change FROM outbox ORDER BY n")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get::<_, String>(1)?)))?;
        let mut out = Vec::new();
        for row in rows {
            let (n, text) = row?;
            out.push((n, serde_json::from_str(&text)?));
        }
        Ok(out)
    }

    /// Changes made together go in together; their numbers in the outbox,
    /// for `withdraw`.
    pub fn push(&mut self, changes: &[Change]) -> Result<Vec<i64>> {
        let tx = self.conn.transaction()?;
        let mut numbers = Vec::with_capacity(changes.len());
        for change in changes {
            tx.execute(
                "INSERT INTO outbox (change) VALUES (?1)",
                [serde_json::to_string(change)?],
            )?;
            numbers.push(tx.last_insert_rowid());
        }
        let touched: Vec<String> = changes
            .iter()
            .filter_map(object_of)
            .map(|id| id.to_string())
            .collect();
        bump(&tx, &touched)?;
        tx.commit()?;
        Ok(numbers)
    }

    /// Takes changes back out of the outbox before they are sent (ADR
    /// 0104 §3): all of `numbers` or none. `false` when one is gone or on
    /// its way, and nothing was taken.
    pub fn withdraw(&mut self, numbers: &[i64]) -> Result<bool> {
        if numbers.iter().any(|n| *n <= self.sent_upto) {
            return Ok(false);
        }
        let tx = self.conn.transaction()?;
        let mut touched = Vec::new();
        for n in numbers {
            let change: Option<String> = tx
                .query_row("SELECT change FROM outbox WHERE n = ?1", [n], |row| {
                    row.get(0)
                })
                .optional()?;
            let Some(change) = change else {
                // Dropped unapplied with the transaction.
                return Ok(false);
            };
            if let Some(id) = serde_json::from_str::<Change>(&change)
                .ok()
                .as_ref()
                .and_then(object_of)
            {
                touched.push(id.to_string());
            }
            tx.execute("DELETE FROM outbox WHERE n = ?1", [n])?;
        }
        bump(&tx, &touched)?;
        tx.commit()?;
        Ok(true)
    }

    /// A push of the outbox up to `upto` starts.
    pub fn sending(&mut self, upto: i64) {
        self.sent_upto = self.sent_upto.max(upto);
    }

    pub fn drop_sent(&mut self, numbers: &[i64]) -> Result<()> {
        let tx = self.conn.transaction()?;
        let mut touched = Vec::new();
        for n in numbers {
            let change: Option<String> = tx
                .query_row("SELECT change FROM outbox WHERE n = ?1", [n], |row| {
                    row.get(0)
                })
                .optional()?;
            // What the server made of it shows when the page comes in; the
            // view without it may differ until then.
            if let Some(id) = change
                .and_then(|c| serde_json::from_str::<Change>(&c).ok())
                .as_ref()
                .and_then(object_of)
            {
                touched.push(id.to_string());
            }
            tx.execute("DELETE FROM outbox WHERE n = ?1", [n])?;
        }
        bump(&tx, &touched)?;
        tx.commit()?;
        Ok(())
    }

    pub fn outbox_len(&self) -> Result<u32> {
        let n: i64 = self
            .conn
            .query_row("SELECT count(*) FROM outbox", [], |row| row.get(0))?;
        Ok(u32::try_from(n).unwrap_or(u32::MAX))
    }

    /// Snoozed reminders by owner, until when.
    pub fn snoozes(&self) -> Result<Vec<(String, String)>> {
        let mut statement = self.conn.prepare("SELECT owner, until FROM snooze")?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// A reminder again at `until`; a second snooze replaces the first.
    pub fn snooze(&self, owner: &str, until: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO snooze (owner, until) VALUES (?1, ?2) \
             ON CONFLICT(owner) DO UPDATE SET until = excluded.until",
            params![owner, until],
        )?;
        Ok(())
    }

    pub fn unsnooze(&self, owner: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM snooze WHERE owner = ?1", [owner])?;
        Ok(())
    }

    /// Files waiting to go up, as hash, media type and size.
    pub fn blob_outbox(&self) -> Result<Vec<(String, String, u64)>> {
        let mut statement = self
            .conn
            .prepare("SELECT hash, media_type, size FROM blob_outbox ORDER BY rowid")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get::<_, i64>(2)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (hash, media_type, size) = row?;
            out.push((hash, media_type, u64::try_from(size).unwrap_or(0)));
        }
        Ok(out)
    }

    /// A file to send up; the same file twice is one row.
    pub fn push_blob(&self, hash: &str, media_type: &str, size: u64) -> Result<()> {
        self.conn.execute(
            "INSERT INTO blob_outbox (hash, media_type, size) VALUES (?1, ?2, ?3) \
             ON CONFLICT(hash) DO NOTHING",
            params![hash, media_type, i64::try_from(size).unwrap_or(i64::MAX)],
        )?;
        Ok(())
    }

    pub fn drop_blob(&self, hash: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM blob_outbox WHERE hash = ?1", [hash])?;
        Ok(())
    }

    /// The replica's own change number: it grows whenever an object of the
    /// view may have changed (ADR 0066 §3).
    pub fn local_seq(&self) -> Result<u64> {
        Ok(self
            .meta("local_seq")?
            .and_then(|s| s.parse().ok())
            .unwrap_or(0))
    }

    /// The objects of the view that may have changed after the local change
    /// number `after`, oldest first. Whether one is still there, the view
    /// says.
    pub fn local_after(&self, after: u64) -> Result<Vec<Id>> {
        let mut statement = self
            .conn
            .prepare("SELECT id FROM local WHERE seq > ?1 ORDER BY seq, id")?;
        let rows = statement.query_map([i64::try_from(after).unwrap_or(i64::MAX)], |row| {
            row.get::<_, String>(0)
        })?;
        let mut out = Vec::new();
        for row in rows {
            if let Ok(id) = row?.parse() {
                out.push(id);
            }
        }
        Ok(out)
    }

    /// Forget the mirror, for another account or server; the outbox goes
    /// with it only when asked.
    pub fn clear(&mut self, outbox_too: bool) -> Result<()> {
        let tx = self.conn.transaction()?;
        let mut touched = ids_in(&tx, "SELECT id FROM object")?;
        if outbox_too {
            for change in ids_in(&tx, "SELECT change FROM outbox")? {
                if let Some(id) = serde_json::from_str::<Change>(&change)
                    .ok()
                    .as_ref()
                    .and_then(object_of)
                {
                    touched.push(id.to_string());
                }
            }
        }
        bump(&tx, &touched)?;
        tx.execute("DELETE FROM object", [])?;
        tx.execute("DELETE FROM meta WHERE key = 'seq'", [])?;
        tx.execute("DELETE FROM snooze", [])?;
        if outbox_too {
            tx.execute("DELETE FROM outbox", [])?;
            tx.execute("DELETE FROM blob_outbox", [])?;
        }
        tx.commit()?;
        Ok(())
    }
}

/// The object a change is about.
#[must_use]
pub fn object_of(change: &Change) -> Option<Id> {
    match change {
        Change::Create { object, .. }
        | Change::Set { object, .. }
        | Change::Add { object, .. }
        | Change::Remove { object, .. }
        | Change::Delete { object, .. } => Some(*object),
        Change::Unknown => None,
    }
}

/// The first column of every row of `sql`.
fn ids_in(tx: &Transaction<'_>, sql: &str) -> Result<Vec<String>> {
    let mut statement = tx.prepare(sql)?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// One more local change number, for every one of `ids`.
fn bump(tx: &Transaction<'_>, ids: &[String]) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let seq: i64 = tx
        .query_row(
            "SELECT coalesce((SELECT CAST(value AS INTEGER) FROM meta WHERE key = 'local_seq'), 0)",
            [],
            |row| row.get::<_, i64>(0),
        )?
        .saturating_add(1);
    tx.execute(
        "INSERT INTO meta (key, value) VALUES ('local_seq', ?1) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [seq.to_string()],
    )?;
    for id in ids {
        tx.execute(
            "INSERT INTO local (id, seq) VALUES (?1, ?2) \
             ON CONFLICT(id) DO UPDATE SET seq = excluded.seq",
            params![id, seq],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use serde_json::json;

    use super::*;

    fn task(n: u8, title: &str) -> AnyObject {
        serde_json::from_value(json!({"kind": "task", "object": {
            "id": format!("01936f5e-8a00-7c3e-9b1a-0000000000{n:02x}"),
            "created": "2026-11-08T08:00:00Z", "modified": "2026-11-08T08:00:00Z",
            "title": title, "flagged": false, "sequential": false,
            "complete_with_last_child": false, "position": 0}}))
        .unwrap()
    }

    #[test]
    fn pages_replace_remove_and_reset() {
        let mut cache = Cache::in_memory().unwrap();
        cache
            .apply_page(&[task(1, "a"), task(2, "b")], &[], 5, false)
            .unwrap();
        assert_eq!(cache.mirror().unwrap().len(), 2);
        assert_eq!(cache.seq().unwrap(), 5);
        let gone = task(2, "b").id().unwrap();
        cache
            .apply_page(&[task(1, "a2")], &[gone], 7, false)
            .unwrap();
        let mirror = cache.mirror().unwrap();
        assert_eq!(mirror, [task(1, "a2")]);
        cache.apply_page(&[task(3, "c")], &[], 9, true).unwrap();
        assert_eq!(cache.mirror().unwrap(), [task(3, "c")]);
        assert_eq!(cache.seq().unwrap(), 9);
    }

    #[test]
    fn the_outbox_keeps_its_order_until_sent() {
        let mut cache = Cache::in_memory().unwrap();
        let change = |title: &str| -> Change {
            serde_json::from_value(json!({"op": "set",
                "object": "01936f5e-8a00-7c3e-9b1a-000000000001", "field": "title",
                "value": title, "hlc": {"wall": 1, "counter": 0, "device": "pixel"}}))
            .unwrap()
        };
        cache.push(&[change("a"), change("b")]).unwrap();
        cache.push(&[change("c")]).unwrap();
        let out = cache.outbox().unwrap();
        assert_eq!(out.len(), 3);
        cache.drop_sent(&[out[0].0, out[1].0]).unwrap();
        assert_eq!(cache.outbox_len().unwrap(), 1);
        assert_eq!(cache.outbox().unwrap()[0].1, change("c"));
    }

    #[test]
    fn withdraw_takes_back_all_or_nothing_and_never_what_was_sent() {
        let mut cache = Cache::in_memory().unwrap();
        let change = |title: &str| -> Change {
            serde_json::from_value(json!({"op": "set",
                "object": "01936f5e-8a00-7c3e-9b1a-000000000001", "field": "title",
                "value": title, "hlc": {"wall": 1, "counter": 0, "device": "pixel"}}))
            .unwrap()
        };
        let first = cache.push(&[change("a")]).unwrap();
        let second = cache.push(&[change("b"), change("c")]).unwrap();
        assert_eq!(second.len(), 2);
        assert!(cache.withdraw(&second).unwrap());
        assert_eq!(cache.outbox().unwrap(), [(first[0], change("a"))]);
        // Gone already: nothing is taken, not even the one still there.
        assert!(!cache.withdraw(&[first[0], second[0]]).unwrap());
        assert_eq!(cache.outbox_len().unwrap(), 1);
        // On its way, or perhaps arrived: left where it is.
        cache.sending(first[0]);
        assert!(!cache.withdraw(&first).unwrap());
        assert_eq!(cache.outbox_len().unwrap(), 1);
        let later = cache.push(&[change("d")]).unwrap();
        assert!(later[0] > first[0]);
        assert!(cache.withdraw(&later).unwrap());
    }

    #[test]
    fn a_blob_waits_once_until_sent_and_leaves_with_the_outbox() {
        let mut cache = Cache::in_memory().unwrap();
        cache.push_blob("aa", "image/jpeg", 3).unwrap();
        cache.push_blob("aa", "image/jpeg", 3).unwrap();
        cache.push_blob("bb", "audio/mp4", 5).unwrap();
        assert_eq!(
            cache.blob_outbox().unwrap(),
            [
                ("aa".to_string(), "image/jpeg".to_string(), 3),
                ("bb".to_string(), "audio/mp4".to_string(), 5)
            ]
        );
        cache.drop_blob("aa").unwrap();
        assert_eq!(cache.blob_outbox().unwrap().len(), 1);
        cache.clear(false).unwrap();
        assert_eq!(cache.blob_outbox().unwrap().len(), 1);
        cache.clear(true).unwrap();
        assert!(cache.blob_outbox().unwrap().is_empty());
    }

    #[test]
    fn the_local_number_names_what_may_have_changed() {
        let mut cache = Cache::in_memory().unwrap();
        assert_eq!(cache.local_seq().unwrap(), 0);
        cache
            .apply_page(&[task(1, "a"), task(2, "b")], &[], 5, false)
            .unwrap();
        let first = cache.local_seq().unwrap();
        assert!(first > 0);
        assert_eq!(cache.local_after(0).unwrap().len(), 2);
        let one = task(1, "a").id().unwrap();
        let change: Change = serde_json::from_value(json!({"op": "set",
            "object": one.to_string(), "field": "title",
            "value": "a2", "hlc": {"wall": 1, "counter": 0, "device": "pc"}}))
        .unwrap();
        cache.push(&[change]).unwrap();
        assert_eq!(cache.local_after(first).unwrap(), [one]);
        let pushed = cache.local_seq().unwrap();
        let n = cache.outbox().unwrap()[0].0;
        cache.drop_sent(&[n]).unwrap();
        assert_eq!(cache.local_after(pushed).unwrap(), [one]);
        // A reset names what left the mirror too.
        let before = cache.local_seq().unwrap();
        cache.apply_page(&[task(3, "c")], &[], 9, true).unwrap();
        assert_eq!(cache.local_after(before).unwrap().len(), 3);
    }
}
