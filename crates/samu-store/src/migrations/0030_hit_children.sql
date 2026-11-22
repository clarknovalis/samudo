-- SPDX-FileCopyrightText: 2026 Marc Seibert
--
-- SPDX-License-Identifier: AGPL-3.0-or-later

-- How a group that is a hit starts when its children are none (ADR 0021
-- §6), NULL for `open`. Without a rebuild.
ALTER TABLE settings ADD COLUMN hit_children TEXT;
