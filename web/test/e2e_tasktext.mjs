// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A task's text is typed in one editor everywhere (terminum-kanban.md AC-106), against a fake terminumd (fake.mjs):
// a new task's, a schedule's and an open task's Text alike: Write and Preview over it, the terminal's type, at
// least 16 lines in sight however many fields the window holds.

import { check, runningIn } from "./check.mjs";
import { drawn, openPage, PHONE, SIZES } from "./fake.mjs";

const AT = 1794435120;
const TASK = { id: "API-12", uid: "abfed14f", title: "Count what the agents used", status: "TODO", archived_at: null, agent: null, severity: "P2",
  owner: "alex", owner_name: "alex", runs_on: null, project: "default", session: null, budget_seconds: null, text_rev: 1, text: "Count it.\n\nThen **once** an hour.",
  files: [], run_holds: null, waits: null, schedule: null, parent: null, children: [], subtasks: 0, subtasks_done: 0, created_at: AT, updated_at: AT,
  history: [{ id: "e1", kind: "event", author_kind: "human", author_name: "alex", at: AT, text: "alex made it" }] };
const META = { projects: [{ id: "default", name: "Default" }], users: [{ id: "alex", name: "alex" }], sessions: [], agents: [], hosts: [], most_bytes: 1e7 };
const LONG = "a line of what the agent gets, long enough to wrap where the box is narrow\n".repeat(80);

/** Each window's text field: its lines in sight, its type, its Write and Preview keys. */
const measure = (page) => page.evaluate(async ([meta, long]) => {
  const of = (area) => { const s = getComputedStyle(area), lh = parseFloat(s.lineHeight) || parseFloat(s.fontSize) * 1.2;
    const seg = area.closest(".xbox")?.previousElementSibling?.querySelector('[role="tablist"]');
    return { lines: Math.round((area.clientHeight - parseFloat(s.paddingTop) - parseFloat(s.paddingBottom)) / lh * 10) / 10, font: s.fontFamily,
      tabs: seg ? [...seg.querySelectorAll('[role="tab"]')].map((b) => b.textContent).join() : "" }; };
  const out = {};
  for (const [dlg, area] of [["newbox", "newtext"], ["schedbox", "schedtext"]]) {
    for (const d of document.querySelectorAll("dialog[open]")) d.close();
    document.getElementById("schedprompt").textContent = long;
    document.getElementById(dlg).showModal();
    out[dlg] = of(document.getElementById(area));
  }
  for (const d of document.querySelectorAll("dialog[open]")) d.close();
  const { taskWindow } = await import("/kanbanwin.js");
  const win = taskWindow({ getMeta: () => meta, runs: { open: async () => false }, openTask() {}, newSubtask() {}, openSchedule() {} });
  await win.open("API-12");
  document.querySelector('#taskbody .tasktext button[aria-label="Edit"]').click();
  const area = document.querySelector("#taskbody .tasktext textarea");
  out.taskbox = of(area);
  // its Preview, as tall as the field, in its place; Write gives the field back as it was
  const tabs = [...document.querySelectorAll('#taskbody .tasktext [role="tab"]')];
  tabs[1].click();
  const shown = area.parentElement.querySelector(".markdown");
  // the fake serves no Panel renderer: the preview holds the words as they stand (drawn Markdown: e2e_liveboard.mjs)
  out.preview = { words: shown.textContent.includes("once"), field: area.hidden };
  out.preview.taller = shown.getBoundingClientRect().height >= parseFloat(getComputedStyle(area).minHeight) - 1;
  tabs[0].click();
  out.back = { shown: !area.hidden && shown.hidden, value: area.value };
  return out;
}, [META, LONG]);

export async function webTaskText(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const f = await openPage(browser, { device, url: "/kanban.html" });
    await f.page.route("**/api/tasks/API-12", (r) => r.fulfill({ contentType: "application/json", body: JSON.stringify(TASK) }));
    const m = await measure(f.page), said = JSON.stringify(m);
    const fields = [m.newbox, m.schedbox, m.taskbox];
    check(`AC-106 a new task's, a schedule's and an open task's text: 16 lines in sight at least (${size})`, fields.every((x) => x.lines >= 15.9), said);
    check(`AC-106 …in one type, the terminal's (${size})`, new Set(fields.map((x) => x.font)).size === 1 && /TermFont/.test(m.newbox.font), said);
    check(`AC-106 …each with Write and Preview over it (${size})`, fields.every((x) => x.tabs === "Write,Preview"), said);
    check(`AC-106 an open task's Preview stands in its field's place, as tall; Write gives the field back as typed (${size})`,
      m.preview.words && m.preview.field && m.preview.taller && m.back.shown && m.back.value === TASK.text, said);
    check("no page errors", !f.errors.length, f.errors.join(" | "));
    await f.close();
  }
}
