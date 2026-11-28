-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- AC-220: saving an off project's pack does not create collector state or enable it.
CREATE TABLE supervisor_settings (
  project TEXT PRIMARY KEY REFERENCES projects ON DELETE CASCADE,
  revision INTEGER NOT NULL,
  pack TEXT NOT NULL
);
CREATE TABLE supervisor_probe_commands (
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  probe TEXT NOT NULL,
  command TEXT NOT NULL,
  PRIMARY KEY (project, probe)
) WITHOUT ROWID;
CREATE TABLE supervisor_probe_candidates (
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  probe TEXT NOT NULL,
  command TEXT NOT NULL,
  approval TEXT,
  PRIMARY KEY (project, probe)
) WITHOUT ROWID;

-- AC-221: attach state belongs to the collector's generation and settings snapshot.
ALTER TABLE project_supervisor ADD COLUMN attachment TEXT;
ALTER TABLE project_supervisor ADD COLUMN settings_revision INTEGER;
CREATE TABLE supervisor_attach_seeds (
  fingerprint TEXT PRIMARY KEY REFERENCES supervisor_signals ON DELETE CASCADE,
  seeded_at INTEGER NOT NULL
) WITHOUT ROWID;
CREATE TABLE supervisor_hold_verdicts (
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  detector TEXT NOT NULL,
  firing TEXT NOT NULL,
  fired_at INTEGER NOT NULL,
  was_right INTEGER NOT NULL CHECK (was_right IN (0, 1)),
  PRIMARY KEY (project, detector, firing)
) WITHOUT ROWID;
