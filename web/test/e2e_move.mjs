// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-07 checks of the pane's menu against a fake terminumd (fake.mjs), at a wide and a phone
// size: Split right and below, Close pane asked first with the program's name, Move window to
// session… listing the other sessions of the same tmux server.

import { check, runningIn } from "./check.mjs";
import { drawn, group, item, openPage, pause, SIZES } from "./fake.mjs";

const SHOWN = "tmux:local:terminum:%1";

export async function webMove(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const layout = [group("main", [item(SHOWN, "api", { sub: ["claude", "~/work/api"] }), item("tmux:local:terminum:%2", "docs", { index: "2" })]),
      group("notes", [item("tmux:local:terminum:%3", "notes")]),
      group("devbox", [item("tmux:devbox:default:%1", "remote", { host: "devbox" })], { host: "devbox", state: "attached" })];
    const f = await openPage(browser, { device, layout });
    const { page, sent } = f;
    await drawn(page);
    const menu = async (id) => { await page.click("#menubtn"); await page.click(id); await pause(page, 100); };
    const last = (t) => sent.filter((m) => m.t === t).at(-1);

    const labels = await page.evaluate(() => ["quickbelow", "quickright", "quickleft"].map((id) => document.getElementById(id).querySelector("span").firstChild.textContent));
    check("AC-88 menu: the splits are the quick shell's (Split below, right, left); no tmux split of the pane",
      labels.join() === "Split below,Split right,Split left" && !await page.$("#splitright"), labels.join());

    await menu("#closepane");
    const asked = await page.evaluate(() => ({ open: document.getElementById("confirmbox").open,
      text: document.getElementById("confirmtext").textContent, yes: document.getElementById("confirmok").textContent }));
    await page.click('#confirmbox button[value="cancel"]');
    await pause(page);
    const kept = !last("kill");
    await menu("#closepane");
    await page.click("#confirmok");
    await pause(page);
    check("AC-07 menu: Close session asks \"Close the session …? The program running in it (claude) ends.\"; Cancel sends nothing, Close Session kills the pane",
      asked.open && /^Close the session .+\? The program running in it \(claude\) ends\.$/.test(asked.text) && asked.yes === "Close Session" && kept
        && last("kill")?.id === SHOWN, JSON.stringify({ asked, kill: last("kill") }));

    await page.click("#menubtn");
    await page.click("#movewin");
    const offered = await page.evaluate(() => [...document.querySelectorAll("#movelist button")].map((b) => b.textContent));
    await page.click('#movelist button[data-session="local/notes"]');
    await pause(page);
    check("AC-07 menu: Move window to session… lists the other sessions of its tmux server (not a host's) and moves the window",
      offered.join() === "notes" && last("move")?.id === SHOWN && last("move").session === "local/notes" && await page.isHidden("#menu"),
      JSON.stringify({ offered, move: last("move") }));

    // a host's pane: its server has one session, there is nowhere to move to
    await page.evaluate(() => document.querySelector('#sessions .item[data-id="tmux:devbox:default:%1"]')?.click());
    await page.waitForFunction(() => document.querySelector("#current .t").textContent === "remote");
    await page.click("#menubtn");
    const noMove = await page.isHidden("#movewin");
    await page.keyboard.press("Escape");
    check("AC-81 menu: a host pane's window has no other session to move to", noMove);
    check("AC-07 menu: no page errors", !f.errors.length, f.errors.join(" | "));
    await f.close();
  }
  runningIn(name);
}
