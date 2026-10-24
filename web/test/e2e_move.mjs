// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-07 checks of a row's pane actions against a fake terminumd (fake.mjs), at a wide and a phone
// size: Split Below, Right and Left (the quick shell's; a phone below only), Close Session asked
// first with the program's name, Move to Session… listing the other sessions of the same tmux
// server; the header's ⋯ holds settings only.

import { check, runningIn } from "./check.mjs";
import { drawn, group, item, openPage, pause, SIZES } from "./fake.mjs";

const SHOWN = "tmux:local:terminum:%1";
const OTHER = "tmux:local:terminum:%2";

export async function webMove(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const layout = [group("main", [item(SHOWN, "api", { sub: ["claude", "~/work/api"] }), item(OTHER, "docs", { index: "2" })]),
      group("notes", [item("tmux:local:terminum:%3", "notes")]),
      group("devbox", [item("tmux:devbox:default:%1", "remote", { host: "devbox" })], { host: "devbox", state: "attached" })];
    const f = await openPage(browser, { device, layout });
    const { page, sent } = f;
    await drawn(page);
    const phone = !!device.hasTouch;
    // a row's menu: a right click on it (a phone's long press opens the same menu: e2e_projects.mjs)
    const rowMenu = async (id) => {
      await page.$eval(`#sessions .item[data-id="${id}"]`, (el) => { const r = el.getBoundingClientRect();
        el.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: r.left + 40, clientY: r.top + 10 })); });
      await page.waitForSelector("#rowmenu[open]");
    };
    const act = async (id, action) => { await rowMenu(id); await page.click(`#rowmenu button[data-action="${action}"]`); await pause(page, 100); };
    const last = (t) => sent.filter((m) => m.t === t).at(-1);
    const shown = () => page.evaluate(() => [...document.querySelectorAll("#rowmenu[open] button")].filter((b) => b.getClientRects().length).map((b) => b.dataset.action));

    const settings = await page.evaluate(() => ["quickbelow", "quickright", "quickleft", "movewin", "closepane"].filter((id) => document.getElementById(id)));
    await rowMenu(OTHER);
    const splits = (await shown()).filter((a) => a.startsWith("split-"));
    await page.click('#rowmenu button[data-action="split-bottom"]');
    await pause(page);
    check("AC-07 the header's ⋯ holds settings only; a row's menu has the splits (the quick shell's: below, right, left; a phone below only), Split Below shows that session with its shell below",
      !settings.length && splits.join() === (phone ? "split-bottom" : "split-bottom,split-right,split-left") && last("quick")?.id === OTHER && last("quick").side === "bottom",
      JSON.stringify({ settings, splits, quick: last("quick") }));

    await act(SHOWN, "close");
    const asked = await page.evaluate(() => ({ open: document.getElementById("confirmbox").open,
      text: `${document.getElementById("confirmtitle").textContent} ${document.getElementById("confirmtext").textContent}`, yes: document.getElementById("confirmok").textContent }));
    await page.click('#confirmbox button[value="cancel"]');
    await pause(page);
    await act(SHOWN, "close");
    await page.click("#confirmbox .dhead .x");   // × as Cancel
    await pause(page);
    const kept = !last("kill") && !await page.evaluate(() => document.getElementById("confirmbox").open);
    await act(SHOWN, "close");
    await page.click("#confirmok");
    await pause(page);
    check("AC-07 a row's Close Session asks \"Close the session …? The program running in it (claude) ends.\"; Cancel sends nothing, Close Session kills the pane",
      asked.open && /^Close the session .+\? The program running in it \(claude\) ends\.$/.test(asked.text) && asked.yes === "Close Session" && kept
        && last("kill")?.id === SHOWN, JSON.stringify({ asked, kill: last("kill") }));

    await act(SHOWN, "movewin");
    const offered = await page.evaluate(() => [...document.querySelectorAll('#rowmenu button[data-action="session"]')].map((b) => b.textContent));
    await page.click('#rowmenu button[data-session="local/notes"]');
    await pause(page);
    check("AC-07 a row's Move to Session… lists the other sessions of its tmux server (not a host's) and moves the window",
      offered.join() === "notes" && last("move")?.id === SHOWN && last("move").session === "local/notes" && !await page.evaluate(() => document.getElementById("rowmenu").open),
      JSON.stringify({ offered, move: last("move") }));

    // a host's pane: its server has one session, there is nowhere to move to
    await rowMenu("tmux:devbox:default:%1");
    const noMove = !(await shown()).includes("movewin");
    await page.keyboard.press("Escape");
    check("AC-81 a host pane's row has no Move to Session…: no other session to move to", noMove);
    check("AC-07 row menu: no page errors", !f.errors.length, f.errors.join(" | "));
    await f.close();
  }
  runningIn(name);
}
