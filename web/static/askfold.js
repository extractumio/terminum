// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Where the floating buttons float (a phone, a tablet, a narrow window) the lines of #asks (tasks that need a
// human: tasklines.js, terminum-kanban.md AC-117; agents that wait or failed: waiting.js, terminum-agents.md
// AC-39) fold into two round counters left of ⟨…⟩: red for the failed, yellow for the waiting, none at 0, so
// nothing lies over the terminal and its rows never change for a line. A counter with a line not looked at
// since it came wears a still ring. A tap opens its lines as a sheet (fabpane.js placeModal); a tap outside,
// Esc, a line pressed or its last line going closes it. The lines keep their own ways (×, open, terminumd).
import { floats, FLOATING } from "./fold.js";
import { placeModal } from "./fabpane.js";

const $ = (id) => document.getElementById(id);
const GAP = 12;                // px between the counters and ⟨…⟩, as in its column
const KINDS = ["failed", "waiting"];
const SAYS = { failed: (n) => `${n} failed`, waiting: (n) => `${n} ${n === 1 ? "waits" : "wait"} for you` };

/** `box`: #asks; `counters`: #askcount, holding a button per kind (`data-kind`). */
export function asksFold({ box, counters }) {
  const body = document.body, buttons = Object.fromEntries(KINDS.map((k) => [k, counters.querySelector(`[data-kind="${k}"]`)]));
  const looked = new Set();    // kind|words of the lines seen in an opened sheet
  let opened = null;

  const lines = (kind) => [...box.querySelectorAll(".ask")].filter((c) => !c.hidden && c.querySelector(`.state.${kind}`));
  const keyOf = (kind, c) => `${kind}|${c.querySelector(".what")?.textContent ?? ""}`;

  function close() {
    if (!opened) return;
    opened = null;
    delete box.dataset.kind;
    delete box.dataset.sheet;                      // the shade goes with it (fabpane.js)
    body.classList.remove("asksopen");
  }

  function open(kind) {
    if (opened === kind) return close();
    opened = kind;
    box.dataset.kind = kind;
    body.classList.add("asksopen");
    for (const c of lines(kind)) looked.add(keyOf(kind, c));
    placeModal(box, close);
    sync();
  }

  /** The counters as the lines stand now; folding only where the buttons float. */
  function sync() {
    const on = floats();
    body.classList.toggle("asksfold", on);
    if (!on) close();
    const now = new Set();
    let any = false;
    for (const kind of KINDS) {
      const mine = lines(kind), b = buttons[kind], n = mine.length;
      for (const c of mine) now.add(keyOf(kind, c));
      if (opened === kind) for (const c of mine) looked.add(keyOf(kind, c));
      if (b.textContent !== String(n)) b.textContent = String(n);
      b.hidden = !on || !n;
      b.setAttribute("aria-label", SAYS[kind](n));
      b.title = SAYS[kind](n);
      b.classList.toggle("fresh", mine.some((c) => !looked.has(keyOf(kind, c))));
      b.setAttribute("aria-expanded", String(opened === kind));
      if (opened === kind && !n) close();
      any ||= on && n > 0;
    }
    for (const k of looked) if (!now.has(k)) looked.delete(k);   // gone: a new one with the same words rings again
    if (counters.hidden === any) counters.hidden = !any;
    place();
  }

  /** Left of ⟨…⟩ on its row; at the top right under the header where it is not shown (Files, View). */
  function place() {
    if (counters.hidden) return;
    const fold = $("foldfab"), main = counters.offsetParent;
    if (!main) return;
    const shown = !fold.hidden && fold.offsetHeight > 0;
    const top = shown ? fold.offsetTop : $("over").offsetTop + GAP;
    const right = shown ? fold.offsetLeft - GAP : main.clientWidth - 14;
    counters.style.top = `${top}px`;
    counters.style.left = `${Math.round(right - counters.offsetWidth)}px`;
  }

  for (const kind of KINDS) {
    buttons[kind].addEventListener("mousedown", (e) => e.preventDefault());   // the terminal keeps the keys
    buttons[kind].addEventListener("click", () => open(kind));
  }
  box.addEventListener("click", (e) => { if (opened && e.target.closest(".go")) close(); });   // its task's or session's place opens
  new MutationObserver(sync).observe(box, { childList: true, subtree: true, characterData: true, attributes: true, attributeFilter: ["hidden", "class"] });
  matchMedia(FLOATING).addEventListener("change", sync);
  addEventListener("resize", place);
  sync();
  return { sync, place };
}
