-- Talon LTM schema (migration v3) — add free-form tags to memories.
-- tags: JSON array of strings, parallel to `entities`. Task 2.5.3.

ALTER TABLE memories ADD COLUMN tags TEXT NOT NULL DEFAULT '[]';
