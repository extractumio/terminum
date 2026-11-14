-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- A project's integration settings (terminum-integration.md AC-186): written only by a signed-in page; a project
-- with no row has the defaults, off.
CREATE TABLE project_integration (
  project TEXT PRIMARY KEY REFERENCES projects ON DELETE CASCADE,
  is_on INTEGER NOT NULL DEFAULT 0,
  target TEXT NOT NULL DEFAULT 'origin/main',
  check_line TEXT NOT NULL DEFAULT '',
  publish_line TEXT NOT NULL DEFAULT '',
  after_line TEXT NOT NULL DEFAULT '',
  batch_most INTEGER NOT NULL DEFAULT 8,
  check_minutes INTEGER NOT NULL DEFAULT 90,
  agent TEXT REFERENCES agents ON DELETE SET NULL,
  agent_budget_minutes INTEGER NOT NULL DEFAULT 60,
  red_rounds_most INTEGER NOT NULL DEFAULT 3,
  prompt TEXT NOT NULL DEFAULT ''
);
