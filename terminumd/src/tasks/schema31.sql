-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- The tasks a message was sent for (terminum.md AC-147): the IDs, comma-separated as the input log's, of the
-- tasks not done whose session its pane was when it was kept.
ALTER TABLE messages ADD COLUMN tasks TEXT;
-- Dispatcher's messages kept before: the tasks of the prompt its gate typed into that pane in the 5 minutes
-- before, as the input log's row names them. A human's are left without: nothing kept says whose they were.
UPDATE messages SET tasks = (SELECT i.tasks FROM inputs i WHERE i.pane = messages.pane AND i.sender = 'dispatcher' AND i.kind = 'prompt'
    AND i.tasks <> '' AND i.at <= messages.at AND i.at > messages.at - 300000 ORDER BY i.at DESC LIMIT 1)
  WHERE sender = 'dispatcher';
