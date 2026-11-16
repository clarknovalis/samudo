-- SPDX-FileCopyrightText: 2026 Marc Seibert
--
-- SPDX-License-Identifier: AGPL-3.0-or-later

-- Scopes, second round (ADR 0043): the inbox as a member, a filter in the
-- search language, what a capture inherits, a schedule.
--
-- `filter`, `capture` and `schedule` hold the JSON of their field, each
-- changed as a whole; NULL when empty.
ALTER TABLE scope ADD COLUMN inbox INTEGER NOT NULL DEFAULT 0 CHECK (inbox IN (0, 1));
ALTER TABLE scope ADD COLUMN filter TEXT CHECK (filter IS NULL OR json_valid(filter));
ALTER TABLE scope ADD COLUMN capture TEXT CHECK (capture IS NULL OR json_valid(capture));
ALTER TABLE scope ADD COLUMN schedule TEXT CHECK (schedule IS NULL OR json_valid(schedule));

-- Until now the inbox was never filtered (ADR 0035 §3). A scope with `only`
-- gets the tick, so every scope shows what it showed before (ADR 0043 §1).
UPDATE scope SET inbox = 1 WHERE mode = 'only';

-- Clients learn of the tick through /changes: every such scope gets a
-- change number of its own after the current one.
UPDATE object
SET seq = (SELECT seq FROM account WHERE id = 1)
        + (SELECT count(*) FROM scope s JOIN object o ON o.id = s.object
           WHERE s.mode = 'only' AND o.id <= object.id)
WHERE id IN (SELECT object FROM scope WHERE mode = 'only');
UPDATE account SET seq = seq + (SELECT count(*) FROM scope WHERE mode = 'only') WHERE id = 1;
