-- SPDX-FileCopyrightText: 2026 Marc Seibert
--
-- SPDX-License-Identifier: AGPL-3.0-or-later

-- The columns of a perspective's table (ADR 0115 §7): its `table` as JSON,
-- NULL for the default columns.
ALTER TABLE perspective ADD COLUMN table_view TEXT;
