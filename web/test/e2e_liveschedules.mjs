// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The board's folded cards and its rows on the columns' tracks against a real private terminumd
// (terminum-kanban.md AC-159, terminum-dispatcher.md AC-138): `node web/test/e2e_live.mjs board chromium`.
import { check, until } from "./check.mjs";
import { boardKit, nap } from "./boardkit.mjs";

export async function liveFolded(page, live) {
  const { j, card, open } = boardKit(page, live);
  const shot = (name) => live.shots && page.screenshot({ path: `${live.shots}${live.shots.endsWith("-") ? "" : "/"}${name}.png` });
  const made = async (title, severity = "P2") => (await j("/api/tasks", "POST", { title, severity, body: "**text** of it" })).id;
  const a = await made("Folded first task with a title long enough to be cut at the line's end", "P1"), b = await made("Folded second");
  // three agents and one more: a row of four on the columns' tracks, the fifth starts the next
  const ids = [];
  for (const n of ["Grid one", "Grid two", "Grid three", "Grid four", "Grid five"]) ids.push((await j("/api/task-agents", "POST", { name: n, kind: "claude", command: "claude" })).id);
  await open();
  const box = (sel) => page.evaluate((s) => { const r = document.querySelector(s)?.getBoundingClientRect(); return r && { x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.width), h: Math.round(r.height) }; }, sel);
  const shown = (sel) => page.evaluate((s) => { const e = document.querySelector(s); return !!e && getComputedStyle(e).display !== "none" && e.getClientRects().length > 0; }, sel);

  // AC-159: a line a card in TODO, its ID, severity and title only
  const lineH = (await box(card(a))).h;
  // one line: ID, severity and title on one row, the title cut, nothing under them
  const row = await page.evaluate((s) => {
    const mid = (q) => { const r = document.querySelector(`${s} ${q}`).getBoundingClientRect(); return r.top + r.height / 2; };
    const t = document.querySelector(`${s} .ctitle`);
    return { spread: Math.max(mid(".cid"), mid(".csev"), mid(".ctitle")) - Math.min(mid(".cid"), mid(".csev"), mid(".ctitle")), cut: getComputedStyle(t).whiteSpace === "nowrap" && t.scrollWidth > t.clientWidth };
  }, card(a));
  check("AC-159 a TODO card is one line: ID, severity, title", row.spread < 4 && row.cut && !(await shown(`${card(a)} .cfoot`)), JSON.stringify(row));
  check("AC-159 it says collapsed", await page.getAttribute(card(a), "aria-expanded") === "false");
  await shot("folded");
  await page.click(`${card(a)} .ctitle`);
  await nap(150);
  check("AC-159 a press shows it whole, no window", (await box(card(a))).h > lineH && await shown(`${card(a)} .cfoot`) && !(await page.isVisible("#taskbox[open]")) && await page.getAttribute(card(a), "aria-expanded") === "true");
  await shot("unfolded");
  await page.click(`${card(b)} .ctitle`);
  await nap(150);
  check("AC-159 another's press folds the first", (await box(card(a))).h === lineH && await shown(`${card(b)} .cfoot`));
  await page.click(`${card(b)} .ctitle`);
  check("AC-159 a press on the whole card opens its window", await until(() => page.isVisible("#taskbox[open]")));
  await page.keyboard.press("Escape");
  await until(async () => !(await page.isVisible("#taskbox[open]")));
  await page.keyboard.press("Escape");
  await nap(100);
  check("AC-159 Esc folds it", (await box(card(b))).h === lineH, String((await box(card(b))).h));
  await j(`/api/tasks/${a}`, "PATCH", { title: "Folded first, renamed" });
  await page.click(`${card(a)} .ctitle`);
  await nap(100);
  await j(`/api/tasks/${a}`, "PATCH", { title: "Folded first, renamed again" });
  check("AC-159 shown whole stays so as the columns are read again", await until(async () => (await page.textContent(`${card(a)} .ctitle`)) === "Folded first, renamed again") && await shown(`${card(a)} .cfoot`));

  // AC-138: an agent a column wide, four a row, + Agent in the fifth track
  const cols = await page.evaluate(() => [...document.querySelectorAll("#board .col")].map((c) => { const r = c.getBoundingClientRect(); return [Math.round(r.x), Math.round(r.width)]; }));
  const agents = await page.evaluate(() => [...document.querySelectorAll(".agents .agentcard")].map((c) => { const r = c.getBoundingClientRect(); return [Math.round(r.x), Math.round(r.width), Math.round(r.y)]; }));
  const add = await box("#addagent");
  const wide = page.viewportSize().width >= 860;
  if (wide) {
    const aligned = agents.slice(0, 4).every(([x, w], i) => Math.abs(x - cols[i][0]) <= 1 && Math.abs(w - cols[i][1]) <= 1);
    check("AC-138 agents stand a column wide on the columns' tracks, four a row", aligned && agents[4][2] > agents[0][2] && Math.abs(agents[4][0] - cols[0][0]) <= 1, JSON.stringify({ cols, agents }));
    check("AC-138 + Agent stands in the fifth track", add.x >= cols[4][0] - 1 && add.x < cols[4][0] + cols[4][1], JSON.stringify(add));
  } else {
    check("AC-138 a phone keeps one row of agents that scrolls", agents.every(([, , y]) => y === agents[0][2]), JSON.stringify(agents));
  }
  await shot("agents");
  for (const id of ids) await j(`/api/task-agents/${id}`, "DELETE");
  for (const id of [a, b]) await j(`/api/tasks/${id}/archive`, "POST", { on: true });
}
