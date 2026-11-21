// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A task's comment as it reads (terminum-kanban.md AC-149): agents write a result or questions as
// lines that each start with a label ("Outcome: attention", "Summary: …", "Details:",
// "Why you are needed: …"). Markdown would join such lines into one paragraph, or into the list
// item above them; here each label starts a block of its own: alone on its line, the heading of
// what follows; before words, their bold lead. Outcome's word is a pill in its color.

import type MarkdownIt from "markdown-it";
import { configuredMd, type MdEnv } from "./markdown-core";

// a label: a capital, then letters, digits and light punctuation, then ":" and a space or the end
const LABEL = /^(\p{Lu}[\p{L}\p{N} ,/'’()&-]*?):(?:[ \t]+(\S.*))?$/u;
const FENCE = /^ {0,3}(`{3,}|~{3,})/;
const words = (s: string) => s.trim().split(/\s+/).length;

/** The label a line starts with, or null: alone up to 6 words, before words up to 4 (a longer
 *  start is a sentence that has a colon in it). */
export function labelOf(line: string): { label: string; rest: string } | null {
  const m = LABEL.exec(line);
  if (!m) return null;
  const [, label, rest = ""] = m;
  const alone = rest === "";
  if (label.length > (alone ? 48 : 32) || words(label) > (alone ? 6 : 4)) return null;
  return { label, rest };
}

/** The text with each label line set apart by blank lines and its label made bold; fenced code
 *  and indented lines stay as written. */
export function setApart(text: string): string {
  const out: string[] = [];
  let fence = "";
  let blankAfter = false;
  for (const line of text.split("\n")) {
    const f = FENCE.exec(line)?.[1];
    if (fence) {
      if (f && f[0] === fence[0] && f.length >= fence.length) fence = "";
      out.push(line);
      continue;
    }
    if (f) fence = f;
    const got = f ? null : labelOf(line);
    if (blankAfter && line.trim() !== "") out.push("");
    blankAfter = false;
    if (!got) { out.push(line); continue; }
    if (out.length && out[out.length - 1].trim() !== "") out.push("");
    out.push(got.rest ? `**${got.label}:** ${got.rest}` : `**${got.label}:**`);
    blankAfter = !got.rest;   // a heading's words below it are a paragraph of their own
  }
  return out.join("\n");
}

const PILL: Record<string, string> = { ok: "ok", done: "ok", attention: "attention", partly: "attention", failed: "failed" };

/** Marks the paragraphs setApart made: "said-label" for a label alone (its colon dropped),
 *  "said-lead" for one before words; Outcome's first word becomes a pill. */
function marks(md: MarkdownIt) {
  md.core.ruler.push("said_labels", (state) => {
    const tokens = state.tokens;
    for (let i = 0; i + 1 < tokens.length; i++) {
      const open = tokens[i], inline = tokens[i + 1];
      if (open.type !== "paragraph_open" || open.level !== 0 || inline.type !== "inline") continue;
      // markdown-it leaves empty text tokens around a paragraph's leading emphasis
      const kids = (inline.children ?? []).filter((c) => c.type !== "text" || c.content !== "");
      if (kids[0]?.type !== "strong_open" || kids[1]?.type !== "text" || kids[2]?.type !== "strong_close") continue;
      const label = kids[1].content;
      if (!label.endsWith(":")) continue;
      if (kids.length === 3) {
        open.attrJoin("class", "said-label");
        kids[1].content = label.slice(0, -1);
        continue;
      }
      open.attrJoin("class", "said-lead");
      const next = kids[3];
      const word = label === "Outcome:" && next?.type === "text" ? /^\s*(\p{L}+)/u.exec(next.content) : null;
      const kind = word ? PILL[word[1].toLowerCase()] : undefined;
      if (!word || !kind) continue;
      const pill = new state.Token("html_inline", "", 0);
      pill.content = ` <span class="said-pill said-${kind}">${md.utils.escapeHtml(word[1])}</span>`;
      next.content = next.content.slice(word[0].length);
      inline.children = [...kids.slice(0, 3), pill, ...kids.slice(3)];
    }
  });
}

const md = configuredMd().use(marks);

/** A comment's Markdown as it reads: the task's rules (taskMarkdownHtml), labels set apart. */
export const renderSaid = (text: string, env: MdEnv): string => md.render(setApart(text), env);
