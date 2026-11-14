// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The charts' scale (charts.js; terminum.md AC-151, AC-152): a "nice" top and its ticks.
import assert from "node:assert/strict";
import { test } from "node:test";
import { niceTicks, pointer, trend } from "../static/charts.js";

test("AC-152 a nice top over the largest value, 1, 2 or 5 times a power of ten apart", () => {
  assert.deepEqual(niceTicks(87), { top: 100, ticks: [0, 50, 100] });
  assert.deepEqual(niceTicks(612), { top: 800, ticks: [0, 200, 400, 600, 800] });
  assert.deepEqual(niceTicks(3), { top: 3, ticks: [0, 1, 2, 3] });
  assert.deepEqual(niceTicks(0.4), { top: 0.4, ticks: [0, 0.1, 0.2, 0.3, 0.4] });
  assert.deepEqual(niceTicks(100), { top: 100, ticks: [0, 50, 100] }, "a round top stays");
});

test("AC-152 nothing to scale: one line at zero", () => {
  assert.deepEqual(niceTicks(0), { top: 1, ticks: [0] });
  assert.deepEqual(niceTicks(Number.NaN), { top: 1, ticks: [0] });
});

test("AC-152 a trend: least squares over the columns but the one going on, its way, flat under 5% of the mean", () => {
  const up = trend([1, 2, 3, 4, 5, 0]);
  assert.equal(up.dir, 1, "the last column (going on) is left out");
  assert.deepEqual([up.from, up.to, up.at(0), up.at(4)], [0, 4, 1, 5]);
  assert.equal(trend([9, 7, 5, 3, 1, 99]).dir, -1);
  assert.equal(trend([100, 101, 100, 102, 101, 0]).dir, 0, "a 1% move is flat");
  assert.equal(trend([0, 0, 0, 0, 0, 0]).dir, 0, "nothing moves");
});

test("AC-152 a trend skips what was not measured, and needs 4 columns", () => {
  const t = trend([null, 2, null, 4, 6, 8, 1]);
  assert.deepEqual([t.dir, t.from, t.to], [1, 1, 5], "nulls are no zeros");
  assert.equal(trend([1, 2, 3, 4]), null, "3 finished columns");
  assert.equal(trend([1, null, 2, null, 3, 4]), null);
  assert.equal(trend([]), null);
});

/** A plot's box, its svg and its columns, as much of the DOM as pointer() touches. */
function plot(n) {
  const marks = Array.from({ length: n }, () => { const on = new Set(); return { on, classList: { add: (c) => on.add(c), remove: (c) => on.delete(c) } }; });
  return { box: { dataset: {} }, s: { addEventListener() {} }, marks };
}

test("AC-152 a column chosen by the keys stays chosen when its focused plot is drawn anew, Enter still opens it", async () => {
  const { box, s, marks } = plot(14), said = [], opened = [];
  globalThis.document = { activeElement: box };
  try {
    pointer(box, s, marks, { at: () => -1, select: (i) => said.push(i), open: (i) => opened.push(i) });
    box.onkeydown({ key: "End", preventDefault() {} });
    assert.deepEqual(said, [13]);
    const again = plot(14).marks;   // a scrollbar came or went: the same box, its columns drawn anew
    pointer(box, s, again, { at: () => -1, select: (i) => said.push(i), open: (i) => opened.push(i) });
    await null;
    assert.deepEqual(said, [13, 13], "chosen again, said again");
    assert.ok(again[13].on.has("on"));
    box.onkeydown({ key: "Enter", preventDefault() {} });
    assert.deepEqual(opened, [13]);
    box.onblur();
    pointer(box, s, plot(14).marks, { at: () => -1, select: (i) => said.push(i) });
    await null;
    assert.deepEqual(said, [13, 13, -1], "let go, a later drawing chooses nothing");
    box.onkeydown({ key: "Home", preventDefault() {} });
    pointer(box, s, plot(12).marks, { at: () => -1, select: (i) => said.push(i) });   // a heat map's 24 columns become 12
    await null;
    assert.deepEqual(said, [13, 13, -1, 0], "a plot of another shape keeps nothing");
    globalThis.document = { activeElement: null };
    box.dataset.chosen = "5/14";
    pointer(box, s, plot(14).marks, { at: () => -1, select: (i) => said.push(i) });
    await null;
    assert.deepEqual(said, [13, 13, -1, 0], "a plot not focused keeps nothing");
  } finally { delete globalThis.document; }
});
