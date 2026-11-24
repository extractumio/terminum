// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The ways to a report from anywhere on the page (terminum-reports.md AC-268, AC-273): its address, its opening, its
// age, type and sender in words. Nothing heavy here: the session list, a task's window and a line open reports through it.

export const REPORT_EVENT = "tm-report";           // reportpane.js opens the report this names

/** The board's address of report `id`. */
export const reportUrl = (id) => `/kanban/?report=${encodeURIComponent(id)}`;

/** Opens report `id`: its pane on the board, else the board with it (AC-268). */
export function openReport(id) {
  if (!document.body.classList.contains("board-page")) { location.href = reportUrl(id); return; }
  if (matchMedia("(max-width: 860px)").matches) document.getElementById("drawerclose")?.click();   // a phone's list sheet gives way
  dispatchEvent(new CustomEvent(REPORT_EVENT, { detail: id }));
}

/** "now", "5 min", "3 h", "2 d": how old. */
export function age(at, now = Date.now() / 1000) {
  const s = Math.max(0, now - at);
  return s < 60 ? "now" : s < 3600 ? `${Math.floor(s / 60)} min` : s < 86400 ? `${Math.floor(s / 3600)} h` : `${Math.floor(s / 86400)} d`;
}

/** Report `r`'s type, as terminumd set it (AC-265): ok, attention, failed, incident, digest or report. */
export const kindOf = (r) => r.kind || "report";

/** Who sent report `r` (AC-267): its routine, the Supervisor, its task or its author, as terminumd names it; then the
 *  agent that wrote it, when that is another name. */
export function senderOf(r) {
  const s = r.source, from = s.sender === "routine" ? `Routine ${s.name}` : s.name;
  return s.sender === "supervisor" || s.sender === "agent" || r.author_name === s.name ? from : `${from} · ${r.author_name}`;
}

