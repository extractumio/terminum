// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The sections' switcher (terminum.md AC-167, sections.js) on the session page against a fake terminumd
// (fake.mjs), at both sizes: its four sections in the list's head with the shown one marked, words where
// they fit and icons where not, the filter folded to a key beside it, its copy in the header while the
// list is hidden, a phone's sheet. The other pages' are checked live (e2e_livehistory.mjs, e2e_liveboardsession.mjs).
import { check, runningIn } from "./check.mjs";
import { drawn, openPage, pause, SIZES } from "./fake.mjs";

const HEAD = ".nav-head .sections";

export async function webSections(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const phone = size.startsWith("390");
    const f = await openPage(browser, { device, url: "/sessions/" });
    const { page } = f;
    await drawn(page);
    const links = await page.evaluate((sel) => [...document.querySelectorAll(`${sel} a`)].map((a) => [a.getAttribute("href"), a.getAttribute("aria-label"), a.getAttribute("aria-current"), !!a.title]), HEAD);
    check("AC-167 the list's head holds the four sections in their order, the session page's marked, each named and told",
      JSON.stringify(links) === JSON.stringify([["/sessions/", "Sessions", "page", true], ["/kanban/", "Kanban", null, true], ["/dashboard/", "Dashboard", null, true], ["/history/", "Messages", null, true]]), JSON.stringify(links));
    check("AC-167 the header's tools hold no way to another section", await page.evaluate(() => document.querySelectorAll("#tools a, #menu a[href^='/']").length) === 0);
    /** What shows of the switcher in `sel`: its words, and whether it and the head's keys stand apart inside the list. */
    const look = (sel) => page.evaluate((sel) => {
      const box = (e) => e.getBoundingClientRect(), seen = (e) => box(e).width > 0 && getComputedStyle(e).visibility !== "hidden";
      const nav = document.querySelector(sel), head = nav.parentElement;
      const parts = [...head.children].filter((e) => seen(e)).map(box);
      return { shown: seen(nav), words: [...nav.querySelectorAll(".word")].filter(seen).map((w) => w.textContent),
        apart: parts.every((b, i) => parts.every((o, j) => i === j || b.right <= o.left + 0.5 || o.right <= b.left + 0.5 || b.bottom <= o.top + 0.5 || o.bottom <= b.top + 0.5)),
        inside: parts.every((b) => b.left >= -0.5 && b.right <= box(head).right + 0.5), lines: new Set(parts.map((b) => Math.round(b.top + b.height / 2))).size,
        least: Math.min(...[...nav.querySelectorAll("a")].map((a) => Math.min(box(a).width, box(a).height))), wide: Math.min(...[...nav.querySelectorAll("a")].map((a) => box(a).width)) };
    }, sel);

    if (phone) {
      check("AC-167 a phone's header holds no switcher", !(await look("#bar > .sections")).shown);
      if (!await page.evaluate(() => document.getElementById("app").classList.contains("nav-open"))) await page.click("#navbtn");
      await pause(page, 300);
      const sheet = await look(HEAD);
      check("AC-167 a phone's sheet: the sections first, the shown one with its word, every one a finger's size, nothing over another",
        sheet.shown && sheet.words.join() === "Sessions" && sheet.least >= 40 && sheet.wide >= 44 && sheet.apart && sheet.inside, JSON.stringify(sheet));
      check("AC-167 a phone's sheet keeps the filter a field and its close key", await page.evaluate(() => document.getElementById("filter").getBoundingClientRect().width > 150) && await page.isVisible("#drawerclose"));
      check(`${name} ${size}: no page errors`, f.errors.length === 0, f.errors.join("; "));
      await f.close();
      continue;
    }

    const setWidth = async (w) => { await page.evaluate((w) => document.documentElement.style.setProperty("--nav-w", `${w}px`), w); await pause(page, 100); };
    await setWidth(300);
    const narrow = await look(HEAD);
    check("AC-167 a 300 px list: icons, one line with the filter's, New session's and the list's keys, nothing over another",
      narrow.shown && !narrow.words.length && narrow.apart && narrow.inside && narrow.lines === 1, JSON.stringify(narrow));
    await setWidth(440);
    const roomy = await look(HEAD);
    check("AC-167 a 440 px list: the shown section keeps its word", roomy.words.join() === "Sessions" && roomy.apart && roomy.inside && roomy.lines === 1, JSON.stringify(roomy));
    await setWidth(200);
    const least = await look(HEAD);
    check("AC-167 the narrowest list: the keys go under the sections, none cut or over another", least.shown && least.apart && least.inside && least.lines === 2, JSON.stringify(least));
    await setWidth(300);

    // the filter: a key until pressed, focused or holding words
    const filter = () => page.evaluate(() => ({ width: Math.round(document.getElementById("filter").getBoundingClientRect().width), focused: document.activeElement?.id === "filter",
      key: Math.round(document.querySelector(".nav-head .search").getBoundingClientRect().width) }));
    const folded = await filter();
    check("AC-167 the filter is a key beside the sections", folded.width <= 2 && folded.key === 30 && !folded.focused, JSON.stringify(folded));
    await page.click(".nav-head .search");
    const opened = await filter();
    check("AC-167 a press opens the field in the sections' place", opened.focused && opened.width > 100 && !(await look(HEAD)).shown, JSON.stringify(opened));
    await page.keyboard.type("ap");
    await page.click("#term");
    check("AC-167 it stays open while it holds words", (await filter()).width > 100 && !(await look(HEAD)).shown);
    await page.click(".nav-head .search");
    await page.keyboard.press("Escape");
    await pause(page, 100);
    check("AC-167 Esc empties and folds it, the sections back", (await filter()).width <= 2 && (await look(HEAD)).shown && await page.inputValue("#filter") === "");
    await page.click("#term");
    await page.keyboard.press("Escape");
    await page.evaluate(() => document.activeElement?.blur());
    await page.keyboard.press("/");
    check("AC-167 \"/\" opens it", (await filter()).focused && (await filter()).width > 100);
    await page.keyboard.press("Escape");

    // the list hidden: the sections stand in the header, after ☰
    check("AC-167 with the list shown the header holds no copy", !(await look("#bar > .sections")).shown);
    await page.click("#navclose");
    await pause(page, 200);
    const bar = await look("#bar > .sections");
    const order = await page.evaluate(() => { const x = (s) => document.querySelector(s).getBoundingClientRect(); return x("#navbtn").right <= x("#bar > .sections").left && x("#bar > .sections").right <= x("#current").left; });
    check("AC-167 with the list hidden the sections stand in the header as icons, between ☰ and the title", bar.shown && !bar.words.length && bar.apart && order, JSON.stringify({ bar, order }));
    await page.click("#navbtn");
    await pause(page, 200);
    check("AC-167 the list back: the header's copy goes", !(await look("#bar > .sections")).shown && (await look(HEAD)).shown);
    check(`${name} ${size}: no page errors`, f.errors.length === 0, f.errors.join("; "));
    await f.close();
  }
}
