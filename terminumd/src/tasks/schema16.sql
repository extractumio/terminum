-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- A turn with no result is no longer a wait for a human (terminum-dispatcher.md AC-123): a run that waits so
-- waits as the agent's own question does, its session closed once quiet (AC-157).
UPDATE runs SET asked = 1 WHERE no_result_wait = 1;
ALTER TABLE runs DROP COLUMN no_result_wait;
