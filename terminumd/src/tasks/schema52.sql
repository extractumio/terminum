-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- AC-246–247: retained decisions, append-only judgments and rule breaker generations.
CREATE TABLE supervisor_actions (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  detector TEXT NOT NULL,
  response TEXT NOT NULL,
  kind TEXT NOT NULL CHECK (kind IN ('hold','fix','run','alert','cause','card','nudge')),
  source TEXT NOT NULL,
  fingerprint TEXT NOT NULL,
  target TEXT NOT NULL,
  at INTEGER NOT NULL,
  baseline TEXT NOT NULL DEFAULT '',
  next_days INTEGER DEFAULT 1 CHECK (next_days IN (1,7,28)),
  due_at INTEGER GENERATED ALWAYS AS (at + next_days * 86400) VIRTUAL,
  UNIQUE (project, kind, source)
);
CREATE INDEX supervisor_actions_due ON supervisor_actions (project, next_days, due_at, id) WHERE next_days IS NOT NULL;
CREATE INDEX supervisor_actions_rule ON supervisor_actions (project, detector, response, id);

CREATE TABLE supervisor_outcomes (
  action INTEGER NOT NULL REFERENCES supervisor_actions ON DELETE CASCADE,
  days INTEGER NOT NULL CHECK (days IN (0,1,7,28)),
  at INTEGER NOT NULL,
  verdict TEXT NOT NULL CHECK (verdict IN ('right','wrong','undecided')),
  evidence TEXT NOT NULL,
  PRIMARY KEY (action, days)
) WITHOUT ROWID;

CREATE TABLE supervisor_breakers (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  detector TEXT NOT NULL,
  response TEXT NOT NULL,
  opened_at INTEGER NOT NULL,
  opened_by INTEGER NOT NULL REFERENCES supervisor_actions,
  config TEXT NOT NULL,
  closed_at INTEGER,
  reset_after_action INTEGER
);
CREATE UNIQUE INDEX supervisor_breakers_open ON supervisor_breakers (project, detector, response)
  WHERE closed_at IS NULL;
CREATE INDEX supervisor_breakers_rule ON supervisor_breakers (project, detector, response, id);

CREATE TABLE supervisor_breaker_cards (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  breaker INTEGER NOT NULL REFERENCES supervisor_breakers ON DELETE CASCADE,
  kind TEXT NOT NULL,
  fingerprint TEXT NOT NULL,
  target TEXT NOT NULL,
  task TEXT NOT NULL REFERENCES tasks ON DELETE CASCADE,
  UNIQUE (breaker, kind, fingerprint, target)
);

INSERT INTO supervisor_actions (project, detector, response, kind, source, fingerprint, target, at)
SELECT project, detector, response, 'hold', CAST(id AS TEXT), fingerprint, target, applied_at
FROM supervisor_holds;

INSERT INTO supervisor_actions (project, detector, response, kind, source, fingerprint, target, at)
SELECT s.project, s.detector,
  CASE max(json_extract(s.scores, '$.impact'), json_extract(s.scores, '$.blast'), json_extract(s.scores, '$.reversibility'))
    WHEN 3 THEN 'R5' WHEN 2 THEN 'R4' WHEN 1 THEN 'R2' ELSE 'R0' END,
  'nudge', n.event, n.fingerprint, n.task, e.at
FROM supervisor_nudges n JOIN supervisor_signals s USING (fingerprint) JOIN events e ON e.id = n.event;

PRAGMA writable_schema = ON;
UPDATE sqlite_schema SET sql = replace(sql, '''hold_released''', '''hold_released'', ''alert_wrong''')
  WHERE type = 'table' AND name = 'supervisor_incident_events';
PRAGMA writable_schema = OFF;

CREATE TABLE supervisor_breaker_reviews (
  breaker INTEGER PRIMARY KEY REFERENCES supervisor_breakers ON DELETE CASCADE,
  revision INTEGER NOT NULL,
  review TEXT NOT NULL
) WITHOUT ROWID;

CREATE INDEX supervisor_actions_incident ON supervisor_actions (project, target, fingerprint, id)
  WHERE kind = 'alert';

INSERT INTO supervisor_actions (project, detector, response, kind, source, fingerprint, target, at)
SELECT i.project, e.detector, CASE e.level WHEN 3 THEN 'R5' ELSE 'R4' END, 'alert',
  CAST(e.id AS TEXT), e.fingerprint, CAST(i.id AS TEXT), e.at
FROM supervisor_incident_events e JOIN supervisor_incidents i ON i.id = e.incident
WHERE e.kind IN ('signal', 'updated') AND e.level >= 2 AND e.fingerprint IS NOT NULL AND e.detector IS NOT NULL;

ALTER TABLE supervisor_holds ADD COLUMN replacement_run TEXT;
ALTER TABLE supervisor_line_pending ADD COLUMN alerts TEXT;

UPDATE supervisor_line_pending AS q SET alerts = (
  SELECT json_group_array(a.id) FROM supervisor_actions a
  JOIN supervisor_incident_events e ON e.id = a.source
  WHERE a.kind = 'alert' AND e.incident = q.incident
    AND e.id = (SELECT max(v.id) FROM supervisor_incident_events v WHERE v.incident = e.incident
      AND v.fingerprint = e.fingerprint AND v.kind IN ('signal','updated')));

-- Older builds retained the fix and its signal, but did not record the historical response.
-- Backfill only when the signal's retained firing predates creation and has not changed.
WITH known AS (
  SELECT t.project, t.id, t.created_at, p.source_round, s.fingerprint, s.detector,
    CASE max(json_extract(s.scores, '$.impact'), json_extract(s.scores, '$.blast'), json_extract(s.scores, '$.reversibility'))
      WHEN 3 THEN 'R5' WHEN 2 THEN 'R4' WHEN 1 THEN 'R2' ELSE 'R0' END AS response,
    row_number() OVER (PARTITION BY t.id ORDER BY s.first_seen, s.fingerprint) AS position
  FROM supervisor_problem_tasks l JOIN tasks t ON t.id = l.task
  JOIN supervisor_problems p ON p.id = l.problem AND p.project = t.project
  JOIN supervisor_problem_signals y ON y.problem = p.id
  JOIN supervisor_signals s ON s.fingerprint = y.fingerprint AND s.project = p.project
  WHERE s.first_seen = s.last_seen AND s.last_seen <= t.created_at
    AND EXISTS (SELECT 1 FROM events e WHERE e.task = t.id AND e.kind = 'made' AND e.author = 'supervisor:' || t.project)
)
INSERT INTO supervisor_actions (project, detector, response, kind, source, fingerprint, target, at, baseline)
SELECT project, detector, response, 'fix', id, fingerprint, id, created_at, coalesce(source_round, '')
FROM known WHERE position = 1;

CREATE VIEW supervisor_blocked_fixes AS
SELECT a.project, a.target AS task FROM supervisor_breakers b
JOIN project_supervisor s ON s.project = b.project AND s.enabled
JOIN supervisor_actions a ON a.project = b.project AND a.detector = b.detector AND a.response = b.response
WHERE b.closed_at IS NULL AND a.kind = 'fix';
