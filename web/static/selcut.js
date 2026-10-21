// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Backspace or Delete over a selection in a coding agent's input deletes it, as in a text field
// (AC-40): the keys come from cut() (boxnav.js), the selection goes and the keys go to the input
// again. A selection elsewhere in the terminal leaves the key as it was: to the input when it has
// the keys, else nothing (and never WebKit's Backspace back a page).

const field = (el) => !!el?.closest?.("input, textarea:not(#kbd), select, [contenteditable]");

/** term: the terminal; cut(range): the keys that delete the range's text, or null; send(keys);
 *  focus(): the input takes the keys again. */
export function selectionCut({ term, cut, send, focus }) {
  document.addEventListener("keydown", (e) => {
    if ((e.key !== "Backspace" && e.key !== "Delete") || e.metaKey || e.ctrlKey || e.altKey || e.shiftKey || e.isComposing) return;
    if (field(e.target) || term.hidden || document.querySelector("dialog[open]")) return;
    const sel = getSelection();
    if (!sel?.rangeCount || sel.isCollapsed || !term.contains(sel.getRangeAt(0).commonAncestorContainer)) return;
    const keys = cut(sel.getRangeAt(0));
    if (!keys) {
      if (!document.activeElement || document.activeElement === document.body) e.preventDefault();   // WebKit would go back a page
      return;
    }
    e.preventDefault();
    e.stopPropagation();                          // the input's own handler would delete one more
    sel.removeAllRanges();
    send(keys);
    focus();
  }, true);
}
