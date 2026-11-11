// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A dialog's parent, for its size (dialogsize.css): as it opens, the dialog open under it (RUN
// over a task's window), else the element its data-parent names (the terminal, for a bookmark,
// Ask, Paste), else the window. Its parent's size is read once, as it opens: a dialog never
// resizes under the user's hands while they type.

let opened = 0;
const order = new WeakMap();                     // dialog -> when it opened, to find the one under a new one

function sized(d) {
  // the last opened before it (one opened in the same moment, after it, is not under it)
  const under = [...document.querySelectorAll("dialog.dlg[open]")].filter((o) => o !== d && order.has(o))
    .sort((a, b) => order.get(b) - order.get(a))[0];
  order.set(d, ++opened);
  const parent = under ?? (d.dataset.parent && document.getElementById(d.dataset.parent));
  const shown = parent && parent.offsetWidth > 0;   // laid out, its size without its pop's scale
  for (const [v, n] of [["--pw", shown && parent.offsetWidth], ["--ph", shown && parent.offsetHeight]]) {
    if (n) d.style.setProperty(v, `${n}px`);
    else d.style.removeProperty(v);
  }
}

new MutationObserver((records) => {
  for (const { target } of records) if (target.matches("dialog.dlg") && target.open) sized(target);
}).observe(document.documentElement, { attributes: true, attributeFilter: ["open"], subtree: true });
