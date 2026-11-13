// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Delayed writes and reads cannot undo a queue gesture (terminum-dispatcher.md AC-169).
import { check, runningIn, until } from "./check.mjs";
import { drawn, openPage, SIZES } from "./fake.mjs";

export async function webQueueOrder(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const f = await openPage(browser, { device }), { page } = f;
    try {
      await drawn(page);
      await page.evaluate(async () => {
        const { queueList } = await import("/kanbanqueue.js");
        const box = document.createElement("section"), error = document.createElement("p");
        box.id = "queue-test";
        box.style.cssText = "position:fixed;inset:0;z-index:10000;background:var(--bg);overflow:auto";
        box.append(error);
        document.body.append(box);
        const tasks = ["API-1", "API-2", "API-3"].map((id) => ({ id, title: id, project: "api" }));
        const queues = { owl: tasks, fox: tasks.slice(0, 2) }, calls = [], waiting = [];
        let held = false, refused = false, refreshing = false, refreshDone, reads = 0, refreshFailed = false;
        const realFetch = window.fetch;
        window.fetch = async (path, options) => {
          if (!path.startsWith("/api/task-agents/") || !path.endsWith("/queue")) return realFetch(path, options);
          const agent = path.split("/")[3], ids = JSON.parse(options.body).tasks, fail = refused;
          calls.push({ agent, ids });
          if (held) await new Promise((done) => waiting.push(done));
          const changed = ids.slice().sort().join() !== queues[agent].map((t) => t.id).sort().join();
          if (!fail && !changed) queues[agent] = ids.map((id) => tasks.find((t) => t.id === id));
          return new Response(JSON.stringify(fail || changed ? { error: changed ? "The queue changed meanwhile: look at it again." : "Queue change refused." } : {}), { status: fail || changed ? 409 : 200 });
        };
        const list = queueList(box, { projectName: () => "API", say: (text) => { error.textContent = text; }, refresh: async (id) => {
          reads++;
          if (refreshing) await new Promise((done) => { refreshDone = done; });
          if (refreshFailed) throw new Error("Queue read refused.");
          return queues[id];
        } });
        const show = (id, queue = queues[id], opening = false) => list.show({ id, name: id }, queue, opening);
        window.queueFixture = { calls, queues, show, tasks, error,
          reads: () => reads, failRefresh: (value) => { refreshFailed = value; },
          hold: (value) => { held = value; }, refuse: (value) => { refused = value; },
          release: () => { held = false; waiting.splice(0).forEach((done) => done()); },
          holdRefresh: () => { refreshing = true; }, releaseRefresh: () => { refreshing = false; refreshDone?.(); } };
        show("owl");
      });
      const rows = () => page.$$eval("#queue-test .item", (items) => items.map((i) => i.dataset.id).join());
      const press = (id, direction) => page.getByRole("button", { name: `Move API-${id} ${direction}`, exact: true }).click();
      const state = () => page.evaluate(() => ({ calls: queueFixture.calls, owl: queueFixture.queues.owl.map((t) => t.id).join() }));
      const expected = "API-3,API-1,API-2";

      await page.evaluate(() => queueFixture.hold(true));
      await press(3, "up");
      await press(3, "up");
      const during = await state();
      check("AC-169 rapid arrow clicks show both moves immediately and send one write at a time", await rows() === expected && during.calls.length === 1, JSON.stringify(during));
      await page.evaluate(() => queueFixture.show("owl", queueFixture.tasks));
      check("AC-169 a board read during pending writes cannot undo their optimistic order", await rows() === expected, await rows());
      await page.evaluate(() => queueFixture.release());
      check("AC-169 both ordered writes reach the store in click order", await until(async () => (await state()).owl === expected && (await state()).calls.length === 2), JSON.stringify(await state()));

      // A board update lands after pointerdown. The same key must survive through its click,
      // which acts on the task's ID in the current order, rather than its rendered row index.
      const key = page.getByRole("button", { name: "Move API-3 down", exact: true }), r = await key.boundingBox();
      await page.mouse.move(r.x + r.width / 2, r.y + r.height / 2);
      await page.mouse.down();
      await page.evaluate(() => { queueFixture.queues.owl = [queueFixture.tasks[0], queueFixture.tasks[2], queueFixture.tasks[1]]; queueFixture.show("owl"); });
      await page.mouse.up();
      check("AC-169 a board update between down and up preserves the arrow click and moves its own task", await until(async () => (await state()).owl === "API-1,API-2,API-3"), JSON.stringify(await state()));

      // A rejected write is reconciled from the store; later queued intent still runs.
      await page.evaluate(() => { queueFixture.hold(true); queueFixture.refuse(true); });
      await press(3, "up");
      await page.evaluate(() => queueFixture.refuse(false));
      await press(3, "up");
      await page.evaluate(() => queueFixture.release());
      check("AC-169 a refused earlier write does not erase a later queued move", await until(async () => (await state()).owl === expected) && await rows() === expected, JSON.stringify(await state()));
      check("AC-169 a refused queue change says why", /Queue change refused/.test(await page.textContent("#queue-test > p")));

      await page.evaluate(() => { queueFixture.refuse(true); });
      await press(3, "down");
      check("AC-169 a lone refused write restores the authoritative queue", await until(async () => await rows() === expected), await rows());
      await page.evaluate(() => { queueFixture.refuse(false); queueFixture.holdRefresh(); });
      const readsBefore = await page.evaluate(() => queueFixture.reads());
      await press(3, "down");
      if (!await until(() => page.evaluate((n) => queueFixture.reads() > n, readsBefore))) throw new Error("the reconciliation read did not start");
      await press(3, "down");
      check("AC-169 a new move during reconciliation appears immediately", await rows() === "API-1,API-2,API-3", await rows());
      await page.evaluate(() => queueFixture.releaseRefresh());
      check("AC-169 reconciliation cannot overwrite a move made during its read", await until(async () => (await state()).owl === "API-1,API-2,API-3" && await rows() === "API-1,API-2,API-3"), JSON.stringify(await state()));
      await page.evaluate(() => queueFixture.failRefresh(true));
      await press(3, "up");
      check("AC-169 a failed reconciliation says why", await until(async () => /Queue read refused/.test(await page.textContent("#queue-test > p"))));
      await page.evaluate(() => queueFixture.failRefresh(false));
      await press(3, "up");
      check("AC-169 a failed reconciliation does not poison later writes", await until(async () => (await state()).owl === expected && await rows() === expected));

      // A task leaves while its row is dragged: stale DOM IDs still go to server validation,
      // but missing rows cannot crash optimistic rendering before the authoritative read.
      await page.evaluate(() => { queueFixture.queues.heron = [queueFixture.tasks[2], queueFixture.tasks[0], queueFixture.tasks[1]]; queueFixture.show("heron"); queueFixture.hold(true); });
      await page.evaluate(() => {
        const item = document.querySelector('#queue-test .item[data-id="API-2"]'), list = item.closest("ol");
        const from = item.closest("li").getBoundingClientRect(), x = from.left + from.width / 3;
        const fire = (type, target, y) => target.dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 75, pointerType: "mouse", button: 0, clientX: x, clientY: y }));
        fire("pointerdown", item, from.top + from.height / 2);
        fire("pointermove", list, from.top - 12);
        queueFixture.queues.heron = queueFixture.queues.heron.filter((t) => t.id !== "API-2");
        queueFixture.show("heron");
        const to = document.querySelector('#queue-test .item[data-id="API-3"]').closest("li").getBoundingClientRect();
        fire("pointerup", list, to.top + to.height / 4);
      });
      check("AC-169 a stale drop remains readable while its write is pending", await until(async () => await rows() === "API-3,API-1") && !f.errors.length, JSON.stringify([await rows(), f.errors]));
      await page.evaluate(() => queueFixture.release());
      check("AC-169 a task leaving during a drag is refused and reconciled without losing the remaining queue", await until(async () =>
        await rows() === "API-3,API-1" && /queue changed meanwhile/.test(await page.textContent("#queue-test > p"))), await rows());
      await page.evaluate(() => queueFixture.show("owl"));

      // Switching agents while an old write fails must not alter the new agent's queue or error.
      await page.evaluate(() => { queueFixture.hold(true); queueFixture.refuse(true); });
      await press(3, "down");
      await page.evaluate(() => { queueFixture.show("fox"); queueFixture.error.textContent = ""; queueFixture.release(); });
      await page.evaluate(() => new Promise((done) => setTimeout(done, 0)));
      check("AC-169 a late refusal belongs to the agent whose queue was changed", await rows() === "API-1,API-2" && !(await page.textContent("#queue-test > p")), await rows());
      await page.evaluate(() => queueFixture.show("owl"));
      check("AC-169 returning to that agent shows its refusal", /Queue change refused/.test(await page.textContent("#queue-test > p")));
      await page.evaluate(() => { queueFixture.error.textContent = ""; queueFixture.show("owl", undefined, true); });
      check("AC-169 reopening the same agent preserves its queue refusal", /Queue change refused/.test(await page.textContent("#queue-test > p")));
      check("no page errors", !f.errors.length, f.errors.join(" | "));
    } finally { await f.close(); }
  }
}
