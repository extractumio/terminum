// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// An IDE's keys on the page (AC-07): ⌘[ and ⌘] back and forward through the sessions shown
// (recent.js), ⌘F the search in the terminal's history (find.js), ⌘B Files open or closed;
// on the terminal's input ⌘Z and ⌘⇧Z undo and redo (undo.js; another field keeps its own).
// Taken before the terminal and the browser (Back, Forward, Find, its bookmarks bar); each
// action says whether it did something, else the browser keeps the key.

/** actions: {back, forward, find, files}, each () => boolean; undo(redo) on the terminal's input. */
export function ideKeys(actions) {
  const BY = { BracketLeft: "back", BracketRight: "forward", KeyF: "find", KeyB: "files" };
  document.addEventListener("keydown", (e) => {
    if (e.code === "KeyZ" && e.metaKey && !e.ctrlKey && !e.altKey) {
      // the terminal's input only: another field keeps its own undo, the list or a button none
      if (e.target.id !== "kbd" && !e.target.closest?.("#term")) return;
      e.preventDefault();
      e.stopPropagation();
      actions.undo(e.shiftKey);
      return;
    }
    const name = BY[e.code];
    if (!name || !e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
    if (!(e.repeat && name !== "back" && name !== "forward") && !actions[name]()) return;   // held: once
    e.preventDefault();
    e.stopPropagation();
  }, true);
}
