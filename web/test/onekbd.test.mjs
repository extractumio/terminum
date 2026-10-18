// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The on-screen keyboard and the hot keys panel are one or the other (AC-18).
import assert from "node:assert/strict";
import test from "node:test";
import { oneKeyboard } from "../static/onekbd.js";

/** A page: its visible height, the keyboard field's focus, the panel. */
function page({ height = 800, width = 390 } = {}) {
  const listeners = [];
  const view = { height, addEventListener: (_, f) => listeners.push(f) };
  const win = { innerWidth: width };
  const state = { focused: false, blurred: 0, panel: { hidden: true } };
  const kbd = { blur: () => { state.blurred++; state.focused = false; view.height = 800; } };
  const show = (on) => { state.panel.hidden = !on; };
  const k = oneKeyboard({ kbd, panel: state.panel, show, view, win, focused: () => state.focused });
  const resize = (h) => { view.height = h; for (const f of listeners) f(); };
  return { k, state, resize, win };
}

test("the on-screen keyboard coming up closes the panel", () => {
  const { k, state, resize } = page();
  k.toggle();
  assert.equal(state.panel.hidden, false);
  state.focused = true;                 // a tap on the terminal: the field has the keys
  resize(470);                          // and the keyboard takes 330 px
  assert.equal(state.panel.hidden, true);
});

test("opening the panel while the on-screen keyboard is up puts the keyboard away", () => {
  const { k, state, resize } = page();
  k.up();                               // the full height, seen once
  state.focused = true;
  resize(470);
  k.toggle();
  assert.deepEqual([state.blurred, state.panel.hidden], [1, false]);
  k.toggle();
  assert.equal(state.panel.hidden, true, "and closes it again");
});

test("a hardware keyboard (an iPad's shortcut bar only) keeps the panel open while typing", () => {
  const { k, state, resize } = page({ height: 1000, width: 1024 });
  k.toggle();
  state.focused = true;
  resize(940);                          // the shortcut bar: 60 px
  assert.equal(state.panel.hidden, false);
  k.toggle(); k.toggle();               // closed and opened again: no blur, the typing goes on
  assert.deepEqual([state.blurred, state.panel.hidden], [0, false]);
});

test("turning the device sets a new full height: a shorter landscape view is no keyboard", () => {
  const { k, state, resize, win } = page({ height: 800, width: 390 });
  k.up();
  state.focused = true;
  win.innerWidth = 844;                 // landscape: shorter, by far, yet no keyboard
  resize(360);
  k.toggle();
  assert.deepEqual([state.blurred, state.panel.hidden], [0, false]);
});
