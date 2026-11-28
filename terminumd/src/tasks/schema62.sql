-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- AC-240: the recurring question a standing rule ends, copied from the store, so the weekly review measures that
-- question before and after the owner accepted the rule. NULL: the decision named none, and its effect is unknown.
ALTER TABLE supervisor_decisions ADD COLUMN recurring TEXT;
