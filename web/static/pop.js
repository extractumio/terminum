// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Where a window flies out of (motion.css): as a .box opens, its transform-origin goes to the press,
// or the focused key, that opened it, held to at most NEAR outside the window: a menu grows out of
// its button, a dialog in the middle from its side towards it. Opened by nothing (a toast, a
// reply), it grows from its middle. A menu's rows are numbered (--i) for their cascade. The
// window's place is set by its opener before this runs (a microtask after it).
const NEAR = 40;       // px outside a window its origin may be
const FRESH = 1000;    // ms a press or key counts as what opened a window (a long press opens a row's menu)
const calm = matchMedia("(prefers-reduced-motion: reduce)");
let by = null;         // the last press: {x, y} or {el}, and when

document.addEventListener("pointerdown", (e) => { by = { x: e.clientX, y: e.clientY, at: performance.now() }; }, { capture: true, passive: true });
document.addEventListener("keydown", (e) => { by = { el: e.target, at: performance.now() }; }, { capture: true, passive: true });

/** The opener's point on the screen, or null when none is fresh. */
function opener() {
  if (!by || performance.now() - by.at > FRESH) return null;
  if (!by.el) return by;
  const r = by.el.isConnected && by.el !== document.body && by.el.getBoundingClientRect();
  if (!r?.width) return null;      // gone, or hidden with what it opened
  return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
}

function opened(box) {
  // its place without its scale: it may already be small (its pop began when its style was computed)
  const s = getComputedStyle(box), k = s.scale === "none" ? 1 : parseFloat(s.scale) || 1;
  const [ox, oy] = s.transformOrigin.split(" ").map(parseFloat), r = box.getBoundingClientRect();
  const left = r.left - ox * (1 - k), top = r.top - oy * (1 - k), w = r.width / k, h = r.height / k;
  const p = opener(), held = (v, lo, size) => Math.round(Math.max(-NEAR, Math.min(size + NEAR, v - lo)));
  box.style.transformOrigin = p ? `${held(p.x, left, w)}px ${held(p.y, top, h)}px` : "";
  if (box.getAttribute("role") === "menu") [...box.children].forEach((c, i) => c.style.setProperty("--i", i));
}

new MutationObserver((changes) => {
  if (calm.matches) return;
  const boxes = new Set();
  for (const m of changes) {
    const b = m.target;
    if (b.classList.contains("box") && (m.attributeName === "open" ? m.oldValue === null && b.open : m.oldValue !== null && !b.hidden)) boxes.add(b);
  }
  boxes.forEach(opened);
}).observe(document.body, { subtree: true, attributes: true, attributeFilter: ["hidden", "open"], attributeOldValue: true });
