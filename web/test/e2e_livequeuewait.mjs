// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-169 / AC-117: an answered task waits for an agent, not the human, even with its old session closed.
import { check, until } from "./check.mjs";
import { boardKit, intoStore, pressCard } from "./boardkit.mjs";

export async function liveQueuedAnswers(page, live, project, agent) {
  const { j, card, open } = boardKit(page, live);
  const made = async (title) => (await j("/api/tasks", "POST", { title, project })).id;
  const [first, answered, unanswered] = [await made("First in line"), await made("Answered question"), await made("Still needs an answer")];
  for (const id of [answered, unanswered]) await j(`/api/tasks/${id}`, "PATCH", { status: "WAIT FOR HUMAN" });
  intoStore(live.app, `UPDATE tasks SET session_key = 'tmux:local:gone:%999' WHERE slug = '${answered}';
    INSERT INTO runs (id, task, project, agent_id, agent_name, kind, command, folder, pane_key, state)
      SELECT 'closed-' || t.slug, t.id, t.project, a.id, a.name, a.kind, a.command, '/srv/example', t.session_key, 'closed'
      FROM tasks t JOIN agents a ON a.id = '${agent}' WHERE t.slug = '${answered}';
    INSERT INTO run_tasks (run, task) SELECT id, task FROM runs WHERE id = 'closed-${answered}';
    UPDATE events SET author = 'dispatcher' WHERE kind = 'move' AND task IN (SELECT id FROM tasks WHERE slug IN ('${answered}', '${unanswered}'));`);
  await j(`/api/tasks/${answered}/comments`, "POST", { text: "The previous session ended." });
  let marks;
  const socket = (ws) => ws.on("framereceived", ({ payload }) => { try { const m = JSON.parse(String(payload)); if (m.t === "taskmarks") marks = m; } catch {} });
  page.on("websocket", socket);
  try {
    await open(`?project=${project}`);
    check("AC-169 fixture: an unanswered card with a closed session says so", /session closed/.test(await page.textContent(card(answered))));
    await j(`/api/tasks/${first}/run`, "POST", { agent, queue: true });
    const queued = await j(`/api/tasks/${answered}/run`, "POST", { agent, queue: true, comment: "Use the recommended option." });
    check("AC-169 the answer is kept as a queued press", queued.code === 200 && !!(await j(`/api/tasks/${answered}`)).waits, JSON.stringify(queued));
    const header = '.col[data-status="WAIT FOR HUMAN"] .count';
    const state = async () => ({ card: await page.textContent(card(answered)), header: await page.textContent(header), marks });
    check("AC-169 answered and queued: the card names its agent and full-queue place, not its closed session", await until(async () => /queued for Queue owl.*2nd/.test((await state()).card)), JSON.stringify(await state()));
    check("AC-169 the column counts only the unanswered task and says how many are queued", await until(async () => (await page.textContent(header)) === "1 · 1 queued"), JSON.stringify(await state()));
    check("AC-117 queued answers leave human-attention counts and lines", await until(() => marks?.waiting === 1 && marks.lines.some((l) => l.id === unanswered) && !marks.lines.some((l) => l.id === answered)), JSON.stringify(marks));
    await pressCard(page, answered);
    check("AC-169 the task window repeats its queue place before the closed session", /queued for Queue owl.*2nd/.test(await page.textContent("#taskbody .fieldvalue")) && !/session closed/.test(await page.textContent("#taskbody .fieldvalue")));
    await j(`/api/task-agents/${agent}/queue`, "POST", { tasks: [answered, first] });
    check("AC-169 reordering also refreshes an open task window", await until(async () => /queued for Queue owl.*1st/.test(await page.textContent("#taskbody .fieldvalue"))));
    await page.keyboard.press("Escape");
    check("AC-169 moving the queue changes the card's place", await until(async () => /queued for Queue owl.*1st/.test(await page.textContent(card(answered)))));
    if (live.shots) await page.screenshot({ path: `${live.shots}${live.shots.endsWith("-") ? "" : "/"}queued-answer.png` });
    await j(`/api/tasks/${answered}/leave-queue`, "POST");
    check("AC-169 cancelling the wait restores the human count and closed-session state", await until(async () => (await page.textContent(header)) === "2" && /session closed/.test(await page.textContent(card(answered))) && marks?.waiting === 2));
    const group = await j(`/api/tasks/${answered}/run`, "POST", { agent, queue: true, also: [unanswered] });
    check("AC-169 a grouped answer queues both tasks as one press", group.code === 200 && await until(async () => (await page.textContent(header)) === "0 · 2 queued" && marks?.waiting === 0), JSON.stringify(group));
    check("AC-169 every queued member shows the same full-queue place", await until(async () => /queued for Queue owl.*2nd/.test(await page.textContent(card(unanswered)))));
    await pressCard(page, unanswered);
    await page.getByRole("button", { name: "Stop waiting", exact: true }).click();
    check("AC-169 Stop waiting on a member cancels the whole queued press", await until(async () => !(await j(`/api/tasks/${answered}`)).waits && !(await j(`/api/tasks/${unanswered}`)).waits && marks?.waiting === 2));
    await page.keyboard.press("Escape");
    await j(`/api/tasks/${unanswered}`, "PATCH", { parent: answered });
    const family = await j(`/api/tasks/${answered}/run`, "POST", { agent, queue: true });
    await open(`?project=${project}&subtasks=1`);
    check("AC-169 a primary-only answer also queues its unfinished subtask", family.code === 200 && await until(async () => (await j(`/api/tasks/${unanswered}`)).wait_position === 2 && marks?.waiting === 0), JSON.stringify(family));
    await pressCard(page, unanswered);
    await j(`/api/tasks/${first}/leave-queue`, "POST");
    check("AC-169 an open carried subtask follows cancellation ahead of it", await until(async () => /queued for Queue owl.*1st/.test(await page.textContent("#taskbody .fieldvalue"))));
    await j(`/api/tasks/${answered}/leave-queue`, "POST");
    check("AC-169 an open carried subtask follows its primary's cancellation", await until(async () => !/queued for/.test(await page.textContent("#taskbody .fieldvalue")) && marks?.waiting === 1));
    await page.keyboard.press("Escape");
  } finally {
    page.off("websocket", socket);
    for (const id of [first, answered, unanswered]) await j(`/api/tasks/${id}/archive`, "POST", { archive: true });
  }
}
