-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- AC-218: project and detector windows in which bounded hook tracking lost observations.
CREATE TABLE supervisor_hook_overflow (
    project TEXT NOT NULL REFERENCES project_supervisor ON DELETE CASCADE,
    reason TEXT NOT NULL,
    detector TEXT NOT NULL,
    first_at INTEGER NOT NULL,
    last_at INTEGER NOT NULL,
    count INTEGER NOT NULL,
    PRIMARY KEY (project, last_at, reason, detector, first_at)
) WITHOUT ROWID;

-- AC-255: an incident delivered in the watchdog's transaction proves timely detection.
UPDATE supervisor_miss_checks AS m SET signal = e.fingerprint, signal_state = 'raised',
    raised_at = e.at, level = e.level
FROM supervisor_incident_events e JOIN supervisor_incidents i ON i.id = e.incident
WHERE m.source = 'watchdog' AND m.raised_at IS NULL AND i.project = m.project
    AND e.at = m.observed_at AND e.detector = 'self.collector_dead'
    AND e.level = 3 AND e.kind IN ('signal','updated');

-- Previous red checks kept the selected fingerprint, but neither the failing attempt nor
-- the evidence as it stood then. An earlier subject match alone cannot verify coverage.
UPDATE supervisor_miss_checks AS m SET signal_state = 'unverified', raised_at = NULL,
    first_evidence_at = observed_at, level = NULL
WHERE m.source = 'target_red' AND m.signal IS NOT NULL;
