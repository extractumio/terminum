// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-16: a panel of another build reloads itself only once its workspace is saved: a save that
// fails (said by the panel) holds the reload, and a later attempt that saves lets it happen.
import assert from "node:assert/strict";
import { test } from "node:test";
import { load } from "./bundle.mjs";

const stubs = {
  name: "stubs",
  setup(b) {
    b.onResolve({ filter: /^\.\/api$/ }, (a) => ({ path: a.path, namespace: "stub" }));
    b.onLoad({ filter: /.*/, namespace: "stub" }, () => ({
      loader: "js",
      contents: `const s = new Map(); export const session = { get: (k) => s.get(k) ?? null, set: (k, v) => s.set(k, v) };`,
    }));
  },
};
let ticks = [];
globalThis.window = { addEventListener() {}, setTimeout() {}, setInterval: (fn) => { ticks.push(fn); return ticks.length; } };
globalThis.clearInterval = (id) => { ticks[id - 1] = null; };
let reloads = 0;
globalThis.location = { reload: () => reloads++ };
const { Upgrade } = await load("src/upgrade.ts", { plugins: [stubs], define: { __BUILD__: '"old"' } });
const settle = () => new Promise((r) => setTimeout(r, 0));
const tick = async () => { for (const fn of ticks) fn?.(); await settle(); };

test("a failed save of the workspace holds the reload and says so; a later save lets it happen", async () => {
  ticks = []; reloads = 0;
  const said = [], saves = [false, false, true];
  const up = new Upgrade({ busy: () => false, beforeReload: async () => saves.shift(), waiting: (t) => said.push(t) });
  up.build("b1");
  await settle();
  assert.equal(reloads, 0);
  assert.deepEqual(said, ["Update ready — reloads once the panel state is saved"]);
  await tick();
  assert.equal(reloads, 0, "still not saved");
  await tick();
  assert.equal(reloads, 1);
  await tick();
  assert.equal(reloads, 1, "one reload: the timer is stopped");
});

test("a save that throws counts as not saved", async () => {
  ticks = []; reloads = 0;
  let calls = 0;
  const up = new Upgrade({ busy: () => false, beforeReload: async () => { if (!calls++) throw new Error("gone"); return true; }, waiting() {} });
  up.build("b2");
  await settle();
  assert.equal(reloads, 0);
  await tick();
  assert.equal(reloads, 1);
});

test("a save still running is not started again by the next attempt", async () => {
  ticks = []; reloads = 0;
  let calls = 0, finish;
  const up = new Upgrade({ busy: () => false, beforeReload: () => { calls++; return new Promise((r) => { finish = r; }); }, waiting() {} });
  up.build("b3");
  await tick(); await tick();
  assert.equal(calls, 1);
  finish(true); await settle();
  assert.equal(reloads, 1);
});

test("an edit begun while the save ran holds the reload until it is saved", async () => {
  ticks = []; reloads = 0;
  let busy = false, calls = 0;
  const up = new Upgrade({ busy: () => busy, beforeReload: async () => { if (!calls++) busy = true; return true; }, waiting() {} });
  up.build("b4");
  await settle();
  assert.equal(reloads, 0);
  busy = false;
  await tick();
  assert.equal(reloads, 1);
});
