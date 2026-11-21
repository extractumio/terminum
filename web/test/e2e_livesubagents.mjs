// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
import { nap } from "./boardkit.mjs";
import { check, slack } from "./check.mjs";

export async function subagentWaits({ task, agent, run, history, paneOf, status, said, tmux }) {
  // AC-127, AC-157 and AC-210: a draft can leave the pane open beyond both watchdog thresholds.
  const prerequisite = await task("Publish the API");
  const waitingSubs = await Promise.all([
    ["human", "SUBAGENT QUESTIONS", "", "WAIT FOR HUMAN", "Why you are needed:"],
    ["dependency", "SUBAGENT DEPENDENCY", `--wait-on ${prerequisite}`, "TODO", `It waits for ${prerequisite}`],
  ].map(async ([name, title, more, expected, reason]) => {
    const id = await task(title);
    await run(id, { agent: await agent("Waiting " + name, more, "codex") });
    check(`AC-127 ${name} wait was established`, await said(id, reason, 15000) && await status(id) === expected, await history(id));
    const pane = await paneOf(id);
    return { id, expected, pane };
  }));
  await nap(slack(6500)); // the 300× clock has seen more than 20 minutes
  for (const { id, expected, pane } of waitingSubs) {
    const hist = (await history(id)).replace(/\s*\n\s*/g, " ");
    check(`AC-127 silent subagent leaves ${expected} intact with no request`, await status(id) === expected
      && tmux("capture-pane", "-p", "-t", pane).includes("draft during wait") && !/silent subagent|Subagent sub-1/.test(hist), hist);
  }
}
