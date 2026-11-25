// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A window's button letters (keyletters.js, terminum.md AC-281): one rule, free letters only, none twice.
import assert from "node:assert/strict";
import { test } from "node:test";
import { FREE, ariaKey, keyName, letters } from "../static/keyletters.js";

const of = (labels, taken) => letters(labels, taken).map((l) => l?.letter ?? null);

test("AC-281 the rightmost chooses first, a word's first letter before its others", () => {
  assert.deepEqual(of(["Add files…", "Discard", "Start"]), ["A", "D", "S"]);
  assert.deepEqual(of(["Add files…", "Discard", "Create"]), ["F", "D", "A"], "Create has no free initial: its a; Add files… its files");
  assert.deepEqual(of(["Archive", "Stop waiting", "Give up", "Save"]), ["A", "O", "G", "S"]);
});

test("AC-281 a button appearing at the left changes no letter at its right", () => {
  const without = of(["Archive", "Save"]), withIt = of(["Archive", "Give up", "Save"]);
  assert.deepEqual([withIt[0], withIt[2]], without);
});

test("AC-281 never a letter a browser, the system or a text field keeps; never one twice", () => {
  for (const labels of [["Run"], ["Create", "Edit", "Next", "Quit"], ["Copy", "Paste", "Redo", "Inspect", "Text"]]) {
    const got = of(labels).filter(Boolean);
    assert.ok(got.every((l) => FREE.includes(l)), `${labels}: ${got}`);
    assert.equal(new Set(got).size, got.length);
  }
  assert.deepEqual(of(["Run", "Next"]), [null, null], "no free letter: no key");
  assert.ok(!/[CEIJKMNPQRTUVWXZ]/.test(FREE));
});

test("AC-281 a letter given by its button comes first; the place underlined is the label's own", () => {
  assert.deepEqual(of(["Save", "Discard"], ["S"]), ["A", "D"]);
  const [l] = letters(["Stop waiting"], ["S"]);
  assert.equal("Stop waiting"[l.at].toUpperCase(), l.letter);
  assert.equal(letters(["Straße"])[0].at, 0);
});

test("AC-281 keys named for the keyboard: ⌘⇧ on a Mac, Ctrl+Shift elsewhere", () => {
  assert.equal(keyName("D", true), "⌘⇧D");
  assert.equal(keyName("D", false), "Ctrl+Shift+D");
  assert.equal(keyName("Enter", true), "⌘⇧↩");
  assert.equal(keyName("Enter", false), "Ctrl+Shift+Enter");
  assert.equal(keyName("/", true, false), "⌘/");
  assert.equal(ariaKey("D", false), "Control+Shift+D");
});
