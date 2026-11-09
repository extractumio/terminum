// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The ways between the session page and the board.
import { read, write } from "./listparts.js";

const SELECTION = "tm.taskselection";

export const taskUrl = (id) => `/kanban/${id}`;

/** The board's new-task window, for a session and its project (AC-106), a subtask of `parent` (AC-112). */
export const newTaskUrl = (session, project, parent) => `/kanban/?new=1&session=${encodeURIComponent(session)}${project ? `&for=${encodeURIComponent(project)}` : ""}${parent ? `&parent=${encodeURIComponent(parent)}` : ""}`;

/** A session's own address on its page: `#/<key>/`, as readable as a key can stay (AC-134). */
export const sessionHash = (key) => `#/${encodeURIComponent(key).replaceAll("%3A", ":")}/`;

/** The session page showing session `key`; `then` "plan": with its Plan pane open; "msg:<id>": that sent
 *  message in view (terminum.md AC-151); "line:<n>:<text>": that text in view, looked for nearest line n (AC-116). */
export const sessionUrl = (key, then) => `/sessions/${then ? `?then=${encodeURIComponent(then).replaceAll("%3A", ":").replace(/[()]/g, (c) => `%${c.charCodeAt(0).toString(16)}`)}` : ""}${sessionHash(key)}`;

/** The words a record's line is found by again: its first line, as a link can carry them. */
export const lineWords = (record) => ((record.by ? record.note : record.text) ?? "").split("\n").map((l) => l.trim()).find(Boolean)?.slice(0, 80) ?? "";

/** The session this page's address names, if any. */
export function addressedSession() {
  const said = /^#\/(.+?)\/?$/.exec(location.hash)?.[1];
  try { return said ? decodeURIComponent(said) : null; } catch { return null; }
}

/** What that window takes, once; in private mode it opens empty. */
export const leaveSelection = (text) => write(SELECTION, text);

export function takeSelection() {
  const text = read(SELECTION) ?? "";
  write(SELECTION, null);
  return text;
}
