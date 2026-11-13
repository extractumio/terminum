// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// ← on a file selects its folder once the tree revealed it, and only then: not when the reveal
// failed. A newer selection meanwhile: tree-reveal.test.mjs, with the real reveal.
import assert from "node:assert/strict";
import { test } from "node:test";
import { load } from "./bundle.mjs";

const { TreeInput } = await load("src/tree-input.ts");

function setup() {
  const root = { path: "/r", loaded: true, total: 1, children: new Map(), pages: new Map(), pending: new Set(), gen: 1 };
  let finish;
  const t = {
    root, cursor: 3, anchor: 3, rowH: 20, el: { clientHeight: 200 }, expanded: new Set(), selected: new Set(["/r/src/a.rs"]),
    selectionRun: 0, picks: 0, rows: { version: 0, filter: "", root, nodes: new Map(), creatingIn: () => 0, load() {} },
    cb: { changed() {} }, queue() {},
    descAt: () => ({ isDir: false, p: "/r/src/a.rs", d: { dir: { path: "/r/src" } } }),
    reveal: () => new Promise((r) => { finish = r; }),
  };
  const input = new TreeInput(t);
  const left = () => input.onKey({ key: "ArrowLeft", metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, preventDefault() {} });
  return { t, left, finish: async (found) => { finish(found); await new Promise((r) => setTimeout(r, 0)); } };
}

test("← on a file selects its folder once it is revealed", async () => {
  const { t, left, finish } = setup();
  left();
  await finish(true);
  assert.deepEqual([...t.selected], ["/r/src"]);
});

test("← does not select the old folder when the reveal failed (a new root meanwhile)", async () => {
  const { t, left, finish } = setup();
  left();
  t.selected = new Set(["/other/x"]);        // the new root's own selection
  await finish(false);
  assert.deepEqual([...t.selected], ["/other/x"]);
});
