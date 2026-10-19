// The sessions shown last (AC-07): Back and Forward as an IDE's.
import { test } from "node:test";
import assert from "node:assert/strict";
import { recentSessions } from "../static/recent.js";

function make(kept = [], gone = new Set()) {
  const saved = { recent: kept }, shown = [];
  const store = { get: (k, d) => saved[k] ?? d, set: (k, v) => { saved[k] = v; } };
  const r = recentSessions({ store, exists: (id) => !gone.has(id), show: (id) => { shown.push(id); r.shown(id); } });
  return { r, shown, saved };
}

test("back and forward walk the sessions shown; a pick drops what was ahead", () => {
  const { r, shown, saved } = make();
  for (const id of ["a", "b", "c"]) r.shown(id);
  assert.ok(r.back() && r.back());
  assert.equal(r.back(), false, "nothing before the first");
  assert.deepEqual(shown, ["b", "a"]);
  assert.ok(r.forward());
  r.shown("d");
  assert.equal(r.forward(), false, "a pick ends what was ahead");
  assert.deepEqual(saved.recent, ["a", "b", "d"]);
});

test("a session shown again moves to the end; at most 20; gone panes are skipped", () => {
  const { r, shown, saved } = make([], new Set(["b"]));
  for (let i = 0; i < 25; i++) r.shown(`p${i}`);
  assert.equal(saved.recent.length, 20);
  assert.equal(saved.recent[0], "p5");
  r.shown("p10");
  assert.equal(saved.recent.at(-1), "p10");
  assert.equal(saved.recent.filter((x) => x === "p10").length, 1);
  const g = make(["a", "b", "c"], new Set(["b"]));
  assert.ok(g.r.back());
  assert.deepEqual(g.shown, ["a"], "b is gone: back goes to a");
});
