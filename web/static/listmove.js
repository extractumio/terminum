// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A project's sessions that change order glide to their new places (AC-92: a session whose agent
// takes a prompt goes to its project's top). The list is drawn anew each time (nav.js renderNav):
// where each project's rows stood is read before, where they stand after, and the rows of a project
// whose order changed slide from the one to the other (FLIP); the one that rose passes over the
// others. A redraw while they glide (the list is drawn often) carries them on from where they are.
// Nothing moves when only a row's height or the list around it changed, nor under
// prefers-reduced-motion.

const DURATION = 360;
const EASE = "cubic-bezier(.2, .8, .2, 1)";      // tokens.css --ease-out
const still = matchMedia("(prefers-reduced-motion: reduce)");

const gliding = new WeakMap();                  // li -> its glide, while it runs

/** Each project's top rows as drawn in `list`: project id -> [[session id, li, top, ms left of
 *  its glide (0: none)]], top as seen (a glide's offset in it), within the list's content. */
function places(list) {
  const base = list.getBoundingClientRect().top - list.scrollTop, out = new Map();
  const left = (li) => { const a = gliding.get(li); return a?.playState === "running" ? DURATION - a.currentTime : 0; };
  for (const ul of list.querySelectorAll("ul[data-project]")) {
    out.set(ul.dataset.project, [...ul.children].map((li) => [li.firstElementChild?.dataset.id, li, li.getBoundingClientRect().top - base, left(li)]));
  }
  return out;
}

const sameOrder = (a, b) => {
  const common = new Set(a.filter((id) => b.includes(id)));
  const x = a.filter((id) => common.has(id)), y = b.filter((id) => common.has(id));
  return x.every((id, i) => id === y[i]);
};

/** Reads the rows' places, then `draw()` redraws `list`; a project whose rows changed order glides. */
export function glide(list, draw) {
  const before = still.matches || !list.isConnected ? null : places(list);
  draw();
  if (!before?.size) return;
  for (const [pid, rows] of places(list)) {
    const was = before.get(pid);
    if (!was) continue;
    const moved = !sameOrder(was.map((r) => r[0]), rows.map((r) => r[0]));
    const old = new Map(was.map(([id, li, t, ms]) => [id, { t, ms, rising: li.classList.contains("rising") }]));
    for (const [id, li, now] of rows) {
      const o = old.get(id);
      if (!o || !(moved || o.ms > 0)) continue;   // a new row, or one standing still in an unchanged order
      const dy = o.t - now;
      if (Math.abs(dy) < 1) continue;
      if (dy > 0 || o.rising) li.classList.add("rising");   // passes over the rows it leaves behind (projects.css)
      const a = li.animate([{ transform: `translateY(${dy}px)` }, { transform: "none" }], { duration: moved ? DURATION : Math.max(o.ms, 60), easing: EASE });
      gliding.set(li, a);
      a.finished.then(() => li.classList.remove("rising"), () => {});
    }
  }
}
