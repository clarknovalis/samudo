// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! What can go wrong in the storage layer.

use thiserror::Error;

/// A storage failure. A change the store refuses is not an error but a
/// `ChangeOutcome::Rejected`; this is for what a caller can only report.
#[derive(Debug, Error)]
pub enum Error {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),

    /// Another process has this database open for writing.
    #[error(
        "the database {path} is already in use by another samu process{}",
        holder.as_ref().map_or(String::new(), |who| format!(" ({who})"))
    )]
    DatabaseInUse {
        path: String,
        holder: Option<String>,
    },

    /// The database is newer than this build, or holds something this build
    /// cannot read.
    #[error("{what}")]
    Unsupported { what: String },

    /// A row that does not read back as its `samu-api` type: the schema and
    /// the types have drifted apart.
    #[error("corrupt row: {what}")]
    Corrupt { what: String },
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
