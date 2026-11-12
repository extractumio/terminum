// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The viewer's save and re-check against a stand-in api whose answers the test releases in any
// order: an older answer never replaces a newer one, and a save clears "Deleted on disk".
import assert from "node:assert/strict";
import { test } from "node:test";
import { load } from "./bundle.mjs";

// api, toast and ask stand-ins: each call waits until the test settles it
const stubs = {
  name: "stubs",
  setup(b) {
    b.onResolve({ filter: /^\.\/(api|dialogs)$/ }, (a) => ({ path: a.path, namespace: "stub" }));
    b.onLoad({ filter: /.*/, namespace: "stub" }, (a) => ({
      loader: "js",
      contents: a.path === "./api"
        ? `export class ApiError extends Error { constructor(m, status, code) { super(m); this.status = status; this.code = code; } }
           globalThis.__ApiError = ApiError;
           export const basename = (p) => p.split("/").pop();
           export const toast = (m) => globalThis.__toasts.push(m);
           export const api = (...args) => new Promise((resolve, reject) => globalThis.__calls.push({ args, resolve, reject }));`
        : `export const ask = async () => globalThis.__answer;`,
    }));
  },
};
const { saveDoc, revalidateDoc } = await load("src/viewer-save.ts", { plugins: [stubs] });
const apiError = (status, code) => new globalThis.__ApiError(code, status, code);

const tick = () => new Promise((r) => setTimeout(r, 0));
function setup() {
  globalThis.__calls = []; globalThis.__toasts = []; globalThis.__answer = "cancel";
  const text = "v1";
  const d = {
    file: { path: "/r/a.md", etag: "e1", writable: true, text, size: 2 },
    // the editor state the save reads: one document, never typed into here
    state: { doc: { eq: () => true }, sliceDoc: () => text },
    dirty: false, diskChanged: false, scroll: {}, host: null,
  };
  const redraw = (_p, update) => update();
  const host = { revalidate: async () => {}, tabsChanged() {}, redraw() {}, discard() {} };
  return { d, redraw, host };
}
const file = (etag, text) => ({ path: "/r/a.md", etag, writable: true, text, size: text.length });

test("an older re-check answering last does not replace a newer one's file", async () => {
  const { d, redraw } = setup();
  const older = revalidateDoc("/r/a.md", d, redraw);
  const newer = revalidateDoc("/r/a.md", d, redraw);
  const [a, b] = globalThis.__calls;
  b.resolve(file("e3", "v3")); await newer;
  a.resolve(file("e2", "v2")); await older;
  assert.equal(d.file.etag, "e3");
  assert.equal(d.file.text, "v3");
});

test("a newer re-check answering last still applies", async () => {
  const { d, redraw } = setup();
  const older = revalidateDoc("/r/a.md", d, redraw);
  const newer = revalidateDoc("/r/a.md", d, redraw);
  const [a, b] = globalThis.__calls;
  a.resolve(file("e2", "v2")); await older;
  assert.equal(d.file.etag, "e1", "the older answer is dropped");
  b.resolve(file("e3", "v3")); await newer;
  assert.equal(d.file.etag, "e3");
});

test("a re-check sent before a save and answering after it is dropped", async () => {
  const { d, redraw, host } = setup();
  const check = revalidateDoc("/r/a.md", d, redraw);
  const save = saveDoc("/r/a.md", d, false, host);
  const [get, put] = globalThis.__calls;
  assert.equal(put.args[0], "PUT");
  put.resolve({ etag: "e9" }); assert.equal(await save, true);
  get.resolve(file("e2", "old bytes")); await check;
  assert.equal(d.file.etag, "e9");
  assert.equal(d.file.text, "v1");
});

test("a single re-check still marks a file deleted, and a save that recreates it clears the mark", async () => {
  const { d, redraw, host } = setup();
  const check = revalidateDoc("/r/a.md", d, redraw);
  globalThis.__calls[0].reject(apiError(404, "not_found"));
  await check;
  assert.equal(d.deleted, true, "404 marks it deleted");
  // the user saves: If-Match fails (no file), they choose Overwrite, the forced save recreates it
  globalThis.__answer = "overwrite";
  const save = saveDoc("/r/a.md", d, false, host);
  await tick();
  globalThis.__calls[1].reject(apiError(412, "conflict"));
  await tick(); await tick();
  const forced = globalThis.__calls[2];
  assert.deepEqual(forced.args[2].headers, { "If-Match": '"*"' });
  forced.resolve({ etag: "e5" });
  assert.equal(await save, true);
  assert.equal(d.deleted, false);
  assert.equal(d.file.etag, "e5");
});

test("a failed save keeps the deleted mark", async () => {
  const { d, host } = setup();
  d.deleted = true;
  const save = saveDoc("/r/a.md", d, true, host);
  globalThis.__calls[0].reject(new Error("disk full"));
  assert.equal(await save, false);
  assert.equal(d.deleted, true);
  assert.deepEqual(globalThis.__toasts, ["disk full"]);
});
