-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- Supervisor signals (terminum-supervisor.md AC-211). Evidence is an array of typed store references.
CREATE TABLE supervisor_signals (
  fingerprint TEXT PRIMARY KEY NOT NULL,
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  detector TEXT NOT NULL,
  subject_kind TEXT NOT NULL,
  subject_id TEXT NOT NULL,
  metric TEXT NOT NULL,
  evidence TEXT NOT NULL,
  measure TEXT NOT NULL,
  scores TEXT NOT NULL,
  first_seen INTEGER NOT NULL,
  last_seen INTEGER NOT NULL,
  count INTEGER NOT NULL,
  state TEXT NOT NULL CHECK (state IN ('new', 'shadow', 'raised', 'held', 'resolved', 'recurred', 'muted', 'rejected'))
);
CREATE INDEX supervisor_signals_rule ON supervisor_signals (project, detector);

-- The latest complete evaluation per rule. An unavailable detector never advances it.
CREATE TABLE supervisor_evaluations (
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  detector TEXT NOT NULL,
  evaluated_at INTEGER NOT NULL,
  PRIMARY KEY (project, detector)
) WITHOUT ROWID;


-- Resolve a task outcome to its latest retained successful launch.
CREATE INDEX inputs_run_launch ON inputs (run, at, id)
  WHERE component = 'dispatcher.start' AND kind = 'launch' AND error IS NULL;
