-- SPDX-FileCopyrightText: 2026 Marc Seibert
--
-- SPDX-License-Identifier: AGPL-3.0-or-later

-- The transcript of an audio attachment (ADR 0140 §2, §3): the text, NULL
-- until a service has answered; whether one is asked for or failed; and
-- why it failed.
ALTER TABLE attachment ADD COLUMN transcript TEXT;
ALTER TABLE attachment ADD COLUMN transcript_state TEXT
    CHECK (transcript_state IN ('pending', 'failed'));
ALTER TABLE attachment ADD COLUMN transcript_error TEXT;

CREATE INDEX attachment_by_transcript_state ON attachment (transcript_state)
    WHERE transcript_state IS NOT NULL;
