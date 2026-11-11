-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- Auto-Dispatch's two ways of picking (terminum-dispatcher.md AC-170): 'agent', a round of the picking agent, or
-- 'order', terminumd itself down the board with no agent. A start then has no round: it names its project itself.
ALTER TABLE projects ADD COLUMN auto_mode TEXT NOT NULL DEFAULT 'agent' CHECK (auto_mode IN ('agent', 'order'));
CREATE TABLE auto_starts_new (
  task TEXT NOT NULL REFERENCES tasks,
  round TEXT REFERENCES tasks,
  project TEXT NOT NULL,
  at INTEGER NOT NULL
);
INSERT INTO auto_starts_new (task, round, project, at) SELECT s.task, s.round, r.project, s.at FROM auto_starts s JOIN auto_rounds r ON r.task = s.round ORDER BY s.rowid;
DROP TABLE auto_starts;
ALTER TABLE auto_starts_new RENAME TO auto_starts;
CREATE INDEX auto_starts_round ON auto_starts (round);
CREATE INDEX auto_starts_task ON auto_starts (task, at);
CREATE INDEX auto_starts_project ON auto_starts (project, at);
