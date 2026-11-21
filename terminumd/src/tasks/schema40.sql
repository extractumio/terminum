-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- A project's red target run again by terminumd (terminum-integration.md AC-198): the steps its Bug task names and the
-- commit whose plan they come from; when it is looked at next, and the target commit last run red (or not started);
-- the last green run that ended such a Bug task, to tell a step red now and then.
CREATE TABLE integration_red (
  project TEXT PRIMARY KEY,
  steps TEXT NOT NULL,
  of_sha TEXT NOT NULL,
  next_at INTEGER NOT NULL,
  ran_sha TEXT,
  green_steps TEXT,
  green_at INTEGER
);
