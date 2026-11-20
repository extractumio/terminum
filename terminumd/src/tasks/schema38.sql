-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- A run that waits for other tasks (terminum-dispatcher.md AC-210): the store keys, as a JSON array, of the tasks that
-- must be DONE before its kept press goes on; NULL for a wait for room alone (AC-158).
ALTER TABLE run_waits ADD COLUMN after TEXT;
