-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- Supervisor runtime (AC-216 to AC-219). Source changes carry references, not content.
CREATE TABLE project_supervisor (
  project TEXT PRIMARY KEY NOT NULL REFERENCES projects ON DELETE CASCADE,
  enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
  generation INTEGER NOT NULL,
  enabled_at INTEGER NOT NULL,
  cursor INTEGER NOT NULL DEFAULT 0,
  last_ok INTEGER,
  last_evaluation INTEGER,
  last_sample INTEGER,
  last_baseline INTEGER,
  reported_dead INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE supervisor_changes (
  seq INTEGER PRIMARY KEY AUTOINCREMENT,
  project TEXT NOT NULL REFERENCES project_supervisor ON DELETE CASCADE,
  source TEXT NOT NULL,
  source_id TEXT NOT NULL
);
CREATE INDEX supervisor_changes_project ON supervisor_changes (project, seq);

CREATE TRIGGER supervisor_event AFTER INSERT ON events BEGIN
  INSERT INTO supervisor_changes (project, source, source_id)
    SELECT s.project, 'events', NEW.id FROM tasks t JOIN project_supervisor s ON s.project = t.project
    WHERE t.id = NEW.task AND s.enabled = 1;
END;

CREATE TRIGGER supervisor_event_changed AFTER UPDATE OF kind, text, data ON events
WHEN OLD.kind IS NOT NEW.kind OR OLD.text IS NOT NEW.text OR OLD.data IS NOT NEW.data BEGIN
  INSERT INTO supervisor_changes (project, source, source_id)
    SELECT s.project, 'events', NEW.id FROM tasks t JOIN project_supervisor s ON s.project = t.project
    WHERE t.id = NEW.task AND s.enabled = 1;
END;

CREATE TRIGGER supervisor_run_added AFTER INSERT ON runs BEGIN
  INSERT INTO supervisor_changes (project, source, source_id)
    SELECT project, 'runs', NEW.id FROM project_supervisor WHERE project = NEW.project AND enabled = 1;
END;
CREATE TRIGGER supervisor_run_changed AFTER UPDATE OF state, project, pane_key ON runs
WHEN OLD.state IS NOT NEW.state OR OLD.project IS NOT NEW.project OR OLD.pane_key IS NOT NEW.pane_key BEGIN
  INSERT INTO supervisor_changes (project, source, source_id)
    SELECT project, 'runs', NEW.id FROM project_supervisor WHERE project IN (OLD.project, NEW.project) AND enabled = 1;
END;
CREATE TRIGGER supervisor_fire_added AFTER INSERT ON schedule_fires BEGIN
  INSERT INTO supervisor_changes (project, source, source_id)
    SELECT s.project, 'schedule_fires', NEW.id FROM schedules f JOIN project_supervisor s ON s.project = f.project
    WHERE f.id = NEW.schedule AND s.enabled = 1;
END;
CREATE TRIGGER supervisor_fire_changed AFTER UPDATE OF state, missed ON schedule_fires
WHEN OLD.state IS NOT NEW.state OR OLD.missed IS NOT NEW.missed BEGIN
  INSERT INTO supervisor_changes (project, source, source_id)
    SELECT s.project, 'schedule_fires', NEW.id FROM schedules f JOIN project_supervisor s ON s.project = f.project
    WHERE f.id = NEW.schedule AND s.enabled = 1;
END;

CREATE TRIGGER supervisor_integration_return AFTER INSERT ON integration_returns BEGIN
  INSERT INTO supervisor_changes (project, source, source_id)
    SELECT s.project, 'integration_returns', NEW.task FROM tasks t JOIN project_supervisor s ON s.project = t.project
    WHERE t.id = NEW.task AND s.enabled = 1;
END;
CREATE TRIGGER supervisor_integration_return_changed AFTER UPDATE OF at ON integration_returns BEGIN
  INSERT INTO supervisor_changes (project, source, source_id)
    SELECT s.project, 'integration_returns', NEW.task FROM tasks t JOIN project_supervisor s ON s.project = t.project
    WHERE t.id = NEW.task AND s.enabled = 1;
END;
CREATE TRIGGER supervisor_integration_round_added AFTER INSERT ON integration_rounds BEGIN
  INSERT INTO supervisor_changes (project, source, source_id)
    SELECT s.project, 'integration_rounds', NEW.batch || ':' || NEW.round
    FROM integration_batches b JOIN project_supervisor s ON s.project = b.project
    WHERE b.id = NEW.batch AND s.enabled = 1;
END;
CREATE TRIGGER supervisor_integration_round_changed AFTER UPDATE OF result, ended ON integration_rounds
WHEN OLD.result IS NOT NEW.result OR OLD.ended IS NOT NEW.ended BEGIN
  INSERT INTO supervisor_changes (project, source, source_id)
    SELECT s.project, 'integration_rounds', NEW.batch || ':' || NEW.round
    FROM integration_batches b JOIN project_supervisor s ON s.project = b.project
    WHERE b.id = NEW.batch AND s.enabled = 1;
END;
CREATE TRIGGER supervisor_integration_batch_changed AFTER UPDATE OF state, ended_at ON integration_batches
WHEN OLD.state IS NOT NEW.state OR OLD.ended_at IS NOT NEW.ended_at BEGIN
  INSERT INTO supervisor_changes (project, source, source_id)
    SELECT project, 'integration_batches', NEW.id FROM project_supervisor WHERE project = NEW.project AND enabled = 1;
END;

CREATE TABLE supervisor_points (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project TEXT NOT NULL REFERENCES project_supervisor ON DELETE CASCADE,
  metric TEXT NOT NULL,
  subject TEXT NOT NULL,
  at INTEGER NOT NULL,
  value REAL NOT NULL,
  resets_at INTEGER,
  UNIQUE (project, metric, subject, at)
);
CREATE INDEX supervisor_points_time ON supervisor_points (project, at);

CREATE TABLE supervisor_hook_counts (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  project TEXT NOT NULL REFERENCES project_supervisor ON DELETE CASCADE,
  agent TEXT NOT NULL,
  session TEXT NOT NULL,
  at INTEGER NOT NULL,
  event TEXT NOT NULL,
  count INTEGER NOT NULL,
  UNIQUE (project, agent, session, at, event)
);
CREATE INDEX supervisor_hook_counts_time ON supervisor_hook_counts (project, at);

CREATE TRIGGER supervisor_task_moved AFTER UPDATE OF project ON tasks
WHEN OLD.project IS NOT NEW.project BEGIN
  INSERT INTO supervisor_changes (project, source, source_id)
    SELECT project, 'tasks', NEW.id FROM project_supervisor WHERE project IN (OLD.project, NEW.project) AND enabled = 1;
END;
