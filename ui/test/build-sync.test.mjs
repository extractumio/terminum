// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The UI build rewrites only changed files of dist/: an unchanged file keeps its time, so cargo does
// not compile terminumd again when the UI's sources did not change (TERM-227).
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { test } from "node:test";
import { listFiles, syncDir } from "../build-sync.mjs";
import { shortTmp } from "../../web/test/testrun.mjs";

function folder(t) {
  const dir = fs.mkdtempSync(path.join(shortTmp(), "tm-sync-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  return dir;
}

const old = new Date("2020-01-01T00:00:00Z");
function put(dir, rel, text) {
  fs.mkdirSync(path.dirname(path.join(dir, rel)), { recursive: true });
  fs.writeFileSync(path.join(dir, rel), text);
  fs.utimesSync(path.join(dir, rel), old, old);
}

test("unchanged files keep their time, changed and new ones are written, gone ones removed", (t) => {
  const dir = folder(t);
  put(dir, "index.html", "same");
  put(dir, "main.js", "old code");
  put(dir, "chunks/stale-AAAA.js", "gone");
  syncDir(dir, new Map([
    ["index.html", Buffer.from("same")],
    ["main.js", new TextEncoder().encode("new code")],   // esbuild's outputs are Uint8Arrays
    ["chunks/fresh-BBBB.js", Buffer.from("added")],
  ]));
  assert.equal(fs.statSync(path.join(dir, "index.html")).mtimeMs, old.getTime(), "unchanged: its time kept");
  assert.notEqual(fs.statSync(path.join(dir, "main.js")).mtimeMs, old.getTime());
  assert.equal(fs.readFileSync(path.join(dir, "main.js"), "utf8"), "new code");
  assert.deepEqual(listFiles(dir).sort(), ["chunks/fresh-BBBB.js", "index.html", "main.js"]);
});

test("a second sync of the same files writes nothing", (t) => {
  const dir = folder(t);
  const files = new Map([["a/b/c.js", Buffer.from("x")], ["top.js", Buffer.from("y")]]);
  syncDir(dir, files);
  for (const rel of files.keys()) fs.utimesSync(path.join(dir, rel), old, old);
  syncDir(dir, files);
  for (const rel of files.keys()) assert.equal(fs.statSync(path.join(dir, rel)).mtimeMs, old.getTime(), rel);
});

test("folders a removal leaves empty go, the folder itself stays; a missing dist is made", (t) => {
  const dir = path.join(folder(t), "dist");
  assert.deepEqual(listFiles(dir), []);
  syncDir(dir, new Map([["deep/er/x.js", Buffer.from("1")], ["keep.js", Buffer.from("2")]]));
  syncDir(dir, new Map([["keep.js", Buffer.from("2")]]));
  assert.equal(fs.existsSync(path.join(dir, "deep")), false, "emptied folders go");
  assert.deepEqual(listFiles(dir), ["keep.js"]);
  syncDir(dir + "/", new Map());   // however it is written, the walk up stops at dist
  assert.deepEqual(fs.readdirSync(dir), []);
});
