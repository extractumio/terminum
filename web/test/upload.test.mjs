// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Why a file over 100 MB is not uploaded (upload.js, AC-36).
import assert from "node:assert/strict";
import { test } from "node:test";
import { tooLarge } from "../static/upload.js";

test("over 100 MB: said in words, the file's path to be given instead", () => {
  assert.equal(tooLarge("a.zip"), "a.zip is larger than the 100 MB an upload may be: type or paste the path to the file instead.");
});
