// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Bookmarks and reminders belong to a project (AC-91): the host and folder of the session they
// were made in. Every live session of the project shows them; once none is left they are
// orphaned and shown only in the list below the sessions. One made before 0.8.0 has no folder:
// its project is its own pane.

/** Whether bookmark `b` belongs to the project of layout row `it` (a pane). */
export const inProject = (b, it) => !!it && (b.dir ? b.dir === it.dir && (b.host ?? null) === (it.host ?? null) : b.sid === it.id);

/** Bookmark `b`'s project as one key: its host and folder, or its pane before 0.8.0. */
export const projectKey = (b) => (b.dir ? `${b.host ?? ""}\0${b.dir}` : `\0\0${b.sid}`);
/** The keys a pane `it` matches: its host and folder, and itself (bookmarks before 0.8.0). */
export const paneKeys = (it) => [`${it.host ?? ""}\0${it.dir}`, `\0\0${it.id}`];

/** Whether no live pane of `panes` is in bookmark `b`'s project. */
export const orphaned = (b, panes) => !panes.some((it) => inProject(b, it));
