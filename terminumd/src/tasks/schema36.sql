-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- A run after its tmux server's end (terminum-dispatcher.md AC-205): the identity (`@terminum-id`) of the server a walk
-- last saw its pane in, so a pane gone under a server started anew is told from a window closed by hand.
ALTER TABLE runs ADD COLUMN server_id TEXT;
