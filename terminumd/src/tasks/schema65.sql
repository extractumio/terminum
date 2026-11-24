-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- AC-211: a signal's count grows only on a repeat, but signals written before that rule counted every evaluation
-- pass, so their counts (hundreds, thousands) say nothing true. Their repeats cannot be rebuilt from the store: each
-- keeps one occurrence, or two when it recurred. A ruling's count follows its signal's (AC-236), so an accepted
-- problem still reopens on the next repeat its signal counts.
UPDATE supervisor_signals SET count = min(count, CASE WHEN state = 'recurred' THEN 2 ELSE 1 END);
UPDATE supervisor_problem_signals SET ruling_count = min(ruling_count,
  (SELECT count FROM supervisor_signals s WHERE s.fingerprint = supervisor_problem_signals.fingerprint))
  WHERE ruling_count IS NOT NULL;
