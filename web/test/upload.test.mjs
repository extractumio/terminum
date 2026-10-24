// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Why a file over 100 MB is not uploaded; what rows dragged from Files carry (upload.js, AC-36).
import assert from "node:assert/strict";
import { test } from "node:test";
import { draggedPaths, tooLarge } from "../static/upload.js";

test("over 100 MB: said in words, the file's path to be given instead", () => {
  assert.equal(tooLarge("a.zip"), "a.zip is larger than the 100 MB an upload may be: type or paste the path to the file instead.");
});

test("rows dragged from Files: absolute paths with their host; anything else carries nothing", () => {
  assert.deepEqual(draggedPaths(JSON.stringify({ host: null, paths: ["/a/b", "/a/c d"] })), { host: null, paths: ["/a/b", "/a/c d"] });
  assert.deepEqual(draggedPaths(JSON.stringify({ host: "devbox", paths: ["/srv/x"] })), { host: "devbox", paths: ["/srv/x"] });
  assert.deepEqual(draggedPaths(JSON.stringify({ paths: ["rel/x", "/ok", "/new\nline", "/esc\x1b[D", 7] })), { host: null, paths: ["/ok"] }, "relative, control characters, not text: dropped");
  for (const raw of ["", "not json", "{}", JSON.stringify({ paths: [] }), JSON.stringify({ paths: "/a" })]) assert.equal(draggedPaths(raw), null, raw);
});
