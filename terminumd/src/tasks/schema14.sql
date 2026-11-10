-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- Schedules (terminum-schedules.md §9): a human's task that runs on a cadence on one agent, each firing a new task.
CREATE TABLE schedules (
  id TEXT PRIMARY KEY,
  number INTEGER NOT NULL UNIQUE,
  project TEXT NOT NULL REFERENCES projects,
  title TEXT NOT NULL,
  body TEXT NOT NULL,
  severity TEXT NOT NULL CHECK (severity IN ('P0', 'P1', 'P2', 'P3')),
  agent_id TEXT NOT NULL,
  cron TEXT,
  once_at INTEGER,
  budget_seconds INTEGER NOT NULL,
  paused INTEGER NOT NULL DEFAULT 0,
  paused_why TEXT,
  last_due INTEGER NOT NULL,
  author TEXT NOT NULL REFERENCES users,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  deleted_at INTEGER,
  CHECK ((cron IS NULL) <> (once_at IS NULL))
);
CREATE INDEX schedules_agent ON schedules (agent_id, deleted_at, paused);
CREATE TABLE schedule_fires (
  id INTEGER PRIMARY KEY,
  schedule TEXT NOT NULL REFERENCES schedules,
  due_at INTEGER NOT NULL,
  at INTEGER NOT NULL,
  task TEXT REFERENCES tasks,
  state TEXT NOT NULL CHECK (state IN ('made', 'started', 'failed', 'skipped', 'called off')),
  missed INTEGER NOT NULL DEFAULT 0,
  why TEXT,
  prompt TEXT
);
CREATE INDEX schedule_fires_schedule ON schedule_fires (schedule, id);
CREATE UNIQUE INDEX schedule_fires_task ON schedule_fires (task) WHERE task IS NOT NULL;
CREATE INDEX schedule_fires_made ON schedule_fires (state) WHERE state = 'made';
