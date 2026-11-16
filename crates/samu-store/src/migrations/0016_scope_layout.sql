-- SPDX-FileCopyrightText: 2026 Marc Seibert
--
-- SPDX-License-Identifier: AGPL-3.0-or-later

-- How the views show a scope (ADR 0072): its layout as JSON, NULL for
-- none. A column that may be NULL: no rebuild.
ALTER TABLE scope ADD COLUMN layout TEXT;
