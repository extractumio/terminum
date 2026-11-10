-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- Auto-Dispatcher (terminum-dispatcher.md AC-170). A project's setting: the agent that picks (none: off), the one that
-- runs a task naming none, what its last round saw, why it turned itself off, and since when it is on.
ALTER TABLE projects ADD COLUMN auto_agent TEXT;
ALTER TABLE projects ADD COLUMN auto_default TEXT;
ALTER TABLE projects ADD COLUMN auto_seen TEXT;
ALTER TABLE projects ADD COLUMN auto_off_why TEXT;
ALTER TABLE projects ADD COLUMN auto_on_at INTEGER NOT NULL DEFAULT 0;
-- The agent of the pool a human chose to run a task.
ALTER TABLE tasks ADD COLUMN agent TEXT;
-- A round: the task Dispatcher makes for the picking agent, with the free slots and the candidates it was given.
CREATE TABLE auto_rounds (
  task TEXT PRIMARY KEY REFERENCES tasks,
  project TEXT NOT NULL,
  slots INTEGER NOT NULL,
  candidates TEXT NOT NULL,
  at INTEGER NOT NULL
);
-- What a round started: the task, on which agent.
CREATE TABLE auto_starts (
  task TEXT NOT NULL REFERENCES tasks,
  round TEXT NOT NULL REFERENCES tasks,
  agent_id TEXT NOT NULL,
  at INTEGER NOT NULL
);
