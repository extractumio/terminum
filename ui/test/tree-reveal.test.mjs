// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// An older reveal never replaces a selection or cursor the user picked after it started (AC-62):
// Reveal in tree, ← on a file, a new file's reveal and a re-root's restore. Folders opening or
// refreshing meanwhile do not stop it.
import assert from "node:assert/strict";
import { test } from "node:test";
import { load, stubs, tick } from "./bundle.mjs";

Object.defineProperty(globalThis, "localStorage", { value: { getItem: () => null }, configurable: true });
const { Tree } = await load("src/tree.ts");
const { TreeInput } = await load("src/tree-input.ts");

const LISTINGS = { "/r": [["a", "d"], ["b.rs", "f"], ["c.rs", "f"]], "/r/a": [["x.rs", "f"], ["y.rs", "f"]] };
const fill = (n) => {
  const rows = LISTINGS[n.path] ?? [];
  Object.assign(n, { loaded: true, writable: true, total: rows.length, gen: 1 });
  n.pages.set(0, rows.map(([name, k]) => ({ n: name, k })));
};
const dir = (path) => { const n = { path, children: new Map(), pages: new Map(), pending: new Set() }; fill(n); return n; };

/** A Tree over /r whose folder lookups wait until `release()`; `input` is its real TreeInput. */
function fixture() {
  const root = dir("/r"), nodes = new Map([[root.path, root]]);
  let gate, release;
  const hold = () => { gate = new Promise((r) => { release = r; }); };
  hold();
  const t = Object.assign(Object.create(Tree.prototype), {
    root, nodes, expanded: new Set(), selected: new Set(), cursor: -1, anchor: -1, rowH: 20, version: 0,
    el: { clientHeight: 200, scrollTop: 0 }, epoch: 0, expandRun: 0, selectionRun: 0, picks: 0,
    edit: { cancel() {} }, cb: { changed() {} }, queue() {}, touch() { t.version++; },
    load: async (n) => fill(n),
    locate: async (n, name) => { await gate; return n.pages.get(0).findIndex((r) => r.n === name); },
  });
  t.rows = { get version() { return t.version; }, filter: "", get root() { return t.root; }, nodes, creatingIn: () => 0, load() {} };
  const key = (k) => input.onKey({ key: k, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, preventDefault() {} });
  const input = new TreeInput(t);
  return { t, input, key, release: async () => { release(); await tick(); }, hold };
}

test("Reveal in tree expands the file's folders and puts the cursor on it", async () => {
  const { t, release } = fixture();
  const pending = t.reveal("/r/a/y.rs", true, ++t.picks);
  await release();
  assert.equal(await pending, true);
  assert.deepEqual([...t.expanded], ["/r/a"]);
  assert.equal(t.cursor, 2);                              // a, x.rs, y.rs
});

test("a click or a key while the folders open or the lookup runs wins over an older reveal", async () => {
  for (const [what, pick] of [
    ["click", (f) => f.input.select("/r/c.rs", 2)],
    ["⌘-click", (f) => f.input.select("/r/c.rs", 2, "toggle")],
    ["↓", (f) => { f.t.cursor = 1; f.key("ArrowDown"); }],
  ]) {
    const f = fixture();
    const pending = f.t.reveal("/r/a/x.rs", true, ++f.t.picks);   // would put the cursor on row 1
    pick(f);
    await f.release();
    assert.equal(await pending, null, what);
    assert.equal(f.t.cursor, 2, what);
  }
});

test("a folder read or refreshed meanwhile does not stop Reveal in tree", async () => {
  const { t, release } = fixture();
  const pending = t.reveal("/r/a/y.rs", true, ++t.picks);
  const refresh = t.refreshDirs(["/r"]);                 // disk activity bumps selectionRun, not picks
  await release();
  await refresh;
  assert.equal(await pending, true);
  assert.equal(t.cursor, 2);
});

test("a newer Reveal in tree wins over an older one", async () => {
  const { t, release } = fixture();
  const older = t.reveal("/r/a/y.rs", true, ++t.picks);
  const newer = t.reveal("/r/c.rs", false, ++t.picks);
  await release();
  assert.equal(await older, null);
  assert.equal(await newer, true);
  assert.equal(t.cursor, 4);                              // a, x.rs, y.rs, b.rs, c.rs
});

test("a missing file is not found, and a re-root meanwhile is not a miss", async () => {
  const missing = fixture();
  const gone = missing.t.reveal("/r/zz.rs");
  await missing.release();
  assert.equal(await gone, false);
  const moved = fixture();
  const pending = moved.t.reveal("/r/b.rs");
  moved.t.epoch++;
  await moved.release();
  assert.equal(await pending, null);
  assert.equal(moved.t.cursor, -1);
});

test("← on a file selects its folder, but not after a newer click", async () => {
  for (const clicked of [false, true]) {
    const f = fixture(), a = dir("/r/a");
    f.t.expanded.add(a.path);
    f.t.nodes.set(a.path, a);
    f.t.root.children.set("a", { idx: 0, node: a });
    f.t.cursor = 2;                                       // y.rs
    f.key("ArrowLeft");
    if (clicked) f.input.select("/r/c.rs", 4);
    await f.release();
    assert.deepEqual([...f.t.selected], [clicked ? "/r/c.rs" : "/r/a"], `clicked: ${clicked}`);
    assert.equal(f.t.cursor, clicked ? 4 : 0, `clicked: ${clicked}`);
  }
});

test("a re-root's restore leaves the cursor where the user clicked while it loaded", async () => {
  for (const clicked of [false, true]) {
    const f = fixture();
    let loaded;
    Object.assign(f.t, { render() {}, load: (n) => new Promise((r) => { loaded = () => { fill(n); r(); }; }) });
    const restore = f.t.setRoot("/r", [], ["/r/c.rs"]);
    if (clicked) f.input.select("/r/b.rs", 1);
    loaded();
    await f.release();
    await restore;
    assert.equal(f.t.cursor, clicked ? 1 : 2, `clicked: ${clicked}`);
  }
});

const { commitPath } = await load("src/file-actions.ts", { plugins: [stubs({
  "./api": `export const api = async () => ({ path: "/r/new.txt" }); export class ApiError extends Error {}
    export const basename = (p) => p.slice(p.lastIndexOf("/") + 1); export const toast = () => {};`,
  "./dialogs": "export const ask = async () => null;",
})] });

test("a new file is selected and revealed, unless the user picked something while it was made", async () => {
  for (const clicked of [false, true]) {
    let refreshed;
    const revealed = [];
    const tree = {
      picks: 0, selected: new Set(["/r/b.rs"]),
      refreshDirs: () => new Promise((r) => { refreshed = r; }),
      reveal: async (...args) => { revealed.push(args); return true; },
    };
    const pending = commitPath("file", "/r", "new.txt", undefined, { host: null, current: () => true, tree, viewer: {}, open() {}, changed() {} });
    await tick();
    if (clicked) { tree.picks++; tree.selected = new Set(["/r/c.rs"]); }
    refreshed();
    assert.equal(await pending, null);
    assert.deepEqual([...tree.selected], [clicked ? "/r/c.rs" : "/r/new.txt"], `clicked: ${clicked}`);
    assert.deepEqual(revealed, clicked ? [] : [["/r/new.txt", false, 1]], `clicked: ${clicked}`);
  }
});
