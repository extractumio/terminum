// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// What Enter sends (sent.js, AC-90): an agent's input box, a shell's command line.
import assert from "node:assert/strict";
import { test } from "node:test";
import { sentText } from "../static/sent.js";

const row = (txt, o = {}) => ({ txt, hard: true, cur: false, end: null, ...o });

test("a shell's command line after its prompt, with the rows it wrapped from", () => {
  assert.equal(sentText([row("alex@mac api % git status", { cur: true })], null, true), "git status");
  assert.equal(sentText([row("~/api ❯ make test")], null, true), "", "no cursor: nothing");
  const wrapped = [row("out"), row("$ echo one two", { hard: false }), row(" three", { cur: true })];
  assert.equal(sentText(wrapped, null, true), "echo one two three");
  assert.equal(sentText([row("$ echo a > b", { cur: true })], null, true), "echo a > b", "the first prompt mark only");
  assert.equal(sentText([row("vim text", { cur: true })], null, false), "", "not at a shell prompt: nothing");
  assert.equal(sentText([row("alex@mac ~ %  export X=1 ", { cur: true })], null, true), " export X=1", "a leading space kept: terminumd keeps it out");
});

test("an agent's input box: its typed rows, not a faint suggestion", () => {
  const rows = [row("output"), row("────", {}), row("❯ fix the tests", { end: 15 }), row("  and the lint", { cur: true }), row("────"), row("  ? for shortcuts")];
  assert.equal(sentText(rows, { top: 1, bottom: 4 }, false), "fix the tests\nand the lint");
  const faint = [row("────"), row("❯ Try \"write a test\"", { end: 2, cur: true }), row("────")];
  assert.equal(sentText(faint, { top: 0, bottom: 2 }, false), "", "only the prompt typed");
  const codex = [row(""), row("› explain this", { cur: true, end: 14 }), row("")];
  assert.equal(sentText(codex, { top: 0, bottom: 2 }, false), "explain this");
});
