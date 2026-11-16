-- SPDX-FileCopyrightText: 2026 Marc Seibert
--
-- SPDX-License-Identifier: AGPL-3.0-or-later

-- Block perspectives (ADR 0116 §1): the blocks as JSON, NULL for a plain
-- perspective, and `dedupe` as JSON, NULL for the default (true).
ALTER TABLE perspective ADD COLUMN blocks TEXT;
ALTER TABLE perspective ADD COLUMN dedupe TEXT;
