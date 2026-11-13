// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-123, AC-136: a monitor-only Stop against real private terminumd and tmux, with the measured
// Claude Code footer and hooks. The runner's clock scales an idle hour to 12 seconds; no model.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { boardKit, nap } from "./boardkit.mjs";
import { check, until } from "./check.mjs";

export async function liveBackground(page, opts) {
  const { work, tmux } = opts;
  const { j, cli, card, open, newPane, keyOf, hook, endPanes } = boardKit(page, opts);
  const project = (await j("/api/task-projects", "POST", { name: "Background", dir: work })).id;
  const marker = path.join(work, "monitor-running");
  const fake = fileURLToPath(new URL("../../scripts/fake_agent.py", import.meta.url));
  fs.writeFileSync(marker, "running");
  const agent = (await j("/api/task-agents", "POST", { project, name: "Monitor", kind: "claude", command: `python3 -I ${fake} ${cli} --monitor-file ${marker}` })).id;
  const id = (await j("/api/tasks", "POST", { project, title: "Wait for MONITOR" })).id;
  const read = () => j(`/api/tasks/${id}`);
  const words = async () => (await read()).history.map((e) => e.text ?? "").join(" | ");
  try {
    await open(`?project=${project}`);
    await j(`/api/tasks/${id}/run`, "POST", { agent });
    let pane;
    check("AC-123 the monitor-only footer is live", await until(async () => {
      pane = (await read()).session?.key?.split(":").pop();
      return pane && /1 monitor/.test(tmux("capture-pane", "-p", "-J", "-t", pane));
    }, 15000), await words());
    // Force wrapping: Dispatcher must capture with -J, as the session row does.
    tmux("resize-window", "-t", pane, "-x", "38", "-y", "24");
    check("AC-136 background work keeps its card pulsing", await until(() => page.isVisible(`${card(id)} .pbar.busy`)));
    check("AC-136 a Stop alone writes no plain stop while the monitor runs", !(await words()).includes("The agent stopped with this task open"), await words());
    await nap(14500);   // more than one idle hour, with neither an open tool nor CPU work
    check("AC-123 monitor-only work stays IN PROGRESS past an idle hour", (await read()).status === "IN PROGRESS", await words());
    check("AC-123 monitor-only work gets no idle request", !/request for a result|still open and your turn ended|No activity for/.test(await words()), await words());
    fs.unlinkSync(marker);
    check("AC-136 once the monitor ends a plain stop is recorded", await until(async () => (await words()).includes("The agent stopped with this task open"), 15000), await words());
    check("AC-136 a plain stop is recorded once", (await read()).history.filter((e) => e.text === "The agent stopped with this task open").length === 1);

    const failedPane = newPane("failedstop"), key = await keyOf(failedPane);
    hook(failedPane, "SessionStart", { source: "startup" });
    hook(failedPane, "UserPromptSubmit", { prompt: "go" });
    const failed = (await j("/api/tasks", "POST", { project, title: "Failure stays immediate", session: key })).id;
    await j(`/api/tasks/${failed}`, "PATCH", { status: "IN PROGRESS" });
    hook(failedPane, "StopFailure", { error: "rate_limit" });
    check("AC-136 failed stops keep their reason at once", await until(async () => (await j(`/api/tasks/${failed}`)).history.some((e) => e.text === "The agent stopped: rate_limit")));
  } finally {
    fs.rmSync(marker, { force: true });
    endPanes();
  }
}
