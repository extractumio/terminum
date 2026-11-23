-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- AC-251 and AC-252: one review lifecycle per task design, with attributed verdicts.
CREATE TABLE supervisor_design_gates (
  task TEXT PRIMARY KEY REFERENCES tasks ON DELETE CASCADE,
  project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
  tier INTEGER NOT NULL CHECK(tier IN (2, 3)),
  evidence TEXT NOT NULL,
  scope TEXT NOT NULL,
  existing_review INTEGER NOT NULL CHECK(existing_review IN (0, 1)),
  opened INTEGER NOT NULL,
  deadline INTEGER NOT NULL,
  brief TEXT,
  owner_question TEXT REFERENCES events ON DELETE SET NULL,
  blocker TEXT REFERENCES tasks ON DELETE SET NULL,
  status TEXT NOT NULL DEFAULT 'waiting' CHECK(status IN ('waiting', 'escalated', 'closed')),
  architecture_round TEXT UNIQUE REFERENCES supervisor_rounds ON DELETE SET NULL,
  independent_round TEXT UNIQUE REFERENCES supervisor_rounds ON DELETE SET NULL
);
CREATE INDEX supervisor_design_open ON supervisor_design_gates (project, status, tier DESC, opened) WHERE status <> 'closed';

CREATE TABLE supervisor_design_records (
  task TEXT NOT NULL REFERENCES tasks ON DELETE CASCADE,
  role TEXT NOT NULL CHECK(role IN ('architecture', 'independent')),
  author TEXT NOT NULL REFERENCES users,
  tier INTEGER NOT NULL CHECK(tier IN (2, 3)),
  scope TEXT NOT NULL,
  verdict TEXT NOT NULL CHECK(verdict IN ('approved', 'changes_requested')),
  record TEXT NOT NULL,
  at INTEGER NOT NULL,
  PRIMARY KEY (task, role)
) WITHOUT ROWID;

CREATE TABLE supervisor_design_verify_misses (
  task TEXT NOT NULL REFERENCES tasks ON DELETE CASCADE,
  attempt TEXT NOT NULL,
  PRIMARY KEY (task, attempt)
) WITHOUT ROWID;
