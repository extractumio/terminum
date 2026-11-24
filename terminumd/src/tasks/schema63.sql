-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- terminum-reports.md AC-265: a report's type, shown beside it in the Inbox, set by terminumd alone (reports/sources.rs
-- kind_of). Reports kept before it get theirs from what made them: an incident's, the Supervisor's digest, and a
-- result terminumd made a report of, whose summary was its Outcome's word; that summary becomes its Summary's line.
ALTER TABLE reports ADD COLUMN kind TEXT NOT NULL DEFAULT 'report' CHECK (kind IN ('report', 'ok', 'attention', 'failed', 'incident', 'digest'));
UPDATE reports SET kind = 'incident' WHERE incident IS NOT NULL;
UPDATE reports SET kind = 'digest' WHERE incident IS NULL AND task IS NULL AND role = 'terminum' AND title LIKE 'Supervisor digest, %';
UPDATE reports SET kind = lower(summary) WHERE incident IS NULL AND role = 'terminum' AND lower(summary) IN ('ok', 'attention', 'failed');
UPDATE reports SET summary = coalesce(nullif(substr(trim(substr(body, instr(body, 'Summary:') + 8,
      CASE WHEN instr(substr(body, instr(body, 'Summary:') + 8), char(10)) > 0 THEN instr(substr(body, instr(body, 'Summary:') + 8), char(10)) - 1 ELSE 500 END),
    ' *_' || char(9)), 1, 500), ''), summary)
  WHERE kind IN ('ok', 'attention', 'failed') AND lower(summary) = kind AND instr(body, 'Summary:') > 0;
