// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// What Ask pastes (ask.js, AC-37): the selection quoted and numbered, the question, a rule.
import assert from "node:assert/strict";
import { test } from "node:test";
import { askSends, askText, centerOn } from "../static/ask.js";

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

test("the dialog centered on the terminal, kept 8 px inside the window", () => {
  const term = { left: 300, top: 40, width: 900, height: 700 };
  assert.deepEqual(centerOn(term, 600, 300, 1400, 900), { x: 450, y: 240 });
  assert.deepEqual(centerOn({ left: 0, top: 0, width: 400, height: 300 }, 600, 400, 420, 900), { x: 8, y: 8 }, "larger than the terminal: from the window's edge");
  assert.deepEqual(centerOn({ left: 1000, top: 0, width: 400, height: 900 }, 600, 300, 1400, 900), { x: 792, y: 300 }, "never past the window's right edge");
});
