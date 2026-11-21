// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Transcripts of audio attachments (ADR 0140): which are asked for, and
//! writing what the service answered.
//!
//! `transcript`, `transcript_state` and `transcript_error` are the
//! server's. A client asks by creating an attachment with
//! `transcript_state` `pending`, or through the route. The server's
//! changes are stamped with the device `server`, carry no author and go
//! into no logbook, like those of a calendar.

use jiff::Timestamp;
use rusqlite::Transaction;
use serde_json::Value as Json;

use super::calendar::{corrupt, differing, write_fields};
use super::{Head, head, id_of_blob, load, stamp_of};
use crate::core::api::{Attachment, Id, ObjectKind, TranscriptState};
use crate::core::voice::{VOICE_SOURCE, is_audio, voice_title};
use crate::rows::{ATTACHMENT_SERVER_FIELDS, Object};
use crate::{AccountStore, Result};

/// What a transcript service made of a recording.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transcribed {
    /// What was said; empty when nothing was.
    Text(String),
    /// Why there is no transcript.
    Failed(String),
}

/// What became of asking for a transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asked {
    /// The attachment as it waits now.
    Pending(Box<Attachment>),
    /// The attachment is not audio.
    NotAudio,
    /// No such attachment.
    Missing,
}

/// A new attachment may ask for its transcript and say nothing else about
/// one (ADR 0140 §3).
pub(super) fn check_create(value: &Json) -> std::result::Result<(), String> {
    let given = |field: &str| value.get(field).is_some_and(|v| !v.is_null());
    for field in ATTACHMENT_SERVER_FIELDS {
        if *field != "transcript_state" && given(field) {
            return Err(format!("{field} of an attachment is the server's"));
        }
    }
    if !given("transcript_state") {
        return Ok(());
    }
    if value.get("transcript_state").and_then(Json::as_str) != Some("pending") {
        return Err("a new attachment's transcript_state is `pending` or absent".into());
    }
    let audio = value
        .get("media_type")
        .and_then(Json::as_str)
        .is_some_and(is_audio);
    if audio {
        Ok(())
    } else {
        Err("only an audio attachment has a transcript".into())
    }
}

fn live_attachment(tx: &Transaction<'_>, id: Id) -> Result<Option<(Head, Attachment)>> {
    let Some(h) = head(tx, id)?.filter(|h| !h.deleted && h.kind == ObjectKind::Attachment) else {
        return Ok(None);
    };
    match load(tx, id, &h)? {
        Object::Attachment(a) => Ok(Some((h, a))),
        _ => Ok(None),
    }
}

/// Writes `after` where it differs from `before`, as the server.
fn server_write(
    tx: &Transaction<'_>,
    h: &Head,
    before: Object,
    after: Object,
    now: Timestamp,
) -> Result<()> {
    let fields = differing(
        &before.to_json().map_err(corrupt)?,
        &after.to_json().map_err(corrupt)?,
    );
    if fields.is_empty() {
        return Ok(());
    }
    write_fields(tx, h.rowid, &after, &fields, now)
}

/// Fills title and note of a voice note from its transcript (ADR 0140
/// §4): only on a task marked as one, and only what nobody has changed
/// since it was made.
fn fill_voice_note(tx: &Transaction<'_>, owner: Id, text: &str, now: Timestamp) -> Result<()> {
    let Some(h) = head(tx, owner)?.filter(|h| !h.deleted && h.kind == ObjectKind::Task) else {
        return Ok(());
    };
    let Object::Task(before) = load(tx, owner, &h)? else {
        return Ok(());
    };
    if before.properties.get("source").map(String::as_str) != Some(VOICE_SOURCE) {
        return Ok(());
    }
    let Some(title) = voice_title(text) else {
        return Ok(());
    };
    let mut after = before.clone();
    if stamp_of(tx, h.rowid, "title")?.is_none() {
        after.title = title;
    }
    if stamp_of(tx, h.rowid, "note")?.is_none() && before.note.as_deref().is_none_or(str::is_empty)
    {
        after.note = Some(text.trim().to_string());
    }
    server_write(tx, &h, Object::Task(before), Object::Task(after), now)
}

impl AccountStore {
    /// The live attachments whose transcript is asked for, oldest first.
    pub fn pending_transcripts(&self) -> Result<Vec<Id>> {
        let mut statement = self.conn.prepare(
            "SELECT o.uuid FROM attachment a JOIN object o ON o.id = a.object \
             WHERE o.deleted_at IS NULL AND a.transcript_state = 'pending' ORDER BY o.id",
        )?;
        let ids = statement
            .query_map([], |row| row.get::<_, Vec<u8>>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        ids.iter().map(|b| id_of_blob(b)).collect()
    }

    /// Asks for the transcript of an audio attachment: it waits from now
    /// on, and an earlier failure is forgotten. A transcript it has stays
    /// until the new one is there.
    pub fn ask_transcript(&mut self, id: Id, now: Timestamp) -> Result<Asked> {
        let tx = self.conn.transaction()?;
        let Some((h, before)) = live_attachment(&tx, id)? else {
            return Ok(Asked::Missing);
        };
        if !is_audio(&before.media_type) {
            return Ok(Asked::NotAudio);
        }
        let mut after = before.clone();
        after.transcript_state = Some(TranscriptState::Pending);
        after.transcript_error = None;
        server_write(
            &tx,
            &h,
            Object::Attachment(before),
            Object::Attachment(after.clone()),
            now,
        )?;
        tx.commit()?;
        Ok(Asked::Pending(Box::new(after)))
    }

    /// Writes what the service answered and ends the waiting. A transcript
    /// with words fills the voice note it belongs to (ADR 0140 §4). `false`
    /// when the attachment is gone.
    pub fn store_transcript(
        &mut self,
        id: Id,
        outcome: &Transcribed,
        now: Timestamp,
    ) -> Result<bool> {
        let tx = self.conn.transaction()?;
        let Some((h, before)) = live_attachment(&tx, id)? else {
            return Ok(false);
        };
        let mut after = before.clone();
        match outcome {
            Transcribed::Text(text) => {
                after.transcript = Some(text.trim().to_string());
                after.transcript_state = None;
                after.transcript_error = None;
            }
            Transcribed::Failed(reason) => {
                after.transcript_state = Some(TranscriptState::Failed);
                after.transcript_error = Some(reason.clone());
            }
        }
        let owner = before.owner;
        server_write(
            &tx,
            &h,
            Object::Attachment(before),
            Object::Attachment(after),
            now,
        )?;
        if let Transcribed::Text(text) = outcome {
            fill_voice_note(&tx, owner, text, now)?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// Ends the waiting without an answer: for a server that has no
    /// transcript service (ADR 0140 §1).
    pub fn drop_transcript_request(&mut self, id: Id, now: Timestamp) -> Result<()> {
        let tx = self.conn.transaction()?;
        let Some((h, before)) = live_attachment(&tx, id)? else {
            return Ok(());
        };
        if before.transcript_state != Some(TranscriptState::Pending) {
            return Ok(());
        }
        let mut after = before.clone();
        after.transcript_state = None;
        server_write(
            &tx,
            &h,
            Object::Attachment(before),
            Object::Attachment(after),
            now,
        )?;
        tx.commit()?;
        Ok(())
    }
}
