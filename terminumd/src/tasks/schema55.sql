-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- AC-253, AC-254 and AC-257: decisions and reversible detector settings.
CREATE TABLE supervisor_config_changes (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  detector TEXT NOT NULL,
  class TEXT NOT NULL CHECK (class IN ('tune','extend','owner','engine')),
  author TEXT NOT NULL,
  at INTEGER NOT NULL,
  generation INTEGER NOT NULL,
  revision INTEGER NOT NULL,
  proposal TEXT NOT NULL,
  replay TEXT NOT NULL,
  previous TEXT NOT NULL,
  next TEXT NOT NULL,
  applied_at INTEGER,
  applied_revision INTEGER,
  reverted_at INTEGER,
  decision TEXT,
  task TEXT REFERENCES tasks ON DELETE SET NULL,
  question TEXT UNIQUE REFERENCES events ON DELETE SET NULL
);
CREATE INDEX supervisor_config_project ON supervisor_config_changes (project, id DESC);
CREATE INDEX supervisor_config_detector ON supervisor_config_changes (project, detector, applied_revision DESC) WHERE applied_at IS NOT NULL;

CREATE INDEX supervisor_config_author ON supervisor_config_changes (project, author);

CREATE TABLE supervisor_detector_lifecycle (
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  detector TEXT NOT NULL,
  live_since INTEGER NOT NULL,
  weekly_at INTEGER,
  removal_at INTEGER,
  PRIMARY KEY (project, detector)
) WITHOUT ROWID;

CREATE INDEX supervisor_signals_detector ON supervisor_signals (project, detector, last_seen);
