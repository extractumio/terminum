// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A task's window's layout (terminum-kanban.md AC-149) against a fake terminumd (fake.mjs), on the session page,
// which opens the same window as the board (AC-150): the task's part on one grid (its lines on one left edge, 12 px
// apart, its fields alike, three to a line, two on a phone), its status whole on a line, the three parts a section
// break apart, a screen quoted in a line of history as it stood, nothing past the window's edge.

import { check, runningIn } from "./check.mjs";
import { drawn, openPage, SIZES } from "./fake.mjs";

const AT = 1794435120, FILE = "0123456789abcdef0123456789abcdef";
const SCREEN = ` ▐▛███▜▌   Claude Code\n${"─".repeat(200)}\n❯ [Pasted text #1 +23 lines]`;
const TASK = {
  id: "API-12", uid: "abfed14f", title: "Count what the agents used", status: "WAIT FOR HUMAN", archived_at: null,
  agent: { name: "Claude Opus", color: 0, conversation: "1fcaa51f-bc32-4238-a077-9c45c2044cf1" }, severity: "P2", owner: "alex", owner_name: "alex", runs_on: null,
  project: "default", session: { key: "k1", closed: false }, budget_seconds: 3600, text_rev: 1, text: "Count it after each run, then once an hour.",
  files: [{ id: FILE, name: "a screenshot with a long name 2026-11-11 at 22.11.27.png", bytes: 35000 }], run_holds: null, waits: null,
  schedule: { key: "SCH-1", title: "Nightly sweep" }, parent: null, children: [], subtasks: 0, subtasks_done: 0, created_at: AT - 60, updated_at: AT,
  history: [
    { id: "e1", kind: "event", author_kind: "system", author_name: "Dispatcher", at: AT, text: `Session did not start: no Enter\n\n\`\`\`\n${SCREEN}\n\`\`\`` },
    { id: "e2", kind: "comment", author_kind: "human", author_name: "alex", at: AT + 60, text: "Both windows." },
  ],
};
const META = { projects: [{ id: "default", name: "Default" }], users: [{ id: "alex", name: "alex" }], sessions: [{ key: "k1", title: "api" }], agents: [{ id: "a1", name: "Claude Opus", color: 0 }], hosts: [], most_bytes: 1e7 };

/** Opens the window on TASK and measures it: every box as [left, top, right, bottom], rounded. */
const measure = (page) => page.evaluate(async (meta) => {
  const { taskWindow } = await import("/kanbanwin.js");
  await taskWindow({ getMeta: () => meta, runs: { open: async () => false }, openTask() {}, newSubtask() {}, openSchedule() {} }).open("API-12");
  await new Promise((r) => setTimeout(r, 100));
  const box = (e) => { const r = e.getBoundingClientRect(); return [r.left, r.top, r.right, r.bottom].map(Math.round); };
  const body = document.getElementById("taskbody"), own = body.querySelector(".taskown"), q = (s) => own.querySelector(s);
  return {
    fields: [...own.querySelectorAll(".fieldgrid > .fieldrow")].map((f) => ({ name: f.firstChild.textContent.trim(), row: box(f), field: box(f.querySelector(".field")) })),
    lines: [...own.children].map((e) => ({ cls: e.className, box: box(e) })),
    inner: [box(own)[0] + parseFloat(getComputedStyle(own).paddingLeft), box(own)[2] - parseFloat(getComputedStyle(own).paddingRight)].map(Math.round),
    status: { box: box(q(".stbox")), agent: box(q(".cagent")), conv: q(".cconv").getClientRects().length, value: box(q(".fieldvalue")), words: q(".stbox").textContent },
    keys: box(q(".tasktext .seckeys")), parts: [...body.children].map(box),
    // el() asks for a symbol's text form (U+FE0E): not part of what the screen showed
    said: { words: body.querySelector(".by-system .evtext").textContent, screen: body.querySelector(".evscreen")?.textContent.replaceAll("\uFE0E", ""), tag: body.querySelector(".evscreen")?.tagName,
      wide: body.querySelector(".evscreen").scrollWidth > body.querySelector(".evscreen").clientWidth },
    over: body.scrollWidth - body.clientWidth,
  };
}, META);

export async function webTaskWin(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const phone = size !== "1400×900", per = phone ? 2 : 3;
    const f = await openPage(browser, { device });
    await f.page.route("**/api/tasks/API-12", (r) => r.fulfill({ contentType: "application/json", body: JSON.stringify(TASK) }));
    await drawn(f.page);
    const m = await measure(f.page), said = JSON.stringify({ fields: m.fields, lines: m.lines, inner: m.inner, status: m.status, keys: m.keys });
    check("AC-149 its fields in the new-task window's order: Project, Severity, Agent, Owner, Session, Budget", m.fields.map((x) => x.name).join() === "Project,Severity,Agent,Owner,Session,Budget, minutes", said);
    const lefts = [...new Set(m.fields.map((x) => x.field[0]))], tops = [...new Set(m.fields.map((x) => x.field[1]))];
    const spread = (at) => { const all = m.fields.map(at); return Math.max(...all) - Math.min(...all); };   // boxes are rounded: a pixel apart is alike
    check(`AC-149 its fields on one grid: ${per} to a line, alike in width and height, a label 4 px over each`,
      lefts.length === per && tops.length === 6 / per && spread((x) => x.field[2] - x.field[0]) <= 1 && spread((x) => x.field[3] - x.field[1]) <= 1 && m.fields.every((x, i) => x.field[0] === lefts[i % per] && x.field[0] === x.row[0]), said);
    check("AC-149 the task's lines stand on one left edge, its fields and Text's keys on one right edge",
      m.lines.every((l) => l.box[0] === m.inner[0]) && m.fields.filter((_, i) => i % per === per - 1).every((x) => x.field[2] === m.inner[1]) && m.keys[2] === m.inner[1], said);
    const gaps = m.lines.slice(1).map((l, i) => [l.cls, l.box[1] - m.lines[i].box[3]]);
    check("AC-149 its groups stand 12 px apart, a second line of when and where 4 px under the first", gaps.every(([cls, gap]) => gap === (cls === "taskfrom" ? 4 : 12)), JSON.stringify(gaps));
    const mid = (b) => (b[1] + b[3]) / 2;
    check("AC-149 its status is whole on a line with its agent, its conversation's ID unbroken where it fits",
      m.status.words === "WAIT FOR HUMAN" && m.status.box[3] - m.status.box[1] <= 24 && Math.abs(mid(m.status.box) - mid(m.status.agent)) <= 1 && (phone || m.status.conv === 1), said);
    check("AC-149 the task, its subtasks, its comments and history, the comment field: a section break (24 px) apart", m.parts.slice(1).every((b, i) => b[1] - m.parts[i][3] === 24), JSON.stringify(m.parts));
    check("AC-149 a screen quoted in a line of history stands as it was drawn, scrolling sideways in its own box",
      m.said.tag === "PRE" && m.said.screen === SCREEN && m.said.words === "Session did not start: no Enter" && m.said.wide, JSON.stringify(m.said));
    check("AC-149 nothing reaches past the window's edge", m.over <= 0, String(m.over));
    check("no page errors", !f.errors.length, f.errors.join(" | "));
    await f.close();
  }
}
