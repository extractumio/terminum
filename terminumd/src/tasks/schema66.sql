-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- AC-237: checked findings and budget observations have independent lifetimes.
ALTER TABLE supervisor_rounds ADD COLUMN usage_receipt TEXT;
ALTER TABLE supervisor_rounds ADD COLUMN first_usage_at INTEGER;
ALTER TABLE supervisor_rounds ADD COLUMN final_usage_at INTEGER;
ALTER TABLE supervisor_rounds ADD COLUMN delayed_usage INTEGER NOT NULL DEFAULT 0;
ALTER TABLE supervisor_rounds ADD COLUMN budget_overrun_at INTEGER;
CREATE UNIQUE INDEX supervisor_rounds_receipt ON supervisor_rounds(usage_receipt)
    WHERE usage_receipt IS NOT NULL;

-- The previous protocol only requested readings at turn or session end.
UPDATE supervisor_rounds SET first_usage_at = (SELECT min(at) FROM usage WHERE usage.run = supervisor_rounds.run),
    final_usage_at = (SELECT max(at) FROM usage WHERE usage.run = supervisor_rounds.run
        AND at >= coalesce(supervisor_rounds.found_at, supervisor_rounds.ended_at));
UPDATE supervisor_rounds SET delayed_usage = 1 WHERE first_usage_at > coalesce(found_at, ended_at);
UPDATE supervisor_rounds SET budget_overrun_at = (SELECT max(at) FROM usage WHERE usage.run = supervisor_rounds.run)
    WHERE token_cap < (SELECT sum(0.0 + input + output + cache_read + cache_write)
        FROM usage WHERE usage.run = supervisor_rounds.run);
UPDATE supervisor_rounds SET failure = NULL WHERE finding IS NOT NULL AND budget_overrun_at IS NOT NULL
    AND failure = 'The Supervisor round exceeded its token limit.';
UPDATE supervisor_rounds SET usage_receipt = lower(hex(randomblob(16))) WHERE ended_at IS NULL;
CREATE TRIGGER supervisor_round_receipt AFTER INSERT ON supervisor_rounds
BEGIN
    UPDATE supervisor_rounds SET usage_receipt = lower(hex(randomblob(16))) WHERE run = NEW.run;
END;
