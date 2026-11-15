// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Where the floating buttons float (a phone, a tablet, a narrow window) the lines of #asks (tasks that need a
// human: tasklines.js, terminum-kanban.md AC-117; agents that wait or failed: waiting.js, terminum-agents.md
// AC-39) fold into two round counters left of ⟨…⟩ (on the board in its head's row): red for the failed, yellow for
// the waiting, none at 0, so nothing lies over the terminal or the board and their rows never move for a line. A
// counter with a line not looked at since it came wears a ring whose glow pulses slowly (askfold.css). A tap opens its lines as a sheet (fabpane.js placeModal); a tap outside,
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

  // one pass over the lines: each with its kind (its mark) and its key (kind and words)
  const read = () => [...box.querySelectorAll(".ask")].filter((c) => !c.hidden).map((c) => {
    const kind = KINDS.find((k) => c.querySelector(`.state.${k}`));
    return { kind, key: `${kind}|${c.querySelector(".what")?.textContent ?? ""}` };
  }).filter((l) => l.kind);

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
    placeModal(box, close);
    sync();                                        // its lines are looked at
  }

  /** The counters as the lines stand now; folding only where the buttons float. */
  function sync() {
    const on = floats();
    body.classList.toggle("asksfold", on);
    if (!on) close();
    const all = read();
    for (const l of all) if (l.kind === opened) looked.add(l.key);
    for (const k of looked) if (!all.some((l) => l.key === k)) looked.delete(k);   // gone: a new one with the same words rings again
    for (const kind of KINDS) {
      const mine = all.filter((l) => l.kind === kind), b = buttons[kind], n = mine.length, says = SAYS[kind](n);
      b.textContent = n;
      b.hidden = !on || !n;
      b.title = says;
      b.setAttribute("aria-label", says);
      b.classList.toggle("fresh", mine.some((l) => !looked.has(l.key)));
      b.setAttribute("aria-expanded", String(opened === kind));
      if (opened === kind && !n) close();
    }
    counters.hidden = KINDS.every((k) => buttons[k].hidden);
    place();
  }

  /** Left of ⟨…⟩ on its row; at the top right under the header where it is not shown (Files, View). The board has no
   *  ⟨…⟩: its counters stand in its head's row. */
  function place() {
    const fold = $("foldfab");
    if (!fold) return;
    const main = counters.offsetParent;            // none while hidden
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
  let frame = 0;
  addEventListener("resize", () => { frame ||= requestAnimationFrame(() => { frame = 0; place(); }); });
  sync();
  return { place };
}
