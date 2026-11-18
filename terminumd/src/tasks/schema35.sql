-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- A red Verify's failed steps run again on the target (terminum-integration.md AC-198): the run on the target keeps
-- the target's commit, the task's failed steps and the task's own attempt (its log); a project's open Bug task for
-- its red target holds the tasks queued behind it.
ALTER TABLE integration_verifies ADD COLUMN target_sha TEXT;
ALTER TABLE integration_verifies ADD COLUMN steps TEXT NOT NULL DEFAULT '';
ALTER TABLE integration_verifies ADD COLUMN red_attempt TEXT NOT NULL DEFAULT '';
ALTER TABLE project_integration ADD COLUMN red_bug TEXT;
