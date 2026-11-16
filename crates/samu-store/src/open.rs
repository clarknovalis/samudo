// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Opening a database with the pragmas every connection needs.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, ErrorCode};

use crate::Result;

fn configure(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "busy_timeout", 5_000)?;
    journal_mode_wal(conn)?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}

/// Switches to WAL. On a fresh file that needs an exclusive lock, and SQLite
/// answers "database is locked" at once rather than wait for it while another
/// connection does the same (it does not call the busy handler when raising a
/// lock, to avoid a deadlock). So it tries again for as long as the busy
/// timeout. Once the file is in WAL, the pragma changes nothing and needs no
/// such lock.
fn journal_mode_wal(conn: &Connection) -> Result<()> {
    let mut tries = 0;
    loop {
        match conn.pragma_update(None, "journal_mode", "WAL") {
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == ErrorCode::DatabaseBusy && tries < 100 =>
            {
                tries += 1;
                std::thread::sleep(Duration::from_millis(50));
            }
            result => return Ok(result?),
        }
    }
}

/// Opens (or creates) a database file. Does not lock and does not migrate.
pub(crate) fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    configure(&conn)?;
    Ok(conn)
}

/// Opens an existing database for reading only: no lock, no migration,
/// no change to its journal mode.
pub(crate) fn open_read_only(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.pragma_update(None, "busy_timeout", 5_000)?;
    Ok(conn)
}

/// An in-memory database, for tests.
pub(crate) fn open_in_memory() -> Result<Connection> {
    let conn = Connection::open_in_memory()?;
    configure(&conn)?;
    Ok(conn)
}
