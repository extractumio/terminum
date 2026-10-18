// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The device's on-screen keyboard and the hot keys panel are one or the other (AC-18): both up
// leave no room for the line being typed. The page cannot ask whether the on-screen keyboard
// shows; the visible height says it: while the keyboard field has the keys, the view is far
// shorter than its full height (an iPad's hardware keyboard only adds its shortcut bar, ~60 px).
export const KEYBOARD_PX = 150;   // shorter than this: no on-screen keyboard (a shortcut bar, a URL bar)

/** Watches `view` (a visualViewport): closes the panel (show(false)) when the on-screen keyboard
 *  comes up; toggle() opens the panel, putting the keyboard away first, or closes it. */
export function oneKeyboard({ kbd, panel, show, view = globalThis.visualViewport, win = globalThis, focused = () => document.activeElement === kbd }) {
  let full = 0, width = -1;
  const up = () => {
    if (!view) return false;
    if (win.innerWidth !== width) { width = win.innerWidth; full = 0; }   // turned or resized: a new full height
    full = Math.max(full, view.height);
    return focused() && full - view.height > KEYBOARD_PX;
  };
  const check = () => { if (up() && !panel.hidden) show(false); };
  view?.addEventListener("resize", check);
  return {
    up,
    toggle() {
      if (!panel.hidden) return show(false);
      if (up()) kbd.blur();                                           // the on-screen keyboard goes, the panel comes
      show(true);
    },
  };
}
