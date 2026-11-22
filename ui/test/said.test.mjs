// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// terminum-kanban.md AC-149: a comment's label lines (a result's Outcome, Summary, Details; a question's
// Why you are needed, Recommended, Details) each start a block of their own (src/said.ts), only in a comment.
import assert from "node:assert/strict";
import { test } from "node:test";
import { load } from "./bundle.mjs";

const { renderMd } = await load("src/markdown-core.ts");
const { labelOf } = await load("src/said.ts");
const html = (t, said = true) => renderMd(t, { dir: "/", imagePath: () => "", said });

test("AC-149 happy path: a scheduled result's lines read as sections, not one paragraph", () => {
  const h = html("Outcome: attention\nSummary: 12 integrations landed,\nall green.\nDetails:\nNew features:\n- TERM-621: told at its next call.\n- TERM-623: an idle agent hears.\nChanged / fixed:\n- TERM-616: one tool.\nFor the human: nothing");
  assert.match(h, /^<p class="said-lead"><strong>Outcome:<\/strong> <span class="stbox word st-wait-for-human">attention<\/span> <\/p>/);
  assert.match(h, /<p class="said-lead"><strong>Summary:<\/strong> 12 integrations landed,\nall green\.<\/p>/);
  assert.match(h, /<p class="said-label">Details<\/p>\n<p class="said-label">New features<\/p>\n<ul>/);
  // the label after a list ends it: never a lazy continuation of the last item
  assert.match(h, /TERM-623: an idle agent hears\.<\/li>\n<\/ul>\n<p class="said-label">Changed \/ fixed<\/p>/);
  assert.match(h, /<p class="said-lead"><strong>For the human:<\/strong> nothing<\/p>/);
});

test("AC-149 happy path: an agent's questions keep Details out of the last recommended answer", () => {
  const h = html("Why you are needed: the build Mac stayed busy.\nRecommended:\n1. RUN TERM-613 again when it is quiet.\nDetails: Commit 1635162b holds the fix.");
  assert.match(h, /<p class="said-label">Recommended<\/p>\n<ol>\n<li>RUN TERM-613 again when it is quiet\.<\/li>\n<\/ol>\n<p class="said-lead"><strong>Details:<\/strong> Commit/);
});

test("AC-149 happy path: Outcome's word takes its status's box", () => {
  assert.match(html("Outcome: ok"), /stbox word st-done">ok</);
  assert.match(html("Outcome: failed, see below"), /stbox word st-failed">failed<\/span> , see below/);
  assert.match(html("Outcome: Done, merged."), /stbox word st-done">Done</);
  assert.doesNotMatch(html("Outcome: unsure"), /stbox/);
});

test("AC-149 edge: code, indented lines, sentences, an author's bold and task text stay as written", () => {
  assert.match(html("```\nNote: inside code\n```"), /<pre><code>Note: inside code\n<\/code><\/pre>/);
  assert.match(html("- item\n  Note: indented"), /<li>item\nNote: indented<\/li>/);
  assert.match(html("**Note:** mine"), /^<p><strong>Note:<\/strong> mine<\/p>/);
  assert.equal(html("Summary: a\nDetails: b", false), "<p>Summary: a\nDetails: b</p>\n");   // a task's text: no label rule
  assert.equal(labelOf("In the last 24 hours (to 2026-11-21 07:00) 12 integrations landed"), null);
  assert.equal(labelOf("This is why the lock is held: the walk waits"), null);   // a sentence, over 4 words
  assert.equal(labelOf("see https://example.com"), null);
  assert.equal(labelOf("lowercase: no"), null);
  assert.deepEqual(labelOf("Critical, need human to decide:"), { label: "Critical, need human to decide", rest: "" });
  assert.equal(labelOf("A label that runs on for far too many words:"), null);
});

test("AC-149 failure: a label's words are escaped, and raw HTML stays off", () => {
  const h = html("Outcome: <b>ok</b>\nSummary: <script>alert(1)</script>");
  assert.doesNotMatch(h, /<script>|<b>/);
  assert.match(h, /&lt;script&gt;/);
});
