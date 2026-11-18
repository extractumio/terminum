// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// What the session page holds of the board: the session its address asks for (terminum-kanban.md
// AC-134) and the lines of tasks that need a human (AC-117), folded into two counters where the buttons float.
import { addressedSession, sessionHash, taskUrl } from "./tasklink.js";
import { findItem, serverDown } from "./nav.js";
import { taskLines } from "./tasklines.js";
import { asksFold } from "./askfold.js";
import { navSession, pageStarter } from "./navsession.js";

/** `box` holds the lines; `projects()`: the list's projects, which name a task's line; `here()`: the project of the
 *  session shown, or null, and `menu(key, pick)` the projects' menu: the list head's [+ Session] (terminum-dispatcher.md AC-142). */
export function pageTasks({ box, embedded, say, show, projects, here, menu }) {
  let asked = addressedSession(), then = new URLSearchParams(location.search).get("then");
  // Back and Forward go through the sessions pressed
  if (!embedded) addEventListener("popstate", () => { const id = addressedSession(); if (id) show(id); });
  const lines = taskLines(box, (id) => { location.href = taskUrl(id); }, (text) => say(text, 6000), (id) => projects().find((p) => p.id === id));
  if (!embedded) navSession({ inView: here, menu, start: pageStarter() });
  const counts = asksFold({ box, counters: document.getElementById("askcount") });   // the waiting agents' lines too (waiting.js)
  return {
    /** The session asked for, before any list. */
    asked,
    /** The first list with rows says whether it is there; the page then goes on as after any closing. */
    layout(groups) {
      if (!asked || !groups.length) return;
      const line = /^line:(\d+):(.*)$/s.exec(then ?? "");
      if (!findItem(groups, asked) && !serverDown(groups, asked)) say("That session has ended.", 4000);
      else if (then === "plan") setTimeout(() => document.dispatchEvent(new Event("planopen")));
      // the Message history's Open in session (terminum.md AC-151): that message brought into view
      else if (/^msg:\d+$/.test(then ?? "")) {
        const id = Number(then.slice(4));                // read now: `then` is gone when the event goes
        setTimeout(() => document.dispatchEvent(new CustomEvent("sentjump", { detail: id })));
      }
      // a task's link to the record it was made of (terminum-kanban.md AC-116): that line brought into view
      else if (line) setTimeout(() => document.dispatchEvent(new CustomEvent("linejump", { detail: { n: Number(line[1]), text: line[2] } })));
      then = null;
      asked = null;
    },
    marks(m) { if (!embedded) lines.came(m); },
    /** The counters beside ⟨…⟩, after the floating buttons were placed. */
    placeCounts: counts.place,
    /** Every session has its own address (AC-134): `press`: a step of Back, any other change none. */
    address(id, press = false) {
      const hash = sessionHash(id);
      const search = new URLSearchParams(location.search);
      search.delete("then");                        // said once: Back never opens it again
      const url = `${location.pathname}${search.size ? `?${search}` : ""}${hash}`;
      if (!embedded && `${location.pathname}${location.search}${location.hash}` !== url) history[press ? "pushState" : "replaceState"](history.state, "", url);
    },
  };
}
