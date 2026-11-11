-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- Roles of terminum's callers (terminum-agents.md AC-174, AC-181). A token: what a program outside a pane calls
-- `terminum tool` with, kept as a hash alone; its role and its one project.
CREATE TABLE tokens (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL UNIQUE,
  hash TEXT NOT NULL UNIQUE,
  role TEXT NOT NULL CHECK (role IN ('manager', 'worker')),
  project TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  expires_at INTEGER,
  last_used_at INTEGER
);
