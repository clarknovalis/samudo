-- SPDX-FileCopyrightText: 2026 Marc Seibert
--
-- SPDX-License-Identifier: AGPL-3.0-or-later

-- The kind of a session (ADR 0039 §2): a person or an agent, chosen when
-- signing in. Sessions from before are people's.
ALTER TABLE session ADD COLUMN kind TEXT NOT NULL DEFAULT 'human'
    CHECK (kind IN ('human', 'agent'));
