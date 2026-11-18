// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The session list's head key [+ Session] (terminum-dispatcher.md AC-142), on the board and the session page: the
// board's + Session for the project in view (the session shown, or the one project the board shows), else a menu of
// the projects. The plain tmux session keeps its key at the list's foot.
import { sessionStarter } from "./kanbansession.js";
import { api, say } from "./kanbanutil.js";

/** `inView()`: the project in view, or null; `menu(key, pick)`: the projects' menu under the key; `start(p)`. */
export function navSession({ inView, menu, start }) {
  const key = document.getElementById("navnew");
  const titled = () => {
    const p = inView();
    key.title = p ? `A session of an agent in ${p.name}, no task` : "A session of an agent in a project, no task";
    if (p) key.removeAttribute("aria-haspopup"); else key.setAttribute("aria-haspopup", "menu");   // a press opens a menu only with none in view
  };
  key.addEventListener("pointerenter", titled);
  key.addEventListener("focus", titled);
  key.onclick = () => {
    const p = inView();
    if (p) start(p); else menu(key, start);
  };
}

/** The session page's starter: the board's, with the projects' agents and slots read at the press (no board here). */
export function pageStarter() {
  let meta = {}, slots = { full: [], full_projects: [] };
  const start = sessionStarter({ getMeta: () => meta, slots: () => slots, addAgent: () => { location.href = "/kanban/"; }, askFolder: (_, words) => say(words) });
  return async (p) => {
    try {
      const [m, page] = await Promise.all([api("/api/tasks/meta"), api("/api/tasks?counts=1&limit=1")]);
      meta = m;
      slots = page.slots ?? slots;
    } catch (e) { return say(e.message); }
    const known = meta.projects?.find((x) => x.id === p.id);
    return known ? start(known) : say(`${p.name}: no such project.`);
  };
}
