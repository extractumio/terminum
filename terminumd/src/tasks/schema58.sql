-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- AC-262: one review task per outside reference, even when probes repeat it.
CREATE TABLE supervisor_outside_references (
    project TEXT NOT NULL REFERENCES projects ON DELETE CASCADE,
    link TEXT NOT NULL,
    PRIMARY KEY (project, link)
) WITHOUT ROWID;
