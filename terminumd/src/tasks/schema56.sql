-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- AC-255 to AC-259: independent observations, measured owner load, quarterly reports.
CREATE INDEX supervisor_signals_subject_seen ON supervisor_signals (project, subject_kind, subject_id, first_seen);

ALTER TABLE supervisor_signals ADD COLUMN episode_raised_at INTEGER;
UPDATE supervisor_signals SET episode_raised_at = last_seen WHERE state IN ('new','raised','held','recurred');

CREATE TRIGGER supervisor_signal_initial_raise AFTER INSERT ON supervisor_signals
WHEN NEW.state IN ('new','raised','held','recurred')
BEGIN
  UPDATE supervisor_signals SET episode_raised_at = NEW.first_seen WHERE fingerprint = NEW.fingerprint;
END;

CREATE TRIGGER supervisor_signal_episode AFTER UPDATE OF state ON supervisor_signals
WHEN OLD.state IS NOT NEW.state
BEGIN
  UPDATE supervisor_signals SET episode_raised_at = CASE
    WHEN NEW.state = 'resolved' THEN NULL
    WHEN NEW.state IN ('new','raised','held','recurred') THEN coalesce(episode_raised_at, NEW.last_seen)
    ELSE episode_raised_at END
  WHERE fingerprint = NEW.fingerprint;
END;

CREATE TABLE supervisor_miss_checks (
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  source TEXT NOT NULL,
  subject_kind TEXT NOT NULL,
  subject_id TEXT NOT NULL,
  occurrence TEXT NOT NULL,
  observed_at INTEGER NOT NULL,
  first_evidence_at INTEGER NOT NULL,
  signal TEXT,
  signal_state TEXT,
  raised_at INTEGER,
  level INTEGER,
  PRIMARY KEY (project, source, subject_kind, subject_id, occurrence)
) WITHOUT ROWID;
CREATE INDEX supervisor_miss_checks_time ON supervisor_miss_checks (project, observed_at);

CREATE TABLE supervisor_fix_observations (
  problem INTEGER NOT NULL REFERENCES supervisor_problems ON DELETE CASCADE,
  landed_at INTEGER NOT NULL,
  confirmed_at INTEGER NOT NULL,
  PRIMARY KEY (problem, landed_at)
) WITHOUT ROWID;

CREATE TABLE supervisor_page_dispatches (
  id INTEGER PRIMARY KEY,
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  at INTEGER NOT NULL
);
CREATE INDEX supervisor_page_dispatches_time ON supervisor_page_dispatches (project, at);

ALTER TABLE supervisor_verdicts ADD COLUMN active_seconds INTEGER;

ALTER TABLE supervisor_review_runs ADD COLUMN audit INTEGER NOT NULL DEFAULT 0 CHECK (audit IN (0,1));

CREATE TABLE supervisor_audits (
  project TEXT PRIMARY KEY REFERENCES projects ON DELETE CASCADE,
  schedule TEXT UNIQUE REFERENCES schedules,
  unavailable TEXT
);
CREATE TABLE supervisor_audit_schedules (
  schedule TEXT PRIMARY KEY REFERENCES schedules,
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE
);
CREATE TABLE supervisor_audit_reports (
  task TEXT PRIMARY KEY REFERENCES tasks ON DELETE CASCADE,
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  at INTEGER NOT NULL,
  report TEXT NOT NULL
);
CREATE INDEX supervisor_audit_reports_time ON supervisor_audit_reports (project, at);

CREATE VIEW supervisor_task_overrides AS
SELECT a.project, a.id AS action, a.fingerprint, e.id AS event, e.at
FROM events e JOIN tasks t ON t.id = e.task
JOIN supervisor_actions a ON a.id = (
  SELECT id FROM supervisor_actions WHERE project = t.project AND target = t.id AND at <= e.at
    AND kind IN ('fix','run','nudge') ORDER BY at DESC, id DESC LIMIT 1)
WHERE e.author = 'human'
  AND ((e.kind = 'archive' AND e.text LIKE 'Archived by %' AND a.kind = 'fix')
    OR (e.kind = 'move' AND json_extract(e.data,'$.from') IN ('IN PROGRESS','INTEGRATION')
        AND json_extract(e.data,'$.to') IN ('TODO','FAILED'))
    OR (e.kind = 'why' AND e.text = 'The human gave up on its run.' AND a.kind = 'run'));

CREATE VIEW supervisor_scheduled_reviews AS
SELECT project, schedule, 0 AS audit FROM supervisor_reviews
UNION ALL SELECT project, schedule, 1 FROM supervisor_audit_schedules;
