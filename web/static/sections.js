// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// terminum's sections and the one switcher between them (terminum.md AC-167): every page imports this module,
// which draws the switcher from this list into the page's `nav.sections`, the page's own section marked.
// A new section is a line here: its `id` is its address's first part, `pages` those of its further pages.
import { icon } from "./icons.js";
import { el } from "./listparts.js";

/** In the switcher's order: `id` its address (`/<id>/`), `label` on its key, `title` what it holds (the key's tooltip). */
const SECTIONS = [
  { id: "sessions", label: "Sessions", icon: "square-terminal", title: "Terminals and their coding agents" },
  { id: "kanban", label: "Kanban", icon: "board", title: "Tasks and their sessions" },
  { id: "dashboard", label: "Dashboard", icon: "chart-line", title: "How much you drive sessions" },
  { id: "history", label: "Messages", icon: "messages-square", title: "What you sent, and what schedules ran", pages: ["scheduled"] },
];

/** Draws the switcher into each `nav.sections` of the page, the section of the page's address marked as shown:
 *  once, as the module loads (a page's script imports it; its markup is parsed by then). */
function mountSections() {
  const page = location.pathname.split("/")[1];
  const current = SECTIONS.find((s) => s.id === page || s.pages?.includes(page))?.id;
  for (const nav of document.querySelectorAll("nav.sections")) {
    nav.replaceChildren(...SECTIONS.map((s) => {
      const a = el("a");
      a.href = `/${s.id}/`;
      a.title = s.title;
      a.setAttribute("aria-label", s.label);   // its word may be folded (controls.css)
      if (s.id === current) a.setAttribute("aria-current", "page");
      a.append(icon(s.icon, 15), el("span", "word", s.label));
      return a;
    }));
  }
}

mountSections();

/** The switcher's Kanban opens the board of `project` (an id; none: the board as this browser left it), the project
 *  of the session shown (terminum-kanban.md AC-135). */
export function boardFor(project) {
  for (const a of document.querySelectorAll("nav.sections a")) {
    if (a.pathname === "/kanban/") a.setAttribute("href", project ? `/kanban/?project=${encodeURIComponent(project)}` : "/kanban/");
  }
}
