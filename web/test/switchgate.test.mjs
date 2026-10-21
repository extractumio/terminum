// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Another pane opened (AC-04): its messages wait for its first frame, then go at once, in order.
import { test } from "node:test";
import assert from "node:assert/strict";
import { MOST, switchGate } from "../static/switchgate.js";

function gate() {
  const out = { released: [], resets: 0, timers: [] };
  const g = switchGate({
    release: (queue, reset) => { reset(); out.resets++; out.released.push(queue.map((m) => m.n)); },
    later: (fn, ms) => { out.timers.push({ fn, ms, on: true }); return out.timers.length - 1; },
    cancel: (i) => { if (out.timers[i]) out.timers[i].on = false; },
  });
  return { g, out };
}

test("the new pane's messages wait until its first frame, then all go in order after one reset", () => {
  const { g, out } = gate();
  g.start("B", () => {});
  assert.equal(g.take({ t: "theme", sid: "B", n: 0 }), true);
  assert.equal(g.take({ t: "hist", sid: "B", n: 1 }), true);
  assert.deepEqual(out.released, []);
  assert.equal(g.take({ t: "screen", sid: "B", n: 2 }), true);
  assert.deepEqual(out.released, [[0, 1, 2]]);
  assert.equal(out.resets, 1);
  assert.equal(g.holding(), false);
  assert.equal(g.take({ t: "screen", sid: "B", n: 3 }), false, "after it, frames go as they come");
});

test("messages of another pane, or without one, never wait; the pane's error ends the wait", () => {
  const { g, out } = gate();
  g.start("B", () => {});
  assert.equal(g.take({ t: "screen", sid: "A" }), false);
  assert.equal(g.take({ t: "layout", groups: [] }), false);
  g.take({ t: "error", sid: "B", n: 6, msg: "" });
  assert.deepEqual(out.released, [], "an empty error (tmux is back) waits on");
  g.take({ t: "error", sid: "B", n: 7, msg: "This pane is closed" });
  assert.deepEqual(out.released, [[6, 7]], "a pane gone: said at once");
});

test("it waits 1.5 s at most; a pane opened meanwhile replaces the one waiting, never drawn", () => {
  const { g, out } = gate();
  g.start("B", () => {});
  g.take({ t: "hist", sid: "B", n: 1 });
  assert.equal(out.timers[0].ms, MOST);
  assert.equal(MOST, 1500);
  g.start("C", () => {});
  assert.equal(out.timers[0].on, false, "the first wait's timer is gone");
  g.take({ t: "hist", sid: "C", n: 2 });
  out.timers[1].fn();
  assert.deepEqual(out.released, [[2]], "only C's, after its timeout");
});
