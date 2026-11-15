//! The files of attachments (ADR 0039 §4): named by their SHA-256, written
//! once and never changed, in a directory of their own next to the account
//! database.
//!
//! A file is written to a temporary name, checked against the hash it was
//! announced with, synced and only then renamed into place, so a file under
//! its hash is always whole. The row in `blob` follows the file
//! ([`crate::AccountStore::add_blob`]); a file without a row is harmless.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use rand::TryRngCore;
use sha2::{Digest, Sha256};

use crate::{Error, Result};

/// The largest file the server keeps (ADR 0039 §4).
pub const MAX_BLOB_BYTES: u64 = 50 * 1024 * 1024;

/// Whether `hash` names a blob: 64 lowercase hex digits.
#[must_use]
pub fn valid_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Why a file was not kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlobRefused {
    /// The name is not a SHA-256.
    NotAHash,
    /// Longer than the writer's limit, at most [`MAX_BLOB_BYTES`].
    TooLarge { limit: u64 },
    /// The bytes have another hash than announced.
    WrongHash { actual: String },
}

impl std::fmt::Display for BlobRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAHash => write!(f, "a blob is named by 64 lowercase hex digits"),
            Self::TooLarge { limit } => write!(f, "a blob has at most {limit} bytes"),
            Self::WrongHash { actual } => write!(f, "the bytes have the hash {actual}"),
        }
    }
}

/// A file kept: its hash and size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobWritten {
    pub hash: String,
    pub size: u64,
}

/// The blob directory of one account.
#[derive(Debug, Clone)]
pub struct BlobDir {
    dir: PathBuf,
}

impl BlobDir {
    #[must_use]
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    /// `<name>.blobs` next to the account database `<name>.db`.
    #[must_use]
    pub fn beside(database: &Path) -> Self {
        Self::new(database.with_extension("blobs"))
    }

    /// Where the file of a blob is, or would be; `None` for what is not a
    /// hash, so no path can be built from a request that leaves the
    /// directory.
    #[must_use]
    pub fn path(&self, hash: &str) -> Option<PathBuf> {
        valid_hash(hash).then(|| self.dir.join(&hash[..2]).join(hash))
    }

    /// Opens a blob's file for reading; `None` if there is none.
    pub fn open(&self, hash: &str) -> Result<Option<File>> {
        let Some(path) = self.path(hash) else {
            return Ok(None);
        };
        match File::open(path) {
            Ok(file) => Ok(Some(file)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Starts writing a file of at most [`MAX_BLOB_BYTES`].
    pub fn writer(&self) -> Result<BlobWriter> {
        self.writer_up_to(MAX_BLOB_BYTES)
    }

    /// Starts writing a file of at most `limit` bytes, never more than
    /// [`MAX_BLOB_BYTES`] (ADR 0097 §2).
    pub fn writer_up_to(&self, limit: u64) -> Result<BlobWriter> {
        let tmp_dir = self.dir.join("tmp");
        fs::create_dir_all(&tmp_dir)?;
        let mut name = [0u8; 16];
        rand::rngs::OsRng
            .try_fill_bytes(&mut name)
            .map_err(|e| Error::Unsupported {
                what: format!("no random bytes: {e}"),
            })?;
        let name: String = name.iter().map(|b| format!("{b:02x}")).collect();
        let tmp = tmp_dir.join(name);
        let file = File::create_new(&tmp)?;
        Ok(BlobWriter {
            dir: self.clone(),
            file: Some(file),
            tmp,
            hasher: Sha256::new(),
            size: 0,
            limit: limit.min(MAX_BLOB_BYTES),
        })
    }

    /// Writes a whole file from a reader, for tests and the command line.
    pub fn put(&self, hash: &str, mut from: impl Read) -> Result<Result<BlobWritten, BlobRefused>> {
        let mut writer = self.writer()?;
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            let n = from.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            if let Err(refused) = writer.write(&buffer[..n])? {
                return Ok(Err(refused));
            }
        }
        writer.finish(hash)
    }
}

/// A file being written. Dropped without [`BlobWriter::finish`], it leaves
/// nothing behind.
#[derive(Debug)]
pub struct BlobWriter {
    dir: BlobDir,
    file: Option<File>,
    tmp: PathBuf,
    hasher: Sha256,
    size: u64,
    limit: u64,
}

impl BlobWriter {
    /// Appends bytes; refused once the file grows past its limit.
    pub fn write(&mut self, bytes: &[u8]) -> Result<Result<(), BlobRefused>> {
        let size = self.size + bytes.len() as u64;
        let too_large = BlobRefused::TooLarge { limit: self.limit };
        if size > self.limit {
            return Ok(Err(too_large));
        }
        let Some(file) = self.file.as_mut() else {
            return Ok(Err(too_large));
        };
        file.write_all(bytes)?;
        self.hasher.update(bytes);
        self.size = size;
        Ok(Ok(()))
    }

    /// Checks the hash and moves the file into place. The same file again
    /// is no error: the one already there is kept.
    pub fn finish(mut self, expected: &str) -> Result<Result<BlobWritten, BlobRefused>> {
        let Some(target) = self.dir.path(expected) else {
            return Ok(Err(BlobRefused::NotAHash));
        };
        let actual: String = self
            .hasher
            .clone()
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if actual != expected {
            return Ok(Err(BlobRefused::WrongHash { actual }));
        }
        if let Some(file) = self.file.take() {
            file.sync_all()?;
        }
        if target.exists() {
            fs::remove_file(&self.tmp)?;
        } else {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(&self.tmp, &target)?;
        }
        Ok(Ok(BlobWritten {
            hash: actual,
            size: self.size,
        }))
    }
}

impl Drop for BlobWriter {
    fn drop(&mut self) {
        // After `finish` the file is gone from here already; otherwise it is
        // a half-written one. Nothing to report either way.
        self.file.take();
        let _ = fs::remove_file(&self.tmp);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, reason = "tests may unwrap")]

    use super::*;
    use crate::store::tests::TempDir;

    fn sha(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    #[test]
    fn a_file_is_kept_under_its_hash_and_read_back() {
        let tmp = TempDir::new("blob-kept");
        let blobs = BlobDir::new(tmp.dir().join("a.blobs"));
        let hash = sha(b"hello");
        let written = blobs.put(&hash, &b"hello"[..]).unwrap().unwrap();
        assert_eq!(
            written,
            BlobWritten {
                hash: hash.clone(),
                size: 5
            }
        );
        let mut back = String::new();
        blobs
            .open(&hash)
            .unwrap()
            .unwrap()
            .read_to_string(&mut back)
            .unwrap();
        assert_eq!(back, "hello");
        // Nothing half-written stays behind.
        assert_eq!(
            fs::read_dir(tmp.dir().join("a.blobs/tmp")).unwrap().count(),
            0
        );
    }

    #[test]
    fn the_same_file_twice_is_no_error() {
        let tmp = TempDir::new("blob-twice");
        let blobs = BlobDir::new(tmp.dir().join("a.blobs"));
        let hash = sha(b"twice");
        blobs.put(&hash, &b"twice"[..]).unwrap().unwrap();
        blobs.put(&hash, &b"twice"[..]).unwrap().unwrap();
        assert_eq!(
            fs::read_dir(tmp.dir().join("a.blobs/tmp")).unwrap().count(),
            0
        );
    }

    #[test]
    fn a_wrong_hash_keeps_nothing() {
        let tmp = TempDir::new("blob-wrong");
        let blobs = BlobDir::new(tmp.dir().join("a.blobs"));
        let claimed = sha(b"other");
        let refused = blobs.put(&claimed, &b"these bytes"[..]).unwrap();
        assert_eq!(
            refused,
            Err(BlobRefused::WrongHash {
                actual: sha(b"these bytes")
            })
        );
        assert!(blobs.open(&claimed).unwrap().is_none());
        assert_eq!(
            fs::read_dir(tmp.dir().join("a.blobs/tmp")).unwrap().count(),
            0
        );
    }

    #[test]
    fn a_name_that_is_not_a_hash_builds_no_path() {
        let blobs = BlobDir::new("/nowhere");
        assert!(blobs.path("../../etc/passwd").is_none());
        assert!(blobs.path(&"A".repeat(64)).is_none());
        assert!(blobs.path(&"a".repeat(64)).is_some());
    }

    #[test]
    fn a_file_past_the_limit_is_refused() {
        let tmp = TempDir::new("blob-large");
        let blobs = BlobDir::new(tmp.dir().join("a.blobs"));
        let mut writer = blobs.writer().unwrap();
        let chunk = vec![0u8; 1024 * 1024];
        let mut refused = None;
        for _ in 0..=MAX_BLOB_BYTES / chunk.len() as u64 {
            if let Err(r) = writer.write(&chunk).unwrap() {
                refused = Some(r);
                break;
            }
        }
        assert_eq!(
            refused,
            Some(BlobRefused::TooLarge {
                limit: MAX_BLOB_BYTES
            })
        );
        drop(writer);
        assert_eq!(
            fs::read_dir(tmp.dir().join("a.blobs/tmp")).unwrap().count(),
            0
        );
    }

    #[test]
    fn a_writer_keeps_its_own_limit_but_never_more_than_the_largest() {
        let tmp = TempDir::new("blob-limit");
        let blobs = BlobDir::new(tmp.dir().join("a.blobs"));
        let mut writer = blobs.writer_up_to(10).unwrap();
        assert_eq!(writer.write(&[0u8; 10]).unwrap(), Ok(()));
        assert_eq!(
            writer.write(&[0u8; 1]).unwrap(),
            Err(BlobRefused::TooLarge { limit: 10 })
        );
        let mut writer = blobs.writer_up_to(u64::MAX).unwrap();
        assert_eq!(writer.limit, MAX_BLOB_BYTES);
        assert_eq!(writer.write(&[0u8; 1]).unwrap(), Ok(()));
    }
}
