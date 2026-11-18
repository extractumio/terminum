// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// terminum.md AC-39 and terminum-kanban.md AC-117 against a fake terminumd (fake.mjs): a waiting line names its
// project, "<name>:" (32 characters at most, then "…") and a 4 px stripe in its color at its left edge; a session in no
// project and a task in Default have neither, and every mark stands where the others' do; a rename shows in place.
import { check, runningIn, until } from "./check.mjs";
import { drawn, group, item, openPage, SIZES } from "./fake.mjs";

const ID = (n) => `tmux:local:terminum:%${n}`;
const asking = (n, title) => item(ID(n), title, { index: String(n), state: "waiting" });
const LONG = "Customer portal redesign and migration plan";        // 43 characters
const projects = (name) => ({ t: "projects", items: [
  { id: "pa", name: "Terminum", color: 0, host: null, dir: "", members: [ID(4)] },
  { id: "pb", name, color: 3, host: null, dir: "", members: [ID(5)] }], order: [] });
const marks = (lines) => ({ t: "taskmarks", marks: {}, counts: {}, waiting: lines.length, failed: 0, lines });
const tline = (id, project) => ({ id, project, failed: false, words: `${id} waits for you`, key: `1|${id}` });

/** Where the buttons float, the lines are folded into counters (AC-117): the yellow one opens them. */
const reveal = async (page) => { if (await page.isVisible('#askcount [data-kind="waiting"]') && !(await page.isVisible("#asks"))) await page.click('#askcount [data-kind="waiting"]'); };

const lines = (page) => page.evaluate(() => [...document.querySelectorAll("#asks .ask:not([hidden])")].map((c) => {
  const r = c.getBoundingClientRect(), m = c.querySelector(".state").getBoundingClientRect(), p = c.querySelector(".proj");
  const shadow = getComputedStyle(c).boxShadow, ink = c.style.getPropertyValue("--proj");
  return { text: c.querySelector(".go").textContent, name: p?.textContent ?? null, title: c.querySelector(".go").title, markAt: Math.round(m.left - r.left),
    stripe: /inset/.test(shadow) && /\b4px 0px 0px/.test(shadow) && !!ink,
    ink, nameW: p ? p.getBoundingClientRect().width / r.width : 0, weight: p ? getComputedStyle(p).fontWeight : null };
}));

export async function webBannerProj(browser, name) {
  for (const [size, device] of SIZES) {
    const at = `(${name} ${size})`;
    runningIn(`${name} ${size}`);
    const f = await openPage(browser, { device });
    const { page } = f;
    await drawn(page);
    f.say(projects(LONG));
    f.say({ t: "layout", groups: [group("main", [item(ID(1), "api"), asking(4, "build"), asking(5, "portal"), asking(6, "scratch")])] });
    await until(async () => (await page.locator('#askcount [data-kind="waiting"]').isVisible()) || (await lines(page)).length === 3, 6000);
    await reveal(page);
    await until(async () => (await lines(page)).length === 3, 6000);
    let got = await lines(page);
    const by = (t) => got.find((l) => l.text.includes(t));
    check(`AC-39 ${at} a session of a project: "<name>:" before its words, weight 600, a stripe in its color`,
      by("build")?.text.startsWith("Terminum:build waits") && by("build").name === "Terminum:" && by("build").weight === "600" && by("build").stripe, JSON.stringify(got));
    check(`AC-39 ${at} a name over 32 characters: its first 32, then "…:", the key's title the whole name`,
      by("portal")?.name === `${[...LONG].slice(0, 32).join("").trimEnd()}…:` && by("portal").title === `Open portal (${LONG})`, JSON.stringify(by("portal")));
    check(`AC-39 ${at} a session in no project: no stripe, no name, its mark where the others' stand`,
      by("scratch")?.name === null && !by("scratch").ink && new Set(got.map((l) => l.markAt)).size === 1, JSON.stringify(got));
    if (size.startsWith("390")) check(`AC-39 ${at} on a phone the name takes at most 45% of the line, nothing scrolls sideways`,
      by("portal").nameW <= 0.451 && await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), JSON.stringify(by("portal")));
    await page.evaluate(() => { window.__kept = [...document.querySelectorAll("#asks .ask")].find((c) => c.textContent.includes("portal")); });
    f.say(projects("Billing"));
    check(`AC-39 ${at} a rename shows in place, the line not added anew`,
      await until(async () => (await lines(page)).some((l) => l.name === "Billing:")) && await page.evaluate(() => window.__kept?.isConnected && window.__kept.textContent.startsWith("Billing:")), JSON.stringify(await lines(page)));

    f.say({ t: "layout", groups: [group("main", [item(ID(1), "api")])] });
    f.say(marks([tline("T-7", "pa"), tline("T-8", "default")]));
    await until(async () => (await page.evaluate(() => document.querySelectorAll("#asks .taskline").length)) === 2, 6000);
    await reveal(page);
    got = await lines(page);
    check(`AC-117 ${at} a task's line names the project terminumd says, with its stripe`,
      by("T-7")?.text.startsWith("Terminum:T-7 waits for you") && by("T-7").stripe, JSON.stringify(got));
    check(`AC-117 ${at} a task in Default: no stripe, no name`, by("T-8")?.name === null && !by("T-8").ink, JSON.stringify(got));
    check(`no page error ${at}`, f.errors.length === 0, f.errors.join("; "));
    await f.context.close();
  }
}
