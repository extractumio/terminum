-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- terminum-reports.md AC-265: a report's type, shown beside it in the Inbox, set by terminumd alone (reports/sources.rs
-- kind_of). Reports kept before it get theirs from what made them: an incident's, the Supervisor's digest, and a
-- result terminumd made a report of, whose summary was its Outcome's word (the page leaves out a summary that only
-- repeats its type).
ALTER TABLE reports ADD COLUMN kind TEXT NOT NULL DEFAULT 'report' CHECK (kind IN ('report', 'ok', 'attention', 'failed', 'incident', 'digest'));
UPDATE reports SET kind = 'incident' WHERE incident IS NOT NULL;
UPDATE reports SET kind = 'digest' WHERE incident IS NULL AND task IS NULL AND role = 'terminum' AND title LIKE 'Supervisor digest, %';
UPDATE reports SET kind = lower(summary) WHERE incident IS NULL AND role = 'terminum' AND lower(summary) IN ('ok', 'attention', 'failed');
