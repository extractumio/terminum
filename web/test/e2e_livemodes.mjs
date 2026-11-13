// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Focused modes of the private-server live runner; no option means its full suite.
export const SORT_ALONE = process.argv.includes("boardsort");
export const BOARD_ALONE = process.argv.includes("board");
export const TASKS_ALONE = process.argv.includes("boardtasks");   // while working on a task's family and its end
export const WAITING_ALONE = process.argv.includes("waiting");  // RUN before a waiting session closes, with a normal clock
export const POOL_ALONE = process.argv.includes("boardpool");     // while working on the pool and what runs on it
export const SMOKE = process.argv.includes("smoke");
export const HISTORY_ALONE = process.argv.includes("history");   // while working on the Message history and the Dashboard
export const AUTO_ALONE = process.argv.includes("auto");             // while working on Auto-Dispatcher
export const SCHEDULES_ALONE = process.argv.includes("schedules");   // while working on schedules and folded cards
export const NOTES_ALONE = process.argv.includes("notes");       // while working on bookmarks, reminders and their tasks
export const ICONS_ALONE = process.argv.includes("iconkeys");    // while working on icon keys' focus
export const REQUESTS_ALONE = process.argv.includes("requests");
export const QUEUE_ALONE = process.argv.includes("queue");  // the agent queue, without the scheduling suites
export const MAIN_ALONE = process.argv.includes("main");             // while working on a main task's run with its subtasks
export const ARCHIVED_ALONE = process.argv.includes("archived");     // archived tasks, their hooks and tools
