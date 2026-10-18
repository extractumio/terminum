// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Checks of terminum's own parts of the session list and the page against a fake terminumd
// (fake.mjs), at a wide and a phone size: "New session" (AC-07), a host's group and its menu,
// "Add host" (AC-81), "Attach tmux sessions" here and on hosts (AC-89), the restore report and Restore again (AC-15), settings.json's page
// defaults, messages and hot keys, and custom.css (AC-18, AC-19), and the page's words.

import { check, runningIn } from "./check.mjs";
import { drawn, group, item, LAYOUT, openPage, pause, SIZES } from "./fake.mjs";

export async function webTmux(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const phone = size.startsWith("390");
    let layout = LAYOUT;
    const settings = { page: { textSize: 15, lines: "grid", screenMode: "auto" }, messages: ["settings.json: size must be latest, largest or manual"],
      hotkeys: { rows: [["F5", "C-r", "text:git status", "Paste"], ["S-Enter", "Up", "M-x", "Fn"]] }, css: "1" };
    let css = "body { --custom-probe: one; }";
    const f = await openPage(browser, { device, settings, css: () => css, hook: (m, say) => {
      if (m.t === "newsession" && m.name.includes(":")) say({ t: "error", msg: `bad session name: ${m.name}` });
      else if (m.t === "newsession") { layout = [...layout, group(m.name, [item("tmux:local:terminum:%9", "zsh")])]; say({ t: "layout", groups: layout }); }
    } });
    const { page, sent } = f;
    await drawn(page);
    const last = (t) => sent.filter((m) => m.t === t).at(-1);
    const drawer = async () => { if (phone) { await page.click("#navbtn"); await pause(page, 300); } };

    // settings.json's page defaults, its message once, the hot keys, custom.css
    await page.click(phone ? "#keysfab" : "#keysbtn");
    await pause(page);
    const got = await page.evaluate(() => ({ size: document.getElementById("sizeval").textContent,
      grid: document.querySelector('#tools [data-mode="grid"]').getAttribute("aria-pressed"), toast: document.getElementById("toast").textContent,
      keys: [...document.querySelectorAll("#hotkeys .keyrow")].map((r) => [...r.children].map((b) => b.textContent)),
      probe: getComputedStyle(document.body).getPropertyValue("--custom-probe").trim(),
      link: document.getElementById("customcss").getAttribute("href"),
      after: [...document.querySelectorAll('link[rel="stylesheet"]')].at(-1)?.id }));
    check("AC-19 settings: page defaults apply to a browser with no choice of its own (text size, Grid), its message is a toast",
      got.size === (phone ? "13" : "15") && got.grid === "true" && got.toast === settings.messages[0], JSON.stringify(got));
    check("AC-18 settings: the hot keys panel shows hotkeys.json's rows", JSON.stringify(got.keys) ===
      JSON.stringify([["F5", "^R", "git status", "Paste"], ["⇧↩", "↑", "⌥x", "Fn"]]), JSON.stringify(got.keys));
    const typed = async (label) => { const n = sent.length; await page.click(`#hotkeys button.key:text-is("${label}")`); await pause(page, 100); return sent.slice(n).filter((m) => m.t === "in").map((m) => m.data).join(""); };
    const keys = [await typed("F5"), await typed("^R"), await typed("git status"), await typed("⇧↩"), await typed("⌥x")];
    check("AC-18 settings: its keys type what a terminal sends", JSON.stringify(keys) === JSON.stringify(["\x1b[15~", "\x12", "git status", "\x1b[13;2u", "\x1bx"]), JSON.stringify(keys));
    check("AC-19 custom.css: linked after the page's stylesheets, and applied", got.after === "customcss" && got.link === "/custom.css?v=1" && got.probe === "one", JSON.stringify(got));
    css = "body { --custom-probe: two; }";
    const toasts = [];
    await page.exposeFunction("toastSeen", (t) => toasts.push(t));
    await page.evaluate(() => new MutationObserver(() => window.toastSeen(document.getElementById("toast").textContent))
      .observe(document.getElementById("toast"), { childList: true, characterData: true, subtree: true }));
    f.say({ t: "settings", ...settings, css: "2", hotkeys: { error: "hotkeys.json: line 2: no key named Fx" } });
    await page.waitForFunction(() => getComputedStyle(document.body).getPropertyValue("--custom-probe").trim() === "two", null, { timeout: 3000 }).catch(() => {});
    for (let i = 0; i < 60 && !toasts.includes("hotkeys.json: line 2: no key named Fx"); i++) await pause(page, 100);   // after the toast shown now
    const again = await page.evaluate(() => ({ probe: getComputedStyle(document.body).getPropertyValue("--custom-probe").trim(),
      keys: document.querySelectorAll("#hotkeys button.key").length, first: document.querySelector("#hotkeys button.key").textContent }));
    check("AC-19 custom.css: a new stamp loads it again, without a reload", again.probe === "two", JSON.stringify(again));
    check("AC-18 settings: a wrong hotkeys.json is said in a toast and the default panel stays; a message already said is not said again",
      again.keys === 24 && again.first === "Copy" && toasts.includes("hotkeys.json: line 2: no key named Fx") && !toasts.includes(settings.messages[0]),
      JSON.stringify({ again, toasts }));
    await page.click(phone ? "#keysfab" : "#keysbtn");

    // New session: an inline name; terminumd's refusal shows in the form; done, the form goes
    await drawer();
    // a phone's footer is a row of blobs: alike but for the blob's outline, each its own and no circle
    const foot = await page.evaluate((phone) => [...document.querySelectorAll(".nav-foot > button")].map((b) => {
      const s = getComputedStyle(b), i = getComputedStyle(b.querySelector("svg"));
      return [b.id, [s.height, s.width, s.backgroundColor, s.paddingLeft, s.fontWeight, s.justifyContent, i.width, i.backgroundColor].join(" "),
        phone ? i.borderRadius : s.borderRadius];
    }), phone);
    const radii = foot.map(([, , r]) => r);
    check("the session list's footer buttons look alike (a shared class, not one rule per id); on a phone their blobs differ and none is round",
      foot.length === 4 && new Set(foot.map(([, look]) => look)).size === 1
      && (phone ? new Set(radii).size === 4 && !radii.some((r) => /^50%( 50%)*$/.test(r)) : new Set(radii).size === 1), JSON.stringify(foot));
    await page.click("#newsessbtn");
    await page.fill("#newsessname", "bad:name");
    await page.keyboard.press("Enter");
    await pause(page);
    const refused = await page.evaluate(() => ({ err: document.getElementById("newsesserr").textContent, open: !document.getElementById("newsess").hidden,
      status: document.getElementById("status").hidden }));
    const sized = await page.evaluate(() => ({ field: getComputedStyle(document.getElementById("newsessname")).fontSize,
      form: getComputedStyle(document.getElementById("newsess")).fontSize, size: getComputedStyle(document.documentElement).getPropertyValue("--size").trim(),
      on: getComputedStyle(document.getElementById("addtmuxon")).fontSize,
      viewport: document.querySelector('meta[name="viewport"]').content }));
    check("AC-07 New session: the form and its fields speak at the terminal's size, on a touch screen too (maximum-scale keeps iOS from zooming in)",
      sized.form === sized.size && sized.field === sized.size && sized.on === sized.size && /maximum-scale=1\b/.test(sized.viewport), JSON.stringify(sized));
    await page.fill("#newsessname", "scratch");
    await page.keyboard.press("Enter");
    await pause(page, 300);
    check("AC-07 New session: an inline name sends newsession; tmux's refusal shows in the form; the new session closes it",
      refused.err === "bad session name: bad:name" && refused.open && refused.status && last("newsession")?.name === "scratch"
        && await page.isHidden("#newsess") && await page.isVisible('#sessions .grp[data-wid="local/scratch"]'), JSON.stringify(refused));

    // Add host
    if (await page.isHidden("#addhostbtn")) await drawer();
    await page.click("#addhostbtn");
    const first = await page.evaluate(() => document.activeElement?.id);
    await page.keyboard.type("alex@devbox.example -p 2222");
    await page.check("#addhostfiles");
    await page.click('#addhost button[type="submit"]');
    await pause(page);
    const waits = await page.isVisible("#addhost");
    f.say({ t: "hostadded", name: "devbox" });
    await pause(page);
    check("AC-81 Add host: what follows ssh, no name, the files tick; it closes and clears once terminumd added it",
      first === "addhostssh" && waits && JSON.stringify(last("addhost")) === JSON.stringify({ t: "addhost", name: "", recipe: "alex@devbox.example -p 2222", files: true })
        && await page.isHidden("#addhost") && await page.inputValue("#addhostssh") === "" && !(await page.isChecked("#addhostfiles")),
      JSON.stringify({ first, waits, sent: last("addhost") }));

    // a host's group: cornflower blue, its state; rows name the host in violet; its menu
    const host = (state, extra = {}) => group("devbox", [item("tmux:devbox:default:%1", "build", { host: "devbox", sub: ["make", "~/src"] })],
      { host: "devbox", state, new: state === "attached" ? "New window" : null, ...extra });   // as terminumd: no tmux, no new window
    f.say({ t: "layout", groups: [...LAYOUT, host("reconnecting", { why: "reconnecting (2 s)" })] });
    await pause(page);
    const seen = await page.evaluate(() => { const g = document.querySelector('#sessions .grp[data-wid="devbox/terminum"]'), c = (el) => getComputedStyle(el).color;
      return { label: g.querySelector("h2 .label").textContent, state: g.querySelector(".gstate").textContent,
        color: c(g.querySelector("h2 .label")), row: c(g.querySelector(".item .host")),
        rowText: g.querySelector(".item .host").textContent }; });
    const ATTACH = "ssh -t -- devbox.example 'tmux attach -t =terminum'";
    const WHY = "kex_exchange_identification: read: Connection reset by peer; Connection reset by 10.0.0.5 port 22";
    f.say({ t: "layout", groups: [...LAYOUT, host("failed", { why: WHY, session: "terminum", attach: ATTACH })] });
    await pause(page);
    const failed = await page.evaluate(() => { const g = document.querySelector('#sessions .grp[data-wid="devbox/terminum"]'), s = g.querySelector(".gstate");
      return { label: g.querySelector("h2 .label").textContent, state: s.textContent, lines: Math.round(s.getBoundingClientRect().height / parseFloat(getComputedStyle(s).lineHeight)),
        inside: s.getBoundingClientRect().right <= g.getBoundingClientRect().right }; });
    check("AC-81 a host group that is not attached offers no new window", await page.isHidden('#sessions .grp[data-wid="devbox/terminum"] h2 .new'));
    const tsess = await page.evaluate(() => { const h = document.querySelector('#sessions .grp[data-wid="devbox/terminum"] h2');
      return { sess: h.querySelector(".tsess")?.textContent, copy: h.querySelector(".attach")?.title }; });
    check("AC-01 a group names its tmux session and has a copy button for the attach command",
      tsess.sess === "terminum" && tsess.copy === `Copy: ${ATTACH}`, JSON.stringify(tsess));
    check("AC-81 host group: its name in cornflower blue, its state on a line under it (a long one wrapped in full), rows naming the host in violet",
      seen.label === "devbox" && seen.state === "reconnecting (2 s)" && seen.color === "rgb(100, 149, 237)" && seen.row === "rgb(169, 139, 255)" && seen.rowText === "devbox"
        && failed.label === "devbox" && failed.state === WHY && failed.lines >= 2 && failed.inside, JSON.stringify({ seen, failed }));
    await page.click('#sessions .grp[data-wid="devbox/terminum"] .hostbtn');
    const offered = await page.evaluate(() => [...document.querySelectorAll("#hostmenu button")].map((b) => b.textContent));
    await page.click('#hostmenu button[data-action="retry"]');
    await pause(page, 100);
    const retry = last("hostaction");
    await page.click('#sessions .grp[data-wid="devbox/terminum"] .hostbtn');
    await page.click('#hostmenu button[data-action="remove"]');
    const ask = await page.textContent("#confirmtext");
    await page.click("#confirmok");
    for (let i = 0; i < 40 && last("hostaction")?.action !== "remove"; i++) await pause(page, 50);   // a loaded machine: up to 2 s
    check("AC-81 host group: its menu retries now, turns it off, removes it (asked first)",
      offered.join() === "Retry now,Off,Remove" && retry?.name === "devbox" && retry.action === "retry" && /^Remove devbox\?/.test(ask)
        && last("hostaction").action === "remove", JSON.stringify({ offered, retry, remove: last("hostaction") }));

    // Attach tmux sessions (AC-89): "On" this Mac or a host, the running sessions as ticks, a name for one
    // that does not run yet; a late answer for another place is ignored; a group that waits; detach
    if (await page.isHidden("#addtmuxbtn")) await drawer();
    await page.click("#addtmuxbtn");
    const asked = last("tmuxsessions");
    f.say({ t: "tmuxsessions", server: "devbox:default", names: ["late"] });
    f.say({ t: "tmuxsessions", server: "local:default", names: ["work", "notes"], ends: ["notes"] });
    await pause(page);
    const ticks = await page.evaluate(() => [...document.querySelectorAll("#addtmuxticks input")].map((b) => b.value));
    const warned = await page.evaluate(() => [...document.querySelectorAll("#addtmuxticks .tick")].map((l) => [l.querySelector("input").value, l.querySelector("span")?.textContent ?? ""]));
    check("AC-89 a session tmux destroys with its last client (destroy-unattached) is said beside its tick",
      warned.length === 2 && warned[0][1] === "" && /destroy-unattached is on: tmux closes it when terminum is its last client/.test(warned[1][1]), JSON.stringify(warned));
    await page.check('#addtmuxticks input[value="work"]');
    await page.fill("#addtmuxname", "later");
    await page.keyboard.press("Enter");
    await pause(page);
    const waiting = await page.isVisible("#addtmux");
    f.say({ t: "tmuxadded", server: "local:default", sessions: ["work", "later"] });
    await pause(page);
    check("AC-89 Attach tmux sessions: on this Mac, the running sessions are ticks (a late answer for another place ignored); ticked and named attach in one message; it closes once attached",
      JSON.stringify(asked) === JSON.stringify({ t: "tmuxsessions", server: "local:default" }) && ticks.join() === "work,notes" && waiting
        && JSON.stringify(last("addtmux")) === JSON.stringify({ t: "addtmux", server: "local:default", sessions: ["work", "later"] })
        && await page.isHidden("#addtmux"), JSON.stringify({ asked, ticks, waiting, sent: last("addtmux") }));
    const own = group("later", [], { wid: "local:default/later", attached: "default", where: "tmux", server: "local:default",
      state: "absent", why: "not running on your tmux: attached when it starts" });
    f.say({ t: "layout", groups: [...LAYOUT, own] });
    await pause(page);
    const absent = await page.textContent('#sessions .grp[data-wid="local:default/later"] .gstate');
    await page.click('#sessions .grp[data-wid="local:default/later"] .hostbtn');
    const detach = await page.evaluate(() => [...document.querySelectorAll("#hostmenu button")].map((b) => b.textContent));
    await page.click('#hostmenu button[data-action="detach"]');
    await pause(page, 100);
    check("AC-89 a session of the user's tmux that does not run is a group saying so; its menu detaches it",
      absent === "not running on your tmux: attached when it starts" && detach.join() === "Detach from terminum"
        && JSON.stringify(last("detachtmux")) === JSON.stringify({ t: "detachtmux", server: "local:default", session: "later" }), JSON.stringify({ absent, detach }));

    // a host's other sessions (AC-89): offered once a host just added is attached; later by its icon and menu
    await page.click("#addhostbtn");
    await page.fill("#addhostssh", "alex@box.example");
    await page.keyboard.press("Enter");
    await pause(page);
    f.say({ t: "hostadded", name: "box" });
    const boxg = (state, others) => group("box", [item("tmux:box:default:%1", "build", { host: "box" })], { host: "box", session: "terminum", state, others, server: "box:default" });
    f.say({ t: "layout", groups: [...LAYOUT, boxg("reconnecting", 0)] });
    await pause(page);
    const early = await page.isVisible("#addtmux");
    f.say({ t: "layout", groups: [...LAYOUT, boxg("attached", 2)] });
    await pause(page);
    const offered2 = { open: await page.isVisible("#addtmux"), on: await page.inputValue("#addtmuxon"), asked: last("tmuxsessions") };
    f.say({ t: "tmuxsessions", server: "box:default", names: ["api", "db"] });
    await pause(page);
    const boxTicks = await page.evaluate(() => [...document.querySelectorAll("#addtmuxticks input")].map((b) => `${b.value}:${b.checked}`));
    await page.click("#addtmuxcancel");
    check("AC-89 after Add host, once the host is attached with other sessions, Attach tmux sessions opens on it with them unticked",
      !early && offered2.open && offered2.on === "box:default" && offered2.asked?.server === "box:default" && boxTicks.join() === "api:false,db:false",
      JSON.stringify({ early, offered2, boxTicks }));
    const icon = await page.getAttribute('#sessions .grp[data-wid="box/terminum"] h2 .import', "title");
    await page.click('#sessions .grp[data-wid="box/terminum"] h2 .import');
    await pause(page);
    const byIcon = { open: await page.isVisible("#addtmux"), on: await page.inputValue("#addtmuxon") };
    await page.click("#addtmuxcancel");
    await page.click('#sessions .grp[data-wid="box/terminum"] .hostbtn');
    const boxMenu = await page.evaluate(() => [...document.querySelectorAll("#hostmenu button")].map((b) => b.textContent));
    await page.click('#hostmenu button[data-action="import"]');
    await pause(page);
    const byMenu = await page.inputValue("#addtmuxon");
    await page.click("#addtmuxcancel");
    f.say({ t: "layout", groups: [...LAYOUT, boxg("attached", 0)] });
    await pause(page);
    const none = await page.isHidden('#sessions .grp[data-wid="box/terminum"] h2 .import');
    check("AC-89 later: the host's import icon (with the count) and its menu item open Attach tmux sessions on it; no icon when nothing else runs",
      icon === "Attach tmux sessions running on box (2)" && byIcon.open && byIcon.on === "box:default" && boxMenu.join() === "Retry now,Off,Attach tmux sessions…,Remove"
        && byMenu === "box:default" && none, JSON.stringify({ icon, byIcon, boxMenu, byMenu, none }));
    if (phone) await page.click("#drawerclose");

    // the restore report, and Restore again after it
    await page.click("#menubtn");
    const before = await page.isHidden("#restoreagain");
    await page.keyboard.press("Escape");
    const text = "Restored 2 sessions, 4 windows. 1 pane ran ssh devbox: start it again";
    f.say({ t: "restore", report: { text, sessions: 2, windows: 4, programs: ["ssh devbox"], errors: [], notes: [] } });
    await pause(page);
    const banner = await page.textContent("#restored > span");
    await page.click("#restored .x");
    await page.click("#menubtn");
    await page.click("#restoreagain");
    await pause(page, 100);
    check("AC-15 restore: the report's text is a banner with a close button; Restore again shows only after a report",
      before && banner === text && await page.isHidden("#restored") && last("restore")?.t === "restore", JSON.stringify({ before, banner }));

    // a newer release (AC-17): the chip and its menu
    f.say({ t: "update", latest: "v0.4.0" });
    await pause(page);
    const chip = await page.textContent("#updchip");
    await page.click("#updchip");
    const items = await page.evaluate(() => [...document.querySelectorAll("#updmenu button")].map((b) => b.textContent));
    await page.click("#updskip");
    await pause(page, 100);
    const skip = last("updatechoice");
    await page.click("#updchip");
    await page.click("#updoff");
    await pause(page, 100);
    const off = last("updatechoice");
    f.say({ t: "update", latest: null });
    await pause(page);
    check("AC-17 update: the chip \"↑ v0.4.0\" offers the upgrade command, Skip and Don't check; null hides it",
      chip === "↑ v0.4.0" && items.join() === "Copy upgrade command,Skip v0.4.0,Don't check for updates" && skip?.action === "skip" && skip.tag === "v0.4.0"
        && off?.action === "off" && await page.isHidden("#updchip"), JSON.stringify({ chip, items, skip, off }));

    // an error about the shown pane, and its end (an empty message: tmux is back)
    f.say({ t: "error", msg: "Connection to devbox lost", sid: "tmux:local:terminum:%1" });
    await pause(page);
    const err = await page.textContent("#status");
    f.say({ t: "error", msg: "", sid: "tmux:local:terminum:%1" });
    await pause(page);
    check("AC-81 error: the shown pane's error is on the status line, an empty one clears it", err === "Connection to devbox lost" && await page.isHidden("#status"));

    // the page's words: terminum and tmux
    f.say({ t: "layout", groups: [] });
    await pause(page);
    const words = await page.evaluate(() => ({ title: document.title, empty: document.querySelector("#sessions .empty")?.textContent,
      iterm: /iTerm/.test(document.body.innerText + document.title + [...document.querySelectorAll("[title]")].map((e) => e.title).join(" ")) }));
    check("AC-01 words: the page is terminum's and its list tmux's (\"No tmux sessions.\"), no iTerm anywhere",
      words.title === "terminum" && words.empty === "No tmux sessions." && !words.iterm, JSON.stringify(words));
    check("AC-19 settings: no page errors", !f.errors.length, f.errors.join(" | "));
    await f.close();
  }
  runningIn(name);
}
