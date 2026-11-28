-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- AC-233–235: retained investigations, their admission inputs and checked results.
ALTER TABLE supervisor_signals ADD COLUMN revision INTEGER NOT NULL DEFAULT 1;
CREATE TRIGGER supervisor_signal_revision AFTER UPDATE OF evidence, scores, state ON supervisor_signals
WHEN new.evidence <> old.evidence
  OR (new.state IN ('new', 'raised', 'recurred') AND new.state <> old.state)
  OR max(json_extract(new.scores, '$.impact'), json_extract(new.scores, '$.blast'), json_extract(new.scores, '$.reversibility'))
   > max(json_extract(old.scores, '$.impact'), json_extract(old.scores, '$.blast'), json_extract(old.scores, '$.reversibility'))
BEGIN
  UPDATE supervisor_signals SET revision = revision + 1 WHERE fingerprint = new.fingerprint;
END;

CREATE TABLE supervisor_signal_weeks (
  fingerprint TEXT NOT NULL REFERENCES supervisor_signals ON DELETE CASCADE,
  week INTEGER NOT NULL,
  PRIMARY KEY (fingerprint, week)
) WITHOUT ROWID;

-- First and last seen are the only weeks recoverable from older signals.
INSERT OR IGNORE INTO supervisor_signal_weeks
  SELECT fingerprint, first_seen / 604800 FROM supervisor_signals WHERE state <> 'shadow'
  UNION SELECT fingerprint, last_seen / 604800 FROM supervisor_signals WHERE state <> 'shadow';

CREATE TABLE supervisor_round_requests (
  fingerprint TEXT PRIMARY KEY NOT NULL REFERENCES supervisor_signals ON DELETE CASCADE,
  owner_requested INTEGER NOT NULL DEFAULT 0,
  reason TEXT
);

CREATE TABLE supervisor_rounds (
  id TEXT PRIMARY KEY NOT NULL,
  run TEXT NOT NULL UNIQUE REFERENCES runs,
  project TEXT NOT NULL,
  generation INTEGER NOT NULL,
  fingerprint TEXT NOT NULL,
  signal_revision INTEGER NOT NULL,
  case_file TEXT NOT NULL,
  deadline INTEGER NOT NULL,
  finding TEXT,
  found_at INTEGER,
  refused INTEGER NOT NULL DEFAULT 0,
  failure TEXT,
  ended_at INTEGER
);
CREATE UNIQUE INDEX supervisor_rounds_open ON supervisor_rounds (fingerprint) WHERE ended_at IS NULL;
CREATE INDEX supervisor_rounds_latest ON supervisor_rounds (fingerprint, signal_revision DESC);
CREATE INDEX supervisor_rounds_active ON supervisor_rounds (run) WHERE ended_at IS NULL;
ALTER TABLE supervisor_panes ADD COLUMN round TEXT REFERENCES supervisor_rounds;
