-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- A project's red target re-run by terminumd (terminum-integration.md AC-198): the steps its Bug task names and the
-- commit whose plan they come from; the target commit last found red, when, and whether a re-run there was red too;
-- when terminumd last looked; and the last green re-run that closed such a Bug task, to tell a step red now and then.
CREATE TABLE integration_red (
  project TEXT PRIMARY KEY,
  steps TEXT NOT NULL,
  of_sha TEXT NOT NULL,
  red_sha TEXT NOT NULL,
  red_at INTEGER NOT NULL,
  retried INTEGER NOT NULL DEFAULT 0,
  looked_at INTEGER NOT NULL,
  green_steps TEXT,
  green_at INTEGER
);
