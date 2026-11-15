// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-13: a delayed resize is judged against the shown pane's latest request, across a reload.
import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import test from "node:test";
import { observeWidths } from "./e2e_livesplit.mjs";

test("AC-13 tracks the latest requested width across a page reload, apart from its quick shell", () => {
  const page = new EventEmitter(), widths = observeWidths(page);
  const socket = () => {
    const ws = new EventEmitter();
    page.emit("websocket", ws);
    return (m) => ws.emit("framesent", { payload: JSON.stringify(m) });
  };
  const first = socket();
  first({ t: "sub", id: "tmux:local:test:%0", cols: 87 });
  assert.equal(widths.get("%0"), 87);
  const reloaded = socket();
  reloaded({ t: "sub", id: "tmux:local:test:%0", cols: 56 });
  assert.equal(widths.get("%0"), 56, "the old 87-column frame is not readiness for the pending 56-column request");
  const quick = socket();
  quick({ t: "sub", id: "tmux:local:test:%1", cols: 45 });
  quick({ t: "size", cols: 60 });
  assert.equal(widths.get("%0"), 56, "the quick shell's size cannot replace its parent's request");
  reloaded({ t: "size", cols: 87 });
  assert.equal(widths.get("%0"), 87, "closing the quick shell asks for the full view again");
  reloaded({ t: "keys", text: "x" });
  reloaded({ t: "sub", id: "tmux:local:test:%2" });
  reloaded({ t: "size", cols: 90 });
  assert.equal(widths.get("%0"), 87, "another pane leaves the first pane's request alone");
  assert.equal(widths.get("%2"), 90);
});
