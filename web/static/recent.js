// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The sessions shown last (AC-07): up to 20, kept in this browser; ⌘[ goes back through them
// and ⌘] forward again, as an IDE's Back and Forward. Showing another session drops what was
// ahead and moves it to the end; a pane that is gone is skipped.
const MOST = 20, KEY = "recent";

/** store: the page's; exists(id): the pane is listed; show(id): shows it. */
export function recentSessions({ store, exists, show }) {
  let list = store.get(KEY, []).filter((x) => typeof x === "string").slice(-MOST);
  let at = list.length - 1, moving = false;
  function go(step) {
    for (let i = at + step; i >= 0 && i < list.length; i += step) {
      if (!exists(list[i])) continue;
      at = i;
      moving = true;
      try { show(list[i]); } finally { moving = false; }
      return true;
    }
    return false;
  }
  return {
    /** A session was shown: by a pick it goes last; by Back or Forward the place moves only. */
    shown(id) {
      if (moving || !id || list[at] === id) return;
      list = [...list.slice(0, at + 1).filter((x) => x !== id), id].slice(-MOST);
      at = list.length - 1;
      store.set(KEY, list);
    },
    back: () => go(-1),
    forward: () => go(1),
  };
}
