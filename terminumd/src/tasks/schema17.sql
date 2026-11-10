-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- A wait's place in its agent's queue, which a human can change (terminum-dispatcher.md AC-169): lowest first.
ALTER TABLE run_waits ADD COLUMN pos INTEGER NOT NULL DEFAULT 0;
UPDATE run_waits SET pos = (SELECT COUNT(*) FROM run_waits o WHERE o.at < run_waits.at OR (o.at = run_waits.at AND o.rowid <= run_waits.rowid));
