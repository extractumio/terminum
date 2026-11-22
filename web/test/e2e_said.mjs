// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A comment as it reads (terminum-kanban.md AC-149, ui/src/said.ts) in a task's window against a fake terminumd, with
// ui/dist's renderers as terminumd serves them: a scheduled run's result and an agent's questions written as label
// lines read as sections (a label alone heads what follows, a label before words is their lead, never joined to the
// line or list item before it), Outcome's word a status box in its status's hue, lines of a readable measure, in a dark and
// a light theme. TM_TEST_SHOTS=<folder> keeps a screenshot of each.

import fs from "node:fs";
import path from "node:path";
import { check, runningIn } from "./check.mjs";
import { drawn, openPage, serveRenderers, THEME, SIZES } from "./fake.mjs";

const AT = 1794435120;
const RESULT = `Outcome: attention
Summary: In the last 24 hours 12 integrations landed on origin/main, covering 11 tasks. The nightly full run on the newest main is red on one regression, AC-123 (API-47, P1).
Details:
New features:
- API-21 (AC-209): when someone comments on a task, the agent working on it is told at its next tool call.
- API-23 (AC-209): an idle agent hears of a new comment with its next prompt.
Changed / fixed:
- API-16 (AC-207): a Codex attached to a shared app-server lists one tool, \`why_no_terminum_tools\`.
For the human: close API-41, then run API-36 again.`;
const QUESTIONS = `Why you are needed: the shared build Mac stayed above its load limit for the whole 15-minute gate.
Recommended:
1. RUN API-13 again when the build Mac is quiet. Other options: leave it for a maintenance window.
Details: Commit 1635162b holds the fix and its regression test; the targeted run passed 488 script tests.`;
const said = (id, text, more = {}) => ({ id, kind: "comment", author_kind: "agent", author_name: "Claude Opus", at: AT, text, ...more });
const task = (id, status, comment) => ({ id, uid: "abfed14f", title: "Report on what landed", status, archived_at: null, agent: { name: "Claude Opus", color: 0, conversation: null },
  severity: "P2", owner: "alex", owner_name: "alex", runs_on: null, project: "default", session: null, budget_seconds: null, text_rev: 1, text: "Report.",
  files: [], run_holds: null, waits: null, schedule: null, parent: null, children: [], subtasks: 0, subtasks_done: 0, created_at: AT - 60, updated_at: AT, history: [comment] });
const TASKS = { "API-30": task("API-30", "DONE", said("r1", RESULT)), "API-31": task("API-31", "WAIT FOR HUMAN", said("q1", QUESTIONS, { questions: true })) };
const META = { projects: [{ id: "default", name: "Default" }], users: [{ id: "alex", name: "alex" }], sessions: [], agents: [{ id: "a1", project: "default", name: "Claude Opus", color: 0 }], hosts: [], most_bytes: 1e7 };

/** The window on a task, and what its leading comment's text is made of. */
const read = (page, id) => page.evaluate(async ({ id, meta }) => {
  const { taskWindow } = await import("/kanbanwin.js");
  window.taskwin ??= taskWindow({ getMeta: () => meta, runs: { open: async () => false }, openTask() {}, newSubtask() {}, openSchedule() {} });
  await window.taskwin.tryClose();
  await window.taskwin.open(id);
  const md = document.querySelector("#taskbody .leads .ev-comment .markdown");
  for (let i = 0; i < 100 && !md.querySelector("p"); i++) await new Promise((r) => setTimeout(r, 20));   // the renderers load on first need
  const pill = md.querySelector(".stbox"), fs = parseFloat(getComputedStyle(md).fontSize);
  return {
    blocks: [...md.children].map((e) => `${e.tagName}${e.className ? "." + e.className : ""}:${e.textContent.trim().slice(0, 24)}`),
    pill: pill && { words: pill.textContent, bg: getComputedStyle(pill).backgroundColor, fg: getComputedStyle(pill).color },
    measure: Math.round(md.getBoundingClientRect().width / fs), over: md.scrollWidth - md.clientWidth,
    gapAbove: (() => { const l = md.querySelector("p.said-label:not(:first-child)"); return l ? Math.round(l.getBoundingClientRect().top - l.previousElementSibling.getBoundingClientRect().bottom) : null; })(),
  };
}, { id, meta: META });

export async function webSaid(browser, name) {
  for (const [size, device] of SIZES) {
    for (const light of [false, true]) {
      runningIn(`${name} ${size}${light ? " light" : ""}`);
      const f = await openPage(browser, { device, hook: (m, say) => {
        if (light && m.t === "sub") say({ t: "theme", sid: m.id, theme: { ...THEME, bg: "#ffffff", fg: "#1f2328", dark: false } });
      } });
      await serveRenderers(f.page);
      for (const id of Object.keys(TASKS)) await f.page.route(`**/api/tasks/${id}`, (r) => r.fulfill({ contentType: "application/json", body: JSON.stringify(TASKS[id]) }));
      await drawn(f.page);
      const shot = (id) => process.env.TM_TEST_SHOTS && f.page.screenshot({ path: path.join(process.env.TM_TEST_SHOTS, `said-${id}-${name}-${size}${light ? "-light" : ""}.png`) });
      if (process.env.TM_TEST_SHOTS) fs.mkdirSync(process.env.TM_TEST_SHOTS, { recursive: true });
      const r = await read(f.page, "API-30");
      await shot("API-30");
      const q = await read(f.page, "API-31");
      await shot("API-31");
      const got = JSON.stringify({ r, q });
      check("AC-149 a result's label lines read as sections: Outcome and Summary as leads, Details and each group a heading over its list",
        r.blocks.join("|") === "P.said-lead:Outcome: attention|P.said-lead:Summary: In the last 24 |P.said-label:Details|P.said-label:New features|UL:API-21 (AC-209): when so|P.said-label:Changed / fixed|UL:API-16 (AC-207): a Codex|P.said-lead:For the human: close API", got);
      check("AC-149 Outcome's word is a status box in the waiting hue, its ink readable", r.pill?.words === "attention" && r.pill.bg === "rgb(255, 176, 32)" && r.pill.fg !== r.pill.bg, got);
      check("AC-149 an agent's questions: Details stands after the recommended answers, not inside the last one",
        q.blocks.join("|") === "P.said-lead:Why you are needed: the |P.said-label:Recommended|OL:RUN API-13 again when th|P.said-lead:Details: Commit 1635162b", got);
      check("AC-149 a comment's lines keep a readable measure (at most 80 ems), nothing past its edge; a heading has room above it",
        r.measure <= 80 && r.over <= 0 && q.over <= 0 && r.gapAbove >= 12, got);
      check("no page errors", !f.errors.length, f.errors.join(" | "));
      await f.close();
    }
  }
}
