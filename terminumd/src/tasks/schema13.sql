-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- A run whose agent asked a human itself: its session closes once its turn is over (terminum-dispatcher.md AC-157).
ALTER TABLE runs ADD COLUMN asked INTEGER NOT NULL DEFAULT 0;
-- A RUN at a slot's limit, kept until there is room (AC-158): the press as its route takes it.
CREATE TABLE run_waits (
  task TEXT PRIMARY KEY REFERENCES tasks,
  agent_id TEXT NOT NULL,
  project TEXT NOT NULL,
  status TEXT NOT NULL,
  body TEXT NOT NULL,
  at INTEGER NOT NULL
);
