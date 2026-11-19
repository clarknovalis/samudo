// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! SQLite storage for samu: one database file per account (ADR 0005,
//! ADR 0018).
//!
//! Schema changes only through numbered migrations in this crate. Internal
//! rowids, UUIDv7 to the outside, timestamps as ISO-8601 text with `Z`
//! (catastr ADR 0016).

mod blob;
mod error;
mod lock;
pub mod migrations;
mod open;
mod rows;
mod server;
mod store;

pub use blob::{BlobDir, BlobRefused, BlobWriter, BlobWritten, MAX_BLOB_BYTES, valid_hash};
pub use error::{Error, Result};
pub use lock::DatabaseLock;
pub use rows::Object;
pub use samu_core as core;
pub use server::{
    AccountInfo, Ended, HookRow, PairingCode, RESERVED_DEVICE, Registered, SESSION_ID_LEN,
    ServerStore, SessionRow, is_reserved_device, new_token, token_digest, valid_account_name,
};
pub use store::{
    AccountStore, Changed, Delivered, Fetched, KnownResource, LogFilter, OwnedSnapshot, Pending,
    RestoreRefused, SecretKey, StoredSource, Synced, WriteJob,
};
