-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- AC-233: a verification round's interim hold ends with its round (`until` is the round's deadline) unless code
-- confirms its signal; the round's case file keeps the comparison that justified it.
ALTER TABLE supervisor_holds ADD COLUMN round TEXT;
ALTER TABLE supervisor_holds ADD COLUMN until INTEGER;
ALTER TABLE supervisor_holds ADD COLUMN end_failed TEXT;
CREATE INDEX supervisor_holds_interim ON supervisor_holds (round) WHERE until IS NOT NULL;
-- AC-213: a checked round's critical escalation, which stands while its signal keeps the evidence it was judged on.
CREATE TABLE supervisor_escalations (
    fingerprint TEXT PRIMARY KEY,
    project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
    round TEXT NOT NULL,
    score TEXT NOT NULL CHECK (score IN ('blast', 'reversibility')),
    evidence TEXT NOT NULL,
    reason TEXT NOT NULL,
    at INTEGER NOT NULL
);
-- The timeline's new events: a round's escalation, and how an interim hold ended when it did not end released.
PRAGMA writable_schema = ON;
UPDATE sqlite_schema SET sql = replace(sql, '''alert_wrong''', '''alert_wrong'', ''escalated'', ''hold_confirmed'', ''hold_end_failed''')
  WHERE type = 'table' AND name = 'supervisor_incident_events';
PRAGMA writable_schema = OFF;
