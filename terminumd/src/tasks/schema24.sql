-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- Who started a run and how (terminum-dispatcher.md AC-183): the user who asked for it (a human, or Dispatcher for a
-- schedule's firing and Auto-Dispatcher's choice) and the way's word. A run older than this has neither.
ALTER TABLE runs ADD COLUMN started_by TEXT REFERENCES users;
ALTER TABLE runs ADD COLUMN started_how TEXT;
