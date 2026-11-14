-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- A project's integration settings (terminum-integration.md AC-186): written only by a signed-in page; a project
-- with no row has the defaults, off (Settings::default in integration.rs: the one place they are).
CREATE TABLE project_integration (
  project TEXT PRIMARY KEY REFERENCES projects ON DELETE CASCADE,
  is_on INTEGER NOT NULL,
  target TEXT NOT NULL,
  check_line TEXT NOT NULL,
  publish_line TEXT NOT NULL,
  after_line TEXT NOT NULL,
  batch_most INTEGER NOT NULL,
  check_minutes INTEGER NOT NULL,
  agent TEXT REFERENCES agents ON DELETE SET NULL,
  agent_budget_minutes INTEGER NOT NULL,
  red_rounds_most INTEGER NOT NULL,
  prompt TEXT NOT NULL
);
