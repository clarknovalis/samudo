//! `samu-server export` and `samu-server restore` (ADR 0074 §5, §6): an
//! account as a directory, read from its database file directly, without
//! a session and also while the server runs; and back into an empty
//! account, with the server stopped.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use jiff::Timestamp;
use samu_store::core::api::{EXPORT_BLOBS, Export};
use samu_store::{AccountStore, BlobDir, ServerStore, valid_hash};

use crate::app::account_path;

/// Writes a text file only when its content differs, through a temporary
/// file: a file that did not change keeps its time, and a reader never
/// sees half a file.
fn write_if_changed(path: &Path, text: &str) -> Result<bool> {
    if fs::read_to_string(path).is_ok_and(|old| old == text) {
        return Ok(false);
    }
    let mut tmp = path.as_os_str().to_os_string();
    tmp.push(".tmp");
    fs::write(&tmp, text).with_context(|| format!("writing {}", path.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))?;
    Ok(true)
}

/// `samu-server export --account <name> <dir>`.
pub(crate) fn export(data: &Path, account: &str, dir: &Path) -> Result<()> {
    let db = account_path(data, account);
    if !db.exists() {
        bail!("no account {account} in {}", data.display());
    }
    let store = AccountStore::open_read_only(&db)?;
    let export = store.export(account, Timestamp::now())?;
    drop(store);
    let files = export.to_files()?;

    let blobs_out = dir.join(EXPORT_BLOBS);
    fs::create_dir_all(&blobs_out).with_context(|| format!("creating {}", dir.display()))?;
    let mut changed = 0;
    for (name, text) in &files {
        if write_if_changed(&dir.join(name), text)? {
            changed += 1;
        }
    }
    let blobs = BlobDir::beside(&db);
    let mut wanted = HashSet::new();
    for blob in &export.files {
        wanted.insert(blob.hash.clone());
        let to = blobs_out.join(&blob.hash);
        if to.exists() {
            continue;
        }
        let from = blobs
            .path(&blob.hash)
            .with_context(|| format!("{} is not a file hash", blob.hash))?;
        fs::copy(&from, &to)
            .with_context(|| format!("copying {} to {}", from.display(), to.display()))?;
        changed += 1;
    }
    // A file of an attachment that is gone goes too; only files named by a
    // hash, nothing else someone put there.
    for entry in fs::read_dir(&blobs_out)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if valid_hash(&name) && !wanted.contains(&name) {
            fs::remove_file(entry.path())?;
            changed += 1;
        }
    }
    eprintln!(
        "exported {account}: {} objects, {} logbook entries, {} files, {changed} written",
        export.objects.len(),
        export.logbook.len(),
        export.files.len()
    );
    Ok(())
}

/// Reads an export directory.
pub(crate) fn read(dir: &Path) -> Result<Export> {
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }
    Ok(Export::from_files(|name| {
        fs::read_to_string(dir.join(name)).ok()
    })?)
}

/// `samu-server restore --account <name> <dir>`.
pub(crate) fn restore(data: &Path, account: &str, dir: &Path) -> Result<()> {
    let export = read(dir)?;
    let server = ServerStore::open(&data.join("server.db"))?;
    if !server.accounts()?.iter().any(|a| a.name == account) {
        bail!("no account {account}: create it first with `samu-server account add {account}`");
    }
    drop(server);
    let db = account_path(data, account);
    if let Some(parent) = db.parent() {
        fs::create_dir_all(parent)?;
    }
    // Refuses while the server runs: it holds the lock.
    let mut store = AccountStore::open(&db)?;
    if !store.is_empty()? {
        bail!("account {account} is not empty; a restore only goes into an empty account");
    }
    // The bytes first, each checked against its hash on the way in.
    let blobs = BlobDir::beside(&db);
    for blob in &export.files {
        let path = dir.join(EXPORT_BLOBS).join(&blob.hash);
        let file = fs::File::open(&path).with_context(|| format!("reading {}", path.display()))?;
        if let Err(refused) = blobs.put(&blob.hash, file)? {
            bail!("{}: {refused:?}", path.display());
        }
    }
    if let Err(refused) = store.restore(&export)? {
        bail!("{refused}");
    }
    eprintln!(
        "restored {account} from {}: {} objects, {} logbook entries, {} files; \
         set the sources of calendars again",
        dir.display(),
        export.objects.len(),
        export.logbook.len(),
        export.files.len()
    );
    Ok(())
}
