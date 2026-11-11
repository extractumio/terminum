-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- What the human did in a page (terminum.md AC-175), a row per clock minute, kind, place and control: how many
-- times, never what. `at` and `ended`: the first and the last of them, unix ms. `amount`: pixels scrolled or
-- dragged, seconds in front. `project`: the pane's ('' in none); NULL where the page shows no pane.
CREATE TABLE user_interaction (
  id INTEGER PRIMARY KEY,
  at INTEGER NOT NULL,
  ended INTEGER NOT NULL,
  kind TEXT NOT NULL,
  surface TEXT NOT NULL,
  target TEXT NOT NULL DEFAULT '',
  n INTEGER NOT NULL,
  amount INTEGER NOT NULL DEFAULT 0,
  pane TEXT,
  project TEXT,
  device TEXT NOT NULL
);
CREATE INDEX user_interaction_at ON user_interaction (at);
-- Whether Auto-Dispatcher chose a task (the Dashboard's Tasks tab, AC-176).
CREATE INDEX auto_starts_task ON auto_starts (task, at);
