// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The board where room is short (terminum-kanban.md AC-105 "a phone's one-row head", AC-117; boardphone.css).
// Where the buttons float, the lines of tasks that need the human fold into the head's two counters (askfold.js).
// On a phone (600 px or narrower, or on its side: BOARD_PHONE) the head is one row: Find and the filters open under
// it by the Find key, lit while the board is narrowed; Auto-Dispatch, the agents and the schedules stand behind one
// line under the columns' switch, which opens them as a sheet. They are the page's own parts, moved, never copies.
import { asksFold } from "./askfold.js";
import { placeModal } from "./fabpane.js";
import { $ } from "./kanbanutil.js";
import { el, pointerKey } from "./listparts.js";
import { BOARD_PHONE } from "./phonewin.js";

const phone = matchMedia(BOARD_PHONE);
const bar = $("bar"), find = $("findkey"), pool = $("pool"), line = $("poolline"), auto = $("autokeys"), clear = $("clearfilters");

asksFold({ box: $("asks"), counters: $("askcount") });

function finding(on) {
  bar.classList.toggle("finding", on);
  find.setAttribute("aria-expanded", String(on));
  if (on) $("find").focus();
}
pointerKey(find).addEventListener("click", () => finding(!bar.classList.contains("finding")));   // an icon key: no focus from a press

/** The Find key says a narrowed board: kanbanfilters.js shows Clear filters exactly then. */
function lit() {
  const on = !clear.hidden;
  find.classList.toggle("lit", on);
  find.title = on ? "Find and filters: the board is narrowed" : "Find and filters";
  find.setAttribute("aria-label", find.title);
}
new MutationObserver(lit).observe(clear, { attributes: true, attributeFilter: ["hidden"] });

/** The line's words, on a phone: Auto-Dispatch's state first, then how many agents and schedules the shown projects
 *  have, as their sections say them (`data-n`: kanbanagents.js, kanbansched.js); written only when they change. */
let said = "";
function say() {
  if (!phone.matches) return;
  const parts = [["Auto", $("autostate").textContent], ["Agents", pool.querySelector(".agents")?.dataset.n ?? 0], ["Scheduled", pool.querySelector(".schedsection")?.dataset.n ?? 0]];
  if (said === (said = String(parts))) return;
  line.replaceChildren(...parts.map(([name, value]) => { const s = el("span", "poolpart", `${name} `); s.append(el("b", null, String(value))); return s; }));
}
const watched = new MutationObserver(say);
watched.observe(pool, { attributes: true, attributeFilter: ["data-n"], subtree: true });
watched.observe($("autostate"), { childList: true, characterData: true, subtree: true });

const open = () => "sheet" in pool.dataset;        // placeModal's own mark (fabpane.js)
function close() {
  delete pool.dataset.sheet;                       // its shade goes with it
  line.setAttribute("aria-expanded", "false");
}
line.addEventListener("click", () => {
  if (open()) return close();
  line.setAttribute("aria-expanded", String(placeModal(pool, close, false)));   // no focus on its first key: that is Auto-Dispatch's switch
});

/** Auto-Dispatch's keys: in the head on a computer, first in the sheet on a phone. */
function arrange() {
  if (phone.matches) pool.prepend(auto);
  else { $("boardacts").prepend(auto); close(); finding(false); }
  say();
}
phone.addEventListener("change", arrange);
arrange();
lit();
