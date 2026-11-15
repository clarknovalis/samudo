-- An emoji for folders and projects (ADR 0059 §3): not empty, at most 32
-- bytes. A colour for a scope from a fixed palette (ADR 0060 §3). Columns
-- that may be NULL: no rebuild.
ALTER TABLE folder ADD COLUMN icon TEXT
    CHECK (icon IS NULL OR (length(CAST(icon AS BLOB)) BETWEEN 1 AND 32));
ALTER TABLE project ADD COLUMN icon TEXT
    CHECK (icon IS NULL OR (length(CAST(icon AS BLOB)) BETWEEN 1 AND 32));
ALTER TABLE scope ADD COLUMN color TEXT
    CHECK (color IS NULL OR color IN
        ('violet', 'blue', 'teal', 'green', 'yellow', 'orange', 'red', 'pink'));
