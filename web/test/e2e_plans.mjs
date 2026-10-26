// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Plans and notes (AC-97, AC-98) against a fake terminumd (fake.mjs), at both sizes: a row's percent,
// track and notes count; the header's progress; the Plan pane opened from either, its items, ETA,
// source and notes, Done / Reopen / Delete / Insert into chat; plannew asks again; another session
// closes it; the agent's text stays text. Alone: node web/test/e2e_all.mjs webPlans.
import { check, runningIn } from "./check.mjs";
import { drawn, group, item, openPage, pause, SIZES } from "./fake.mjs";

const A = "tmux:local:terminum:%1", B = "tmux:local:terminum:%2", C = "tmux:local:terminum:%3";
const HOSTILE = "<img src=x onerror=window.__pwned=1>";
let NOW = 0;                                       // set as each size starts: the ages are of that moment
const rowPlan = () => ({ done: 3, total: 7, pct: 43, eta: NOW + 3600, updated: NOW - 300, stale: false, partial: false });
const layout = () => [group("main", [item(A, "api", { plan: rowPlan(), notes: 2 }),
  item(B, "docs", { index: "2", plan: { ...rowPlan(), stale: true } }), item(C, "notes", { index: "3" })])];
const ITEMS = [["read the code", "completed"], ["write the check", "in_progress"], ["fix the page", "pending"],
  ["ship it", "blocked"], [HOSTILE, "pending"], ["red \x1b[31mtext\x1b[0m and a bell\x07", "pending"]];
const plan = () => ({ title: "Session plans", doing: "writing the browser check", items: ITEMS.map(([text, status]) => ({ text, status })),
  done: 3, total: 7, pct: 43, eta: NOW + 3600, eta_given: NOW - 600, updated: NOW - 300, source: "mcp", partial: true, agent: "claude" });
const notes = () => [{ id: 7, kind: "bug", text: "old done one", file: null, status: "done", at: NOW - 900, done_at: NOW - 800 },
  { id: 9, kind: "followup", text: "retry loop never resets", file: null, status: "open", at: NOW - 60, done_at: null },
  { id: 8, kind: "question", text: HOSTILE, file: "src/a.rs", status: "open", at: NOW - 120, done_at: null }];

const q = (page, fn, arg) => page.evaluate(fn, arg);
const paneOpen = (page) => q(page, () => !document.getElementById("planpane").hidden);
const asked = (f, from, key = A) => f.sent.slice(from).filter((m) => m.t === "plan" && m.key === key).length;

export async function webPlans(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const phone = size.startsWith("390");
    NOW = Math.floor(Date.now() / 1000);
    const f = await openPage(browser, { device, layout: layout(), hook: (m, say) => {
      if (m.t === "plan") say({ t: "plan", key: m.key, plan: plan(), notes: notes() });
    } });
    const { page } = f;
    await drawn(page);
    await pause(page, 200);

    // the rows (AC-97, AC-98)
    const rows = await q(page, ([a, b, c]) => {
      const r = (id) => document.querySelector(`.item[data-id="${id}"]`);
      const pct = (id) => r(id)?.querySelector(".plpct");
      return { a: pct(a)?.textContent, title: pct(a)?.title, track: !!r(a)?.querySelector(".pltrack .plfill"),
        fill: r(a)?.querySelector(".pltrack")?.style.getPropertyValue("--pct"), notes: r(a)?.querySelector(".st .notes")?.textContent,
        bStale: pct(b)?.classList.contains("stale"), bOpacity: pct(b) && getComputedStyle(pct(b)).opacity, aOpacity: pct(a) && getComputedStyle(pct(a)).opacity,
        c: !!(pct(c) || r(c)?.querySelector(".pltrack")), cNotes: !!r(c)?.querySelector(".st .notes") };
    }, [A, B, C]);
    check("AC-97 a row with a plan shows 43% and its track filled to 43%", rows.a === "43%" && rows.track && rows.fill === "43%", JSON.stringify(rows));
    check("AC-97 the percent's hover says done of total, the ETA and the age", /^3 of 7 · ETA ~\d\d:\d\d · updated 5 min ago$/.test(rows.title ?? ""), rows.title);
    check("AC-97 a stale plan's percent is dimmed", rows.bStale && Number(rows.bOpacity) < 1 && Number(rows.aOpacity) === 1, JSON.stringify(rows));
    check("AC-97 a row without a plan shows neither percent nor track; AC-98 its notes count shows only with notes", !rows.c && rows.notes === "2" && !rows.cNotes, JSON.stringify(rows));

    // the header (the shown session is A)
    const head = await q(page, () => { const b = document.getElementById("curplan"); return { hidden: b.hidden, text: document.getElementById("curplant").textContent, title: b.title }; });
    check("AC-97 the header shows 3/7 · 43% · ETA ~hh:mm · 5 min ago", !head.hidden && /^3\/7 · 43% · ETA ~\d\d:\d\d · 5 min ago$/.test(head.text) && /2 notes/.test(head.title), JSON.stringify(head));

    const fit = await q(page, () => { const r = (s) => document.querySelector(s).getBoundingClientRect(), t = r("#current .t"), p = r("#curplan"), e = r("#rename");
      return { title: Math.round(t.width), need: document.querySelector("#current .t").scrollWidth, plan: [Math.round(p.left), Math.round(p.right)], rename: Math.round(e.left), shown: e.width > 0 }; });
    check("AC-97 the header's progress leaves the title its room and ends before ✎", fit.title >= Math.min(fit.need - 4, 40) && (!fit.shown || fit.plan[1] <= fit.rename), JSON.stringify(fit));

    // the header opens the pane: it asks, the reply is drawn
    let from = f.sent.length;
    await page.click("#curplan");
    await page.waitForSelector("#planbody .plitems", { timeout: 3000 }).catch(() => {});
    const drawnPane = await q(page, () => {
      const b = document.getElementById("planbody"), t = (s) => b.querySelector(s)?.textContent;
      return { title: t(".pltitle"), doing: t(".pldoing"), marks: [...b.querySelectorAll(".plitem .plmark")].map((e) => e.textContent).join(""),
        partial: t(".plpartial"), meta: [...b.querySelectorAll(".plmeta")].map((e) => e.textContent),
        notes: [...b.querySelectorAll(".plnote")].map((e) => `${e.dataset.id}:${e.classList.contains("open") ? "open" : "done"}`),
        expanded: document.getElementById("curplan").getAttribute("aria-expanded") };
    });
    check("AC-97 a press on the header's progress opens the Plan pane and asks {t:plan, key}", await paneOpen(page) && asked(f, from) === 1, JSON.stringify(f.sent.slice(from)));
    check("AC-97 the pane shows the title, what the agent does, ✓ ▶ ○ ⚠ and the partial note",
      drawnPane.title === "Session plans" && drawnPane.doing === "writing the browser check" && drawnPane.marks === "✓▶○⚠○○" && /partial/.test(drawnPane.partial ?? ""), JSON.stringify(drawnPane));
    check("AC-97 the pane's ETA is the agent's estimate with its time given, and the source",
      drawnPane.meta.some((m) => /^ETA ~\d\d:\d\d \(agent's estimate, given \d\d:\d\d\)$/.test(m)) && drawnPane.meta.some((m) => /MCP/.test(m)), JSON.stringify(drawnPane.meta));
    check("AC-98 open notes first, newest first, then the done ones", drawnPane.notes.join(",") === "9:open,8:open,7:done", JSON.stringify(drawnPane.notes));

    // hostile text: the agent's words are text
    const hostile = await q(page, (h) => {
      const b = document.getElementById("planbody");
      return { img: !!document.querySelector("#planpane img"), pwned: window.__pwned, item: [...b.querySelectorAll(".pltext")].some((e) => e.textContent === h),
        note: [...b.querySelectorAll(".plnotetext")].some((e) => e.textContent === h), ansi: [...b.querySelectorAll(".pltext")].some((e) => e.textContent.includes("\x1b[31m")),
        children: [...b.querySelectorAll(".pltext, .plnotetext")].every((e) => !e.children.length) };
    }, HOSTILE);
    check("AC-97 an item or a note in HTML or with control characters stays literal text: no element made, nothing run",
      !hostile.img && hostile.pwned === undefined && hostile.item && hostile.note && hostile.ansi && hostile.children, JSON.stringify(hostile));

    // Done, Reopen, Delete (twice), each sent as {t:note}
    const press = (sel) => page.click(sel);
    from = f.sent.length;
    await press('.plnote[data-id="9"] .placts .primary');
    await press('.plnote[data-id="7"] .placts button:last-child');
    await pause(page, 100);
    const acts = f.sent.slice(from).filter((m) => m.t === "note");
    check("AC-98 Done and Reopen send {t:note, key, id, act}", acts.length === 2 && acts[0].key === A && acts[0].id === 9 && acts[0].act === "done" && acts[1].id === 7 && acts[1].act === "open",
      JSON.stringify(acts));
    from = f.sent.length;
    await press('.plnote[data-id="8"] .del');
    await pause(page, 100);
    const once = f.sent.slice(from).filter((m) => m.t === "note").length, armedText = await q(page, () => document.querySelector('.plnote[data-id="8"] .del').textContent);
    await press('.plnote[data-id="8"] .del');
    await pause(page, 100);
    const del = f.sent.slice(from).filter((m) => m.t === "note");
    check("AC-98 Delete asks again: the first press sends nothing, the second deletes", once === 0 && armedText === "Delete?" && del.length === 1 && del[0].act === "delete" && del[0].id === 8,
      JSON.stringify({ once, armedText, del }));

    // plannew of its key asks again; of another key it does not
    from = f.sent.length;
    f.say({ t: "plannew", key: B });
    f.say({ t: "plannew", key: A });
    await pause(page, 150);
    check("AC-97 {t:plannew} of the shown key makes the open pane ask again (another key's not)", asked(f, from) === 1 && asked(f, from, B) === 0, JSON.stringify(f.sent.slice(from)));

    // phone: finger sizes, full width under the header, no sideways scroll
    if (phone) {
      const ph = await q(page, () => {
        const p = document.getElementById("planpane").getBoundingClientRect(), bar = document.getElementById("bar").getBoundingClientRect();
        const small = [...document.querySelectorAll("#planpane button")].filter((b) => b.offsetParent).map((b) => b.getBoundingClientRect())
          .filter((r) => r.height < 44 || r.width < 44).length;
        const cur = document.getElementById("curplan").getBoundingClientRect();
        return { left: Math.round(p.left), width: Math.round(p.width), top: Math.round(p.top), bar: Math.round(bar.bottom), small, cur: [Math.round(cur.width), Math.round(cur.height)],
          scroll: document.documentElement.scrollWidth - innerWidth };
      });
      check("a phone: the Plan pane is full width just under the header", ph.left === 0 && ph.width === 390 && Math.abs(ph.top - ph.bar) <= 1, JSON.stringify(ph));
      check("a phone: every button of the pane and the header's progress is a finger's size (44px)", ph.small === 0 && ph.cur[0] >= 44 && ph.cur[1] >= 44, JSON.stringify(ph));
      check("a phone: no sideways scroll with the pane open", ph.scroll <= 0, JSON.stringify(ph));
    }

    // Insert into chat: pastes into the pane's input, no Enter, and closes the pane
    from = f.sent.length;
    await press('.plnote[data-id="9"] .tochat');
    await pause(page, 150);
    const pasted = f.sent.slice(from).filter((m) => m.t === "in").map((m) => m.data).join("");
    check("AC-98 Insert into chat pastes the note into the input without Enter and closes the pane",
      pasted.includes("followup: retry loop never resets") && !pasted.includes("\r") && !await paneOpen(page), JSON.stringify({ pasted, sent: f.sent.slice(from) }));

    // a row's percent shows its session and opens the pane; showing another session closes it
    if (phone) { await page.click("#current .t"); await pause(page, 300); }
    from = f.sent.length;
    await page.click(`.item[data-id="${B}"] .plpct`);
    await pause(page, 300);
    check("AC-97 a press on a row's percent shows its session and opens its Plan pane", await paneOpen(page) && asked(f, from, B) >= 1 && asked(f, from, A) === 0,
      JSON.stringify(f.sent.slice(from).filter((m) => m.t !== "in")));
    await q(page, (c) => document.querySelector(`.item[data-id="${c}"]`).click(), C);
    await pause(page, 300);
    const gone = await q(page, () => document.getElementById("curplan").hidden);
    check("AC-97 showing another session closes the pane; the header's progress hides without a plan or notes", !await paneOpen(page) && gone, JSON.stringify({ gone }));

    const scroll = await q(page, () => document.documentElement.scrollWidth - innerWidth);
    check("no sideways page scroll (plans)", scroll <= 0, String(scroll));
    check("no page errors (plans)", !f.errors.length, f.errors.join(" | "));
    await f.close();
  }
  runningIn(name);
}
