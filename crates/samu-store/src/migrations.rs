// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Numbered schema migrations (the runner of catastr-store, `migrations.rs`).
//!
//! `PRAGMA user_version` is the truth; `schema_history` records what ran and
//! when. Each migration runs in its own transaction, so a failing one leaves
//! the database at the version before it. The transaction takes the write
//! lock at its start and reads the version again inside it: two processes
//! opening a fresh file at once would otherwise both apply the same
//! migration, and the second fails with "duplicate column name".

use jiff::Timestamp;
use rusqlite::{Connection, Transaction, TransactionBehavior};

use crate::{Error, Result};

#[derive(Debug, Clone, Copy)]
struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
    /// Runs with foreign keys off, for a table rebuilt the way SQLite
    /// documents it; the foreign keys are checked before the commit.
    rebuilds: bool,
}

/// Every migration, in order. Append only: an applied migration is never
/// edited, because a database that already ran it would not see the change.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "0001_initial",
        sql: include_str!("migrations/0001_initial.sql"),
        rebuilds: false,
    },
    Migration {
        version: 2,
        name: "0002_delivery",
        sql: include_str!("migrations/0002_delivery.sql"),
        rebuilds: false,
    },
    Migration {
        version: 3,
        name: "0003_property",
        sql: include_str!("migrations/0003_property.sql"),
        rebuilds: false,
    },
    Migration {
        version: 4,
        name: "0004_workflow",
        sql: include_str!("migrations/0004_workflow.sql"),
        rebuilds: false,
    },
    Migration {
        version: 5,
        name: "0005_reminder",
        sql: include_str!("migrations/0005_reminder.sql"),
        rebuilds: true,
    },
    Migration {
        version: 6,
        name: "0006_scope",
        sql: include_str!("migrations/0006_scope.sql"),
        rebuilds: true,
    },
    Migration {
        version: 7,
        name: "0007_comment",
        sql: include_str!("migrations/0007_comment.sql"),
        rebuilds: true,
    },
    Migration {
        version: 8,
        name: "0008_scope_round_two",
        sql: include_str!("migrations/0008_scope_round_two.sql"),
        rebuilds: false,
    },
    Migration {
        version: 9,
        name: "0009_settings",
        sql: include_str!("migrations/0009_settings.sql"),
        rebuilds: true,
    },
    Migration {
        version: 10,
        name: "0010_calendar",
        sql: include_str!("migrations/0010_calendar.sql"),
        rebuilds: true,
    },
    Migration {
        version: 11,
        name: "0011_forecast_events",
        sql: include_str!("migrations/0011_forecast_events.sql"),
        rebuilds: false,
    },
    Migration {
        version: 12,
        name: "0012_priority",
        sql: include_str!("migrations/0012_priority.sql"),
        rebuilds: false,
    },
    Migration {
        version: 13,
        name: "0013_icon_and_scope_color",
        sql: include_str!("migrations/0013_icon_and_scope_color.sql"),
        rebuilds: false,
    },
    Migration {
        version: 14,
        name: "0014_appointment_write",
        sql: include_str!("migrations/0014_appointment_write.sql"),
        rebuilds: false,
    },
    Migration {
        version: 15,
        name: "0015_scope_only",
        sql: include_str!("migrations/0015_scope_only.sql"),
        rebuilds: false,
    },
    Migration {
        version: 16,
        name: "0016_scope_layout",
        sql: include_str!("migrations/0016_scope_layout.sql"),
        rebuilds: false,
    },
    Migration {
        version: 17,
        name: "0017_template",
        sql: include_str!("migrations/0017_template.sql"),
        rebuilds: true,
    },
    Migration {
        version: 18,
        name: "0018_settings_keys",
        sql: include_str!("migrations/0018_settings_keys.sql"),
        rebuilds: false,
    },
    Migration {
        version: 19,
        name: "0019_report",
        sql: include_str!("migrations/0019_report.sql"),
        rebuilds: false,
    },
    Migration {
        version: 20,
        name: "0020_template_key",
        sql: include_str!("migrations/0020_template_key.sql"),
        rebuilds: false,
    },
    Migration {
        version: 21,
        name: "0021_perspective",
        sql: include_str!("migrations/0021_perspective.sql"),
        rebuilds: true,
    },
    Migration {
        version: 22,
        name: "0022_habit",
        sql: include_str!("migrations/0022_habit.sql"),
        rebuilds: true,
    },
    Migration {
        version: 23,
        name: "0023_daily_reminder",
        sql: include_str!("migrations/0023_daily_reminder.sql"),
        rebuilds: true,
    },
    Migration {
        version: 24,
        name: "0024_own_scope",
        sql: include_str!("migrations/0024_own_scope.sql"),
        rebuilds: false,
    },
    Migration {
        version: 25,
        name: "0025_appointment_alarms",
        sql: include_str!("migrations/0025_appointment_alarms.sql"),
        rebuilds: false,
    },
    Migration {
        version: 26,
        name: "0026_perspective_table",
        sql: include_str!("migrations/0026_perspective_table.sql"),
        rebuilds: false,
    },
    Migration {
        version: 27,
        name: "0027_perspective_blocks",
        sql: include_str!("migrations/0027_perspective_blocks.sql"),
        rebuilds: false,
    },
    Migration {
        version: 28,
        name: "0028_transcript",
        sql: include_str!("migrations/0028_transcript.sql"),
        rebuilds: false,
    },
];

/// The migrations of the server file: accounts and sessions (ADR 0019 §2).
const SERVER_MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "0001_server",
        sql: include_str!("migrations/server_0001.sql"),
        rebuilds: false,
    },
    Migration {
        version: 2,
        name: "0002_hook",
        sql: include_str!("migrations/server_0002.sql"),
        rebuilds: false,
    },
    Migration {
        version: 3,
        name: "0003_pairing",
        sql: include_str!("migrations/server_0003.sql"),
        rebuilds: false,
    },
    Migration {
        version: 4,
        name: "0004_session_kind",
        sql: include_str!("migrations/server_0004.sql"),
        rebuilds: false,
    },
    Migration {
        version: 5,
        name: "0005_session_grant",
        sql: include_str!("migrations/server_0005.sql"),
        rebuilds: false,
    },
    Migration {
        version: 6,
        name: "0006_account_registered",
        sql: include_str!("migrations/server_0006.sql"),
        rebuilds: false,
    },
];

/// What a call of [`migrate`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    pub from: i64,
    pub to: i64,
    pub applied: Vec<&'static str>,
    /// What a migration has to tell whoever runs the server, a line each
    /// (ADR 0094 §6).
    pub notes: Vec<String>,
}

/// The newest schema this build knows.
#[must_use]
pub fn latest_version() -> i64 {
    MIGRATIONS.last().map_or(0, |m| m.version)
}

/// The schema version of a database.
pub fn schema_version(conn: &Connection) -> Result<i64> {
    Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0))?)
}

/// Brings an account database to [`latest_version`].
pub fn migrate(conn: &Connection) -> Result<MigrationReport> {
    run(conn, MIGRATIONS)
}

/// Brings the server file to its newest schema.
pub fn migrate_server(conn: &Connection) -> Result<MigrationReport> {
    run(conn, SERVER_MIGRATIONS)
}

fn run(conn: &Connection, migrations: &[Migration]) -> Result<MigrationReport> {
    let from = schema_version(conn)?;
    let latest = migrations.last().map_or(0, |m| m.version);
    if from > latest {
        return Err(Error::Unsupported {
            what: format!(
                "the database is at schema version {from}, this build knows {latest} \
                 — use a newer build rather than downgrading the schema"
            ),
        });
    }
    let mut applied = Vec::new();
    for migration in migrations.iter().filter(|m| m.version > from) {
        if migration.rebuilds {
            // Outside a transaction, or SQLite ignores it.
            conn.pragma_update(None, "foreign_keys", "OFF")?;
        }
        let result = apply(conn, migration);
        if migration.rebuilds {
            conn.pragma_update(None, "foreign_keys", "ON")?;
        }
        if result? {
            applied.push(migration.name);
        }
    }
    let notes = if applied.contains(&"0024_own_scope") {
        own_scope_notes(conn)?
    } else {
        Vec::new()
    };
    Ok(MigrationReport {
        from,
        to: schema_version(conn)?,
        applied,
        notes,
    })
}

/// What 0024 left without an own scope although it was `scope_only`: no
/// `only` scope unlocked it, so it is shown everywhere now (ADR 0094 §6).
fn own_scope_notes(conn: &Connection) -> Result<Vec<String>> {
    let mut statement = conn.prepare(
        "SELECT x.what, x.title FROM ( \
             SELECT 'folder' AS what, x.title, x.object FROM folder x \
                 WHERE x.scope_only = 1 AND x.own_scope IS NULL \
             UNION ALL SELECT 'project', x.title, x.object FROM project x \
                 WHERE x.scope_only = 1 AND x.own_scope IS NULL \
             UNION ALL SELECT 'tag', x.title, x.object FROM tag x \
                 WHERE x.scope_only = 1 AND x.own_scope IS NULL \
         ) x JOIN object o ON o.id = x.object WHERE o.deleted_at IS NULL \
         ORDER BY x.what, x.title",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(format!(
            "{} „{}“ was scope_only, but no scope with only unlocked it: \
             shown everywhere now; choose its own scope (ADR 0094 §6)",
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?
        ))
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Applies one migration, or nothing if another connection got there first.
/// Returns whether it ran here.
fn apply(conn: &Connection, migration: &Migration) -> Result<bool> {
    let transaction = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    if schema_version(&transaction)? >= migration.version {
        return Ok(false);
    }
    transaction.execute_batch(migration.sql)?;
    if migration.rebuilds {
        let broken: i64 =
            transaction.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |row| {
                row.get(0)
            })?;
        if broken > 0 {
            return Err(Error::Corrupt {
                what: format!(
                    "{}: {broken} rows point at nothing after the rebuild",
                    migration.name
                ),
            });
        }
    }
    transaction.execute(
        "INSERT INTO schema_history (version, name, applied_at) VALUES (?1, ?2, ?3)",
        rusqlite::params![
            migration.version,
            migration.name,
            Timestamp::now().to_string()
        ],
    )?;
    // Not a parameter: SQLite does not bind pragma values.
    transaction.execute_batch(&format!("PRAGMA user_version = {}", migration.version))?;
    transaction.commit()?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use std::path::PathBuf;
    use std::sync::{Arc, Barrier};

    use super::*;

    struct TempDir(PathBuf);

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Regression (5bc0cc6a): two processes opening a fresh server file at
    /// once both read version 0 and both ran `server_0004`, the second
    /// failing with "duplicate column name: kind".
    #[test]
    fn two_connections_migrating_a_fresh_file_at_once_apply_each_migration_once() {
        for round in 0..20 {
            let dir = TempDir(std::env::temp_dir().join(format!(
                "samu-store-migrate-{}-{}",
                std::process::id(),
                uuid::Uuid::now_v7()
            )));
            std::fs::create_dir_all(&dir.0).unwrap();
            let path = dir.0.join("server.db");
            let barrier = Arc::new(Barrier::new(2));
            let threads: Vec<_> = (0..2)
                .map(|_| {
                    let path = path.clone();
                    let barrier = Arc::clone(&barrier);
                    std::thread::spawn(move || {
                        barrier.wait();
                        let conn = crate::open::open(&path)?;
                        migrate_server(&conn).map(|report| report.applied.len())
                    })
                })
                .collect();
            let applied: usize = threads
                .into_iter()
                .map(|thread| {
                    thread
                        .join()
                        .unwrap()
                        .unwrap_or_else(|e| panic!("round {round}: {e}"))
                })
                .sum();
            assert_eq!(applied, SERVER_MIGRATIONS.len(), "round {round}");
            let conn = crate::open::open(&path).unwrap();
            let history: i64 = conn
                .query_row("SELECT count(*) FROM schema_history", [], |row| row.get(0))
                .unwrap();
            assert_eq!(history, SERVER_MIGRATIONS.len() as i64, "round {round}");
        }
    }
}
