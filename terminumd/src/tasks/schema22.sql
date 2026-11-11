-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- Auto-Dispatch's cap (terminum-dispatcher.md AC-170): the most tasks it starts in a project in 24 hours.
ALTER TABLE projects ADD COLUMN auto_day_most INTEGER NOT NULL DEFAULT 10;
