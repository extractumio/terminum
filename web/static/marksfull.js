// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Agents' bookmarks or reminders that went to make room (AC-98): a line a scope and kind over the
// terminal, terminumd's words; Open shows the pane that made room with its bookmarks' (reminders')
// pane, × closes the line on every page (terminumd tells them).
import { closeButton } from "./banners.js";

/** The line of one banner `b` ({scope, remind, most, label, sid}). */
export function fullText(b) {
  return `Agents' ${b.remind ? "reminders" : "bookmarks"} of ${b.label} are over ${b.most}: the oldest are deleted automatically. Review them.`;
}

/** box: #marksfull; send: to terminumd; open(b): shows b's pane and its pane of that kind. */
export function marksFull({ box, send, open }) {
  function line(b) {
    const row = document.createElement("div");
    row.className = "fullline";
    const text = document.createElement("span");
    text.textContent = fullText(b);
    const go = document.createElement("button");
    go.type = "button";
    go.className = "link";
    go.textContent = b.remind ? "Open reminders" : "Open bookmarks";
    go.onclick = () => open(b);
    row.append(text, go, closeButton("Close", () => send({ t: "marksfullseen", scope: b.scope, remind: b.remind })));
    return row;
  }
  return {
    /** terminumd's list, the whole of it each time. */
    show(items) {
      const list = Array.isArray(items) ? items : [];
      box.replaceChildren(...list.map(line));
      box.hidden = !list.length;
    },
  };
}
