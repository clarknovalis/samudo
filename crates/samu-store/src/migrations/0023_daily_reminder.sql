-- A daily reminder (ADR 0093): the anchor `daily` with a time of day.
--
-- `reminder.anchor` has a CHECK listing the anchors, and SQLite cannot
-- change a CHECK, so the table is rebuilt (like 0005). Only this table:
-- `object` stays as it is. Runs with foreign keys off; the runner checks
-- them before it commits.
CREATE TABLE reminder_new (
    object      INTEGER PRIMARY KEY REFERENCES object (id) ON DELETE CASCADE,
    owner       BLOB NOT NULL CHECK (length(owner) = 16),
    anchor      TEXT NOT NULL CHECK (anchor IN ('at', 'due', 'planned', 'defer', 'daily')),
    at          TEXT,
    before      TEXT,
    time        TEXT,
    CHECK ((anchor = 'at') = (at IS NOT NULL)),
    CHECK ((anchor = 'daily') = (time IS NOT NULL)),
    CHECK (anchor != 'daily' OR before IS NULL)
);

INSERT INTO reminder_new (object, owner, anchor, at, before)
    SELECT object, owner, anchor, at, before FROM reminder;

DROP TABLE reminder;
ALTER TABLE reminder_new RENAME TO reminder;
CREATE INDEX reminder_by_owner ON reminder (owner);
