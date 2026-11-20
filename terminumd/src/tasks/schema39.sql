-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- AC-122: the event through which each task's last run took its human comments.
ALTER TABLE tasks ADD COLUMN run_comments TEXT;
ALTER TABLE runs ADD COLUMN pending_comments TEXT;
UPDATE tasks SET run_comments = (
  SELECT id FROM events WHERE task = tasks.id AND author = 'dispatcher'
    AND ((kind = 'session' AND text = 'Dispatcher linked a session')
      OR (kind = 'why' AND text IN ('The agent took it.', 'The human ran it again.', 'The project''s manager ran it again.')))
  ORDER BY rowid DESC LIMIT 1
) WHERE EXISTS (SELECT 1 FROM run_tasks WHERE task = tasks.id);
