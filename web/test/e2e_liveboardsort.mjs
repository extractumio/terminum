// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Real column sorts and project preferences through terminumd and its private store (AC-105).
import { check, until } from "./check.mjs";
import { boardKit, intoStore } from "./boardkit.mjs";

export async function liveBoardSort(page, opts) {
  const { j, open } = boardKit(page, opts);
  const suffix = Math.random().toString(36).slice(2, 8);
  const project = (await j("/api/task-projects", "POST", { name: `Sort ${suffix}`, prefix: "SRT" })).id;
  const made = async (title, severity) => (await j("/api/tasks", "POST", { title, severity, project })).id;
  const urgent = await made("Old urgent", "P0"), first = await made("First ordinary", "P2"), second = await made("Second ordinary", "P2");
  const col = '.col[data-status="TODO"]', key = (s) => `${col} [data-sort="${s}"]`;
  const ids = () => page.$$eval(`${col} .card`, (l) => l.map((c) => c.dataset.id).join());
  const reads = async (wanted) => until(async () => await ids() === wanted.join());
  // Fixed timestamps make recency independent of wall-clock seconds, on a busy runner too.
  intoStore(opts.app, `UPDATE tasks SET updated_at = CASE slug WHEN '${urgent}' THEN 1 WHEN '${first}' THEN 2 WHEN '${second}' THEN 3 END WHERE project = '${project}';`);
  await open(`?project=${project}`);
  check("AC-105 live default sorts priority then updated time", await reads([urgent, second, first]), await ids());
  await page.click(key("recent"));
  check("AC-105 live recent sorts across severities", await reads([second, first, urgent]), await ids());
  check("AC-105 live API refuses an unknown sort", (await j(`/api/tasks?project=${project}&sort=wrong`)).error === "Sort: one of recent, priority");
  await open(`?project=${project}`);
  check("AC-105 live project choice survives reload", await reads([second, first, urgent]) && await page.getAttribute(key("recent"), "aria-pressed") === "true");
  await j(`/api/tasks/${first}`, "PATCH", { title: "Updated ordinary" });
  check("AC-105 live edits move a card to the recent top", await reads([first, second, urgent]), await ids());
  await page.click(key("priority"));
  check("AC-105 live priority retains recency inside each severity", await reads([urgent, first, second]), await ids());
  await page.dragAndDrop(`${col} .card[data-id="${second}"]`, `${col} .card[data-id="${first}"]`);
  check("AC-105 live in-column drop leaves the selected sort intact", await reads([urgent, first, second]), await ids());
  await j(`/api/tasks/${second}/comments`, "POST", { text: "A recent comment" });
  check("AC-105 live comments move cards within their priority", await reads([urgent, second, first]), await ids());
}
