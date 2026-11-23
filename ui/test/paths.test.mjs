// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
import assert from "node:assert/strict";
import { test } from "node:test";
import { load } from "./bundle.mjs";
const { isUnder, decodePath, typedFolder } = await load("src/paths.ts");

test("filesystem root includes descendants without confusing sibling prefixes", () => {
  assert.equal(isUnder("/tmp/project", "/"), true);
  assert.equal(isUnder("/", "/"), true);
  assert.equal(isUnder("/tmp/project/file", "/tmp/project"), true);
  assert.equal(isUnder("/tmp/project2", "/tmp/project"), false);
});

test("literal percent filenames remain usable and encoded separators decode", () => {
  assert.equal(decodePath("100%.png"), "100%.png");
  assert.equal(decodePath("a%23b%3Fc.png"), "a#b?c.png");
});

test("AC-275: a typed folder is full, from the home, or relative to the shown one", () => {
  const at = (t, home = "/Users/alex") => typedFolder(t, "/Users/alex/work/api", home);
  assert.deepEqual(at("/var/log/"), { path: "/var/log" });
  assert.deepEqual(at("  ../web "), { path: "/Users/alex/work/web" });
  assert.deepEqual(at("src/./db"), { path: "/Users/alex/work/api/src/db" });
  assert.deepEqual(at("~"), { path: "/Users/alex" });
  assert.deepEqual(at("~/notes/.."), { path: "/Users/alex" });
  assert.deepEqual(at("/../.."), { path: "/" });
  assert.match(at("~/x", null).error, /home is not known/);
  assert.match(at("~bob/x").error, /full path/);
  assert.match(at("   ").error, /Type a folder/);
});
