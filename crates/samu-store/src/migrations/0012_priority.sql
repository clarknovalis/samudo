-- SPDX-FileCopyrightText: 2026 Marc Seibert
--
-- SPDX-License-Identifier: AGPL-3.0-or-later

-- The priority of a task (ADR 0056 §3): A, B, C or none. A column that may
-- be NULL: no rebuild.
ALTER TABLE task ADD COLUMN priority TEXT
    CHECK (priority IS NULL OR priority IN ('A', 'B', 'C'));
