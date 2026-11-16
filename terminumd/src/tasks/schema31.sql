-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- A task's attributes (terminum-kanban.md AC-199): its type, the word its title is shown with (the title is kept
-- without it); a link to an outside tracker; the release it is meant for; the tasks that block it and the tasks
-- that follow it, each with its reason.
-- one of tasks/attrs.rs TYPES, kept there: a CHECK here would need the schema rewritten to widen
ALTER TABLE tasks ADD COLUMN type TEXT;
ALTER TABLE tasks ADD COLUMN external_link TEXT;
ALTER TABLE tasks ADD COLUMN target TEXT;
-- the title as every list and text shows it: its type's word, then the title
ALTER TABLE tasks ADD COLUMN shown TEXT GENERATED ALWAYS AS (CASE type WHEN 'Feature' THEN 'Feat: ' WHEN 'Bug' THEN 'Bug: '
  WHEN 'Debt' THEN 'Debt: ' WHEN 'Story' THEN 'Story: ' ELSE '' END || title) VIRTUAL;
-- by the rows' keys: a task's ID may change (AC-148), its links stay
CREATE TABLE task_links (
  task TEXT NOT NULL REFERENCES tasks ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('blocked_by', 'follow_ups')),
  other TEXT NOT NULL REFERENCES tasks ON DELETE CASCADE,
  reason TEXT NOT NULL DEFAULT '',
  place INTEGER NOT NULL,
  PRIMARY KEY (task, kind, other)
);
CREATE INDEX task_links_other ON task_links (other);
-- a title that began with its type's word ("Bug: …", any case, as tasks/attrs.rs split_title reads it) keeps it as
-- its type, the title the rest; one that would be empty stays; nothing in the history, the times stay
UPDATE tasks SET
  type = CASE lower(substr(title, 1, instr(title, ':') - 1)) WHEN 'feat' THEN 'Feature' WHEN 'feature' THEN 'Feature'
    WHEN 'bug' THEN 'Bug' WHEN 'debt' THEN 'Debt' WHEN 'story' THEN 'Story' END,
  title = ltrim(substr(title, instr(title, ':') + 1))
  WHERE type IS NULL AND lower(substr(title, 1, instr(title, ':') - 1)) IN ('feat', 'feature', 'bug', 'debt', 'story')
    AND ltrim(substr(title, instr(title, ':') + 1)) <> '';
