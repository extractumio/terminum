-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- A landing in seven stages (terminum-integration.md AC-198): the commands of Prepare, Verify and Cleanup beside
-- Check, Publish and After; a queued task's Verify, run before it may join a batch.
ALTER TABLE project_integration ADD COLUMN prepare_line TEXT NOT NULL DEFAULT '';
ALTER TABLE project_integration ADD COLUMN verify_line TEXT NOT NULL DEFAULT '';
ALTER TABLE project_integration ADD COLUMN cleanup_line TEXT NOT NULL DEFAULT '';
-- 'verifying' while its Prepare and Verify run or wait to; NULL ready for a batch
ALTER TABLE integration_entries ADD COLUMN stage TEXT;
-- a task's Prepare and Verify window, kept apart from its entry so taking the task out still ends it
CREATE TABLE integration_verifies (
  task TEXT PRIMARY KEY,
  project TEXT NOT NULL,
  commit_sha TEXT NOT NULL,
  pane TEXT,
  tree TEXT,
  started INTEGER
);
