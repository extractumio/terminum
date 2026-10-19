// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A Markdown table in Claude Code's diff of an edit (difftable.js, AC-31).
import assert from "node:assert/strict";
import { test } from "node:test";
import { diffTable } from "../static/difftable.js";

const L = (...t) => t.map((text) => ({ text }));

test("numbered diff rows of a table: its header, the rows, removed and added marked", () => {
  const t = diffTable(L("      5   | Field | Value |", "      6   |---|---|", "      7 - | Version | 0.11.1 |", "      7 + | Version | 0.12.0 |", "      8   | Date | 2026-10-19 |", "    ..."), 0);
  assert.deepEqual(t.head, ["Field", "Value"]);
  assert.deepEqual(t.rows, [["Version", "0.11.1"], ["Version", "0.12.0"], ["Date", "2026-10-19"]]);
  assert.deepEqual(t.marks, ["-", "+", ""]);
  assert.equal(t.to, 4, "up to the hunk's end");
});

test("rows without the header shown, an escaped pipe, one row only", () => {
  const t = diffTable(L("  110 - | AC-04 | [SHOULD / P1] | a \\| b |", "  110 + | AC-04 | [SHOULD / P1] | The pane |"), 0);
  assert.equal(t.head, null);
  assert.deepEqual(t.rows[0], ["AC-04", "[SHOULD / P1]", "a | b"]);
  assert.equal(diffTable(L("  12   | one row |", "  13   text"), 0), undefined, "one row is no table");
  assert.equal(diffTable(L("  12   plain code", "  13   more"), 0), undefined);
});
