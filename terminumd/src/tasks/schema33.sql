-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- Each Prepare and Verify run keeps its own log (terminum-integration.md AC-198), including one already running
-- at upgrade: completing it must leave an earlier verify-<number>.log intact.
ALTER TABLE integration_verifies ADD COLUMN attempt TEXT NOT NULL DEFAULT '';
UPDATE integration_verifies SET attempt = lower(hex(randomblob(16)));
