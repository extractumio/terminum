// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A project's sessions that change order glide to their new places (AC-92: a session whose agent
// takes a message goes to its project's top). The list is drawn anew each time (nav.js renderNav):
// when a project's rows come in another order, where they stood is read before the new rows go in
// and where they stand after, and each slides from the one to the other (FLIP); the one that rose
// passes over the others. A redraw while they glide (the list is drawn often) carries them on from
// where they are. A redraw that moves nothing reads no layout. Nothing moves under
// prefers-reduced-motion.

const still = matchMedia("(prefers-reduced-motion: reduce)");
let gliding = 0;                                 // glides running, so a redraw carries them on
let motion = null;                               // [ms, easing]: tokens.css's, read once

/** Each project's rows under `root`: project id -> their session ids in order. */
const orders = (root) => new Map([...root.querySelectorAll("ul[data-project]")]
  .map((ul) => [ul.dataset.project, [...ul.children].map((li) => li.firstElementChild?.dataset.id)]));

/** Whether the ids both lists hold stand in the same order. */
function sameOrder(a, b) {
  const inA = new Set(a), common = b.filter((id) => inA.has(id)), inB = new Set(common);
  return a.filter((id) => inB.has(id)).every((id, i) => id === common[i]);
}

/** Session id -> [li, top as seen, the glide it has], for `pids`' rows; top within the list's content. */
function places(list, pids) {
  const base = list.getBoundingClientRect().top - list.scrollTop, out = new Map();
  for (const ul of list.querySelectorAll("ul[data-project]")) {
    if (!pids.has(ul.dataset.project)) continue;
    for (const li of ul.children) {
      const a = li.getAnimations().find((x) => x.id === "glide");
      out.set(li.firstElementChild?.dataset.id, [li, li.getBoundingClientRect().top - base, a]);
    }
  }
  return out;
}

/** Puts `frag` in `list` for its old rows; a project whose rows changed order glides there. */
export function glide(list, frag) {
  if (still.matches || !list.isConnected) return list.replaceChildren(frag);
  const was = orders(list), now = orders(frag);
  const moved = new Set([...now].filter(([pid, ids]) => was.has(pid) && !sameOrder(was.get(pid), ids)).map(([pid]) => pid));
  if (!moved.size && !gliding) return list.replaceChildren(frag);
  const pids = new Set(gliding ? now.keys() : moved);
  const before = places(list, pids);
  list.replaceChildren(frag);
  motion ??= (() => { const s = getComputedStyle(document.documentElement);
    return [parseFloat(s.getPropertyValue("--dur-pop")) || 350, s.getPropertyValue("--ease-out").trim() || "ease-out"]; })();
  for (const [id, [li, top]] of places(list, pids)) {
    const [old, t, a] = before.get(id) ?? [];
    if (!old) continue;                           // a new row
    const left = a?.playState === "running" ? motion[0] - a.currentTime : 0;
    a?.cancel();
    const pid = li.parentElement.dataset.project;
    if (!(moved.has(pid) || left > 0)) continue;  // a row standing still in an unchanged order
    const dy = t - top;
    if (Math.abs(dy) < 1) continue;
    if (dy > 0 || old.classList.contains("rising")) li.classList.add("rising");   // passes over the rows it leaves (projects.css)
    const g = li.animate([{ transform: `translateY(${dy}px)` }, { transform: "none" }],
      { id: "glide", duration: moved.has(pid) ? motion[0] : Math.max(left, 60), easing: motion[1] });
    gliding++;
    g.finished.then(() => li.classList.remove("rising"), () => {}).finally(() => { gliding--; });
  }
}
