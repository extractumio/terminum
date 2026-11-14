-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- The words of a task's line that a human closed or saw (terminum-kanban.md AC-117): kept here, not in a browser,
-- so every browser shows the same lines; other words (a new ask, another reason) bring the line back.
ALTER TABLE tasks ADD COLUMN line_seen TEXT;
