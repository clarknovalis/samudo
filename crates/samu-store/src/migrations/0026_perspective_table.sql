-- The columns of a perspective's table (ADR 0115 §7): its `table` as JSON,
-- NULL for the default columns.
ALTER TABLE perspective ADD COLUMN table_view TEXT;
