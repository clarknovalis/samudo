-- Workflows (ADR 0024): an exclusive tag group is one; its children are the
-- states. A state has a key to pick it by and may ask for a note; a group
-- may be the workflow of projects that name none. A project names its own.
ALTER TABLE tag ADD COLUMN key TEXT CHECK (key IS NULL OR length(key) = 1);
ALTER TABLE tag ADD COLUMN ask_note INTEGER NOT NULL DEFAULT 0 CHECK (ask_note IN (0, 1));
ALTER TABLE tag ADD COLUMN default_workflow INTEGER NOT NULL DEFAULT 0
    CHECK (default_workflow IN (0, 1));
ALTER TABLE project ADD COLUMN workflow BLOB CHECK (workflow IS NULL OR length(workflow) = 16);

CREATE INDEX tag_by_parent ON tag (parent);

-- The device of the change a logbook entry comes from (ADR 0024 §5).
ALTER TABLE logbook ADD COLUMN device TEXT NOT NULL DEFAULT '';
