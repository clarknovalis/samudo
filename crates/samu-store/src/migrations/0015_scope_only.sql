-- SPDX-FileCopyrightText: 2026 Marc Seibert
--
-- SPDX-License-Identifier: AGPL-3.0-or-later

-- Folders, projects and tags shown only in a scope that names them or
-- something below them (ADR 0069, 0070). A column with a default: no
-- rebuild.
ALTER TABLE folder ADD COLUMN scope_only INTEGER NOT NULL DEFAULT 0 CHECK (scope_only IN (0, 1));
ALTER TABLE project ADD COLUMN scope_only INTEGER NOT NULL DEFAULT 0 CHECK (scope_only IN (0, 1));
ALTER TABLE tag ADD COLUMN scope_only INTEGER NOT NULL DEFAULT 0 CHECK (scope_only IN (0, 1));
