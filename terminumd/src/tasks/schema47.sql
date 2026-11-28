-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- AC-231: Dispatcher marks its round and Talk panes before exposing their tools.
-- A mark outlives a disabled or deleted project so a surviving pane cannot inherit another role.
CREATE TABLE supervisor_panes (
  pane_key TEXT PRIMARY KEY NOT NULL,
  project TEXT NOT NULL,
  generation INTEGER NOT NULL
);
