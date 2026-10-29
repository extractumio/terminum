// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-35 for e2e_all.mjs: a long path in the output is shown short, "[...]" between its start and
// its end; copying and ⌘-click take it whole; a click on "[...]" shows it whole for good. AC-31:
// a full row naming an image file has nothing after it, no line below it.
import { check } from "./check.mjs";
import { open } from "./e2e_widgets.mjs";

const DIR = "/private/tmp/claude-501/-Users-alex-p-terminum/8247383d-e6a6-4b78-bdcd-1e99c0bd7b52/scratchpad/hdr";
const LONG = `${DIR}/after-top.png`;                 // 109 characters
const FILES = { [DIR]: ["after-top.png"], "/tmp/shots": ["shot.png"] };
const mod = process.platform === "darwin" ? "Meta" : "Control";

export async function webPathFold(browser, name) {
  for (const [size, device] of [["1200×800", { viewport: { width: 1200, height: 800 } }], ["390×844", { viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true }]]) {
    const at = `[${name} ${size}]`;
    const f = await open(browser, { device, cwd: DIR, files: FILES, history: [],
      screen: [`Shots: ${LONG} (the top bar)`, "Short: /tmp/shots/shot.png", ("Saved /tmp/shots/shot.png and the line goes on " + "word ".repeat(10)).slice(0, 80).trimEnd(),
        `Other: ${DIR}/before-top.png`],
      cols: size === "1200×800" ? 80 : null });
    const { page } = f;
    const shown = () => page.evaluate(() => [...document.querySelectorAll("#screen .ln")].map((el) => ({
      text: el.textContent,
      folds: [...el.querySelectorAll(".pfold")].filter((m) => m.getClientRects().length).length,
      hidden: [...el.querySelectorAll(".pmid")].filter((m) => !m.getClientRects().length).map((m) => m.textContent).join(""),
    })));
    const rows = await shown();
    check(`AC-35 ${at} a path of 109 characters shows its first 36 and last 40 with "[...]" between; the row's text stays whole; a short path is not folded`,
      rows[0].folds === 1 && rows[0].hidden === LONG.slice(36, -40) && rows[0].text === `Shots: ${LONG} (the top bar)` && rows[1].folds === 0
        && rows[2].folds === 0, JSON.stringify(rows));

    // copying the row copies the whole path
    const copied = await page.evaluate(() => {
      const el = document.querySelector("#screen .ln");
      getSelection().selectAllChildren(el);
      const dt = new DataTransfer();
      document.dispatchEvent(new ClipboardEvent("copy", { clipboardData: dt, bubbles: true, cancelable: true }));
      getSelection().removeAllRanges();
      return dt.getData("text/plain");
    });
    check(`AC-35 ${at} a copy of the row has the whole path`, copied.includes(LONG), JSON.stringify(copied));

    // ⌘-click on its shown end opens the whole path
    if (size === "1200×800") {
      // once its file is known to be there (underlined): only then a path opens
      await page.waitForFunction((p) => [...(CSS.highlights?.get("lnk") ?? [])].some((r) => {
        const x = new Range(); x.setStart(r.startContainer, r.startOffset); x.setEnd(r.endContainer, r.endOffset); return x.toString() === p;
      }), LONG, { timeout: 5000 }).catch(() => {});
      const end = await page.evaluate(() => {
        const r = new Range(), t = [...document.querySelector("#screen .ln").querySelectorAll(".pmid")].at(-1).nextSibling;
        r.setStart(t, 5); r.setEnd(t, 6);
        const b = r.getBoundingClientRect();
        return { x: b.left + b.width / 2, y: b.top + b.height / 2 };
      });
      await page.keyboard.down(mod);
      await page.mouse.click(end.x, end.y);
      await page.keyboard.up(mod);
      const src = await page.waitForFunction(() => decodeURIComponent(document.getElementById("file").getAttribute("src") ?? "").includes("after-top.png")
        && document.getElementById("file").getAttribute("src"), null, { timeout: 3000 }).then((h) => h.jsonValue().then(decodeURIComponent), () => "not opened");
      check(`AC-35 ${at} ⌘-click on the shown end of a folded path opens the whole path`, src.includes(LONG), src);
      await page.click('button[data-view="term"]').catch(() => {});
      await page.waitForFunction(() => document.body.dataset.shown === "term", null, { timeout: 3000 }).catch(() => {});
    }

    // a click (a tap on a phone) on "[...]" shows it whole, types nothing, and so it stays when the row is drawn again
    const before = f.sent.filter((m) => m.t === "in").length;
    const box = await page.evaluate(() => { const b = document.querySelector("#screen .pfold").getBoundingClientRect(); return { x: b.left + b.width / 2, y: b.top + b.height / 2 }; });
    if (size === "390×844") await page.touchscreen.tap(box.x, box.y); else await page.mouse.click(box.x, box.y);
    await page.waitForTimeout(200);
    const opened = await shown();
    f.say({ t: "screen", sid: "S1", n: 4, cols: 80, rows: 4, ch: [[0, { r: [[`Shots: ${LONG} (the top bar, again)`, null, null, 0]], e: true }]] });
    await page.waitForFunction(() => document.querySelector("#screen .ln").textContent.includes("again"));
    const again = await shown();
    check(`AC-35 ${at} a click on "[...]" shows the path whole, types nothing into the pane, and it stays whole when its row is drawn again`,
      opened[0].folds === 0 && opened[0].hidden === "" && again[0].folds === 0 && again[0].hidden === "" && f.sent.filter((m) => m.t === "in").length === before,
      JSON.stringify({ opened: opened[0], again: again[0] }));

    // AC-31 0.6.11: a row as wide as the view naming an image file that is there: nothing after it, no line below it
    if (size === "1200×800") {
      const onSaved = () => [...(CSS.highlights?.get("lnk") ?? [])].some((r) => r.startContainer.parentElement?.closest(".ln")?.textContent.startsWith("Saved"));
      await page.waitForFunction(onSaved, null, { timeout: 8000 }).catch(() => {});   // its file found: when a button would have come
      await page.waitForTimeout(300);
      const saved = await page.evaluate(() => {
        const rows = [...document.querySelectorAll("#screen .ln")], row = rows.find((el) => el.textContent.startsWith("Saved"));
        const found = [...(CSS.highlights?.get("lnk") ?? [])].some((r) => r.startContainer.parentElement?.closest(".ln") === row);
        return row && { found, shadow: !!row.shadowRoot, added: rows.filter((el) => el.shadowRoot && !el.classList.contains("wg-host") && !el.classList.contains("wg-fold")).length,
          h: Math.round(row.getBoundingClientRect().height), h0: Math.round(rows[1].getBoundingClientRect().height) };
      });
      check(`AC-31 ${at} a row as wide as the view naming an image file has nothing after it and no line below it`,
        !!saved && saved.found && !saved.shadow && saved.added === 0 && saved.h === saved.h0, JSON.stringify(saved));
    }
    // a found text in a folded path's hidden middle shows that path whole (find.js marks it through unfoldAt)
    const found = await page.evaluate(async () => {
      const { unfoldAt } = await import("/pathfold.js");
      const row = [...document.querySelectorAll("#screen .ln")].find((l) => l.textContent.startsWith("Other:")), mid = row?.querySelector(".pmid");
      if (!mid) return null;
      const r = new Range(); r.setStart(mid.firstChild, 2); r.setEnd(mid.firstChild, 6);
      unfoldAt(r);
      return { open: mid.classList.contains("open"), mark: row.querySelector(".pfold").classList.contains("open") };
    });
    check(`AC-35 ${at} a found text in a path's hidden middle shows the path whole`, found?.open && found?.mark, JSON.stringify(found));
    check(`AC-35 ${at} no page errors`, f.errors.length === 0, f.errors.join(" | "));
    await page.close();
  }
}
