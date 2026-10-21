// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A split window's banner (splitnote.js, AC-13): its words, and the window's next pane.
import assert from "node:assert/strict";
import { test } from "node:test";
import { nextPane, splitWords } from "../static/splitnote.js";

test("the banner says how many panes and how wide this one is; zoomed, that it is; none for one pane", () => {
  assert.equal(splitWords(null, 80), null);
  assert.equal(splitWords({ panes: 2, zoomed: false, wcols: 117 }, 35), "This window is split: 2 panes, this one 35 of 117 columns");
  assert.equal(splitWords({ panes: 3, zoomed: true, wcols: 117 }, 117), "This window is split: 3 panes, this one zoomed");
});

test("Other pane: the next pane of the same window on the same server, round; none alone", () => {
  const it = (id, window) => ({ id, window });
  const groups = [{ items: [it("tmux:local:terminum:%4", "@2"), it("tmux:local:terminum:%9", "@3"), it("tmux:local:terminum:%100", "@2")] },
    { items: [it("tmux:devbox:default:%5", "@2")] }];
  assert.equal(nextPane(groups, "tmux:local:terminum:%4"), "tmux:local:terminum:%100");
  assert.equal(nextPane(groups, "tmux:local:terminum:%100"), "tmux:local:terminum:%4", "round");
  assert.equal(nextPane(groups, "tmux:local:terminum:%9"), null, "a window of one pane");
  assert.equal(nextPane(groups, "tmux:devbox:default:%5"), null, "@2 on another server is another window");
  assert.equal(nextPane(groups, "tmux:local:terminum:%77"), null, "not listed");
});
