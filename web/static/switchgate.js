// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Another pane opened (AC-04): the view keeps the pane shown before, still, until the new pane's
// first frame, then draws the new one at once. terminumd sizes the window to the grid sent with
// "sub" before that frame, so the new pane never shows at the window's old size first (scaled)
// and jumps, nor a blank view. Its messages wait meanwhile and go in order, in one task: the
// browser paints only where they end.

export const MOST = 1500;      // ms a switch waits at most (a frame that does not come)

/** release(queue, reset): the messages that waited, after reset() made the view ready for them. */
export function switchGate({ release, later = setTimeout, cancel = clearTimeout }) {
  let held = null;
  const open = () => {
    const h = held;
    held = null;
    if (h) { cancel(h.timer); release(h.queue, h.reset); }
  };
  return {
    /** Pane `sid` was opened: its messages wait; `reset` readies the view when they go. A pane
     *  opened before this one and still waiting is never drawn. */
    start(sid, reset) {
      if (held) cancel(held.timer);
      held = { sid, reset, queue: [] };
      held.timer = later(open, MOST);
    },
    /** True when `m` waits (it is the new pane's); its first frame ends the wait. */
    take(m) {
      if (!held || !m.sid || m.sid !== held.sid) return false;
      held.queue.push(m);
      if (m.t === "screen" || (m.t === "error" && m.msg)) open();       // an empty one: tmux is back
      return true;
    },
    holding: () => !!held,
  };
}
