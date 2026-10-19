// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A floating button's pane (the sent history's, the reminders'): the button opens and closes
// it without taking the keys from the terminal; Esc and the pane's × close it; it stands left
// of its button, level with it, or under it where the view is too narrow.

/** Places `pane` beside `fab` (nothing while it is hidden). */
export function placeBeside(fab, pane) {
  if (pane.hidden) return;
  const left = fab.offsetLeft - 8 - pane.offsetWidth, fits = left >= 8;
  pane.style.top = `${fits ? fab.offsetTop : fab.offsetTop + fab.offsetHeight + 8}px`;
  pane.style.left = `${fits ? left : Math.max(8, fab.offsetLeft + fab.offsetWidth - pane.offsetWidth)}px`;
}

/** fab, pane, close: elements; onToggle(open) after it opened or closed. {toggle(on?), open(), place()}. */
export function fabPane({ fab, pane, close, onToggle }) {
  let open = false;
  function toggle(on = !open) {
    open = on;
    pane.hidden = !on;
    fab.setAttribute("aria-expanded", String(on));
    onToggle(on);
    place();
  }
  const place = () => placeBeside(fab, pane);
  fab.addEventListener("click", () => toggle());
  fab.addEventListener("mousedown", (e) => e.preventDefault());   // the terminal keeps the keys
  close.addEventListener("click", () => toggle(false));
  pane.addEventListener("keydown", (e) => { if (e.key === "Escape") { e.preventDefault(); toggle(false); } });
  return { toggle, open: () => open, place };
}
