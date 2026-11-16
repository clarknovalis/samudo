-- SPDX-FileCopyrightText: 2026 Marc Seibert
--
-- SPDX-License-Identifier: AGPL-3.0-or-later

-- The key that calls a template after the capture prefix (ADR 0084 §1):
-- one letter or digit, NULL for none; unique among live templates, which
-- the store checks (a tombstone keeps its row). NULL allowed: no rebuild.
ALTER TABLE template ADD COLUMN key TEXT;
