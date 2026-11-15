//! One writer per database, enforced by the operating system (as in
//! catastr-store, `lock.rs`).
//!
//! An advisory lock on a file beside the database rather than a PID file: the
//! kernel drops it when the process dies, including on `SIGKILL`.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::{Error, Result};

/// Holds the exclusive claim on one database file for as long as it lives.
#[derive(Debug)]
pub struct DatabaseLock {
    /// Kept open: the lock belongs to the open file, not to the path.
    file: File,
}

impl DatabaseLock {
    /// The lock file belonging to a database.
    #[must_use]
    pub fn path_for(database: &Path) -> PathBuf {
        let mut name = database.as_os_str().to_os_string();
        name.push(".lock");
        PathBuf::from(name)
    }

    /// Claims the database, or reports who holds it.
    pub fn acquire(database: &Path) -> Result<Self> {
        let path = Self::path_for(database);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        if file.try_lock().is_err() {
            return Err(Error::DatabaseInUse {
                path: database.display().to_string(),
                holder: std::fs::read_to_string(&path)
                    .ok()
                    .map(|text| text.trim().to_string())
                    .filter(|text| !text.is_empty()),
            });
        }
        // Written only once the lock is held. Failing to write costs only a
        // nicer message, so it does not refuse the lock.
        let mut file = file;
        let _ = file.set_len(0);
        let _ = write!(file, "pid {}", std::process::id());
        let _ = file.flush();
        Ok(Self { file })
    }
}

impl Drop for DatabaseLock {
    fn drop(&mut self) {
        // The kernel releases the lock; this only clears the note. The file
        // stays, since removing it would race with a process about to lock it.
        let _ = self.file.set_len(0);
    }
}
