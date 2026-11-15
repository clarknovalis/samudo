-- An exclusive object names its own scope (ADR 0094). A column with no
-- default: no rebuild. `scope_only` stays in the tables but is no longer
-- read or written; the wire field is derived from `own_scope` (§6).
ALTER TABLE folder ADD COLUMN own_scope BLOB CHECK (own_scope IS NULL OR length(own_scope) = 16);
ALTER TABLE project ADD COLUMN own_scope BLOB CHECK (own_scope IS NULL OR length(own_scope) = 16);
ALTER TABLE tag ADD COLUMN own_scope BLOB CHECK (own_scope IS NULL OR length(own_scope) = 16);

-- What each live `only` scope unlocked under ADR 0069 §3: its members and
-- everything above them (a project's folders, a folder's parents, a tag's
-- parents).
CREATE TEMP TABLE unlocked AS
WITH RECURSIVE up(scope, uuid) AS (
    SELECT m.owner, m.member
    FROM scope_member m
    JOIN scope s ON s.object = m.owner
    JOIN object o ON o.id = s.object
    WHERE m.present = 1 AND s.mode = 'only' AND o.deleted_at IS NULL
    UNION
    SELECT up.scope, f.parent
    FROM up JOIN object o ON o.uuid = up.uuid JOIN folder f ON f.object = o.id
    WHERE f.parent IS NOT NULL
    UNION
    SELECT up.scope, p.folder
    FROM up JOIN object o ON o.uuid = up.uuid JOIN project p ON p.object = o.id
    WHERE p.folder IS NOT NULL
    UNION
    SELECT up.scope, t.parent
    FROM up JOIN object o ON o.uuid = up.uuid JOIN tag t ON t.object = o.id
    WHERE t.parent IS NOT NULL
)
SELECT up.uuid AS uuid, so.uuid AS scope, s.position AS position
FROM up JOIN scope s ON s.object = up.scope JOIN object so ON so.id = s.object;

-- Each scope_only object gets the unlocking scope with the smallest
-- position (§6); without one the field stays empty.
UPDATE folder SET own_scope = (
    SELECT u.scope FROM unlocked u JOIN object o ON o.uuid = u.uuid
    WHERE o.id = folder.object ORDER BY u.position, u.scope LIMIT 1)
WHERE scope_only = 1;
UPDATE project SET own_scope = (
    SELECT u.scope FROM unlocked u JOIN object o ON o.uuid = u.uuid
    WHERE o.id = project.object ORDER BY u.position, u.scope LIMIT 1)
WHERE scope_only = 1;
UPDATE tag SET own_scope = (
    SELECT u.scope FROM unlocked u JOIN object o ON o.uuid = u.uuid
    WHERE o.id = tag.object ORDER BY u.position, u.scope LIMIT 1)
WHERE scope_only = 1;

DROP TABLE unlocked;

-- Clients learn of the new field through /changes: every scope_only object
-- gets a change number of its own after the current one (like 0008).
CREATE TEMP TABLE touched AS
SELECT object AS id FROM folder WHERE scope_only = 1
UNION SELECT object FROM project WHERE scope_only = 1
UNION SELECT object FROM tag WHERE scope_only = 1;

UPDATE object
SET seq = (SELECT seq FROM account WHERE id = 1)
        + (SELECT count(*) FROM touched t WHERE t.id <= object.id)
WHERE id IN (SELECT id FROM touched);
UPDATE account SET seq = seq + (SELECT count(*) FROM touched) WHERE id = 1;

DROP TABLE touched;
