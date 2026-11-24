-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- AC-236: a ruling also keeps its signal's count, so an accepted problem reopens only on what the count calls a repeat
-- (AC-211: a counted reading's new event, or a return after resolving), never on a retaken sample or a window's input.
-- Rulings made before it keep the count their signal holds now.
ALTER TABLE supervisor_problem_signals ADD COLUMN ruling_count INTEGER;
UPDATE supervisor_problem_signals SET ruling_count =
  (SELECT count FROM supervisor_signals s WHERE s.fingerprint = supervisor_problem_signals.fingerprint)
  WHERE ruling_evidence IS NOT NULL;
