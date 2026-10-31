// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
import { test } from "node:test";
import assert from "node:assert/strict";
import { Find } from "../static/find.js";
import { Input } from "../static/input.js";
import { uploads } from "../static/upload.js";
import { paneFiles } from "../static/panefiles.js";
import { rangeOfText } from "../static/marktext.js";
import { arrowsTo, cursorCell, typedEnd } from "../static/cursor.js";

const tick = (ms = 0) => new Promise((r) => setTimeout(r, ms));
const replace = (t, key, value) => {
  const old = Object.getOwnPropertyDescriptor(globalThis, key);
  Object.defineProperty(globalThis, key, { value, writable: true, configurable: true });
  t.after(() => { if (old) Object.defineProperty(globalThis, key, old); else delete globalThis[key]; });
};
const deferred = () => { let resolve; const promise = new Promise((r) => { resolve = r; }); return { promise, resolve }; };

test("Find retries identical text after timeout; an old timeout cannot clear the new request", (t) => {
  const elements = { findq: { value: "needle" }, findn: { textContent: "" } }, requests = [], timers = [];
  t.mock.method(globalThis, "setTimeout", (fn) => { timers.push(fn); });
  replace(t, "document", { getElementById: (id) => elements[id] });
  const f = Object.assign(Object.create(Find.prototype), { hits: [], q: "", jumper: { token: 0, clear() {} }, send: (m) => requests.push(m) });
  f.find(); timers[0](); f.find();
  assert.equal(requests.length, 2);
  assert.equal(f.pending, true);
  timers[0]();
  assert.equal(f.pending, true);
  f.found({ error: "unreachable" });
  timers[1]();
  assert.equal(elements.findn.textContent, "unreachable");
});

test("a clipboard read or text conversion begun in A never pastes into B", async (t) => {
  replace(t, "window", { isSecureContext: true });
  const nav = {};
  replace(t, "navigator", nav);
  for (const phase of ["readText", "read", "getType", "text"]) {
    let sid = "A";
    const wait = deferred(), pasted = [], notices = [];
    const blob = { text: () => phase === "text" ? wait.promise : Promise.resolve("synthetic") };
    const item = { types: ["text/plain"], getType: () => phase === "getType" ? wait.promise : Promise.resolve(blob) };
    const clipboard = phase === "readText" ? { readText: () => wait.promise } : { read: () => phase === "read" ? wait.promise : Promise.resolve([item]) };
    nav.clipboard = clipboard;
    const input = Object.assign(Object.create(Input.prototype), { options: { sid: () => sid, status: (m) => notices.push(m) }, paste: (s) => pasted.push(s), pasteDialog: { showModal() { throw Error("retargeted dialog"); } } });
    const pending = Input.prototype.pasteFromClipboard.call(input);
    await tick(); sid = "B";
    wait.resolve(phase === "readText" || phase === "text" ? "synthetic" : phase === "read" ? [item] : blob);
    await pending;
    assert.deepEqual(pasted, [], phase);
    assert.equal(notices.length, 1, phase);
  }
});

test("image decoding cannot retarget an upload to a newly shown pane", async (t) => {
  let sid = "A";
  const wait = deferred(), requested = [], pasted = [], notices = [];
  replace(t, "createImageBitmap", () => wait.promise);
  replace(t, "document", { createElement: () => ({ getContext: () => ({ drawImage() {} }), toBlob: (done) => done(new Blob(["synthetic"], { type: "image/png" })) }) });
  replace(t, "fetch", async (...args) => { requested.push(args); return { ok: true, text: async () => '{"path":"/synthetic.png"}' }; });
  const f = uploads({ sid: () => sid, paste: (s) => pasted.push(s), toast: (m) => notices.push(m), hideToast() {} });
  const pending = f.pasteImage(new Blob(["synthetic"], { type: "image/tiff" }));
  sid = "B";
  wait.resolve({ width: 1, height: 1, close() {} });
  await pending;
  assert.deepEqual(requested, []);
  assert.deepEqual(pasted, []);
  assert.ok(notices.some((m) => m.includes("Another session")));
  await f.pasteImage(new Blob(["synthetic"], { type: "image/png" }));
  assert.equal(requested.length, 1, "same-pane image still uploads");
  assert.ok(requested[0][0].endsWith("sid=B"));
  assert.deepEqual(pasted, ["/synthetic.png"]);
});

test("individual lookups in multiple large folders share a bound of four, including failures", async () => {
  let active = 0, peak = 0, completed = 0;
  const f = paneFiles(async () => ({ cwd: "/sandbox", home: "/sandbox" }), async (url) => {
    peak = Math.max(peak, ++active);
    const q = new URL(url, "http://test").searchParams;
    try {
      await tick(2);
      if (q.get("filter") === "bad") throw Error("failed listing");
      if (q.has("filter")) completed++;
      return { status: "ready", total: 10000, entries: q.has("filter") ? [{ n: q.get("filter"), k: "f" }] : [] };
    } finally { active--; }
  });
  f.state("x"); await tick();
  for (let folder = 0; folder < 4; folder++) for (let n = 0; n < 20; n++) f.state(`${folder}/file${n}`);
  f.state("4/bad");
  for (let n = 0; n < 200 && (completed < 80 || active); n++) await tick(5);
  assert.equal(completed, 80);
  assert.ok(peak <= 4, `peak requests: ${peak}`);
  assert.equal(f.state("0/file0"), "yes");
  assert.equal(f.state("3/file19"), "yes");
});

test("search ranges preserve original offsets across lowercase expansions and text nodes", (t) => {
  replace(t, "document", { createRange: () => ({
    setStart(node, offset) { assert.ok(offset <= node.data.length); this.start = [node, offset]; },
    setEnd(node, offset) { assert.ok(offset <= node.data.length); this.end = [node, offset]; },
  }) });
  const ns = (...texts) => texts.map((data) => ({ data }));
  for (const text of ["İx", "İxy", "𠀀İx"]) {
    const nodes = ns(text), r = rangeOfText(nodes, "x");
    assert.equal(text.slice(r.start[1], r.end[1]), "x");
  }
  const nodes = ns("İ", "x İ", "x");
  const first = rangeOfText(nodes, "x"), second = rangeOfText(nodes, "x", 1), expanded = rangeOfText(nodes, "i\u0307x");
  assert.deepEqual(first.start, [nodes[1], 0]);
  assert.deepEqual(second.start, [nodes[2], 0]);
  assert.deepEqual(expanded.start, [nodes[0], 0]);
  assert.deepEqual(expanded.end, [nodes[1], 1]);
  assert.equal(rangeOfText(nodes, "gone"), null);
});

test("shell movement counts graphemes while terminal protocol columns stay physical", (t) => {
  const cursor = { data: " " }, chunks = [{ data: "a" }, { data: "😀e" }, { data: "\u0301日" }, cursor];
  const row = {}, cur = { closest: () => row }, screen = { querySelector: () => cur, querySelectorAll: () => [row] };
  replace(t, "NodeFilter", { SHOW_TEXT: 4 });
  replace(t, "document", { createTreeWalker: (root) => {
    const nodes = root === cur ? [cursor] : chunks; let i = 0;
    return { nextNode: () => nodes[i++] ?? null };
  } });
  const from = cursorCell(screen);
  assert.equal(from.col, 4, "a, emoji, composed e, CJK");
  assert.equal(arrowsTo(from, { row: 0, col: 1 }), "\x1b[D".repeat(3));
  const data = { r: [[["a", "😀", "", "e\u0301", "日", ""], null, null, 0], [" ", null, null, 256], ["suggestion", null, null, 2]] };
  assert.equal(typedEnd(data), 5, "protocol cells unchanged: last glyph starts in cell five");
  assert.equal(typedEnd(data, true), 4, "editing clamp agrees with DOM positions");
  assert.equal(typedEnd({ r: [["ae", null, null, 0], ["\u0301", 3, null, 0], [" ", null, null, 256], ["suggestion", null, null, 2]] }, true), 2, "a grapheme split across style runs counts once");
});
