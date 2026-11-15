// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The board where room is short (terminum-kanban.md AC-105 "a phone's one-row head", AC-117; boardphone.css).
// Where the buttons float, the lines of tasks that need the human fold into the head's two counters (askfold.js).
// On a phone (600 px or narrower) the head is one row: Find and the filters open under it by the Find key, lit
// while the board is narrowed; Auto-Dispatch, the agents and the schedules stand behind one line under the
// columns' switch, which opens them as a sheet. They are the page's own parts, moved, never copies.
import { asksFold } from "./askfold.js";
import { placeModal } from "./fabpane.js";
import { el } from "./listparts.js";
import { PHONE } from "./phonewin.js";

const $ = (id) => document.getElementById(id);
const phone = matchMedia(PHONE);
const bar = $("bar"), find = $("findkey"), pool = $("pool"), line = $("poolline"), auto = $("autokeys"), clear = $("clearfilters");

asksFold({ box: $("asks"), counters: $("askcount") });

function finding(on) {
  bar.classList.toggle("finding", on);
  find.setAttribute("aria-expanded", String(on));
  if (on) $("find").focus();
}
find.addEventListener("click", () => finding(!bar.classList.contains("finding")));

/** The Find key says a narrowed board: kanbanfilters.js shows Clear filters exactly then. */
function lit() {
  const on = !clear.hidden;
  find.classList.toggle("lit", on);
  find.title = on ? "Find and filters: the board is narrowed" : "Find and filters";
  find.setAttribute("aria-label", find.title);
}
new MutationObserver(lit).observe(clear, { attributes: true, attributeFilter: ["hidden"] });

/** The line's words: Auto-Dispatch's state first, then how many agents and schedules the shown projects have. */
function say() {
  const part = (name, value) => { const s = el("span", "poolpart", `${name} `); s.append(el("b", null, String(value))); return s; };
  const schedules = pool.querySelector(".schedtoggle")?.textContent.replace(/\D+/g, "") || 0;
  line.replaceChildren(part("Auto", $("autostate").textContent), part("Agents", pool.querySelectorAll(".agentcard").length), part("Scheduled", schedules));
}
const watched = new MutationObserver(say);
for (const part of [pool, auto]) watched.observe(part, { childList: true, subtree: true, characterData: true });

let open = false;
function close() {
  if (!open) return;
  open = false;
  delete pool.dataset.sheet;                       // its shade goes with it (fabpane.js)
  line.setAttribute("aria-expanded", "false");
}
line.addEventListener("click", () => {
  if (open) return close();
  open = true;
  line.setAttribute("aria-expanded", "true");
  placeModal(pool, close, false);                  // no focus on its first key: that is Auto-Dispatch's switch
});

/** Auto-Dispatch's keys: in the head on a computer, first in the sheet on a phone. */
function arrange() {
  if (phone.matches) pool.prepend(auto);
  else { $("boardacts").prepend(auto); close(); finding(false); }
}
phone.addEventListener("change", arrange);
arrange();
lit();
say();
