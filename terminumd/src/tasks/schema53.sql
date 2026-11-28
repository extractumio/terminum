-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- AC-248: one retained request and at most one round for an observed occurrence.
CREATE TABLE supervisor_reflections (
  fingerprint TEXT PRIMARY KEY,
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  generation INTEGER NOT NULL,
  subject_kind TEXT NOT NULL,
  subject_id TEXT NOT NULL,
  signal TEXT,
  at INTEGER NOT NULL,
  triggers TEXT NOT NULL,
  detail TEXT NOT NULL,
  reason TEXT,
  retry_at INTEGER,
  round TEXT UNIQUE REFERENCES supervisor_rounds,
  routing_card TEXT REFERENCES tasks ON DELETE SET NULL
);
CREATE INDEX supervisor_reflections_waiting ON supervisor_reflections (at, fingerprint) WHERE round IS NULL;
CREATE INDEX supervisor_reflections_project ON supervisor_reflections (project, at DESC);

CREATE INDEX supervisor_actions_project_at ON supervisor_actions (project, at);
CREATE INDEX supervisor_actions_task_target ON supervisor_actions (project, target, at)
  WHERE kind IN ('fix','run','nudge');

CREATE TABLE supervisor_decision_bands (
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  generation INTEGER NOT NULL,
  detector TEXT NOT NULL,
  response TEXT NOT NULL,
  direction INTEGER NOT NULL,
  PRIMARY KEY (project, detector, response)
) WITHOUT ROWID;

CREATE TABLE supervisor_budget_overruns (
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  generation INTEGER NOT NULL,
  metric TEXT NOT NULL,
  PRIMARY KEY (project, metric)
) WITHOUT ROWID;
