// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A task's window's layout (terminum-kanban.md AC-149) against a fake terminumd (fake.mjs), on the session page,
// which opens the same window as the board (AC-150): the task's part on one grid (its lines on one left edge, 12 px
// apart, its fields alike, three to a line, two on a phone), its status whole on a line, the three parts a section
// break apart, a screen quoted in a line of history as it stood, nothing past the window's edge; its comments a
// conversation with a bar per kind, what the human reacts to first when the task waits for it.

import fs from "node:fs";
import path from "node:path";
import { check, coversWindow, runningIn } from "./check.mjs";
import { drawn, openPage, PHONE, SIZES } from "./fake.mjs";

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
    { id: "e3", kind: "event", author_kind: "human", author_name: "alex", at: AT + 30, text: "alex changed its title\n\n```\nnot a screen\n``` and more" },
    { id: "e2", kind: "comment", author_kind: "human", author_name: "alex", at: AT + 60, text: "Both windows." },
  ],
};
// a task that waits with its agent's questions: every kind of comment before them
const said = (id, author_kind, author_name, at, text, more = {}) => ({ id, kind: "comment", author_kind, author_name, at, text, ...more });
const ASKED = { ...TASK, id: "API-14", files: [], history: [
  { id: "a1", kind: "event", author_kind: "human", author_name: "alex", at: AT - 86400 * 400, text: "alex made it" },
  said("a2", "human", "alex", AT - 40, "Start with v1."),
  said("a3", "agent", "Claude Opus", AT - 30, "On it."),
  said("a4", "system", "Dispatcher", AT - 20, "Session started."),
  said("a5", "agent", "Claude Opus", AT - 10, "v1 or v2?", { questions: true }),
] };
const META = { projects: [{ id: "default", name: "Default" }], users: [{ id: "alex", name: "alex" }], sessions: [{ key: "k1", title: "api" }], agents: [{ id: "a1", name: "Claude Opus", color: 0 }], hosts: [], most_bytes: 1e7 };

/** Opens the window on TASK and measures it: every box as [left, top, right, bottom], rounded. */
const measure = (page) => page.evaluate(async (meta) => {
  const { taskWindow } = await import("/kanbanwin.js");
  window.taskwin = taskWindow({ getMeta: () => meta, runs: { open: async () => false }, openTask() {}, newSubtask() {}, openSchedule() {} });
  await window.taskwin.open("API-12");
  const box = (e) => { const r = e.getBoundingClientRect(); return [r.left, r.top, r.right, r.bottom].map(Math.round); };
  const body = document.getElementById("taskbody"), own = body.querySelector(".taskown"), q = (s) => own.querySelector(s);
  const frame = own.getBoundingClientRect(), pad = getComputedStyle(own), pre = body.querySelector(".evscreen");
  return {
    fields: [...own.querySelectorAll(".fieldgrid > .fieldrow")].map((f) => ({ name: f.firstChild.textContent.trim(), row: box(f), field: box(f.querySelector(".field")) })),
    lines: [...own.children].map((e) => ({ cls: e.className, box: box(e) })),
    inner: [frame.left + parseFloat(pad.paddingLeft), frame.right - parseFloat(pad.paddingRight)].map(Math.round),
    status: { box: box(q(".stbox")), agent: box(q(".cagent")), conv: q(".cconv").getClientRects().length, value: box(q(".fieldvalue")), words: q(".stbox").textContent },
    keys: box(q(".tasktext .seckeys")), parts: [...body.children].map(box),
    // el() asks for a symbol's text form (U+FE0E): not part of what the screen showed
    plain: { words: body.querySelector(".ev-event.by-human .evtext").textContent, screens: body.querySelectorAll(".evscreen").length },
    said: { words: body.querySelector(".by-system .evtext").textContent, screen: pre?.textContent.replaceAll("\uFE0E", ""), tag: pre?.tagName, wide: !!pre && pre.scrollWidth > pre.clientWidth },
    over: body.scrollWidth - body.clientWidth,
    // its keys at the window's foot, outside what scrolls (terminum.md AC-173)
    acts: (() => { body.scrollTop = 0; const a = document.getElementById("taskacts"), r = a.getBoundingClientRect(), d = document.getElementById("taskbox").getBoundingClientRect();
      return { keys: [...a.children].map((b) => b.textContent).join(), inBody: body.contains(a), shown: r.top >= body.getBoundingClientRect().bottom - 1 && r.bottom <= d.bottom + 1, scrolls: body.scrollHeight > body.clientHeight }; })(),
  };
}, META);

/** The window on ASKED, then on TASK as it changes under its reader: the order of its parts and of what was said. */
const conversation = (page) => page.evaluate(async () => {
  const body = document.getElementById("taskbody"), parts = () => [...body.children].map((e) => e.className.split(" ").at(-1));
  const lines = () => [...body.querySelectorAll(".activity .history > li")].map((li) => li.querySelector(".markdown, .evtext").textContent.trim());
  const bar = (cls) => getComputedStyle(body.querySelector(`.ev-comment${cls}`), "::before").backgroundColor;
  await window.taskwin.tryClose();
  await window.taskwin.open("API-14");
  const pad = parseFloat(getComputedStyle(body).paddingRight), edge = body.getBoundingClientRect();
  const asked = { parts: parts(), lead: [...body.querySelectorAll(".leads .history > li")].map((li) => li.className), heading: body.querySelector(".leads h3").textContent, rest: lines(),
    bars: [".by-human", ".by-agent:not(.ev-questions)", ".by-system", ".ev-questions"].map(bar), field: body.querySelector(".newcomment textarea").placeholder,
    times: [...body.querySelectorAll(".activity time")].map((t) => [t.textContent, t.title]), named: body.querySelectorAll(".activity .ev-event strong").length,
    // the scrollbar's room: nothing that scrolls reaches into it, at either side
    room: pad >= 12 && [...body.children].every((e) => { const r = e.getBoundingClientRect(); return r.left >= edge.left + pad - 1 && r.right <= edge.right - pad + 1; }) };
  await window.taskwin.tryClose();
  await window.taskwin.open("API-12");
  const read = { parts: parts(), first: lines()[0] };
  window.agentAsked = true;                            // its agent asks while the window is open
  await window.taskwin.refresh();
  return { asked, read, stays: { parts: parts(), last: lines().at(-1) } };
});

export async function webTaskWin(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const phone = device === PHONE, per = phone ? 2 : 3;
    const f = await openPage(browser, { device });
    await f.page.route("**/api/tasks/API-12", async (r) => r.fulfill({ contentType: "application/json", body: JSON.stringify(await f.page.evaluate(() => window.agentAsked) ? { ...TASK, history: [...TASK.history, ASKED.history.at(-1)] } : TASK) }));
    await f.page.route("**/api/tasks/API-14", (r) => r.fulfill({ contentType: "application/json", body: JSON.stringify(ASKED) }));
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
    check("AC-149 its groups stand 12 px apart", gaps.length === 4 && gaps.every(([, gap]) => gap === 12), JSON.stringify(gaps));
    const mid = (b) => (b[1] + b[3]) / 2;
    check("AC-149 its status is whole on a line with its agent, its conversation's ID unbroken where it fits",
      m.status.words === "WAIT FOR HUMAN" && m.status.box[3] - m.status.box[1] <= 24 && Math.abs(mid(m.status.box) - mid(m.status.agent)) <= 1 && (phone || m.status.conv === 1), said);
    check("AC-149 the task, its subtasks, its comments and history, the comment field: a section break (24 px) apart", m.parts.slice(1).every((b, i) => b[1] - m.parts[i][3] === 24), JSON.stringify(m.parts));
    check("AC-149 a screen quoted in a line of history stands as it was drawn, scrolling sideways in its own box",
      m.said.tag === "PRE" && m.said.screen === SCREEN && m.said.words === "Session did not start: no Enter" && m.said.wide, JSON.stringify(m.said));
    check("AC-149 a line of history that only holds ``` lines stays as it was written", m.plain.screens === 1 && m.plain.words === TASK.history[1].text, JSON.stringify(m.plain));
    check("AC-149 nothing reaches past the window's edge", m.over <= 0, String(m.over));
    check("AC-173 its keys stand at the window's foot, in sight while the task scrolls above them",
      m.acts.keys === "Archive,Save,Run" && !m.acts.inBody && m.acts.shown, JSON.stringify(m.acts));
    // the text being edited: its head holds the word alone, so Text stays 12 px under the fields and 4 px over its field
    await f.page.click('#taskbody .tasktext button[aria-label="Edit"]');
    const edit = await f.page.evaluate(() => { const top = (s) => document.querySelector(`#taskbody ${s}`).getBoundingClientRect();
      const word = top(".tasktext .edithead span"), grid = top(".taskown .fieldgrid"), area = top(".tasktext textarea");
      const keys = [...document.querySelectorAll("#taskbody .tasktext .actions button")], purge = keys[0], save = keys.at(-1);
      return { over: Math.round(word.top - grid.bottom), under: Math.round(area.top - word.bottom), keys: document.querySelectorAll("#taskbody .tasktext .seckeys").length,
        acts: keys.map((b) => b.textContent).join(), left: Math.round(purge.getBoundingClientRect().left - area.left), right: Math.round(area.right - save.getBoundingClientRect().right),
        red: getComputedStyle(purge).color !== getComputedStyle(keys[1]).color }; });
    check("AC-149 a text being edited keeps Text 12 px under the fields and 4 px over its field, its keys gone", edit.over === 12 && edit.under === 4 && edit.keys === 0, JSON.stringify(edit));
    check("AC-109 Purge text stands only by a text being edited: at the left in the danger color, Cancel and Save at the right",
      edit.acts === "Purge text,Cancel,Save" && edit.left === 0 && edit.right === 0 && edit.red, JSON.stringify(edit));
    // its boxes to read or type in, each with its key to the whole window (terminum.md AC-184)
    const keyed = await f.page.evaluate(() => [".tasktext", ".newcomment", ".history"].map((s) => !!document.querySelector(`#taskbody ${s} .xbox:has(> :is(textarea, .evscreen)) > .xkey`)));
    await f.page.click("#taskbody .xbox:has(> .evscreen) > .xkey");
    const screen = await coversWindow(f.page, ".xbox.full:has(> .evscreen)");
    // a board update while it covers the window redraws nothing under it
    const held = await f.page.evaluate(async () => { const pre = document.querySelector("#taskbody .evscreen");
      await window.taskwin.refresh();
      return pre.isConnected && pre.closest(".xbox").classList.contains("full"); });
    await f.page.keyboard.press("Escape");
    const back = await f.page.evaluate(() => ({ full: !!document.querySelector(".xbox.full"), open: document.getElementById("taskbox").open }));
    check("AC-184 its text being edited, the comment field and a quoted screen have their key; the screen's covers the window, Esc gives it back",
      keyed.every(Boolean) && held && screen.covers && !back.full && back.open, JSON.stringify({ keyed, held, screen, back }));
    await f.page.evaluate(() => window.taskwin.refresh(true));
    const c = await conversation(f.page), told = JSON.stringify(c);
    check("AC-149 a task that waits with questions shows them first, the comment field under them, then the task, the rest, its subtasks",
      c.asked.parts.join() === "leads,newcomment,taskown,activity,subtasks" && c.asked.lead.length === 1 && /ev-questions/.test(c.asked.lead[0]) && c.asked.heading === "Asks you" && /^Your answer/.test(c.asked.field), told);
    check("AC-149 and the rest of what was said newest first, without the questions", c.asked.rest.join("|") === "Session started.|On it.|Start with v1.|alex made it", told);
    check("AC-149 each kind of comment has its own bar: a human's cornflower blue, an agent's, Dispatcher's, the questions'",
      new Set(c.asked.bars).size === 4 && c.asked.bars[0] === "rgb(100, 149, 237)", told);
    check("AC-149 a time is no longer than its whole date and time, which its title holds; a line of history names its author once",
      c.asked.times.every(([short, whole]) => short.length <= whole.length && /\d{4}/.test(whole)) && c.asked.named === 0, told);
    check("AC-149 with nothing to react to, the task, its subtasks and what was said oldest first, the comment field last",
      c.read.parts.join() === "taskown,subtasks,activity,newcomment" && c.read.first === "Session did not start: no Enter", told);
    check("AC-149 questions asked while the window is open turn nothing round: they stand last, over the comment field",
      c.stays.parts.join() === "taskown,subtasks,activity,newcomment" && c.stays.last === "v1 or v2?", told);
    check("AC-173 what scrolls keeps clear of the scrollbar's room at the window's edge", c.asked.room, told);
    await f.page.click("#taskbody .tasktext .markdown");
    const ring = await f.page.$eval("#taskbody .tasktext .markdown", (text) => [document.activeElement === text, getComputedStyle(text).outlineStyle]);
    check("AC-149 a text pressed to read takes the keys and shows no focus ring", ring[0] && ring[1] === "none", String(ring));
    if (process.env.TM_TEST_SHOTS) {
      fs.mkdirSync(process.env.TM_TEST_SHOTS, { recursive: true });
      for (const [id, what] of [["API-12", "read"], ["API-14", "asked"]]) {
        await f.page.evaluate(async (task) => { window.agentAsked = false; await window.taskwin.tryClose(); await window.taskwin.open(task); }, id);
        await f.page.screenshot({ path: path.join(process.env.TM_TEST_SHOTS, `taskwin-${what}-${name}-${size}.png`) });
      }
    }
    check("no page errors", !f.errors.length, f.errors.join(" | "));
    await f.close();
  }
}
