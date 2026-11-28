-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- Local Supervisor incidents and their code-written timelines (AC-224–226).
CREATE TABLE supervisor_incidents (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  subject_kind TEXT NOT NULL,
  subject_id TEXT NOT NULL,
  opened_at INTEGER NOT NULL,
  last_firing_at INTEGER NOT NULL,
  closed_at INTEGER,
  acknowledged_at INTEGER,
  snoozed_until INTEGER
);
CREATE INDEX supervisor_incidents_subject ON supervisor_incidents
  (project, subject_kind, subject_id, last_firing_at DESC) WHERE closed_at IS NULL;
CREATE INDEX supervisor_incidents_project ON supervisor_incidents (project, id DESC);
CREATE INDEX supervisor_incidents_attention ON supervisor_incidents (id DESC)
  WHERE closed_at IS NULL OR acknowledged_at IS NULL;

-- The current incident of each firing. Events keep earlier associations after it moves or clears.
CREATE TABLE supervisor_incident_signals (
  fingerprint TEXT PRIMARY KEY NOT NULL REFERENCES supervisor_signals ON DELETE CASCADE,
  incident INTEGER NOT NULL REFERENCES supervisor_incidents ON DELETE CASCADE,
  level INTEGER NOT NULL,
  evidence_hash BLOB NOT NULL,
  value REAL NOT NULL,
  threshold REAL NOT NULL,
  event INTEGER NOT NULL REFERENCES supervisor_incident_events ON DELETE CASCADE
) WITHOUT ROWID;
CREATE INDEX supervisor_incident_signals_incident ON supervisor_incident_signals (incident, level DESC, event DESC);

CREATE TABLE supervisor_incident_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  incident INTEGER NOT NULL REFERENCES supervisor_incidents ON DELETE CASCADE,
  at INTEGER NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('signal', 'updated', 'cleared', 'all_clear', 'transferred', 'continued', 'acknowledged', 'snoozed')),
  actor TEXT NOT NULL,
  fingerprint TEXT,
  detector TEXT,
  metric TEXT,
  level INTEGER,
  value REAL,
  threshold REAL,
  related_id INTEGER
);
CREATE INDEX supervisor_incident_events_timeline ON supervisor_incident_events (incident, id DESC);
CREATE INDEX supervisor_incident_events_summary ON supervisor_incident_events (incident, kind)
  WHERE kind IN ('signal', 'continued');
CREATE INDEX supervisor_incident_events_measurement ON supervisor_incident_events (incident, id DESC)
  WHERE value IS NOT NULL;

-- Let the watchdog recheck any already-reported outage against the real heartbeat.
UPDATE project_supervisor SET reported_dead = 0 WHERE reported_dead = 1;
