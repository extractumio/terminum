-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- A published batch whose After said it cannot run now (status 75, terminum-integration.md AC-191): when it first
-- waited, for the column's minutes and the wait's cap.
ALTER TABLE integration_batches ADD COLUMN waits_since INTEGER;
