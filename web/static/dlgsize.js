// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A dialog's parent, for its size (dialogsize.css): as it opens, the sized dialog open under it
// (RUN over a task's window; the bookmarks' list at the window's side is none), else the element its data-parent names (the terminal, for a bookmark,
// Ask, Paste), else the window. Its parent's size is read once, as it opens: a dialog never
// resizes under the user's hands while they type.

const opened = [];                               // the dialogs in the order they opened, the last on top

function sized(d) {
  const under = opened.filter((o) => o.open && o !== d && o.dataset.size).at(-1);   // a window, not a side list (#marksdlg)
  opened.splice(0, opened.length, ...opened.filter((o) => o.open && o !== d), d);
  const parent = under ?? document.getElementById(d.dataset.parent ?? "");
  const shown = parent?.offsetWidth > 0;         // laid out, its size without its pop's scale
  d.style.setProperty("--pw", shown ? `${parent.offsetWidth}px` : "");
  d.style.setProperty("--ph", shown ? `${parent.offsetHeight}px` : "");
}

new MutationObserver((records) => {
  for (const { target } of records) if (target.matches("dialog.dlg") && target.open) sized(target);
}).observe(document.documentElement, { attributes: true, attributeFilter: ["open"], subtree: true });
