-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- AC-245: retained hold decisions and their inverse actions.
CREATE TABLE supervisor_holds (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  fingerprint TEXT NOT NULL,
  detector TEXT NOT NULL,
  response TEXT NOT NULL,
  level INTEGER NOT NULL CHECK (level BETWEEN 2 AND 3),
  kind TEXT NOT NULL CHECK (kind IN ('stop_run', 'pause_dispatch', 'pause_integration', 'take_out')),
  target TEXT NOT NULL DEFAULT '',
  reason TEXT NOT NULL,
  actor TEXT NOT NULL,
  applied_at INTEGER NOT NULL,
  shadow INTEGER NOT NULL CHECK (shadow IN (0, 1)),
  incident INTEGER REFERENCES supervisor_incidents ON DELETE SET NULL,
  undo_data TEXT,
  pending_run TEXT,
  pending_by TEXT,
  released_by TEXT,
  released_at INTEGER
);
CREATE UNIQUE INDEX supervisor_holds_active ON supervisor_holds (project, kind, target)
  WHERE shadow = 0 AND released_at IS NULL;
CREATE INDEX supervisor_holds_firing ON supervisor_holds (fingerprint, kind, target, applied_at);
CREATE INDEX supervisor_holds_incident ON supervisor_holds (incident, id)
  WHERE shadow = 0 AND released_at IS NULL;
CREATE INDEX supervisor_holds_pending ON supervisor_holds (pending_run) WHERE pending_run IS NOT NULL;
CREATE INDEX supervisor_holds_project ON supervisor_holds (project, id DESC);

-- Extend the timeline's event names without rebuilding the table it references.
PRAGMA writable_schema = ON;
UPDATE sqlite_schema SET sql = replace(sql, '''acknowledged'', ''snoozed''',
  '''acknowledged'', ''snoozed'', ''hold_applied'', ''hold_kept'', ''hold_released''')
  WHERE type = 'table' AND name = 'supervisor_incident_events';
PRAGMA writable_schema = OFF;
