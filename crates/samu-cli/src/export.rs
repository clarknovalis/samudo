// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `samu export` and `samu export-org` (ADR 0074 §5, §8): the account as a
//! directory from `GET /export` and the files from `/blobs`; and an Org
//! file written from such a directory, without a server.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use samu_api::{EXPORT_BLOBS, Export};
use samu_client::Client;
use sha2::{Digest, Sha256};

use crate::export_org;

/// The Org file beside the JSON.
pub(crate) const ORG_FILE: &str = "samu.org";

/// Writes a text file only when it differs, through a temporary file: an
/// unchanged file keeps its time, a reader never sees half of one.
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

fn is_hash(name: &str) -> bool {
    name.len() == 64
        && name
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `samu export <dir> [--org]`.
pub(crate) fn export(client: &Client, dir: &Path, org: bool) -> Result<()> {
    let answer = client.call("GET", "/export", None)?;
    let export: Export = serde_json::from_value(answer.json()?)?;
    let blobs = dir.join(EXPORT_BLOBS);
    fs::create_dir_all(&blobs).with_context(|| format!("creating {}", dir.display()))?;
    let mut written = 0;
    for (name, text) in export.to_files()? {
        if write_if_changed(&dir.join(&name), &text)? {
            written += 1;
        }
    }
    let mut wanted = HashSet::new();
    for blob in &export.files {
        if !is_hash(&blob.hash) {
            bail!("{:?} is not a file hash", blob.hash);
        }
        wanted.insert(blob.hash.clone());
        let to = blobs.join(&blob.hash);
        if to.exists() {
            continue;
        }
        let bytes = client
            .call("GET", &format!("/blobs/{}", blob.hash), None)?
            .body;
        let got = format!("{:x}", Sha256::digest(&bytes));
        if got != blob.hash {
            bail!("the file {} came with another hash ({got})", blob.hash);
        }
        let mut tmp = to.as_os_str().to_os_string();
        tmp.push(".tmp");
        fs::write(&tmp, &bytes)?;
        fs::rename(&tmp, &to)?;
        written += 1;
    }
    for entry in fs::read_dir(&blobs)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if is_hash(&name) && !wanted.contains(&name) {
            fs::remove_file(entry.path())?;
            written += 1;
        }
    }
    if org && write_if_changed(&dir.join(ORG_FILE), &export_org::render(&export))? {
        written += 1;
    }
    eprintln!(
        "exported {}: {} objects, {} logbook entries, {} files, {written} written to {}",
        export.manifest.account,
        export.objects.len(),
        export.logbook.len(),
        export.files.len(),
        dir.display()
    );
    Ok(())
}

/// `samu export-org <dir> [--output <file>|-]`.
pub(crate) fn export_org(dir: &Path, output: Option<&PathBuf>) -> Result<()> {
    if !dir.is_dir() {
        bail!("{} is not a directory", dir.display());
    }
    let export = Export::from_files(|name| fs::read_to_string(dir.join(name)).ok())
        .with_context(|| format!("{} is not an export", dir.display()))?;
    let org = export_org::render(&export);
    match output {
        Some(path) if path.as_os_str() == "-" => print!("{org}"),
        Some(path) => {
            write_if_changed(path, &org)?;
            eprintln!("wrote {}", path.display());
        }
        None => {
            let path = dir.join(ORG_FILE);
            write_if_changed(&path, &org)?;
            eprintln!("wrote {}", path.display());
        }
    }
    Ok(())
}
