-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- Owner-wide critical line and per-incident delivery state (AC-227–229, AC-263).
ALTER TABLE supervisor_incident_signals ADD COLUMN urgent INTEGER NOT NULL DEFAULT 0 CHECK (urgent IN (0, 1));

UPDATE supervisor_incident_signals SET urgent = 1 WHERE fingerprint IN (
  SELECT fingerprint FROM supervisor_signals WHERE json_extract(scores, '$.reversibility') = 3
);

CREATE TABLE supervisor_line (
  id INTEGER PRIMARY KEY CHECK (id = 1),
  revision INTEGER NOT NULL,
  settings TEXT NOT NULL,
  checked_at INTEGER NOT NULL
);

CREATE TABLE supervisor_line_failures (
  channel INTEGER PRIMARY KEY,
  failure TEXT NOT NULL
);

CREATE TABLE supervisor_line_pending (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  incident INTEGER NOT NULL REFERENCES supervisor_incidents ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('opened', 'updated', 'all_clear', 'answer')),
  level INTEGER NOT NULL,
  signals INTEGER NOT NULL,
  urgent INTEGER NOT NULL
);
CREATE INDEX supervisor_line_pending_incident ON supervisor_line_pending (incident, id);

CREATE TABLE supervisor_line_repeats (
  incident INTEGER PRIMARY KEY REFERENCES supervisor_incidents ON DELETE CASCADE,
  escalated INTEGER NOT NULL CHECK (escalated IN (0, 1)),
  last_at INTEGER NOT NULL,
  due_at INTEGER NOT NULL
);
CREATE INDEX supervisor_line_repeats_due ON supervisor_line_repeats (due_at);
