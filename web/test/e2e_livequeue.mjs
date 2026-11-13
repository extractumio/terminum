// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// An agent's queue against a real private terminumd (terminum-dispatcher.md AC-169), at a wide and a phone size:
// offered at a limit and kept only on the human's yes, its dots on the agent, its window's list reordered by a
// drag, a finger's and a mouse's, and by its arrows, a task taken out. The slot is held by a schedule that never
// comes due (terminum-schedules.md AC-161), so no agent runs: `node web/test/e2e_live.mjs schedules chromium`.
import { check, until } from "./check.mjs";
import { boardKit, nap } from "./boardkit.mjs";

export async function liveQueue(page, live) {
  const { j, card, open } = boardKit(page, live);
  const shot = (name) => live.shots && page.screenshot({ path: `${live.shots}${live.shots.endsWith("-") ? "" : "/"}${name}.png` });
  const narrow = page.viewportSize().width < 860;
  const project = (await j("/api/task-projects", "POST", { name: `Queued ${narrow ? "phone" : "wide"}`, dir: live.work })).id;
  const agent = (await j("/api/task-agents", "POST", { name: `Queue owl ${narrow ? "phone" : "wide"}`, kind: "claude", command: "claude", at_once: 1 })).id;
  const two = (await j("/api/task-agents", "POST", { name: `Roomy ${narrow ? "phone" : "wide"}`, kind: "claude", command: "claude", at_once: 3 })).id;
  const kept = await j("/api/schedules", "POST", { title: "Yearly", body: "once a year", project, agent, cron: "0 0 1 1 *", budget_minutes: 5 });
  const made = async (title) => (await j("/api/tasks", "POST", { title, project })).id;
  const [a, b, c] = [await made("Queued first"), await made("Queued second"), await made("Queued third")];
  const queue = async () => ((await j(`/api/tasks?project=${project}&limit=1&counts=1`)).slots.queue?.[agent] ?? []).map((t) => t.id);
  await open(`?project=${project}`);
  const owl = `#agentbar .agentcard[data-agent="${agent}"]`;
  await page.waitForSelector(owl);

  // offered, and nothing waits before the yes
  const asked = await j(`/api/tasks/${a}/run`, "POST", { agent });
  check("AC-169 a press at a limit is offered the queue", /is at its limit of 1: SCH-\d+ \(kept\)\. Add it to the queue: it starts when there is room\./.test(asked.full?.queue ?? "") && asked.full.close === null, JSON.stringify(asked));
  check("AC-169 and nothing waits before the human agrees", !(await j(`/api/tasks/${a}`)).waits && (await queue()).length === 0);
  check("AC-161 a kept slot is a dashed ring, a free one a plain ring, none of them a session's dot", await until(async () => await page.locator(`${owl} .agkept`).count() === 1 && await page.locator(`${owl} .agfree`).count() === 0
    && await page.locator(`#agentbar .agentcard[data-agent="${two}"] .agfree`).count() === 3) && await page.evaluate((s) => getComputedStyle(document.querySelector(s)).borderTopStyle, `${owl} .agkept`) === "dashed");
  check("AC-169 an agent with no queue has no queue key", await page.locator(`${owl} .agqueue`).count() === 0);

  // RUN's window: the offer in its words, Add to queue its yes
  if (narrow) await page.evaluate((s) => document.querySelector(s).scrollIntoView({ block: "center" }), card(a));
  await page.click(`${card(a)} .ctitle`);
  await page.click(`${card(a)} .crun`);
  await page.waitForSelector("#runbox[open]");
  await page.selectOption("#runagent", agent);
  check("AC-169 RUN's window offers the queue", await until(async () => /Add it to the queue/.test(await page.textContent("#runnone")) && (await page.textContent("#rungo")) === "Add to queue"), `${await page.textContent("#runnone")} / ${await page.textContent("#rungo")}`);
  await shot("queue-offer");
  await page.selectOption("#runagent", two);
  check("AC-169 an agent with room is run, not queued", (await page.textContent("#rungo")) === "Run" && !(await page.isVisible("#runnone")));
  await page.selectOption("#runagent", agent);
  await page.fill("#runmessage", "mind the queue");
  await page.click("#rungo");
  const said = async () => [(await queue()).join(), (await j(`/api/tasks/${a}`)).waits, await page.textContent(card(a)), JSON.stringify((await j(`/api/tasks/${a}`)).history)];
  check("AC-169 Add to queue keeps it waiting, its message kept", await until(async () => { const [q, waits, shown, history] = await said(); return q === a && /Queue owl/.test(waits) && /waits for Queue owl/.test(shown) && /mind the queue/.test(history); }),
    JSON.stringify(await said()).slice(0, 900));
  for (const id of [b, c]) await j(`/api/tasks/${id}/run`, "POST", { agent, queue: true });
  check("AC-169 the queue is in the order they were added", (await queue()).join() === [a, b, c].join(), JSON.stringify(await queue()));

  // on the agent: a pale dot each, and the key that opens the queue
  check("AC-169 a pale dot for each queued task, and their count", await until(async () => await page.locator(`${owl} .agqueue .agwait`).count() === 3 && /3 queued/.test(await page.textContent(`${owl} .agqueue`))), await page.textContent(owl));
  const colors = await page.evaluate((s) => ({ wait: getComputedStyle(document.querySelector(`${s} .agwait`)).backgroundColor, kept: getComputedStyle(document.querySelector(`${s} .agkept`)).backgroundColor }), owl);
  check("AC-169 a queued task's dot is filled, a kept slot's ring empty", colors.wait !== colors.kept && /rgba?\(0, 0, 0, 0\)/.test(colors.kept), JSON.stringify(colors));
  await shot("queue-agent");
  await page.click(`${owl} .agqueue`);
  await page.waitForSelector("#agentbox[open]");
  const rows = () => page.$$eval("#agentqueue .agqueued:not(.dragghost) .item", (l) => l.map((i) => i.dataset.id));
  const listed = async (ids) => until(async () => (await rows()).join() === ids.join() && (await queue()).join() === ids.join(), 8000);
  check("AC-169 its key opens the agent's window at its queue", await listed([a, b, c]) && await page.isVisible("#agentqueue .agqueuelist"), JSON.stringify(await rows()));
  const fits = await page.evaluate(() => {
    const box = document.querySelector("#agentbox").getBoundingClientRect(), keys = [...document.querySelectorAll("#agentqueue .acts button")].map((k) => k.getBoundingClientRect());
    return { inside: keys.every((k) => k.left >= box.left && k.right <= box.right + 1), tall: Math.min(...keys.map((k) => Math.min(k.width, k.height))), wide: document.querySelector("#agentbox").scrollWidth <= document.querySelector("#agentbox").clientWidth + 1 };
  });
  check("AC-169 its rows and keys fit the window, no sideways scroll", fits.inside && fits.wide && fits.tall >= (narrow ? 32 : 22), JSON.stringify(fits));
  await shot("queue-window");

  // its arrows
  check("AC-169 the first has no way up, the last no way down", await page.isDisabled(`#agentqueue .agqueued:first-child button[aria-label^="Move"][aria-label$="up"]`) && await page.isDisabled(`#agentqueue .agqueued:last-child button[aria-label$="down"]`));
  await page.click(`#agentqueue button[aria-label="Move ${c} up"]`);
  await page.click(`#agentqueue button[aria-label="Move ${c} up"]`);
  check("AC-169 two presses of an arrow move a task two places", await listed([c, a, b]), JSON.stringify(await rows()));
  await page.click(`#agentqueue button[aria-label="Move ${c} down"]`);
  check("AC-169 an arrow moves a task one place", await listed([a, c, b]), JSON.stringify(await rows()));

  // a drag: a mouse at once; a finger rests on the row first
  const mid = async (id) => page.$eval(`#agentqueue .item[data-id="${id}"]`, (i) => { const r = i.closest("li").getBoundingClientRect(); return { x: r.left + r.width / 3, y: r.top + r.height / 2, h: r.height }; });
  if (narrow) {
    const from = await mid(b), to = await mid(a);
    const finger = (type, at) => page.$eval("#agentqueue .agqueuelist", (list, [type, at, id]) => {
      const target = type === "pointerdown" ? document.querySelector(`#agentqueue .item[data-id="${id}"]`) : list;
      target.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 7, pointerType: "touch", isPrimary: true, button: 0, clientX: at.x, clientY: at.y }));
    }, [type, at, b]);
    await finger("pointerdown", from);
    await nap(450);
    check("AC-169 a finger resting on a row lifts it", await page.locator("#agentqueue .agqueued.lifted").count() === 1);
    await finger("pointermove", { x: from.x, y: from.y - 20 });
    await finger("pointermove", { x: to.x, y: to.y - to.h / 4 });
    await finger("pointerup", { x: to.x, y: to.y - to.h / 4 });
  } else {
    const from = await mid(b), to = await mid(a);
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    await page.mouse.move(from.x, from.y - 12, { steps: 3 });
    check("AC-169 a dragged row leaves its dashed slot, its copy under the pointer", await until(async () => await page.locator("#agentqueue .agqueued.dragging").count() === 1 && await page.locator("#agentqueue .dragghost").count() === 1, 3000));
    const ghost = await page.$eval("#agentqueue .dragghost", (g) => { const r = g.getBoundingClientRect(); return { x: r.left, y: r.top, w: r.width }; }).catch(() => ({ x: 0, y: -1000, w: 0 }));
    check("AC-169 the copy stands where the pointer is", Math.abs(ghost.y + from.h / 2 - (from.y - 12)) < from.h, JSON.stringify({ ghost, from }));
    await page.keyboard.press("Escape");
    check("AC-169 Esc puts the dragged row back, the window stays", await page.locator("#agentqueue .dragghost").count() === 0 && await page.isVisible("#agentbox[open]") && (await rows()).join() === [a, c, b].join(), JSON.stringify(await rows()));
    await page.mouse.up();
    await page.mouse.move(from.x, from.y);
    await page.mouse.down();
    await page.mouse.move(from.x, from.y - 12, { steps: 3 });
    await page.mouse.move(to.x, to.y - to.h / 4, { steps: 6 });
    await page.mouse.up();
  }
  check("AC-169 a row dragged to the top starts first", await listed([b, a, c]), JSON.stringify([await rows(), await queue()]));

  // AC-169: release coordinates decide the drop, even when the last movement was not delivered.
  // Start from a,c,b; the last move places b below a, but the release is inside a's upper half.
  await j(`/api/task-agents/${agent}/queue`, "POST", { tasks: [a, c, b] });
  await listed([a, c, b]);
  const dropped = page.waitForRequest((r) => r.method() === "POST" && r.url().endsWith(`/api/task-agents/${agent}/queue`));
  await page.evaluate(([a, b]) => {
    const item = (id) => document.querySelector(`#agentqueue .item[data-id="${id}"]`);
    const list = document.querySelector("#agentqueue .agqueuelist");
    const rect = (id) => item(id).closest("li").getBoundingClientRect();
    const fire = (type, target, x, y) => target.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 73, pointerType: "mouse", isPrimary: true, button: 0, clientX: x, clientY: y }));
    const from = rect(b), to = rect(a), x = from.left + from.width / 3;
    fire("pointerdown", item(b), x, from.top + from.height / 2);
    fire("pointermove", list, x, to.top + to.height * 3 / 4);
    const atRelease = rect(a);
    fire("pointerup", list, x, atRelease.top + atRelease.height / 4);
  }, [a, b]);
  const posted = (await dropped).postDataJSON();
  check("AC-169 release position decides the sent and persisted queue order", posted.tasks.join() === [b, a, c].join() && await listed([b, a, c]), JSON.stringify([posted, await rows(), await queue()]));

  // an order of a queue that changed is refused; a task taken out
  const stale = await j(`/api/task-agents/${agent}/queue`, "POST", { tasks: [a, b] });
  check("AC-169 an order of another queue is refused", /The queue changed meanwhile/.test(stale.error ?? ""), JSON.stringify(stale));
  await page.click(`#agentqueue button[aria-label="Take ${a} out of the queue"]`);
  check("AC-169 its cross takes a task out: it waits no more, where it stood", await listed([b, c]) && (await j(`/api/tasks/${a}`)).status === "TODO" && !(await j(`/api/tasks/${a}`)).waits
    && (await j(`/api/tasks/${a}`)).history.some((h) => /called off its wait/.test(h.text ?? "")), JSON.stringify([await rows(), await page.textContent("#agenterror"), await page.evaluate(() => document.querySelector("#agentqueue .agqueuelist").className)]));
  check("AC-169 the agent's dots follow", await until(async () => await page.locator(`${owl} .agqueue .agwait`).count() === 2));
  const gone = await j(`/api/tasks/${a}/leave-queue`, "POST");
  check("AC-169 a task that waits no more is not taken out again, nor given up", /waits no more/.test(gone.error ?? "") && (await j(`/api/tasks/${a}`)).status === "TODO", JSON.stringify(gone));
  const keyBox = await page.$eval(`${owl} .agqueue`, (k) => { const r = k.getBoundingClientRect(); return [Math.round(r.width), Math.round(r.height)]; });
  if (narrow) check("AC-169 on a phone the queue's key is a finger tall", keyBox[1] >= 44 && keyBox[0] >= 44, JSON.stringify(keyBox));
  check("AC-169 the window stays open through it", await page.isVisible("#agentbox[open]"));
  for (const id of [b, c]) await j(`/api/tasks/${id}/give-up`, "POST");
  check("AC-169 an empty queue says so", await until(async () => /No task waits for this agent/.test(await page.textContent("#agentqueue"))), await page.textContent("#agentqueue"));
  await page.click('#agentbox button[value="cancel"]');
  await j(`/api/schedules/${kept.id}`, "DELETE");
  for (const id of [agent, two]) await j(`/api/task-agents/${id}`, "DELETE");
}
