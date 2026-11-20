// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// An agent's notice (terminum-agents.md AC-208) against a fake terminumd (fake.mjs), at a wide and a
// phone size: the page asks for the shown pane's windows when it shows a coding agent and when its
// screen changes, never for vim; terminumd's window shows its words and each command with a Copy, focus
// on Close, a second waits behind the first, Esc closes; "Do not show again" sends that kind off, and
// Settings' "Agent notices" shows settings.json's choice and turns every kind back on.

import { check, runningIn } from "./check.mjs";
import { drawn, group, item, line, openPage, pause, SIZES, spyCopies } from "./fake.mjs";

const CODEX = "tmux:local:terminum:%5", VIM = "tmux:local:terminum:%6";
const LAYOUT = [group("main", [item(CODEX, "Codex", { agent: "codex", shell: false, sub: ["codex", "~/p"] }),
  item(VIM, "notes", { shell: false, sub: ["vim", "~/p"] })])];
const SHARED = { t: "notice", id: CODEX, kind: "codexShared", title: "This Codex runs without terminum", parts: [
  { text: "This Codex attached to a shared codex app-server (process 77)." },
  { text: "Quit this Codex and start it again in its own process, one of:" },
  { cmd: "codex -c tui.alternate_screen=never" }, { cmd: "codex --no-daemon" },
  { text: "Or keep plain codex from attaching, then stop the server:" },
  { cmd: "codex features disable daemon_auto_start" }, { cmd: "codex app-server daemon stop" }] };
const FULL = { t: "notice", id: CODEX, kind: "fullScreen", title: "Codex runs full screen", parts: [
  { text: "This Codex draws on tmux's alternate screen, where tmux keeps no scrollback." }, { cmd: "codex -c tui.alternate_screen=never" }] };

const box = (page) => page.evaluate(() => {
  const d = document.getElementById("noticebox"), r = d.getBoundingClientRect(), close = document.getElementById("noticeclose").getBoundingClientRect();
  return { open: d.open, title: document.getElementById("noticetitle").textContent, focus: document.activeElement?.id ?? "",
    cmds: [...d.querySelectorAll(".noticecmd code")].map((c) => c.textContent), copies: d.querySelectorAll(".noticecmd button").length,
    ticked: document.getElementById("noticeoff").checked, inside: r.left >= 0 && r.right <= innerWidth + 0.5 && r.top >= 0 && r.bottom <= innerHeight + 0.5,
    closeH: Math.round(close.height), wide: document.documentElement.scrollWidth <= innerWidth };
});

export async function webNotices(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const phone = size.startsWith("390");
    const f = await openPage(browser, { device, layout: LAYOUT, screen: [line("› Ask Codex to do anything")] });
    const { page } = f;
    await drawn(page);
    await pause(page, 600);
    const asks = (id) => f.sent.filter((m) => m.t === "notice" && m.id === id).length;
    const first = asks(CODEX);
    // its screen goes to the alternate screen: asked again; the same screen again asks nothing
    const frame = (alt) => f.say({ t: "screen", sid: CODEX, full: true, n: 1, cols: 60, rows: 1, alt, ch: [[0, line("› Ask Codex")]] });
    frame(true);
    await pause(page, 700);
    frame(true);
    await pause(page, 700);
    check("AC-208 the page asks terminumd for a coding agent's windows when it shows it and when its screen changes, once a change",
      first === 1 && asks(CODEX) === 2, JSON.stringify({ first, then: asks(CODEX) }));

    await spyCopies(page);
    f.say(SHARED);
    f.say(FULL);
    await page.waitForSelector("#noticebox[open]", { timeout: 3000 }).catch(() => {});
    const one = await box(page);
    check("AC-208 the window shows terminumd's words, each command on its own line with a Copy, Do not show again unticked, focus on Close",
      one.open && one.title === SHARED.title && one.cmds.length === 4 && one.copies === 4 && !one.ticked && one.focus === "noticeclose", JSON.stringify(one));
    check("AC-208 the window stays inside the screen, nothing wider than it" + (phone ? ", Close a finger's size" : ""),
      one.inside && one.wide && (!phone || one.closeH >= 40), JSON.stringify(one));
    await page.click("#noticebox .noticecmd:nth-of-type(1) button");
    await pause(page, 200);
    const copied = await page.evaluate(() => window.copied ?? []);
    check("AC-208 a command's Copy copies exactly it", copied.includes("codex -c tui.alternate_screen=never"), JSON.stringify(copied));
    // a Space typed meanwhile presses Close, never the tick: closed, nothing turned off
    await page.focus("#noticeclose");
    await page.keyboard.press("Space");
    await pause(page, 400);
    const two = await box(page);
    check("AC-208 the second window waits behind the first and shows once it is closed; closing turns nothing off",
      two.open && two.title === FULL.title && !f.sent.some((m) => m.t === "notices"), JSON.stringify(two));
    // Do not show again, closed by Esc: that kind off
    await page.check("#noticeoff");
    await page.keyboard.press("Escape");
    await pause(page, 400);
    const offs = f.sent.filter((m) => m.t === "notices");
    const shut = await box(page);
    check("AC-208 Do not show again then Esc closes the window and turns that kind off with terminumd",
      !shut.open && offs.length === 1 && offs[0].kind === "fullScreen" && offs[0].on === false, JSON.stringify(offs));

    // vim's pane: no ask at all
    await page.evaluate((id) => document.querySelector(`#sessions .item[data-id="${id}"]`)?.click(), VIM);
    await pause(page, 900);
    check("AC-208 a pane that runs no coding agent (vim) is never asked about", asks(VIM) === 0, String(asks(VIM)));

    // Settings: settings.json's kinds turned off show the switch off; turning it on asks for every kind
    f.say({ t: "settings", page: { agentNoticesOff: ["fullScreen"] } });
    await pause(page, 200);
    const shownOff = await page.evaluate(() => document.getElementById("agentnotices").checked);
    await page.evaluate(() => { const s = document.getElementById("agentnotices"); s.checked = true; s.dispatchEvent(new Event("change")); });
    await pause(page, 200);
    const on = f.sent.filter((m) => m.t === "notices").at(-1);
    check("AC-208 Settings' Agent notices shows a kind turned off and turns every kind back on", !shownOff && on?.on === true && !("kind" in on), JSON.stringify({ shownOff, on }));
    check("AC-208 no page errors", !f.errors.length, f.errors.join(" | "));
    await f.close();
  }
  runningIn(name);
}
