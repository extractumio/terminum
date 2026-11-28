-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- AC-260: keep vetted probe observations and explicit availability for replay.
CREATE TABLE supervisor_probe_samples (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project TEXT NOT NULL REFERENCES project_supervisor ON DELETE CASCADE,
    source TEXT NOT NULL,
    config TEXT NOT NULL,
    at INTEGER NOT NULL,
    state TEXT NOT NULL,
    data TEXT CHECK(data IS NULL OR json_valid(data)),
    UNIQUE (project, source, at)
);
CREATE INDEX supervisor_probe_latest ON supervisor_probe_samples (project, source, config, at);
CREATE INDEX supervisor_probe_time ON supervisor_probe_samples (project, at);

-- AC-261: own Verify outcomes are distinct from Prepare, batch and target outcomes.
CREATE TABLE integration_verify_results (
    attempt TEXT PRIMARY KEY,
    project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
    task TEXT NOT NULL REFERENCES tasks ON DELETE CASCADE,
    commit_sha TEXT NOT NULL,
    passed INTEGER NOT NULL CHECK(passed IN (0, 1)),
    agent TEXT,
    at INTEGER NOT NULL
);
CREATE INDEX integration_verify_results_time ON integration_verify_results (project, at);
CREATE TRIGGER supervisor_verify_result AFTER INSERT ON integration_verify_results BEGIN
    INSERT INTO supervisor_changes (project, source, source_id)
    SELECT project, 'integration_verify_results', NEW.attempt FROM project_supervisor WHERE project = NEW.project AND enabled = 1;
END;

CREATE TRIGGER supervisor_integration_enabled AFTER UPDATE OF is_on ON project_integration
WHEN OLD.is_on IS NOT NEW.is_on BEGIN
    INSERT INTO supervisor_changes (project, source, source_id)
    SELECT project, 'project_integration', NEW.project FROM project_supervisor WHERE project = NEW.project AND enabled = 1;
END;

CREATE TRIGGER supervisor_integration_first_enabled AFTER INSERT ON project_integration
WHEN NEW.is_on = 1 BEGIN
    INSERT INTO supervisor_changes (project, source, source_id)
    SELECT project, 'project_integration', NEW.project FROM project_supervisor WHERE project = NEW.project AND enabled = 1;
END;
