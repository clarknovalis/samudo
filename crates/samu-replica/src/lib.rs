//! A client's way to work without the server (ADR 0028, ADR 0066): a
//! mirror of the account's objects, an outbox of changes the server has not
//! yet confirmed, and the sync between them. The Android app uses it
//! through samu-mobile, the replica on a computer through
//! `samu-server replica`. No async runtime: the sync calls the blocking
//! `samu-client`.

mod cache;
mod sync;

pub use cache::{Cache, CacheError, object_of};
pub use sync::{ReadBlob, SyncReport, push_blobs, remember_session, settings, sync, view};

/// Why the sync or a read of the mirror failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The database could not be opened or read.
    #[error("{0}")]
    Store(String),
    /// Not signed in, or the session is gone. The outbox stays.
    #[error("{0}")]
    SignedOut(String),
    /// The server could not be reached; nothing is lost, the next sync
    /// tries again.
    #[error("{0}")]
    Offline(String),
    /// The server refused, with its message.
    #[error("{0}")]
    Refused(String),
}

impl From<CacheError> for Error {
    fn from(error: CacheError) -> Self {
        Self::Store(error.to_string())
    }
}

impl From<samu_client::Error> for Error {
    fn from(error: samu_client::Error) -> Self {
        use samu_client::Error as E;
        match error {
            E::Unauthorized(reason) => Self::SignedOut(reason),
            E::NoServer(reason) => Self::Offline(reason),
            E::Io(e) => Self::Offline(e.to_string()),
            other => Self::Refused(other.to_string()),
        }
    }
}
