-- How the views show a scope (ADR 0072): its layout as JSON, NULL for
-- none. A column that may be NULL: no rebuild.
ALTER TABLE scope ADD COLUMN layout TEXT;
