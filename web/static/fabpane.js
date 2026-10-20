// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A floating button's pane (the sent history's, the reminders', the bookmarks', find's field):
// the button opens and closes it without taking the keys from the terminal; Esc and the pane's ×
// close it; it stands left of its button, its first row centered on the button, or under it
// where the view is too narrow (then `data-under` is set: the buttons below make way).

const GAP = 14;                // px between a button and its pane

/** Places `pane` beside `fab` (nothing while it is hidden). */
export function placeBeside(fab, pane) {
  if (pane.hidden) return;
  const area = fab.offsetParent;
  if (area && fab.offsetTop > area.clientHeight / 2) {      // a button at the bottom (a phone's row): the pane above it
    pane.style.top = `${Math.max(8, fab.offsetTop - 8 - pane.offsetHeight)}px`;
    pane.style.left = `${Math.max(8, Math.min(fab.offsetLeft, area.clientWidth - pane.offsetWidth - 8))}px`;
    delete pane.dataset.under;
    return;
  }
  const left = fab.offsetLeft - GAP - pane.offsetWidth, fits = left >= 8;
  // its first row shown (a header, the find field) level with the button's middle
  const head = [...pane.children].find((c) => c.offsetHeight > 0);
  const mid = head ? head.offsetTop + head.offsetHeight / 2 : fab.offsetHeight / 2;
  pane.style.top = `${fits ? Math.max(8, Math.round(fab.offsetTop + fab.offsetHeight / 2 - mid)) : fab.offsetTop + fab.offsetHeight + 8}px`;
  pane.style.left = `${fits ? left : Math.max(8, fab.offsetLeft + fab.offsetWidth - pane.offsetWidth)}px`;
  if (fits) delete pane.dataset.under; else pane.dataset.under = "";
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
