// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// What Ask pastes (ask.js, AC-37): the selection quoted and numbered, the question, a rule.
import assert from "node:assert/strict";
import { test } from "node:test";
import { askSends, askText } from "../static/ask.js";

test("the quote numbered with four digits, in guillemets, the question under it, a rule", () => {
  assert.equal(askText(7, "retry loop\nnever resets\n", "  why?  "),
    "Quote #0007:\n - Quote Start ----\n«retry loop\nnever resets»\n- Quote End ----\nwhy?\n----");
  assert.ok(askText(12345, "x", "q").startsWith("Quote #12345:\n"), "more digits as they come");
});

test("a blank line after it, none before: typed as Ctrl+J after the paste in an agent's box, inside the paste elsewhere", () => {
  assert.deepEqual(askSends("Q", true), [["paste", "Q"], ["keys", "\n\n"]]);
  assert.deepEqual(askSends("Q", false), [["paste", "Q\n\n"]], "a shell: Ctrl+J would run the line");
});

test("Send to Agent: the paste, then Enter typed a moment after (not a new line in the paste)", () => {
  assert.deepEqual(askSends("Q", true, true), [["paste", "Q"], ["enter", "\r"]]);
});
