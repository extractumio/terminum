// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The device's on-screen keyboard and the hot keys panel are one or the other (AC-18): both up
// leave no room for the line being typed. The page cannot ask whether the on-screen keyboard
// shows; the visible height says it: while the keyboard field has the keys, the view is far
// shorter than its full height (an iPad's hardware keyboard only adds its shortcut bar, ~60 px).
// The keyboard's button is lit only while the on-screen keyboard is up. A key that comes while the
// field has the keys and none is up is a hardware keyboard's: body.hwkbd until the field lets the
// keys go or an on-screen keyboard comes up (style.css hides the keyboard's and hot keys' buttons).
export const KEYBOARD_PX = 150;   // shorter than this: no on-screen keyboard (a shortcut bar, a URL bar)

/** Watches `view` (a visualViewport): closes the panel (show(false)) when the on-screen keyboard
 *  comes up; toggle() opens the panel, putting the keyboard away first, or closes it. `button`: the
 *  keyboard's button, lit while the on-screen keyboard is up; `mark(on)`: a hardware keyboard types. */
export function oneKeyboard({ kbd, panel, show, button = null, view = globalThis.visualViewport, win = globalThis,
  focused = () => document.activeElement === kbd, mark = (on) => document.body.classList.toggle("hwkbd", on) }) {
  let full = 0, width = -1, hard = false;
  const up = () => {
    if (!view) return false;
    if (win.innerWidth !== width) { width = win.innerWidth; full = 0; }   // turned or resized: a new full height
    full = Math.max(full, view.height);
    return focused() && full - view.height > KEYBOARD_PX;
  };
  const paint = (soft = up()) => {
    if (soft || !focused()) hard = false;
    mark(hard);
    button?.setAttribute("aria-pressed", String(soft));
  };
  const check = () => { const soft = up(); if (soft && !panel.hidden) show(false); paint(soft); };
  view?.addEventListener("resize", check);
  kbd.addEventListener?.("focus", () => paint());
  kbd.addEventListener?.("blur", () => paint());
  kbd.addEventListener?.("keydown", () => { if (!hard && focused() && !up()) { hard = true; paint(false); } });
  return {
    up,
    toggle() {
      if (!panel.hidden) return show(false);
      if (up()) kbd.blur();                                           // the on-screen keyboard goes, the panel comes
      show(true);
    },
  };
}
