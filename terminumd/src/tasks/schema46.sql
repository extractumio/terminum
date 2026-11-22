-- SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
-- AC-230: one live answer by option per question, kept in the same history as free-text answers.
CREATE UNIQUE INDEX events_question_choice ON events (task, json_extract(data, '$.choice.question'))
WHERE kind = 'comment' AND json_extract(data, '$.choice.question') IS NOT NULL;
