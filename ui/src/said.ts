// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A task's comment as it reads (terminum-kanban.md AC-149): agents write a result or questions as
// lines that each start with a label ("Outcome: attention", "Summary: …", "Details:",
// "Why you are needed: …"). Markdown would join such lines into one paragraph, or into the list
// item above them; with env.said, a label line at the left edge is a block of its own that ends
// the paragraph or list before it: alone on its line, the heading of what follows; before words,
// their bold lead. Outcome's word is a status box in its status's hue.

import type MarkdownIt from "markdown-it";

type Md = InstanceType<typeof MarkdownIt>;
type BlockRule = Parameters<Md["block"]["ruler"]["before"]>[2];

// a label: a capital, then letters, digits and light punctuation, then ":" and a space or the end
const LABEL = /^(\p{Lu}[\p{L}\p{N} ,/'’()&-]*?):(?:[ \t]+(\S.*))?$/u;

/** The label a line starts with, or null: alone up to 6 words, before words up to 4 (a longer
 *  start is a sentence that has a colon in it). */
export function labelOf(line: string): { label: string; rest: string } | null {
  const m = LABEL.exec(line);
  if (!m) return null;
  const [, label, rest = ""] = m;
  return label.split(/\s+/).length <= (rest ? 4 : 6) ? { label, rest } : null;
}

// Outcome's words as terminumd's close-out form asks them (terminumd/src/tasks/closeout.rs), in the
// status box of the column they mean
const OUTCOME: Record<string, string> = { ok: "done", done: "done", attention: "wait-for-human", partly: "wait-for-human", failed: "failed" };

const labelBlock: BlockRule = (state, start, end, silent) => {
  if (!(state.env as { said?: boolean }).said) return false;
  // at the left edge only; as a terminator (silent) it also ends a list item's lazy line above it
  if (state.sCount[start] !== 0 || (!silent && state.blkIndent !== 0)) return false;
  const got = labelOf(state.src.slice(state.bMarks[start] + state.tShift[start], state.eMarks[start]));
  if (!got) return false;
  if (silent) return true;
  // a lead's words go on, as a paragraph's do, to a blank line or a block that ends a paragraph
  let next = start + 1;
  const ends = state.md.block.ruler.getRules("paragraph");
  if (got.rest) {
    for (; next < end && !state.isEmpty(next); next++) {
      if (state.sCount[next] - state.blkIndent > 3 || state.sCount[next] < 0) continue;
      if (ends.some((rule) => rule(state, next, end, true))) break;
    }
  }
  const open = state.push("paragraph_open", "p", 1);
  open.map = [start, next];
  open.attrSet("class", got.rest ? "said-lead" : "said-label");
  let words = [got.rest, state.getLines(start + 1, next, state.blkIndent, false)].join("\n").trim();
  const kind = got.label === "Outcome" ? /^(\p{L}+)/u.exec(words) : null;
  const status = kind && OUTCOME[kind[1].toLowerCase()];
  if (status) words = words.slice(kind[1].length);
  open.meta = got.rest ? { label: got.label, status: status && { word: kind[1], cls: status } } : null;
  const inline = state.push("inline", "", 0);
  inline.content = got.rest ? words : got.label;
  inline.map = open.map;
  inline.children = [];
  state.push("paragraph_close", "p", -1);
  state.line = next;
  return true;
};

/** The label lines' rule, on with env.said; a lead's label and status box drawn before its words. */
export const saidLabels = (md: Md) => {
  md.block.ruler.before("paragraph", "said_label", labelBlock, { alt: ["paragraph", "list", "blockquote"] });
  const esc = md.utils.escapeHtml;
  md.renderer.rules.paragraph_open = (tokens, idx, opts, _env, self) => {
    const lead = tokens[idx].meta as { label: string; status?: { word: string; cls: string } } | null;
    if (!lead?.label) return self.renderToken(tokens, idx, opts);
    const box = lead.status ? ` <span class="stbox word st-${lead.status.cls}">${esc(lead.status.word)}</span>` : "";
    return `<p class="said-lead"><strong>${esc(lead.label)}:</strong>${box} `;
  };
};
