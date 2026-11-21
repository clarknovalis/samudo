// SPDX-FileCopyrightText: 2026 Marc Seibert
//
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Voice notes (ADR 0140 §4): a task made from a recording, whose title
//! and note come from the transcript once there is one.

use crate::hook::ring_title;

/// The property `source` of a task that was made from a recording. The
/// server fills title and note of such a task from the transcript.
pub const VOICE_SOURCE: &str = "voice";

/// The title of a voice note from its transcript: the first line with
/// text, cut at a word boundary to at most 80 characters, as for the ring
/// (ADR 0010 §3). `None` for a text without words.
#[must_use]
pub fn voice_title(transcript: &str) -> Option<String> {
    ring_title(transcript)
}

/// Whether a media type is audio, the only kind a transcript is made of.
#[must_use]
pub fn is_audio(media_type: &str) -> bool {
    media_type
        .get(..6)
        .is_some_and(|start| start.eq_ignore_ascii_case("audio/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_is_the_first_line_cut_at_a_word() {
        assert_eq!(
            voice_title("  \nBuy milk.\nAnd bread."),
            Some("Buy milk.".into())
        );
        let long = "word ".repeat(40);
        let title = voice_title(&long).unwrap_or_default();
        assert!(title.chars().count() <= 80 && title.ends_with('…'));
        assert_eq!(voice_title(" \n "), None);
    }

    #[test]
    fn audio_is_told_by_the_media_type() {
        assert!(is_audio("audio/mp4"));
        assert!(is_audio("Audio/webm;codecs=opus"));
        assert!(!is_audio("video/webm"));
        assert!(!is_audio("au"));
    }
}
