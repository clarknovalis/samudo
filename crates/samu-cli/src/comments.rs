// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `samu comment`, `samu comments`, `samu attach`, `samu attachments` and
//! `samu blob get` (ADR 0039 §6). Comments and attachments are objects of
//! their own over the ordinary routes; the file of an attachment goes to
//! `PUT /blobs/{sha256}` first.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use samu_api::{
    AnyObject, Attachment, Author, AuthorKind, Blob, Change, ChangesPage, Comment, Id, ObjectKind,
    TranscriptState,
};
use samu_client::Client;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::output::{Format, emit};
use crate::{placeholder, resolve, short, write};

/// The largest file the server keeps (ADR 0039 §4), checked here before
/// the upload.
const MAX_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Debug, Args)]
pub(crate) struct Attach {
    /// The task or project.
    pub(crate) id: String,
    /// The file.
    pub(crate) file: PathBuf,
    /// Hang it on this comment of the task or project instead.
    #[arg(long)]
    pub(crate) to_comment: Option<String>,
    /// The name to show; the file's name by default.
    #[arg(long)]
    pub(crate) name: Option<String>,
    /// The media type, like `image/jpeg`; guessed from the name by default.
    #[arg(long = "type")]
    pub(crate) media_type: Option<String>,
    /// Ask for the transcript of an audio file (ADR 0140 §3). A server
    /// without a transcript service drops the request.
    #[arg(long)]
    pub(crate) transcribe: bool,
}

#[derive(Debug, Subcommand)]
pub(crate) enum BlobCommand {
    /// Write the file of a blob to stdout, or to a file (GET /blobs/{sha256}).
    Get {
        /// The SHA-256, as an attachment names it.
        hash: String,
        /// Write here instead of stdout.
        #[arg(long, short)]
        output: Option<PathBuf>,
    },
}

/// Every object of the account.
fn objects(client: &Client) -> Result<Vec<AnyObject>> {
    let page: ChangesPage =
        serde_json::from_value(client.call("GET", "/changes?after=0", None)?.json()?)?;
    Ok(page.objects)
}

/// The text of `samu comment`: the argument, or stdin for `-`.
fn body_of(text: &str) -> Result<String> {
    if text != "-" {
        return Ok(text.to_string());
    }
    let mut body = String::new();
    std::io::stdin().read_to_string(&mut body)?;
    Ok(body.trim_end_matches(['\r', '\n']).to_string())
}

/// `samu comment <id> <text|->`.
pub(crate) fn comment(client: &Client, format: Format, owner: &str, text: &str) -> Result<()> {
    let owner = resolve(client, owner)?;
    let body = body_of(text)?;
    if body.trim().is_empty() {
        bail!("a comment needs a text");
    }
    let now = jiff::Timestamp::now();
    let comment = Comment {
        id: Id::new(),
        created: now,
        modified: now,
        // The server sets it from the session (ADR 0039 §1).
        created_by: None,
        owner,
        body,
    };
    write(
        client,
        &[Change::Create {
            object: comment.id,
            kind: ObjectKind::Comment,
            value: serde_json::to_value(&comment)?,
            hlc: placeholder(),
        }],
    )?;
    emit(
        format,
        &[json!({"id": comment.id, "short": short(&comment.id)})],
        &["id"],
    )
}

fn author_text(author: Option<&Author>) -> String {
    match author {
        None => String::new(),
        Some(Author {
            device,
            kind: AuthorKind::Agent,
        }) => format!("{device} (agent)"),
        Some(author) => author.device.clone(),
    }
}

fn author_value(author: Option<&Author>) -> Value {
    author.map_or(Value::Null, |a| json!(a))
}

/// `samu comments <id>`: oldest first, with their attachments counted.
pub(crate) fn comments(client: &Client, format: Format, owner: &str) -> Result<()> {
    let owner = resolve(client, owner)?;
    let records = comment_records(&objects(client)?, owner);
    emit(format, &records, &["short", "created", "author", "body"])
}

/// The comments of `owner` among `objects`, as `samu comments` shows them.
pub(crate) fn comment_records(objects: &[AnyObject], owner: Id) -> Vec<Value> {
    let mut found: Vec<&Comment> = objects
        .iter()
        .filter_map(|o| match o {
            AnyObject::Comment(c) if c.owner == owner => Some(c),
            _ => None,
        })
        .collect();
    found.sort_by_key(|c| (c.created, c.id));
    found
        .iter()
        .map(|c| {
            let attachments = objects
                .iter()
                .filter(|o| matches!(o, AnyObject::Attachment(a) if a.owner == c.id))
                .count();
            json!({
                "id": c.id,
                "short": short(&c.id),
                "created": c.created.to_string(),
                "modified": c.modified.to_string(),
                "author": author_text(c.created_by.as_ref()),
                "created_by": author_value(c.created_by.as_ref()),
                "body": c.body,
                "attachments": attachments,
            })
        })
        .collect()
}

/// A media type from a file name, for the common kinds of attachment.
fn guess_type(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match extension.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "svg" => "image/svg+xml",
        "webm" => "audio/webm",
        "ogg" | "oga" | "opus" => "audio/ogg",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "pdf" => "application/pdf",
        "txt" | "org" | "md" => "text/plain",
        "json" => "application/json",
        _ => "application/octet-stream",
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `samu attach`: the file up (PUT /blobs/{sha256}), then the attachment.
pub(crate) fn attach(client: &Client, format: Format, attach: &Attach) -> Result<()> {
    let owner = resolve(client, &attach.id)?;
    let owner = match &attach.to_comment {
        None => owner,
        Some(reference) => {
            let comment = resolve(client, reference)?;
            let belongs = objects(client)?
                .iter()
                .any(|o| matches!(o, AnyObject::Comment(c) if c.id == comment && c.owner == owner));
            if !belongs {
                bail!("{reference:?} is not a comment of {}", attach.id);
            }
            comment
        }
    };
    let size = std::fs::metadata(&attach.file)
        .with_context(|| attach.file.display().to_string())?
        .len();
    if size > MAX_BYTES {
        bail!(
            "{} has {size} bytes; the server keeps at most {MAX_BYTES}",
            attach.file.display()
        );
    }
    let bytes = std::fs::read(&attach.file).with_context(|| attach.file.display().to_string())?;
    let hash = hex(&Sha256::digest(&bytes));
    let media_type = attach
        .media_type
        .clone()
        .unwrap_or_else(|| guess_type(&attach.file).to_string());
    let answer = client.call_bytes("PUT", &format!("/blobs/{hash}"), &media_type, &[], &bytes)?;
    let blob: Blob = serde_json::from_value(answer.json()?)?;
    let name = match &attach.name {
        Some(name) => name.clone(),
        None => attach
            .file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".to_string()),
    };
    let now = jiff::Timestamp::now();
    let attachment = Attachment {
        id: Id::new(),
        created: now,
        modified: now,
        created_by: None,
        owner,
        blob: blob.hash,
        name,
        // What the server recorded first; a blob never changes.
        media_type: blob.media_type,
        size: blob.size,
        transcript: None,
        transcript_state: attach.transcribe.then_some(TranscriptState::Pending),
        transcript_error: None,
    };
    write(
        client,
        &[Change::Create {
            object: attachment.id,
            kind: ObjectKind::Attachment,
            value: serde_json::to_value(&attachment)?,
            hlc: placeholder(),
        }],
    )?;
    emit(
        format,
        &[json!({
            "id": attachment.id,
            "short": short(&attachment.id),
            "blob": attachment.blob,
            "media_type": attachment.media_type,
            "size": attachment.size,
        })],
        &["id"],
    )
}

/// `samu attachments <id>`: those of the object and of its comments.
pub(crate) fn attachments(client: &Client, format: Format, owner: &str) -> Result<()> {
    let owner = resolve(client, owner)?;
    let records = attachment_records(&objects(client)?, owner);
    emit(
        format,
        &records,
        &[
            "short",
            "on",
            "name",
            "media_type",
            "size",
            "blob",
            "transcript_state",
            "transcript",
        ],
    )
}

/// `samu transcribe <attachment>`: asks for the transcript of an audio
/// attachment (POST /attachments/{id}/transcribe, ADR 0140 §3). The
/// answer is the attachment as it waits; the text comes later and shows
/// in `samu attachments`.
pub(crate) fn transcribe(client: &Client, format: Format, attachment: &str) -> Result<()> {
    let id = resolve(client, attachment)?;
    let answer = client.call("POST", &format!("/attachments/{id}/transcribe"), None)?;
    let asked: Attachment = serde_json::from_value(answer.json()?)?;
    emit(
        format,
        &[json!({
            "id": asked.id,
            "short": short(&asked.id),
            "name": asked.name,
            "transcript_state": asked.transcript_state,
            "transcript": asked.transcript,
        })],
        &["short", "name", "transcript_state"],
    )
}

/// The files of `owner` and of its comments among `objects`, as
/// `samu attachments` shows them.
pub(crate) fn attachment_records(objects: &[AnyObject], owner: Id) -> Vec<Value> {
    let comments: Vec<Id> = objects
        .iter()
        .filter_map(|o| match o {
            AnyObject::Comment(c) if c.owner == owner => Some(c.id),
            _ => None,
        })
        .collect();
    let mut found: Vec<&Attachment> = objects
        .iter()
        .filter_map(|o| match o {
            AnyObject::Attachment(a) if a.owner == owner || comments.contains(&a.owner) => Some(a),
            _ => None,
        })
        .collect();
    found.sort_by_key(|a| (a.created, a.id));
    found
        .iter()
        .map(|a| {
            json!({
                "id": a.id,
                "short": short(&a.id),
                "owner": a.owner,
                "on": if a.owner == owner { "self".to_string() } else { short(&a.owner) },
                "name": a.name,
                "media_type": a.media_type,
                "size": a.size,
                "blob": a.blob,
                "transcript": a.transcript,
                "transcript_state": a.transcript_state,
                "transcript_error": a.transcript_error,
                "created": a.created.to_string(),
                "author": author_text(a.created_by.as_ref()),
                "created_by": author_value(a.created_by.as_ref()),
            })
        })
        .collect()
}

/// `samu blob get`.
pub(crate) fn blob(client: &Client, command: &BlobCommand) -> Result<()> {
    let BlobCommand::Get { hash, output } = command;
    let answer = client.call("GET", &format!("/blobs/{hash}"), None)?;
    match output {
        Some(path) => {
            std::fs::write(path, &answer.body).with_context(|| path.display().to_string())?;
            eprintln!("{} bytes to {}", answer.body.len(), path.display());
        }
        None => {
            let mut stdout = std::io::stdout().lock();
            stdout.write_all(&answer.body)?;
            stdout.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_type_is_guessed_from_the_name() {
        assert_eq!(guess_type(Path::new("tafel.JPG")), "image/jpeg");
        assert_eq!(guess_type(Path::new("memo.webm")), "audio/webm");
        assert_eq!(guess_type(Path::new("README")), "application/octet-stream");
    }

    #[test]
    fn an_agent_is_named_as_one() {
        let agent = Author {
            device: "claude".into(),
            kind: AuthorKind::Agent,
        };
        let person = Author {
            device: "laptop".into(),
            kind: AuthorKind::Human,
        };
        assert_eq!(author_text(Some(&agent)), "claude (agent)");
        assert_eq!(author_text(Some(&person)), "laptop");
        assert_eq!(author_text(None), "");
    }
}
