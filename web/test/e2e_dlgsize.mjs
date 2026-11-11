// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Dialogs sized by their use (terminum.md AC-173) against a fake terminumd (fake.mjs): a dialog
// to type in takes 80% of its parent's width and height, its parent the dialog under it (RUN over
// a task's window), else the terminal for what is made from it, else the window, each kind held
// between its floor and ceiling; a question stays as tall as its words; a list's buttons stay at its
// foot when it is empty; on a phone a sheet as tall as the screen.

import { check, runningIn } from "./check.mjs";
import { drawn, openPage, PHONE, WIDE } from "./fake.mjs";

const BIG = { viewport: { width: 1920, height: 1080 } };

/** Opens `ids` one after another (each in a moment of its own, as a person does) and measures them. */
const open = (page, ids) => page.evaluate(async (ids) => {
  (await import("/taskboxes.js")).taskBoxes();
  for (const d of document.querySelectorAll("dialog[open]")) d.close();
  for (const id of ids) { document.getElementById(id).showModal(); await new Promise((r) => setTimeout(r, 30)); }
  const box = (el) => { const r = el.getBoundingClientRect(); return { w: Math.round(r.width), h: Math.round(r.height), bottom: Math.round(r.bottom) }; };
  const out = Object.fromEntries(ids.map((id) => [id, box(document.getElementById(id))]));
  out.term = box(document.getElementById("term"));
  out.win = { w: innerWidth, h: innerHeight };
  out.go = { bottom: Math.round(document.getElementById(ids.at(-1)).querySelector(".actions")?.getBoundingClientRect().bottom ?? 0) };
  for (const id of ["runmessage", "askq", "marknote"]) if (ids.some((d) => document.getElementById(d).contains(document.getElementById(id)))) out[id] = box(document.getElementById(id));
  return out;
}, ids);

const near = (a, b) => Math.abs(a - b) <= 1;
const clamp = (lo, v, hi) => Math.min(hi, Math.max(lo, v));

export async function webDialogSize(browser, name) {
  for (const [size, device] of [["1400×900", WIDE], ["1920×1080", BIG]]) {
    runningIn(`${name} ${size}`);
    const f = await openPage(browser, { device });
    await drawn(f.page);
    const run = await open(f.page, ["taskbox", "runbox"]);
    const task = { w: clamp(560, run.win.w * .8, 920), h: Math.max(420, run.win.h * .8) };
    check(`AC-173 a task's window: 80% of the window, 920 px wide at most (its prose's lines) (${size})`, near(run.taskbox.w, task.w) && near(run.taskbox.h, task.h), JSON.stringify(run));
    check(`AC-173 RUN over a task's window: 80% of that window's width and height, its message taking the rest (${size})`,
      near(run.runbox.w, clamp(520, run.taskbox.w * .8, 960)) && near(run.runbox.h, clamp(480, run.taskbox.h * .8, 880)) && run.runmessage.h > run.runbox.h / 2, JSON.stringify(run));
    const ask = await open(f.page, ["askbox"]);
    check(`AC-173 Ask: 80% of the terminal's width and height, 520–960 px wide, 480–560 px high (a question is short) (${size})`,
      near(ask.askbox.w, clamp(520, ask.term.w * .8, 960)) && near(ask.askbox.h, clamp(480, ask.term.h * .8, 560)) && ask.askq.h > 100, JSON.stringify(ask));
    const yes = await open(f.page, ["confirmbox"]);
    check(`AC-173 a question stays as tall as its words, 480 px wide at most (${size})`, yes.confirmbox.w <= 480 && yes.confirmbox.h < 250, JSON.stringify(yes));
    check("no page errors", !f.errors.length, f.errors.join(" | "));
    await f.close();
    const b = await openPage(browser, { device, url: "/kanban.html" });
    const list = await b.page.evaluate(() => { const d = document.getElementById("archivedbox"); d.showModal();
      const r = d.getBoundingClientRect(), a = d.querySelector(".actions").getBoundingClientRect();   // the bar holds the padding under its keys
      return { h: Math.round(r.height), win: innerHeight, gap: Math.round(r.bottom - a.bottom) }; });
    check(`AC-173 the archived list, empty, keeps its Close at its foot, 80% of the window high (${size})`,
      near(list.h, clamp(360, list.win * .8, 760)) && Math.abs(list.gap) <= 1, JSON.stringify(list));
    await b.close();
  }
  runningIn(`${name} 390×844`);
  const p = await openPage(browser, { device: PHONE });
  await drawn(p.page);
  const sheet = await open(p.page, ["runbox"]);
  const yes = await open(p.page, ["confirmbox"]);
  check("AC-173 on a phone a dialog to type in is a sheet as wide as the screen less 16 px a side and as tall as what is in sight less 16 px a side, its buttons in sight; a question as tall as its words",
    sheet.runbox.w === sheet.win.w - 32 && near(sheet.runbox.h, sheet.win.h - 32) && sheet.go.bottom <= sheet.runbox.bottom && yes.confirmbox.h < 300, JSON.stringify({ sheet, yes }));
  check("no page errors", !p.errors.length, p.errors.join(" | "));
  await p.close();
}
