-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- AC-240–244: decisions, comment identities, review publications and the incident thread.
CREATE TABLE supervisor_decisions (
  id INTEGER PRIMARY KEY,
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  problem INTEGER NOT NULL REFERENCES supervisor_problems ON DELETE CASCADE,
  kind TEXT NOT NULL,
  evidence TEXT NOT NULL,
  question TEXT NOT NULL,
  cost REAL NOT NULL,
  task TEXT REFERENCES tasks ON DELETE SET NULL,
  source_run TEXT NOT NULL REFERENCES supervisor_rounds (run) ON DELETE CASCADE,
  choices TEXT NOT NULL,
  published INTEGER NOT NULL DEFAULT 0 CHECK (published IN (0,1)),
  UNIQUE (problem, kind, evidence)
);
CREATE INDEX supervisor_decisions_project ON supervisor_decisions (project, cost DESC, id);

CREATE TABLE supervisor_nudges (
  fingerprint TEXT NOT NULL REFERENCES supervisor_signals ON DELETE CASCADE,
  task TEXT NOT NULL REFERENCES tasks ON DELETE CASCADE,
  event TEXT NOT NULL REFERENCES events ON DELETE CASCADE,
  PRIMARY KEY (fingerprint, task)
) WITHOUT ROWID;

CREATE TABLE supervisor_digests (
  project TEXT PRIMARY KEY REFERENCES projects ON DELETE CASCADE,
  at INTEGER NOT NULL,
  document TEXT NOT NULL,
  last_ok INTEGER
) WITHOUT ROWID;

CREATE TABLE supervisor_reviews (
  project TEXT PRIMARY KEY REFERENCES projects ON DELETE CASCADE,
  schedule TEXT NOT NULL UNIQUE REFERENCES schedules ON DELETE CASCADE
) WITHOUT ROWID;

CREATE TABLE supervisor_review_items (
  task TEXT NOT NULL REFERENCES tasks ON DELETE CASCADE,
  fingerprint TEXT NOT NULL REFERENCES supervisor_signals ON DELETE CASCADE,
  position INTEGER NOT NULL,
  PRIMARY KEY (task, fingerprint)
) WITHOUT ROWID;

CREATE TABLE supervisor_verdicts (
  id INTEGER PRIMARY KEY,
  task TEXT NOT NULL,
  fingerprint TEXT NOT NULL,
  at INTEGER NOT NULL,
  verdict TEXT NOT NULL CHECK (verdict IN ('useful', 'not_useful', 'wrong')),
  FOREIGN KEY (task, fingerprint) REFERENCES supervisor_review_items ON DELETE CASCADE
);
CREATE INDEX supervisor_verdicts_item ON supervisor_verdicts (task, fingerprint, id DESC);

ALTER TABLE supervisor_incidents ADD COLUMN task TEXT REFERENCES tasks ON DELETE SET NULL;
CREATE UNIQUE INDEX supervisor_incidents_task ON supervisor_incidents (task) WHERE task IS NOT NULL;
ALTER TABLE supervisor_rounds ADD COLUMN incident INTEGER REFERENCES supervisor_incidents ON DELETE SET NULL;
CREATE TABLE supervisor_talks (
  run TEXT PRIMARY KEY REFERENCES runs ON DELETE CASCADE,
  incident INTEGER NOT NULL REFERENCES supervisor_incidents ON DELETE CASCADE,
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  generation INTEGER NOT NULL,
  case_file TEXT NOT NULL
) WITHOUT ROWID;
ALTER TABLE supervisor_panes ADD COLUMN talk TEXT REFERENCES supervisor_talks (run) ON DELETE SET NULL;

CREATE INDEX supervisor_signals_project_last_seen ON supervisor_signals (project, last_seen);
CREATE INDEX supervisor_signals_project_first_seen ON supervisor_signals (project, first_seen);

CREATE TABLE supervisor_review_reports (
  task TEXT PRIMARY KEY REFERENCES tasks ON DELETE CASCADE,
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  at INTEGER NOT NULL,
  result_event TEXT NOT NULL REFERENCES events ON DELETE CASCADE
) WITHOUT ROWID;
CREATE INDEX supervisor_review_reports_project ON supervisor_review_reports (project, at DESC);

CREATE TABLE supervisor_review_runs (
  run TEXT PRIMARY KEY REFERENCES runs ON DELETE CASCADE,
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  cap INTEGER NOT NULL,
  generation INTEGER NOT NULL,
  overrun INTEGER NOT NULL DEFAULT 0
) WITHOUT ROWID;
CREATE INDEX supervisor_review_runs_project ON supervisor_review_runs (project);

CREATE TABLE supervisor_incident_rulings (
  incident INTEGER NOT NULL REFERENCES supervisor_incidents ON DELETE CASCADE,
  fingerprint TEXT NOT NULL REFERENCES supervisor_signals ON DELETE CASCADE,
  evidence TEXT NOT NULL,
  event TEXT NOT NULL REFERENCES events ON DELETE CASCADE,
  PRIMARY KEY (fingerprint, evidence)
) WITHOUT ROWID;

CREATE TABLE supervisor_charter_questions (
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  field TEXT NOT NULL,
  due INTEGER NOT NULL,
  task TEXT NOT NULL REFERENCES tasks,
  PRIMARY KEY (project, field, due)
) WITHOUT ROWID;

ALTER TABLE supervisor_round_requests ADD COLUMN incident INTEGER REFERENCES supervisor_incidents ON DELETE SET NULL;

ALTER TABLE project_supervisor ADD COLUMN review_setup_error TEXT;

CREATE TABLE supervisor_drift_questions (
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  subjects TEXT NOT NULL,
  question TEXT NOT NULL,
  task TEXT REFERENCES tasks ON DELETE SET NULL,
  at INTEGER NOT NULL,
  PRIMARY KEY (project, subjects)
) WITHOUT ROWID;
