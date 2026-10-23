// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// What Ask pastes (ask.js, AC-37): the selection numbered and quoted, compact, the question.
import assert from "node:assert/strict";
import { test } from "node:test";
import { askSends, askText, centerOn, sentCopy } from "../static/ask.js";
import { squeeze } from "../static/asked.js";

test("the quote numbered with four digits on its first line, in guillemets, the question under it", () => {
  assert.equal(askText(7, "retry loop\nnever resets\n", "  why?  "), "Quote #0007: «retry loop\nnever resets»\nwhy?");
  assert.ok(askText(12345, "x", "q").startsWith("Quote #12345: «x»"), "more digits as they come");
});

test("compact: the quote's blank lines and line ends' spaces left out (an agent's box taller than the screen breaks)", () => {
  assert.equal(askText(2, "\n  Tests:   \n\n\n  - cuts  \n\n  - 416\n\n", "ok"), "Quote #0002: «  Tests:\n  - cuts\n  - 416»\nok");
});

test("a new line after it, none before: typed as a Ctrl+J after the paste in an agent's box, inside the paste elsewhere", () => {
  assert.deepEqual(askSends("Q", true), [["paste", "Q"], ["keys", "\n"]]);
  assert.deepEqual(askSends("Q", false), [["paste", "Q\n"]], "a shell: Ctrl+J would run the line");
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

test("the quote as sent is told from the text asked about (asked.js marks the original only)", () => {
  // Claude Code shows the message as sent, indented and wrapped; the original stands in the agent's answer
  const sent = squeeze("❯ Quote #0002:\n  - Quote Start ----\n    «Daily runs failed\n  twice.»").out;
  const original = squeeze("  What is wrong\n\n  - Daily runs failed\n    twice.").out;
  const before = (s) => s.slice(Math.max(0, s.indexOf("Daily") - 40), s.indexOf("Daily"));
  assert.equal(sentCopy(before(sent)), true);
  assert.equal(sentCopy(before(original)), false);
  assert.equal(sentCopy(before(squeeze(askText(3, "Daily runs failed", "why?")).out)), true, "askText's own form");
  assert.equal(sentCopy(before(squeeze("❯ Quote #0003: «Daily runs failed\n  twice.»").out)), true, "as Claude Code shows it");
});
