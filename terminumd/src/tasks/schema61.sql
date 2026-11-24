-- The incident whose alert stands for a critical report (terminum-reports.md AC-276): its own line then stays down.
ALTER TABLE reports ADD COLUMN alerted_by INTEGER REFERENCES supervisor_incidents(id) ON DELETE SET NULL;
CREATE INDEX reports_alerted ON reports (alerted_by) WHERE alerted_by IS NOT NULL;
