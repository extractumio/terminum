-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- Reports and the Inbox (terminum-reports.md AC-265): what agents, routines and the Supervisor give a person to
-- read, apart from tasks; their comments, and their files as tasks' files that name a report.
CREATE TABLE reports (
  id TEXT PRIMARY KEY,
  project TEXT NOT NULL REFERENCES projects,
  number INTEGER NOT NULL,
  slug TEXT NOT NULL UNIQUE,
  title TEXT NOT NULL,
  summary TEXT NOT NULL,
  critical INTEGER NOT NULL DEFAULT 0,
  author TEXT NOT NULL REFERENCES users,
  role TEXT NOT NULL,
  session_key TEXT,
  task TEXT REFERENCES tasks,
  schedule TEXT REFERENCES schedules,
  run TEXT,
  incident INTEGER,
  follows TEXT REFERENCES reports,
  read_at INTEGER,
  line INTEGER NOT NULL DEFAULT 0,
  line_seen INTEGER,
  deleted_at INTEGER,
  replied_at INTEGER,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  -- last: a scan of the columns above never walks a long body's pages
  body TEXT NOT NULL
);
CREATE INDEX reports_inbox ON reports (deleted_at, project, read_at);
-- what every page's marks read after a change: the unread counts and the critical lines standing
CREATE INDEX reports_unread ON reports (project) WHERE deleted_at IS NULL AND read_at IS NULL;
CREATE INDEX reports_lines ON reports (created_at) WHERE line = 1 AND read_at IS NULL AND line_seen IS NULL AND deleted_at IS NULL;
CREATE INDEX reports_task ON reports (task);
CREATE INDEX reports_schedule ON reports (schedule);
CREATE INDEX reports_session ON reports (session_key, created_at);
CREATE INDEX reports_follows ON reports (follows);
CREATE TABLE report_notes (
  id TEXT PRIMARY KEY,
  report TEXT NOT NULL REFERENCES reports ON DELETE CASCADE,
  at INTEGER NOT NULL,
  author TEXT NOT NULL REFERENCES users,
  kind TEXT NOT NULL CHECK (kind IN ('comment', 'reply', 'change')),
  text TEXT NOT NULL
);
CREATE INDEX report_notes_report ON report_notes (report, at);
ALTER TABLE files ADD COLUMN report TEXT REFERENCES reports;
CREATE INDEX files_report ON files (report);
