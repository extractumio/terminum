// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Folded cards (terminum-kanban.md AC-159): TODO's and DONE's are a line each, ID, severity and title; a press
// shows one whole and folds the one before; a press on it then opens its window; Esc folds it.

const FOLDED = new Set(["TODO", "DONE"]);

/** `open(id)`: a task's window. */
export function foldedCards(open) {
  let whole = null;                                // the one folded card shown whole
  return {
    /** What a new card of `t` carries. */
    mark(c, t) {
      if (!FOLDED.has(t.status)) return;
      c.classList.add("compact");
      c.classList.toggle("open", whole === t.id);
      c.setAttribute("aria-expanded", String(whole === t.id));
    },
    /** A press on card `c`. */
    press(c) {
      if (!c.matches(".compact:not(.open)")) return open(c.dataset.id);
      this.fold();
      c.classList.add("open");
      c.setAttribute("aria-expanded", "true");
      whole = c.dataset.id;
    },
    fold() {
      const c = document.querySelector(".card.compact.open");
      c?.classList.remove("open");
      c?.setAttribute("aria-expanded", "false");
      whole = null;
    },
    /** The columns as read: one that left TODO and DONE is a line again if it comes back. */
    keep(cards) {
      if (whole && ![...FOLDED].some((c) => cards.get(c).some((t) => t.id === whole))) whole = null;
    },
  };
}
