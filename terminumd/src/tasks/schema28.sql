-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- The integration queue (terminum-integration.md AC-187): a task in INTEGRATION has its entry, with what it queued;
-- a commit returned for a red check is kept so that it is not queued again unchanged.
-- INTEGRATION among a task's statuses: the CHECK widened in place, as SQLite's ALTER TABLE page allows for a
-- change that leaves every row as it was (a rebuild of tasks would cascade its comments, history and files away);
-- the store bumps schema_version after it, so every connection reads it anew.
PRAGMA writable_schema = ON;
UPDATE sqlite_schema SET sql = replace(sql, '''WAIT FOR HUMAN'', ''FAILED''', '''WAIT FOR HUMAN'', ''INTEGRATION'', ''FAILED''')
  WHERE type = 'table' AND name = 'tasks';
PRAGMA writable_schema = OFF;
CREATE TABLE integration_entries (
  task TEXT PRIMARY KEY REFERENCES tasks ON DELETE CASCADE,
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  commit_sha TEXT NOT NULL,
  result TEXT NOT NULL,
  author TEXT NOT NULL,
  queued_at INTEGER NOT NULL,
  batch INTEGER,
  waits_for TEXT,
  told TEXT NOT NULL DEFAULT ''
);
CREATE INDEX integration_entries_project ON integration_entries (project, queued_at);
CREATE TABLE integration_returns (
  task TEXT NOT NULL REFERENCES tasks ON DELETE CASCADE,
  commit_sha TEXT NOT NULL,
  why TEXT NOT NULL,
  at INTEGER NOT NULL,
  PRIMARY KEY (task, commit_sha)
);
ALTER TABLE project_integration ADD COLUMN size INTEGER NOT NULL DEFAULT 4;
-- A batch and its rounds (AC-188, AC-190, AC-194): one in work on the machine at a time; why a project's queue waits.
CREATE TABLE integration_batches (
  id INTEGER PRIMARY KEY,
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  number INTEGER NOT NULL,
  task TEXT REFERENCES tasks ON DELETE SET NULL,
  tasks TEXT NOT NULL,
  base_sha TEXT NOT NULL,
  candidate_sha TEXT,
  state TEXT NOT NULL,
  why TEXT,
  pane TEXT,
  round INTEGER NOT NULL DEFAULT 1,
  red_rounds INTEGER NOT NULL DEFAULT 0,
  unchanged_rechecks INTEGER NOT NULL DEFAULT 0,
  step_started INTEGER,
  created_at INTEGER NOT NULL,
  ended_at INTEGER
);
CREATE INDEX integration_batches_state ON integration_batches (state);
CREATE TABLE integration_rounds (
  batch INTEGER NOT NULL REFERENCES integration_batches ON DELETE CASCADE,
  round INTEGER NOT NULL,
  tasks TEXT NOT NULL,
  base_sha TEXT NOT NULL,
  candidate_sha TEXT,
  started INTEGER,
  ended INTEGER,
  result TEXT NOT NULL,
  why TEXT,
  PRIMARY KEY (batch, round)
);
ALTER TABLE project_integration ADD COLUMN waits_why TEXT;
ALTER TABLE project_integration ADD COLUMN paused INTEGER NOT NULL DEFAULT 0;
