-- SPDX-FileCopyrightText: 2026 Marc Seibert
--
-- SPDX-License-Identifier: AGPL-3.0-or-later

-- The report (ADR 0080): the logbook read by time across all objects, and
-- what counts as worked on, NULL for `events`. Both without a rebuild.
CREATE INDEX logbook_by_at ON logbook (at);
ALTER TABLE settings ADD COLUMN report_worked TEXT;
