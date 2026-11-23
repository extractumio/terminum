-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- AC-236–239: retained problems, protected yardsticks, and Supervisor spend attribution.
CREATE TABLE supervisor_problems (
  id INTEGER PRIMARY KEY,
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  title TEXT NOT NULL,
  lens TEXT NOT NULL,
  hypothesis TEXT NOT NULL,
  disproof TEXT NOT NULL,
  options TEXT NOT NULL,
  confidence REAL NOT NULL,
  effort INTEGER NOT NULL,
  state TEXT NOT NULL,
  opened_at INTEGER NOT NULL,
  ruling TEXT,
  new_evidence TEXT,
  source_round TEXT REFERENCES supervisor_rounds (run),
  automatic INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX supervisor_problems_project ON supervisor_problems (project, id);

CREATE TABLE supervisor_problem_signals (
  problem INTEGER NOT NULL REFERENCES supervisor_problems ON DELETE CASCADE,
  fingerprint TEXT NOT NULL UNIQUE REFERENCES supervisor_signals ON DELETE CASCADE,
  threshold REAL NOT NULL,
  weeks INTEGER NOT NULL,
  ruling_evidence TEXT,
  clear_at INTEGER,
  clear_seen INTEGER,
  PRIMARY KEY (problem, fingerprint)
) WITHOUT ROWID;

CREATE TABLE supervisor_problem_events (
  id INTEGER PRIMARY KEY,
  problem INTEGER NOT NULL REFERENCES supervisor_problems ON DELETE CASCADE,
  at INTEGER NOT NULL,
  state TEXT NOT NULL,
  detail TEXT NOT NULL
);
CREATE INDEX supervisor_problem_events_by_problem ON supervisor_problem_events (problem, id);

CREATE TABLE supervisor_problem_tasks (
  problem INTEGER NOT NULL REFERENCES supervisor_problems ON DELETE CASCADE,
  task TEXT NOT NULL REFERENCES tasks ON DELETE CASCADE,
  PRIMARY KEY (problem, task)
) WITHOUT ROWID;
CREATE INDEX supervisor_problem_tasks_by_task ON supervisor_problem_tasks (task);

CREATE TABLE supervisor_task_marks (
  task TEXT PRIMARY KEY NOT NULL REFERENCES tasks ON DELETE CASCADE,
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  estimate REAL NOT NULL,
  outside INTEGER NOT NULL DEFAULT 0,
  exception INTEGER NOT NULL DEFAULT 0
) WITHOUT ROWID;
CREATE INDEX supervisor_task_marks_project ON supervisor_task_marks (project);

CREATE TABLE supervisor_fix_runs (
  run TEXT PRIMARY KEY NOT NULL REFERENCES runs ON DELETE CASCADE,
  estimate REAL NOT NULL
) WITHOUT ROWID;

CREATE TABLE supervisor_rank_proposals (
  task TEXT PRIMARY KEY NOT NULL REFERENCES tasks ON DELETE CASCADE,
  problem INTEGER NOT NULL REFERENCES supervisor_problems ON DELETE CASCADE,
  proposed_at INTEGER NOT NULL,
  source_round TEXT REFERENCES supervisor_rounds (run)
) WITHOUT ROWID;

CREATE TABLE supervisor_budget_cards (
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  kind TEXT NOT NULL,
  task TEXT NOT NULL REFERENCES tasks ON DELETE CASCADE,
  PRIMARY KEY (project, kind)
) WITHOUT ROWID;

ALTER TABLE supervisor_rounds ADD COLUMN token_cap INTEGER;
ALTER TABLE supervisor_rounds ADD COLUMN reserved_class INTEGER NOT NULL DEFAULT 0;
CREATE INDEX supervisor_rounds_project ON supervisor_rounds (project, ended_at);
ALTER TABLE projects ADD COLUMN auto_supervisor INTEGER NOT NULL DEFAULT 1 CHECK (auto_supervisor IN (0, 1));

-- Preserve authorship for tasks made by an earlier Supervisor implementation.
INSERT INTO supervisor_task_marks (task, project, estimate)
SELECT t.id, t.project, 0 FROM tasks t WHERE EXISTS (
  SELECT 1 FROM events e WHERE e.task = t.id AND e.kind = 'made' AND e.author = 'supervisor:' || t.project);

CREATE VIEW supervisor_human_actions AS
SELECT e.rowid AS event_order, e.task, e.id, e.at, e.author FROM events e JOIN users u ON u.id = e.author
WHERE e.kind IN ('move','edit','why','comment','archive','link','session','reorder','project')
  AND (u.kind = 'human' OR e.author GLOB 'manager:*' OR json_extract(e.data, '$.by') = 'manager');

CREATE VIEW supervisor_auto_marks AS
SELECT m.task, m.outside = 0 OR EXISTS (SELECT 1 FROM events e JOIN users u ON u.id = e.author
    WHERE e.task = m.task AND e.kind = 'move' AND u.kind = 'human') AS eligible
FROM supervisor_task_marks m;
