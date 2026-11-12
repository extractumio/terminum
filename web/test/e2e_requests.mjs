// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// TERM-37: AC-02, AC-07, AC-09, AC-33, AC-37, AC-88, AC-90 request loss and stale replies in the real page.
import { check, runningIn, until } from "./check.mjs";
import { drawn, group, item, line, openPage, pause, pressFab, pressFoot, SIZES, THEME } from "./fake.mjs";

const SID = "tmux:local:terminum:%1", OTHER = "tmux:local:terminum:%2", QUICK = "tmux:local:terminum:%3";
const layout = [group("main", [item(SID, "one"), item(OTHER, "two"), item(QUICK, "quick")])];
const mark = { id: "A", sid: SID, n: 1001, text: "row 1001", note: "Retry me", title: "one", at: 1 };

export async function webRequests(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    let online = true, deleted = false, restored = false;
    const f = await openPage(browser, { device, layout, available: () => online,
      onSub: (m, say) => {
        say({ t: "theme", sid: m.id, theme: THEME });
        say({ t: "hist", mode: "reset", sid: m.id, first: 1000, oldest: 0, truncated: false,
          lines: Array.from({ length: 100 }, (_, i) => line(`row ${1000 + i}`)) });
        say({ t: "screen", sid: m.id, full: true, n: 1, cols: 80, rows: 1, ch: [[0, line("ready")]] });
        return true;
      }, hook: (m, say) => {
        if (m.t === "sub") say({ t: "marks", items: deleted ? [] : [mark] });
        if (m.t === "unmark" && m.id === "B") {
          if (restored) say({ t: "error", msg: "Delete refused after restoration" });
          return;
        }
        if (m.t === "unmark") { deleted = true; say({ t: "unmark", id: m.id }); say({ t: "markdone", id: m.id }); }
      } });
    const { page, sent } = f;
    const last = (t) => sent.filter((m) => m.t === t).at(-1);
    const count = (t) => sent.filter((m) => m.t === t).length;
    await drawn(page);
    await page.clock.install();
    const scroll = () => page.evaluate(() => { const t = document.getElementById("term"); t.scrollTop = 0; t.dispatchEvent(new Event("scroll")); });
    await scroll();
    await until(() => count("more") > 0);
    const lost = last("more"), n = count("more");
    await page.clock.fastForward(15001);
    check("AC-02 a dropped history reply releases scrolling and says why", (await page.textContent("#toast")).includes("did not answer"));
    await scroll(); await until(() => count("more") > n);
    const retry = last("more");
    f.say({ t: "error", ref: lost.ref, sid: SID, msg: "STALE HISTORY ERROR" });
    f.say({ t: "hist", ref: retry.ref, sid: SID, mode: "prepend", first: 999, oldest: 0, truncated: false, lines: [line("recovered history")] });
    check("AC-02 another scroll retries with a new identity and draws its lines", retry.ref !== lost.ref && await until(() => page.locator("#hist").textContent().then((s) => s.includes("recovered history"))));
    check("AC-02 an earlier error does not answer the retry", !(await page.textContent("body")).includes("STALE HISTORY ERROR"));

    // A cancelled form and a newer form must never share errors or completions.
    if (size.startsWith("390")) { await page.click("#navbtn"); await pause(page); }
    await pressFoot(page, "#newsessbtn");
    await page.fill("#newsessname", "first");
    await page.locator("#newsess").evaluate((e) => { e.requestSubmit(); e.requestSubmit(); });
    await until(() => count("newsession") > 0);
    const oldForm = last("newsession");
    check("AC-07 a double submit creates one request", count("newsession") === 1);
    await page.click("#newsesscancel");
    await pressFoot(page, "#addhostbtn");
    await page.fill("#addhostssh", "devbox.example");
    await page.locator("#addhost").evaluate((e) => e.requestSubmit());
    await until(() => count("addhost") > 0);
    const host = last("addhost");
    f.say({ t: "error", ref: oldForm.ref, msg: "OLD FORM ERROR" });
    f.say({ t: "error", msg: "Unrelated pane error" });
    await pause(page);
    check("AC-07 old and unrelated errors never fill the current form", !(await page.textContent("#addhosterr")).includes("ERROR") && !(await page.textContent("#addhosterr")).includes("Unrelated"));
    f.say({ t: "error", ref: host.ref, msg: "Host was refused" });
    check("AC-07 the matching error releases its own form", await until(() => page.textContent("#addhosterr").then((s) => s === "Host was refused")));
    f.say({ t: "created", ref: oldForm.ref, id: QUICK, stay: false });
    await pause(page);
    check("AC-07 a cancelled creation cannot navigate later", last("sub").id === SID);
    await page.click("#addhostcancel");
    await pressFoot(page, "#newsessbtn");
    await page.fill("#newsessname", "timeout");
    await page.locator("#newsess").evaluate((e) => e.requestSubmit());
    await page.clock.fastForward(15001);
    check("AC-07 a missing reply releases the form with an inline timeout", (await page.textContent("#newsesserr")).includes("did not answer") && !await page.locator('#newsess [type="submit"]').isDisabled());
    await page.click("#newsesscancel");
    if (size.startsWith("390")) { await page.click("#drawerclose"); await pause(page); }

    f.say({ t: "error", msg: "" });
    await pressFab(page, "#sentfab");
    await until(() => count("sentget") > 0);
    const firstSent = last("sentget");
    await page.evaluate(() => document.getElementById("spolder").dispatchEvent(new MouseEvent("click")));
    await until(() => count("sentget") > 1);
    const secondSent = last("sentget");
    f.say({ t: "sentone", ref: secondSent.ref, sid: SID, n: secondSent.n, total: 3, item: { text: "current entry", at: 1, agent: "claude" } });
    f.say({ t: "sentone", ref: firstSent.ref, error: "OLD SENT ERROR" });
    check("AC-90 a stale sent-history error cannot replace the newer entry", await until(() => page.textContent("#spbody").then((s) => s.includes("current entry") && !s.includes("OLD SENT"))));
    await page.click("#spclose");

    await page.locator("#splitfab").evaluate((el) => el.click());
    await until(() => count("quick") > 0);
    const quick = last("quick");
    await page.locator(`#sessions .item[data-id="${OTHER}"]`).evaluate((el) => el.click());
    await until(() => last("sub").id === OTHER);
    f.say({ t: "quickmade", ref: quick.ref, id: QUICK, for: SID });
    await pause(page);
    check("AC-88 a quick-shell completion cannot navigate after a pane change", last("sub").id === OTHER);
    await page.locator(`#sessions .item[data-id="${SID}"]`).evaluate((el) => el.click());
    await until(() => last("sub").id === SID);
    const zoom = await page.evaluate(async () => {
      const z = await import("./zoom.js");
      z.openZoom("/old-image.png"); const old = document.querySelector("#zoom img");
      z.close();
      try { old.onload(); } catch { return false; }
      z.openZoom("/new-image.png"); const current = document.querySelector("#zoom img"), before = current.style.transform;
      old.onload(); const safe = before === current.style.transform;
      z.close(); return safe;
    });
    check("AC-32 image loads after Close neither throw nor fit a newer image", zoom);

    await page.locator("#findfab").evaluate((el) => el.click());
    await page.fill("#findq", "old search");
    await page.locator("#findbox").evaluate((el) => el.requestSubmit());
    await until(() => !!last("find"));
    const search = last("find");
    await page.clock.fastForward(30001);
    f.say({ t: "found", ref: search.ref, sid: SID, hits: [[1001, 0]], total: 1 });
    await pause(page);
    check("AC-34 Find ignores a same-ref answer after its timeout", (await page.textContent("#findn")).includes("did not answer"));
    await page.locator("#findclose").evaluate((el) => el.click());

    // A delete whose Undo expires offline keeps its intent and retries after a new connection.
    await pressFab(page, "#marksfab");
    await page.locator("#markpane").press("Delete");
    online = false; f.sockets.at(-1).close();
    await pause(page);
    await page.clock.fastForward(5001);
    check("AC-33 offline Undo expiry keeps the action and says it is waiting", !deleted && (await page.textContent("#markpane")).includes("Waiting for connection"));
    online = true;
    await page.clock.fastForward(2500);
    check("AC-33 reconnect carries out the delayed delete once", await until(() => deleted) && count("unmark") === 1);
    f.say({ t: "marks", items: [{ ...mark, id: "B" }] });
    await pause(page);
    if (await page.isHidden("#markpane")) await page.locator("#marksfab").evaluate((el) => el.click());
    await page.locator("#markpane").press("Delete");
    await page.evaluate(() => dispatchEvent(new PageTransitionEvent("pagehide", { persisted: true })));
    restored = true;
    await page.evaluate(() => dispatchEvent(new PageTransitionEvent("pageshow", { persisted: true })));
    check("AC-33 restoration clears Undo and shows a matching failure inline without a mark broadcast",
      await until(() => page.textContent("#markpane").then((s) => s.includes("Delete refused after restoration"))) && await page.locator("#markpane .mkundo").count() === 0);
    check("TERM-37 request regressions raise no browser exceptions", f.errors.length === 0, f.errors.join(" | "));
    await f.close();
  }
}
