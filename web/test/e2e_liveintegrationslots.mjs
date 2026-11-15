// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-169, AC-198: reproduce four INTEGRATION waits blocking their own Verify prerequisite on an agent at three.
// Real private terminumd/tmux, stand-in Codex agents and RUN admission; only the integration wait is seeded.
import { fileURLToPath } from "node:url";
import { boardKit, intoStore } from "./boardkit.mjs";
import { check, until } from "./check.mjs";

const FAKE = fileURLToPath(new URL("../../scripts/fake_agent.py", import.meta.url));

export async function liveIntegrationSlots(page, live) {
  const { j, cli, open } = boardKit(page, live);
  const size = page.viewportSize().width < 860 ? "phone" : "wide";
  const project = (await j("/api/task-projects", "POST", { name: `Integration slots ${size}`, dir: live.work })).id;
  const given = { project, name: "Codex slots", kind: "codex", command: `python3 -I ${FAKE} ${cli} --kind codex` };
  const agent = (await j("/api/task-agents", "POST", { ...given, at_once: 4 })).id;
  const tasks = [], panes = new Set();
  const task = async (title) => {
    const id = (await j("/api/tasks", "POST", { project, title: `${title} WORKS` })).id;
    tasks.push(id);
    return id;
  };
  const status = async (id) => (await j(`/api/tasks/${id}`)).status;
  const started = async (id) => until(async () => {
    const t = await j(`/api/tasks/${id}`);
    if (t.session?.key) panes.add(t.session.key.split(":").pop());
    return t.status === "IN PROGRESS";
  }, 20000);
  const slots = async () => (await j(`/api/tasks?project=${project}&limit=1&counts=1`)).slots;
  try {
    await j(`/api/task-projects/${project}/agents-at-once`, "PUT", { limit: 4 });
    const queued = [];
    for (let i = 0; i < 4; i++) {
      const id = await task(`Integration wait ${i}`);
      queued.push(id);
      await j(`/api/tasks/${id}/run`, "POST", { agent });
      if (!await started(id)) throw new Error(`${id} did not take its initial prompt`);
    }
    const limited = await j(`/api/task-agents/${agent}`, "PATCH", { ...given, at_once: 3 });
    if (!limited.ok) throw new Error(`agent limit was not kept: ${JSON.stringify(limited)}`);
    await j(`/api/task-projects/${project}/agents-at-once`, "PUT", { limit: 3 });
    const blocker = await task("Verify prerequisite");
    const waiting = await j(`/api/tasks/${blocker}/run`, "POST", { agent, queue: true });
    check("AC-169 the prerequisite first waits behind four occupied sessions", !!waiting.waits && await status(blocker) === "TODO", JSON.stringify(waiting));

    // Seed the reported idle integration state in this test's store, without running queue stages or publishing.
    const ids = queued.map((id) => `'${id}'`).join(",");
    intoStore(live.app, `UPDATE tasks SET status = 'INTEGRATION' WHERE slug IN (${ids});
      UPDATE runs SET turn_ended = 1, open_tools = 0 WHERE task IN (SELECT id FROM tasks WHERE slug IN (${ids}));`);
    check("AC-169 four integration waits free room for the queued Verify prerequisite", await started(blocker), JSON.stringify(await slots()));
    for (const id of queued) {
      const t = await j(`/api/tasks/${id}`);
      check("AC-169 an integration wait keeps its live session", t.status === "INTEGRATION" && t.session?.closed === false);
    }
    await open(`?project=${project}`);
    const card = `#agentbar .agentcard[data-agent="${agent}"]`;
    check("AC-169 integration waits leave two free slot rings", await until(async () => await page.locator(`${card} .agfree`).count() === 2));

    for (let i = 0; i < 2; i++) {
      const id = await task(`Fill released slot ${i}`);
      const start = await j(`/api/tasks/${id}/run`, "POST", { agent });
      check("AC-169 a new run can use an integration wait's slot", !start.full && await started(id), JSON.stringify(start));
    }
    // A real task transition back to work must immediately restore occupancy, also above the limit.
    await j(`/api/tasks/${queued[0]}`, "PATCH", { status: "IN PROGRESS" });
    const next = await task("Must wait after return");
    const full = await j(`/api/tasks/${next}/run`, "POST", { agent });
    check("AC-169 a returned run counts again and later starts wait", full.full?.words?.includes(queued[0]) && await status(next) === "TODO"
      && (await slots()).agents[agent].length === 4, JSON.stringify(full));
  } finally {
    for (const id of tasks) {
      const key = (await j(`/api/tasks/${id}`).catch(() => ({}))).session?.key;
      if (key) panes.add(key.split(":").pop());
    }
    for (const pane of panes) live.tmux("kill-pane", "-t", pane);
  }
}
