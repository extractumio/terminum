// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Re-finding a re-read folder's expanded children (Pager.relocate) against a stand-in api whose
// answers the test releases: a relocate of an older read never acts after a newer one.
import assert from "node:assert/strict";
import { test } from "node:test";
import { DEFERRED_API, load, stubs, tick } from "./bundle.mjs";

const { Pager } = await load("src/tree-load.ts", { plugins: [stubs({
  "./api": `export const join = (d, n) => d + "/" + n;
    export const basename = (p) => p.split("/").pop();
    export const dirname = (p) => p.slice(0, p.lastIndexOf("/"));
    ${DEFERRED_API}`,
})] });
const listing = (gen, names) => ({ status: "ok", gen, total: 2, writable: true, entries: names.map((n) => ({ n })) });

function setup() {
  globalThis.__calls = [];
  const collapsed = [];
  const n = { path: "/r", loaded: true, writable: true, total: 2, gen: 1, pages: new Map(), pending: new Set(),
    children: new Map([["a", { idx: 1, node: {} }]]) };
  const pager = new Pager({ epoch: 0, query: (x) => x, collapse: (p) => collapsed.push(p), touch() {} });
  return { n, pager, collapsed };
}

test("an older relocate answering after a newer refresh does not collapse the child", async () => {
  const { n, pager, collapsed } = setup();
  const first = pager.load(n, 0);
  globalThis.__calls[0].resolve(listing(2, []));              // re-read: "a" is not on the page, so it is asked for
  await first; await tick();
  assert.equal(globalThis.__calls[1].args[2].query.locate, "a");
  const refresh = pager.load(n, 0, true);
  globalThis.__calls[2].resolve(listing(3, ["b", "a"]));      // the newer read finds it at once
  await refresh; await tick();
  assert.equal(n.children.get("a").idx, 1);
  globalThis.__calls[1].resolve({ located: null });           // the older read's late answer: "not found"
  await tick();
  assert.deepEqual(collapsed, []);
  assert.equal(n.children.get("a").idx, 1);
});

test("a relocate still collapses a child that is gone, and moves one that shifted", async () => {
  const { n, pager, collapsed } = setup();
  n.children.set("c", { idx: 0, node: {} });
  const read = pager.load(n, 0);
  globalThis.__calls[0].resolve(listing(2, ["c"]));
  await read; await tick();
  globalThis.__calls[1].resolve({ located: null });
  await tick();
  assert.deepEqual(collapsed, ["/r/a"]);
  assert.equal(n.children.get("c").idx, 0);
});
