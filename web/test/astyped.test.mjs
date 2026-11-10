// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A command line keeps what was typed (terminum-dispatcher.md AC-137): what Smart Dashes and Smart Quotes put in goes back.
import { test } from "node:test";
import assert from "node:assert/strict";
import { typedBack } from "../static/astyped.js";

test("AC-137 a dash made of -- goes back, the caret after it", () => {
  assert.deepEqual(typedBack("claude -", "claude —", 8), { value: "claude --", at: 9 });
  assert.deepEqual(typedBack("claude --", "claude —d", 9), { value: "claude --d", at: 10 }, "replaced as the next letter is typed");
  assert.deepEqual(typedBack("claude -- x", "claude — x", 8), { value: "claude -- x", at: 9 }, "replaced later, the text after it untouched");
  assert.deepEqual(typedBack("cat  f", "cat – f", 5), { value: "cat - f", at: 5 }, "an en dash: one hyphen between spaces");
});

test("AC-137 smart quotes go back; text before and after keeps its place", () => {
  assert.deepEqual(typedBack("x  y", "x “a” y", 5), { value: "x \"a\" y", at: 5 });
  assert.deepEqual(typedBack("its", "it’s", 3), { value: "it's", at: 3 });
});

test("AC-137 nothing to put back: null; a mark already there stays", () => {
  assert.equal(typedBack("claude", "claude -", 8), null);
  assert.equal(typedBack("say — hi", "say — hi!", 9), null, "a pasted dash before the edit stays");
  assert.equal(typedBack("abc", "ab", 2), null, "a delete");
});
