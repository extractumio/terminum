// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Reminders (AC-91): bookmarks of the Reminder kind, of a project (projects.js). terminumd puts
// one up each time an agent of its project ends a turn; then the pale fuchsia button under the
// bookmarks' blinks while a session of the project is shown, until it is pressed (one seen there
// blinks again only once up anew). It opens the project's open reminders as Markdown, those up
// first: Open shows its session at its text, Close puts it down until the next turn ends,
// Resolve ends it, Delete (twice) removes it. The session list marks a session of a project with
// open reminders by a wavy pink line.
import { fabPane } from "./fabpane.js";
import { icon } from "./icons.js";
import { armed, el, openReminder, quote, when } from "./marks.js";
import { markdownInto } from "./widgetui.js";

const $ = (id) => document.getElementById(id);

/** The open reminders of `items` (bookmarks' wire form), those up first, newest first. */
export function openReminders(items) {
  return [...items].filter(openReminder).sort((a, b) => Number(!!b.due) - Number(!!a.due) || b.at - a.at);
}

/** marks: marks.js's Marks (its items, alive, jump; it shows and places this button with the
 *  bookmarks'); send(msg) to terminumd. */
export function reminders({ marks, send }) {
  const seen = new Set();          // reminders up that the user saw in the pane: they blink no more
  let drawn = "";                  // what the pane shows, as its key: drawn again only when it changes
  const fab = $("remindfab");
  const pane = fabPane({ fab, pane: $("remindpane"), close: $("remindclose"), onToggle: () => { drawn = ""; draw(); } });

  function draw() {
    const all = openReminders(marks.here()), up = all.filter((i) => i.due).length;   // the shown pane's project's
    for (const id of seen) if (!all.some((i) => i.id === id && i.due)) seen.delete(id);   // put down: blinks when up again
    if (pane.open()) for (const i of all) if (i.due) seen.add(i.id);
    fab.classList.toggle("due", all.some((i) => i.due && !seen.has(i.id)));
    $("remindfabn").textContent = all.length ? String(all.length) : "";
    fab.title = up ? `Reminders: ${up} up of ${all.length}` : `Reminders: ${all.length}`;
    fab.setAttribute("aria-label", fab.title);
    if (!all.length && pane.open()) return pane.toggle(false);
    if (!pane.open()) return;
    const key = JSON.stringify(all.map((i) => [i.id, i.due, i.note, i.text, marks.alive(i.sid)]));
    if (key !== drawn) { drawn = key; void drawPane(all); }
  }

  async function drawPane(all) {
    const list = el("ul"), drawing = [];
    for (const item of all) {
      const li = el("li", "rmitem" + (item.due ? " due" : ""));
      const head = el("div", "rmhead");
      head.append(el("span", "rmwhere", [marks.alive(item.sid) ?? `${item.title} (closed)`, item.host].filter(Boolean).join(" · ")),
        el("span", "rmwhen", item.due ? "up now" : when(item.at)));
      const body = el("div", "markdown");
      const md = (item.note ? item.note + "\n\n" : "") + quote(item.text);
      drawing.push(markdownInto(body, md));
      const acts = el("div", "rmacts");
      const act = (label, fn, cls = "") => { const b = el("button", cls, label); b.type = "button"; b.onclick = fn; acts.append(b); };
      act("Open", () => { pane.toggle(false); marks.jump(item); });
      if (item.due) act("Close", () => send({ t: "remind", id: item.id, action: "close" }));
      const del = el("button", "del", "Delete");                    // asks again: a second press deletes it for good
      del.type = "button";
      armed(del, () => ["Delete"], () => send({ t: "unmark", id: item.id }));
      acts.append(del);
      act("Resolve", () => send({ t: "remind", id: item.id, action: "resolve" }), "primary");
      li.append(head, body, acts);
      list.append(li);
    }
    await Promise.all(drawing);
    $("remindlist").replaceChildren(list);
    pane.place();
  }

  $("remindclose").replaceChildren(icon("x", 16));
  return { draw, place: pane.place };
}
