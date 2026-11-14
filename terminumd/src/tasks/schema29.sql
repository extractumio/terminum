-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- A red batch's integration agent (terminum-integration.md AC-194): whether it is to start for the batch's red
-- round, the worktree of its own it works in, and the fixes it added: commits picked onto the target, built into
-- the candidate before the batch's tasks, each round keeping those it was built with.
ALTER TABLE integration_batches ADD COLUMN agent_wanted INTEGER NOT NULL DEFAULT 0;
ALTER TABLE integration_batches ADD COLUMN agent_tree TEXT;
ALTER TABLE integration_batches ADD COLUMN fixes TEXT NOT NULL DEFAULT '[]';
ALTER TABLE integration_rounds ADD COLUMN fixes TEXT NOT NULL DEFAULT '[]';
-- a press, a pool's decision and finish_task find a batch by its integration task
CREATE INDEX integration_batches_task ON integration_batches (task);
CREATE INDEX integration_batches_agent_tree ON integration_batches (id) WHERE agent_tree IS NOT NULL;
