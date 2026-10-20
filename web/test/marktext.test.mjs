// The web app's bookmarks (AC-57): text matched by its letters and digits as the bridge matches
// it, found across text nodes, and the marks' places on the terminal's edge.
import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { loose, part, railSlots, spanOf } from "../static/marktext.js";

const CASES = JSON.parse(fs.readFileSync(new URL("./loose_cases.json", import.meta.url), "utf8"));
const nodes = (...texts) => texts.map((data) => ({ data }));

test("the same letters and digits as the bridge's (bookmarks.py)", () => {
  for (const [text, want] of CASES) assert.equal(loose(text), want, text);
});

test("text is found across nodes and markup, ending after its last letter", () => {
  const ns = nodes("A small ", "tool", " that keeps", " bookmarks.");
  const [[a, i], [b, j]] = spanOf(ns, loose("small tool that keeps"));
  assert.deepEqual([a.data, i, b.data, j], ["A small ", 2, " that keeps", 11]);
  assert.equal(spanOf(ns, "nothere"), null);
  assert.equal(spanOf(ns, ""), null);
});

test("a letter outside the basic plane counts once, as loose() counts it", () => {
  const ns = nodes("x 𠀀y z");
  const [[, i], [, j]] = spanOf(ns, loose("𠀀y"));
  assert.deepEqual([i, j], [2, 5]);
});

test("a letter outside the basic plane before the text moves nothing (the bridge counts code points)", () => {
  const ns = nodes("𠀀 alpha beta");
  const [[, i], [, j]] = spanOf(ns, "alpha");
  assert.equal(ns[0].data.slice(i, j), "alpha");
  assert.equal(part("𠀀 alpha beta", 1, 5), "alpha");
});

test("marks are placed by their lines and merged when close; a line gone from the history has none", () => {
  const slots = railSlots([{ n: 0 }, { n: 500 }, { n: 505 }, { n: 1000 }, { n: -50 }], 0, 1000, 200, 8);
  assert.deepEqual(slots.map((s) => [s.top, s.items.length]), [[0, 1], [100, 2], [200, 1]]);
});

test("the rail is the whole loadable history: a mark moves up as it grows and goes once its line is trimmed (12000 kept)", () => {
  const top = (oldest, end) => railSlots([{ n: 6000 }], oldest, end, 600).map((s) => s.top);
  assert.deepEqual(top(0, 8000), [450]);
  assert.deepEqual(top(0, 12000), [300], "half way up the chat, half way up the rail");
  assert.deepEqual(top(4000, 16000), [100], "older lines dropped at the top: it moved up");
  assert.deepEqual(top(7000, 19000), [], "its line left the history: no mark");
});
