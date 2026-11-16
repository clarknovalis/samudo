-- SPDX-FileCopyrightText: 2026 Marc Seibert
--
-- SPDX-License-Identifier: AGPL-3.0-or-later

-- The web's own key bindings (ADR 0079 §2): the differences from the
-- defaults as JSON, NULL for none. A column that may be NULL: no rebuild.
ALTER TABLE settings ADD COLUMN keys TEXT;
