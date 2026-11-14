// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Archived task transitions through a private daemon, tmux, real hooks and MCP (AC-115, AC-122).
// `node web/test/e2e_live.mjs archived chromium webkit`: desktop and phone; no model calls.
import path from "node:path";
import { fileURLToPath } from "node:url";
import { boardKit, intoStore } from "./boardkit.mjs";
import { check, until } from "./check.mjs";

const FAKE = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../scripts/fake_agent.py");

export async function liveArchived(page, opts) {
  const { tmux, work } = opts;
  const { j, card, open, cli, tool } = boardKit(page, opts);
  const project = (await j("/api/task-projects", "POST", { name: `Archived ${page.viewportSize().width}`, dir: work })).id;
  const agent = (await j("/api/task-agents", "POST", { project, name: "Archive runner", kind: "claude", command: `python3 -I ${FAKE} ${cli} --kind claude` })).id;
  const task = async (title) => (await j("/api/tasks", "POST", { title, project })).id;
  const run = async (id) => j(`/api/tasks/${id}/run`, "POST", { agent });
  const hasStatus = (id, status, ms = 8000) => until(async () => (await j(`/api/tasks/${id}`)).status === status, ms);
  const paneOf = async (id) => (await j(`/api/tasks/${id}`)).session?.key?.split(":").pop();
  await open(`?project=${project}`);

  // AC-122: new work brings an archived live task back, including one already IN PROGRESS (TERM-77).
  const archived = await task("archived FINISHES");
  await run(archived);
  check("AC-122 setup: its agent finishes and its session stays open", await hasStatus(archived, "DONE", 15000));
  const archivedPane = await paneOf(archived);
  for (const was of ["DONE", "IN PROGRESS"]) {
    if (was === "DONE") {
      check("AC-122 a human archives the finished task", await until(async () => (await j(`/api/tasks/${archived}/archive`, "POST", { on: true })).ok));
      await until(async () => !(await page.$(card(archived))));
    } else {
      // A persisted archive beside active work: the archive route refuses a held run (AC-138).
      intoStore(opts.app, `UPDATE tasks SET archived_at = unixepoch() WHERE slug = '${archived}'`);
      await open(`?project=${project}`);
    }
    const before = await j(`/api/tasks/${archived}`);
    check(`AC-122 ${was}: its persisted archive is hidden`, before.archived_at && before.status === was && !(await page.$(card(archived))));
    for (const [name, more] of [["move_task", { to: "IN PROGRESS" }], ["finish_task", { result: "Do not keep this result" }]]) {
      const said = tool(archivedPane, name, { id: archived, ...more });
      check(`AC-115 ${name} refuses an archived task`, said.includes(`${archived} is archived: a human brings it back from the Archived list.`), said);
    }
    const after = await j(`/api/tasks/${archived}`);
    check("AC-115 refused moves and results leave archive, status and history intact", after.archived_at === before.archived_at && after.status === before.status && JSON.stringify(after.history) === JSON.stringify(before.history));
    tmux("send-keys", "-t", archivedPane, "-l", "More work WORKS");
    tmux("send-keys", "-t", archivedPane, "Enter");
    check(`AC-122 ${was}: a human's prompt restores it in progress`, await until(async () => { const t = await j(`/api/tasks/${archived}`); return t.status === "IN PROGRESS" && !t.archived_at; }));
    check(`AC-122 ${was}: its card returns without reloading`, await until(() => page.isVisible(card(archived))));
  }
  tmux("send-keys", "-t", archivedPane, "-l", `${archived} FINISHES`);
  tmux("send-keys", "-t", archivedPane, "Enter");
  check("AC-115 after restoration its agent finishes normally", await hasStatus(archived, "DONE"));

}
