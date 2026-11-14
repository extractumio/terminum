-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- A red batch's integration agent (terminum-integration.md AC-194): whether it is to start for the batch's red
-- round, and the worktree of its own it works in, at the candidate.
ALTER TABLE integration_batches ADD COLUMN agent_state TEXT;
ALTER TABLE integration_batches ADD COLUMN agent_tree TEXT;
