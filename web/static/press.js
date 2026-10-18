// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Where a button was pressed: the origin of its ripple (motion.css), and a fresh name for the
// ripple's animation (a, b) so each press restarts it. Never in the terminal or on the key bar.
const SKIP = "#term, #kbd, .key";
const calm = matchMedia("(prefers-reduced-motion: reduce)");

function mark(b, x, y) {
  b.style.setProperty("--px", x);
  b.style.setProperty("--py", y);
  b.dataset.press = b.dataset.press === "a" ? "b" : "a";
}
const target = (e) => (!calm.matches && e.target instanceof Element && e.target.closest("button")) || null;

document.addEventListener("pointerdown", (e) => {
  const b = e.button === 0 && target(e);
  if (!b || b.disabled || b.closest(SKIP)) return;
  const r = b.getBoundingClientRect();
  mark(b, `${e.clientX - r.left}px`, `${e.clientY - r.top}px`);
}, { capture: true, passive: true });
document.addEventListener("click", (e) => {        // Enter or Space: from the middle
  const b = e.detail === 0 && target(e);
  if (b && !b.closest(SKIP)) mark(b, "50%", "50%");
}, { capture: true, passive: true });
document.addEventListener("touchstart", () => {}, { passive: true });   // iOS Safari applies :active on a tap only with a listener
