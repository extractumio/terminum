// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// terminum-kanban.md AC-149: a comment's label lines (a result's Outcome, Summary, Details; a question's
// Why you are needed, Recommended, Details) each start a block of their own (src/said.ts).
import assert from "node:assert/strict";
import { test } from "node:test";
import { load } from "./bundle.mjs";

const { renderSaid, setApart, labelOf } = await load("src/said.ts");
const ENV = { dir: "/", imagePath: () => "" };
const html = (t) => renderSaid(t, ENV);

test("AC-149 happy path: a scheduled result's lines read as sections, not one paragraph", () => {
  const h = html("Outcome: attention\nSummary: 12 integrations landed.\nDetails:\nNew features:\n- TERM-621: told at its next call.\n- TERM-623: an idle agent hears.\nChanged / fixed:\n- TERM-616: one tool.\nFor the human: nothing");
  assert.match(h, /<p class="said-lead"><strong>Outcome:<\/strong> <span class="said-pill said-attention">attention<\/span><\/p>/);
  assert.match(h, /<p class="said-lead"><strong>Summary:<\/strong> 12 integrations landed\.<\/p>/);
  assert.match(h, /<p class="said-label"><strong>Details<\/strong><\/p>/);
  assert.match(h, /<p class="said-label"><strong>New features<\/strong><\/p>\n<ul>/);
  // the label after a list ends it: never a lazy continuation of the last item
  assert.match(h, /TERM-623: an idle agent hears\.<\/li>\n<\/ul>\n<p class="said-label"><strong>Changed \/ fixed<\/strong><\/p>/);
  assert.match(h, /<p class="said-lead"><strong>For the human:<\/strong> nothing<\/p>/);
});

test("AC-149 happy path: an agent's questions keep Details out of the last recommended answer", () => {
  const h = html("Why you are needed: the build Mac stayed busy.\nRecommended:\n1. RUN TERM-613 again when it is quiet.\nDetails: Commit 1635162b holds the fix.");
  assert.match(h, /<p class="said-label"><strong>Recommended<\/strong><\/p>\n<ol>\n<li>RUN TERM-613 again when it is quiet\.<\/li>\n<\/ol>\n<p class="said-lead"><strong>Details:<\/strong> Commit/);
});

test("AC-149 happy path: Outcome's word takes its status's hue", () => {
  assert.match(html("Outcome: ok"), /said-pill said-ok">ok</);
  assert.match(html("Outcome: failed, see below"), /said-pill said-failed">failed<\/span>, see below/);
  assert.match(html("Outcome: Done"), /said-pill said-ok">Done</);
  assert.doesNotMatch(html("Outcome: unsure"), /said-pill/);
});

test("AC-149 edge: code, indented lines, sentences and times stay as written", () => {
  const fenced = "```\nNote: inside code\n```";
  assert.equal(setApart(fenced), fenced);
  assert.equal(setApart("~~~~\nA: b\n~~~\nC: d\n~~~~"), "~~~~\nA: b\n~~~\nC: d\n~~~~");
  assert.equal(setApart("- item\n  Note: indented"), "- item\n  Note: indented");
  assert.equal(labelOf("In the last 24 hours (to 2026-11-21 07:00) 12 integrations landed"), null);
  assert.equal(labelOf("This is why the lock is held too long: the walk waits"), null);   // a sentence, over 4 words
  assert.equal(labelOf("see https://example.com"), null);
  assert.equal(labelOf("lowercase: no"), null);
  assert.deepEqual(labelOf("Critical, need human to decide:"), { label: "Critical, need human to decide", rest: "" });
  assert.equal(labelOf("A label that runs on for far too many words:"), null);
});

test("AC-149 failure: a label's text is escaped, and raw HTML stays off", () => {
  const h = html("Outcome: <b>ok</b>\nSummary: <script>alert(1)</script>");
  assert.doesNotMatch(h, /<script>|<b>/);
  assert.match(h, /&lt;script&gt;/);
});
