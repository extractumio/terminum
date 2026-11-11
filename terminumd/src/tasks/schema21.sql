-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- Auto-Dispatch's switch and filters (terminum-dispatcher.md AC-170). On or off apart from the agent that picks, so
-- the switch keeps the choice; the severities it takes; the minutes a task stands unchanged before it is taken.
ALTER TABLE projects ADD COLUMN auto_on INTEGER NOT NULL DEFAULT 0;
ALTER TABLE projects ADD COLUMN auto_severities TEXT NOT NULL DEFAULT 'P0,P1';
ALTER TABLE projects ADD COLUMN auto_wait_minutes INTEGER NOT NULL DEFAULT 10;
UPDATE projects SET auto_on = 1, auto_severities = 'P0,P1,P2,P3', auto_wait_minutes = 0 WHERE auto_agent IS NOT NULL;
-- What its last round saw is no longer kept: a round's candidates say what it judged (auto_rounds).
ALTER TABLE projects DROP COLUMN auto_seen;
