-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- A task that left the integration queue while held behind its project's red target (terminum-integration.md AC-198):
-- told to integrate again once that Bug task ends; the row goes when it is told, or when the task is queued again.
CREATE TABLE integration_held_left (
  task TEXT PRIMARY KEY REFERENCES tasks ON DELETE CASCADE,
  bug TEXT NOT NULL REFERENCES tasks ON DELETE CASCADE
);
