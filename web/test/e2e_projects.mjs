// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The session list's rows and projects against a fake terminumd (fake.mjs): each row's Clone
// (AC-07) and its menu on a right click or a long press (Clone, Add to Project, Close, moves),
// projects as groups of their own in their colors with [+], ⋯ Rename (AC-92), the unread mark
// (AC-93), the row's program at the right of its second line, ⌘Z and ⌘⇧Z (AC-03), and the
// search's magnifier out of sight while its field is open.

import { check, runningIn } from "./check.mjs";
import { drawn, group, item, openPage, pause, PHONE, WIDE } from "./fake.mjs";

const ID = (n) => `tmux:local:terminum:%${n}`;
const LAYOUT = [group("main", [item(ID(1), "api", { sub: ["claude", "~/work/api"], dir: "/Users/alex/work/api" }),
  item(ID(2), "docs", { index: "2", sub: ["zsh", "~/work/docs"] }), item(ID(4), "tests", { index: "3", sub: ["zsh", "~/work/api"], unread: true })], { wid: "local/main", new: "New window" }),
  group("notes", [item(ID(3), "notes", { sub: ["fish", "~/notes"] })], { wid: "local/notes", new: "New window" })];
const P1 = { id: "p1", name: "Group 1", color: 0, host: null, dir: "/Users/alex/work/api", members: [ID(1), ID(3)] };
const P2 = { id: "p2", name: "Group 2", color: 1, host: null, dir: "/Users/alex/work/docs", members: [ID(2)] };
const menuItems = (page) => page.evaluate(() => [...document.querySelectorAll("#rowmenu[open] button:not([data-action=x])")].map((b) => `${b.textContent}${b.disabled ? "-" : ""}`));
const last = (sent, t) => sent.filter((m) => m.t === t).pop();

export async function webProjects(browser, name) {
  runningIn(`${name} 1400×900`);
  const f = await openPage(browser, { device: WIDE, layout: LAYOUT });
  const { page, sent } = f;
  await drawn(page);
  const look = await page.evaluate(() => {
    const row = document.querySelector('#sessions .item[data-id$="%2"]');
    const s = row.querySelector(".s");
    return { news: document.querySelectorAll("#sessions h2 .new").length, clones: document.querySelectorAll("#sessions li > .clone").length,
      first: s.firstElementChild?.textContent, prog: s.querySelector(".progs")?.textContent,
      right: Math.round(s.getBoundingClientRect().right - s.querySelector(".progs").getBoundingClientRect().right),
      unread: !!document.querySelector('#sessions li:has(> .item[data-id$="%4"]) .unread'), plain: !!document.querySelector('#sessions li:has(> .item[data-id$="%2"]) .unread') };
  });
  check("AC-07 every row has Clone, no group header has [+]", look.news === 0 && look.clones === 4, JSON.stringify(look));
  check("AC-01 a row's second line: its folder first, its program at the right edge", look.first === "~/work/docs" && look.prog === "zsh" && look.right <= 1, JSON.stringify(look));
  check("AC-93 a row whose agent finished unseen has the unread mark, others none", look.unread && !look.plain, JSON.stringify(look));
  // where the unread mark is: x its middle from Clone's (left of it < 0), y from the middle of the row's second line (its program's)
  const mark = (pg = page) => pg.evaluate(() => {
    const li = document.querySelector('#sessions li:has(> .item[data-id$="%4"])'), u = li.querySelector(".unread"), c = li.querySelector(":scope > .clone"), t = li.querySelector(".t"), s = li.querySelector(".s");
    const mid = (el) => { const r = el.getBoundingClientRect(); return [r.left + r.width / 2, r.top + r.height / 2]; };
    return { w: u.offsetWidth, x: Math.round(mid(u)[0] - mid(c)[0]), y: Math.round(mid(u)[1] - mid(s)[1]), clear: Math.round(c.getBoundingClientRect().left - u.getBoundingClientRect().right),
      bg: getComputedStyle(u).backgroundColor, name: getComputedStyle(t).color, seen: getComputedStyle(u).visibility === "visible",
      round: getComputedStyle(u).borderTopLeftRadius !== getComputedStyle(u).borderTopRightRadius };
  });
  const m1 = await mark();
  await page.hover('#sessions .item[data-id$="%4"]');
  const covered = await mark();
  await page.mouse.move(700, 700);
  check("AC-93 the unread mark: 6 px, not round, in the session name's color, level with the row's second line (its program), under Clone's middle; hidden while Clone shows",
    m1.w === 6 && m1.round && m1.bg === m1.name && Math.abs(m1.y) <= 1 && Math.abs(m1.x) <= 1 && m1.seen && !covered.seen, JSON.stringify({ m1, covered }));
  await page.hover('#sessions .item[data-id$="%2"]');
  await page.click('#sessions li:has(> .item[data-id$="%2"]) > .clone');
  check("AC-07 Clone asks for a window beside it", last(sent, "clone")?.id === ID(2), JSON.stringify(last(sent, "clone")));

  await page.click('#sessions .item[data-id$="%2"]', { button: "right" });
  const items = await menuItems(page);
  check("AC-07 a right click opens the row's menu; the moves its place allows",
    items.join() === "Clone,Add to Project…,Move to the Top,Move Up,Move Down,Move to the Bottom,Close Session", items.join());
  check("AC-07 the menu names its session and has × to close it", await page.evaluate(() => document.querySelector("#rowmenu .mhead span")?.textContent === "docs"));
  await page.click('#rowmenu button[data-action="x"]');
  check("AC-07 × closes the menu", await page.isHidden("#rowmenu"));
  // modal: the page shaded, a press outside closes it and reaches nothing under it
  await page.click('#sessions .item[data-id$="%2"]', { button: "right" });
  const shade = await page.evaluate(() => getComputedStyle(document.getElementById("rowmenu"), "::backdrop").backgroundColor);
  const subs = sent.filter((m) => m.t === "sub").length;
  const r = await page.evaluate(() => { const b = document.querySelector('#sessions .item[data-id$="%1"]').getBoundingClientRect(); return [b.left + 40, b.top + 10]; });
  await page.mouse.click(r[0], r[1]);
  await pause(page, 150);
  check("AC-07 the menu is modal: the page shaded; a press outside closes it and opens nothing",
    !/rgba\(0, 0, 0, 0\)|transparent/.test(shade) && await page.isHidden("#rowmenu") && sent.filter((m) => m.t === "sub").length === subs, JSON.stringify({ shade, subs }));
  await page.click('#sessions .item[data-id$="%2"]', { button: "right" });
  await page.click('#rowmenu button[data-action="down"]');
  check("AC-07 Move Down reorders the session's windows", JSON.stringify(last(sent, "reorder")?.ids) === JSON.stringify([ID(1), ID(4), ID(2)]), JSON.stringify(last(sent, "reorder")));
  await page.click('#sessions .item[data-id$="%2"]', { button: "right" });
  await page.click('#rowmenu button[data-action="project"]');
  await page.click('#rowmenu button[data-action="newproject"]');
  await page.waitForSelector("#namebox[open]", { timeout: 3000 });
  const asked = await page.evaluate(() => ({ value: document.getElementById("namefield").value, yes: document.getElementById("nameok").textContent,
    border: getComputedStyle(document.getElementById("namebox")).borderTopWidth, pad: getComputedStyle(document.getElementById("namefield")).paddingLeft }));
  await page.fill("#namefield", "Rasp");
  await page.press("#namefield", "Enter");
  await pause(page, 100);
  check("AC-92 New project… asks its name at once (Group 1 suggested), Enter creates it",
    asked.value === "Group 1" && asked.yes === "Create" && JSON.stringify(last(sent, "project")) === JSON.stringify({ t: "project", action: "add", id: ID(2), name: "Rasp" }), JSON.stringify({ asked, m: last(sent, "project") }));
  check("AC-92 the name dialog looks as the page's dialogs (no frame, a padded field)", asked.border === "0px" && asked.pad === "12px", JSON.stringify(asked));

  f.say({ t: "projects", items: [P1, P2] });
  await pause(page, 200);
  const grouped = await page.evaluate(() => {
    const secs = [...document.querySelectorAll("#sessions section")];
    const p = secs[0], ids = (sec) => [...sec.querySelectorAll(".item")].map((i) => i.dataset.id.split(":").pop());
    const bars = [...p.querySelectorAll(".pbar")].map((b) => b.style.getPropertyValue("--dot"));
    return { first: p.dataset.pid, name: p.querySelector("h2 .label").textContent, rows: ids(p), main: ids(secs.find((s) => s.dataset.wid === "local/main")),
      notes: !!secs.find((s) => s.dataset.wid === "local/notes"), same: new Set(bars).size === 1,
      mark: [p.querySelector("h2 .toggle").querySelectorAll("svg").length === 1, !p.querySelector("h2 .pdot, h2 .picon")],
      apart: bars[0] !== secs[1].querySelector(".pbar").style.getPropertyValue("--dot"),
      // the space under its last session and above its name, from its tinted box's edges
      pad: (() => { const r = (e) => e.getBoundingClientRect(), lis = p.querySelectorAll("li");
        return [Math.round(r(p.querySelector("h2 .label")).top - r(p).top), Math.round(r(p).bottom - r(lis[lis.length - 1]).bottom)]; })() };
  });
  check("AC-92 projects come first, their sessions out of their tmux groups; a group left empty goes",
    grouped.first === "p1" && grouped.name === "Group 1" && grouped.rows.join() === "%1,%3" && grouped.main.join() === "%4" && !grouped.notes, JSON.stringify(grouped));
  check("AC-92 a project's rows share its color, another project's differs", grouped.same && grouped.apart, JSON.stringify(grouped));
  check("AC-92 a project's name has nothing before it but the chevron", grouped.mark.every(Boolean), JSON.stringify(grouped.mark));
  check("AC-92 under a project's last session as much space as above its name", grouped.pad[0] > 0 && grouped.pad[0] === grouped.pad[1], JSON.stringify(grouped.pad));
  f.say({ t: "projects", items: [{ ...P1, members: [ID(1), ID(3), ID(4)] }, P2] });
  await pause(page, 200);
  const m2 = await mark();
  check("AC-93 in a project too the unread mark takes the session name's color, level with the second line, under Clone's middle", m2.bg === m2.name && Math.abs(m2.x) <= 1 && Math.abs(m2.y) <= 1, JSON.stringify(m2));
  f.say({ t: "projects", items: [P1, P2] });
  await pause(page, 200);
  await page.click('#sessions section[data-pid="p1"] h2 .new');
  check("AC-92 a project's [+] asks for a window after its last session, in the project",
    JSON.stringify(last(sent, "clone")) === JSON.stringify({ t: "clone", id: ID(3), project: "p1" }), JSON.stringify(last(sent, "clone")));
  await page.click('#sessions section[data-pid="p1"] h2 .hostbtn');
  await page.click('#rowmenu button[data-action="rename"]');
  await page.fill("#namefield", "API");
  await page.press("#namefield", "Enter");          // Enter renames (the form's first button is Cancel)
  await pause(page, 100);                          // the dialog's close comes after the click
  check("AC-92 ⋯ Rename… names the project", JSON.stringify(last(sent, "project")) === JSON.stringify({ t: "project", action: "rename", project: "p1", name: "API" }), JSON.stringify(last(sent, "project")));
  await page.dblclick('#sessions section[data-pid="p2"] h2 .label');
  await page.waitForSelector("#namebox[open]", { timeout: 3000 });
  await page.fill("#namefield", "Docs");
  await page.click("#nameok");
  await pause(page, 100);
  const byDouble = last(sent, "project");
  check("AC-92 a double click on a project's name renames it too", byDouble?.action === "rename" && byDouble.project === "p2" && byDouble.name === "Docs", JSON.stringify(byDouble));
  await page.click('#sessions .item[data-id$="%3"]', { button: "right" });
  const inProject = await menuItems(page);
  await page.click('#rowmenu button[data-action="up"]');
  check("AC-92 a project's session moves in the project; its menu removes it from the project",
    inProject.includes("Remove from Group 1") && JSON.stringify(last(sent, "project")?.keys) === JSON.stringify([ID(3), ID(1)]), JSON.stringify({ inProject, m: last(sent, "project") }));

  await page.click('#sessions .item[data-id$="%3"]');
  await pause(page, 200);
  await page.click("#term");
  const n = sent.filter((m) => m.t === "in").length;
  await page.keyboard.press("Meta+KeyZ");
  await page.keyboard.press("Meta+Shift+KeyZ");
  await pause(page, 150);
  const keys = sent.filter((m) => m.t === "in").slice(n).map((m) => m.data);
  check("AC-03 ⌘Z sends the program's undo (Ctrl+_), ⌘⇧Z fish's redo (Alt+/)", keys.join() === "\x1f,\x1b/", JSON.stringify(keys));
  await page.click('#sessions .item[data-id$="%4"]');
  await pause(page, 200);
  await page.click("#term");
  await page.keyboard.press("Meta+Shift+KeyZ");
  await pause(page, 150);
  check("AC-03 ⌘⇧Z where the program has no redo says so", (await page.textContent("#toast")).includes("zsh has no redo"));

  await page.click("#findfab");
  await pause(page, 150);
  // the magnifier stays in sight, ringed as the sent history's button is when open; the field beside it,
  // its row level with the magnifier's middle; every pane 14 px left of its button, its first row so
  const magnifier = await page.evaluate(() => {
    const g = (id) => document.getElementById(id), r = (id) => g(id).getBoundingClientRect(), mid = (b) => Math.round(b.top + b.height / 2);
    return { open: !g("findbox").hidden, seen: getComputedStyle(g("findfab")).visibility, ring: getComputedStyle(g("findfab")).boxShadow,
      level: mid(r("findfab")) - mid(r("findq")), gap: Math.round(r("findfab").left - r("findbox").right), others: getComputedStyle(g("sentfab")).visibility };
  });
  await page.click("#findfab");
  await page.click("#sentfab");
  await pause(page, 200);
  const sentRing = await page.evaluate(() => getComputedStyle(document.getElementById("sentfab")).boxShadow);
  const sentLevel = await page.evaluate(() => { const r = (e) => e.getBoundingClientRect(), f = r(document.getElementById("sentfab")), h = r(document.querySelector("#sentpane .sphead"));
    return Math.round(f.top + f.height / 2) - Math.round(h.top + h.height / 2); });
  await page.click("#sentfab");
  check("the search's magnifier stays in sight while its field is open, ringed as the sent history's button; the field and the sent pane 14 px left of their buttons, their first row level with them",
    magnifier.open && magnifier.seen === "visible" && magnifier.ring === sentRing && magnifier.ring !== "none" && Math.abs(magnifier.level) <= 1 && magnifier.gap === 14 && magnifier.others === "visible" && Math.abs(sentLevel) <= 1,
    JSON.stringify({ magnifier, sentRing, sentLevel }));
  check("no page errors", !f.errors.length, f.errors.join(" | "));
  await f.close();

  runningIn(`${name} 390×844`);
  const p = await openPage(browser, { device: PHONE, layout: LAYOUT });
  await drawn(p.page);
  await p.page.click("#navbtn");
  await pause(p.page, 300);
  await p.page.evaluate(() => {
    const el = document.querySelector('#sessions .item[data-id$="%2"]'), r = el.getBoundingClientRect();
    el.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, pointerType: "touch", clientX: r.left + 40, clientY: r.top + 10 }));
  });
  await pause(p.page, 650);
  const held = await menuItems(p.page);
  check("AC-07 a finger resting on a row opens its menu", held[0] === "Clone" && held.length === 7, held.join());
  const pm = await mark(p.page);
  check("AC-93 on a phone (Clone always there) the unread mark stands just left of Clone, level with the row's second line",
    pm.seen && pm.clear >= 0 && pm.clear <= 4 && Math.abs(pm.y) <= 1, JSON.stringify(pm));
  check("AC-07 on a phone Clone is always there", await p.page.evaluate(() => getComputedStyle(document.querySelector("#sessions li > .clone")).opacity !== "0"));
  check("no page errors", !p.errors.length, p.errors.join(" | "));
  await p.close();
}
