// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Checks of sizing (AC-13), Screen mode (AC-05) and the mouse in it (AC-06) against a fake
// terminumd (fake.mjs), at a wide and a phone size: the size messages and when they go, the
// switch on the alternate screen and the menu's choice per pane, too small a view, a pane
// sized for another screen, and the mouse reports of clicks, drags, the wheel and taps.

import { check, runningIn } from "./check.mjs";
import { drawn, LAYOUT, line, openPage, pause, SIZES } from "./fake.mjs";

const SID = "tmux:local:terminum:%1";
const ROWS = Array.from({ length: 6 }, (_, i) => line(`${i}123456789012345678901234567890`));

export async function webScreen(browser, name) {
  for (const [size, device] of SIZES) {
    runningIn(`${name} ${size}`);
    const phone = size.startsWith("390");
    const f = await openPage(browser, { device });
    const { page, sent } = f;
    await drawn(page);
    await pause(page, 400);
    const sizes = () => sent.filter((m) => m.t === "size");
    const frame = (extra) => f.say({ t: "screen", sid: SID, full: true, n: ROWS.length, cols: 80, rows: ROWS.length, alt: false, ch: ROWS.map((l, i) => [i, l]), ...extra });
    // the grid at the chosen text size, measured apart from the page's own code
    const fits = () => page.evaluate(() => {
      const t = document.getElementById("term"), cs = getComputedStyle(t), px = parseFloat(cs.getPropertyValue("--size"));
      const probe = document.createElement("span");
      probe.textContent = "0".repeat(100); probe.style.cssText = `position:absolute;visibility:hidden;white-space:pre;font-size:${px}px`;
      t.append(probe); const cell = probe.getBoundingClientRect().width / 100; probe.remove();
      const w = t.clientWidth - parseFloat(cs.paddingLeft) - parseFloat(cs.paddingRight), h = t.clientHeight - parseFloat(cs.paddingTop) - parseFloat(cs.paddingBottom);
      return { cols: Math.floor(w / cell), rows: Math.floor(h / (px * 1.25)) };
    });

    const subAt = sent.findIndex((m) => m.t === "sub"), first = sizes()[0], want = await fits();
    check("AC-13 size: after opening a pane the page sends the grid that fits its terminal at the chosen text size",
      sent.indexOf(first) > subAt && first.typing === false && first.screen === false && Math.abs(first.cols - want.cols) <= 1
        && Math.abs(first.rows - want.rows) <= 1, JSON.stringify({ first, want }));
    const n0 = sizes().length;
    await page.evaluate(() => document.getElementById("kbd").focus());
    await page.keyboard.type("ab");
    await pause(page);
    const typing = sizes().slice(n0);
    check("AC-13 size: the first key after a pause sends the size as typing, once a second at most",
      typing.length === 1 && typing[0].typing === true, JSON.stringify(typing));
    await page.click("#menubtn");
    await page.click("#bigger");
    await page.click("#bigger");
    await page.keyboard.press("Escape");
    await pause(page, 150);
    const early = sizes().length;
    await pause(page, 400);
    const bigger = sizes().at(-1);
    check("AC-13 size: a bigger text size sends the smaller grid, 300 ms after the last change",
      early === n0 + 1 && sizes().length === n0 + 2 && bigger.cols < first.cols && bigger.typing === false, JSON.stringify({ early, bigger }));

    // Screen mode on the alternate screen, and back
    frame({ alt: true });
    await pause(page, 200);
    frame({ alt: true, cols: sizes().at(-1).cols });   // terminumd sized the pane to this view
    await pause(page, 200);
    const on = await page.evaluate(() => ({ body: document.body.classList.contains("screenmode"), label: document.getElementById("scrlabel").textContent,
      labelShown: !document.getElementById("scrlabel").hidden, hist: getComputedStyle(document.getElementById("hist")).display,
      lines: [...document.querySelectorAll(".lines-mode")].every((e) => getComputedStyle(e).display === "none"),
      grid: document.getElementById("term").classList.contains("grid") }));
    const onSize = sizes().at(-1);
    frame({ alt: false });
    await pause(page, 200);
    const off = await page.evaluate(() => ({ body: document.body.classList.contains("screenmode"), wrap: document.getElementById("term").classList.contains("wrap"),
      label: document.getElementById("scrlabel").hidden }));
    check("AC-05 Screen mode: the alternate screen switches it on (grid, no history, no Lines buttons, \"Screen · claude\"), sized with screen: true",
      on.body && on.labelShown && on.label === "Screen · claude" && on.hist === "none" && on.lines && on.grid && onSize.screen === true, JSON.stringify({ on, onSize }));
    check("AC-05 Screen mode: off the alternate screen the Lines mode it had is back, sized with screen: false",
      !off.body && off.wrap && off.label && sizes().at(-1).screen === false, JSON.stringify(off));

    // the menu's choice, remembered per pane
    await page.click("#menubtn");
    await page.click('#menu [data-screen="always"]');
    await page.keyboard.press("Escape");
    await pause(page);
    const always = { msg: sent.filter((m) => m.t === "screenmode").at(-1), on: await page.evaluate(() => document.body.classList.contains("screenmode")) };
    await page.evaluate(() => document.querySelector('#sessions .item[data-id="tmux:local:terminum:%2"]').click());
    await pause(page, 300);
    const other = await page.evaluate(() => document.body.classList.contains("screenmode"));
    await page.evaluate((id) => document.querySelector(`#sessions .item[data-id="${id}"]`).click(), SID);
    await pause(page, 300);
    const back = { on: await page.evaluate(() => document.body.classList.contains("screenmode")), msg: sent.filter((m) => m.t === "screenmode").at(-1) };
    check("AC-05 Screen mode: \"Always\" in the menu turns it on for that pane only, and is remembered for it",
      always.msg?.mode === "always" && always.on && !other && back.on && back.msg.mode === "always", JSON.stringify({ always, other, back }));

    // the mouse in Screen mode
    frame({ alt: true, mouse: true });
    await pause(page, 200);
    const at = (x, y) => page.evaluate(([x, y]) => {
      const t = document.querySelectorAll("#screen .ln")[y].querySelector("span").firstChild, r = new Range();
      r.setStart(t, x); r.setEnd(t, x + 1);
      const b = r.getBoundingClientRect();
      return { x: b.left + b.width / 2, y: b.top + b.height / 2 };
    }, [x, y]);
    const mice = () => sent.filter((m) => m.t === "mouse").map((m) => `${m.action}:${m.button}@${m.x},${m.y}` + (m.count ? `x${m.count}` : ""));
    const stepsIn = (list) => list.reduce((n, e) => n + Number(e.match(/x(\d+)$/)?.[1] ?? 1), 0);
    const n1 = mice().length;
    if (!phone) {
      const a = await at(10, 1), b = await at(14, 1);
      await page.mouse.click(a.x, a.y);
      await page.mouse.move(a.x, a.y);
      await page.mouse.down();
      await page.mouse.move(b.x, b.y, { steps: 3 });
      await page.mouse.up();
      await page.mouse.click(a.x, a.y, { button: "right" });
      await page.mouse.move(a.x, a.y);
      await page.mouse.wheel(0, -120);
      await page.mouse.wheel(0, 120);
      await pause(page, 200);
      const got = mice().slice(n1);
      const shift = sent.filter((m) => m.t === "mouse")[n1];
      check("AC-06 mouse: a click, a drag, a right click and the wheel go to the program with 0-based cells",
        got.filter((e) => !e.startsWith("move")).join() === ["press:0@10,1", "release:0@10,1", "press:0@10,1", "release:0@14,1", "press:2@10,1",
          "release:2@10,1", "wheelup:0@10,1", "wheeldown:0@10,1"].join() && got.includes("move:0@14,1") && shift?.mods?.shift === false, JSON.stringify(got));
      // a trackpad: 24 small events, 120 px in all, are a step a text line, sent as one message
      const n2 = mice().length;
      const lines = await page.evaluate(([x, y]) => {
        const term = document.getElementById("term");
        for (let i = 0; i < 24; i++) term.dispatchEvent(new WheelEvent("wheel", { deltaY: 5, deltaMode: 0, clientX: x, clientY: y, bubbles: true, cancelable: true }));
        return Math.floor(120 / parseFloat(getComputedStyle(term).lineHeight));
      }, [a.x, a.y]);
      await pause(page, 200);
      const pad = mice().slice(n2);
      check("AC-06 mouse: a trackpad's small wheel events are a step a text line, a frame's steps one message with their count",
        pad.length === 1 && pad[0].startsWith("wheeldown:0@10,1") && stepsIn(pad) === lines, JSON.stringify({ pad, lines }));
    } else {
      const a = await at(3, 2);
      await page.touchscreen.tap(a.x, a.y);
      await pause(page, 200);
      // a two-finger tap, a swipe and a long press, as touch events (WebKit makes no Touch objects)
      const touches = (steps) => page.evaluate(async (steps) => {
        const term = document.getElementById("term");
        for (const [type, pts, wait] of steps) {
          const list = pts.map(([x, y]) => ({ clientX: x, clientY: y, target: term }));
          const e = new Event(type, { bubbles: true, cancelable: true });
          Object.defineProperties(e, { touches: { value: type === "touchend" ? [] : list }, changedTouches: { value: list } });
          term.dispatchEvent(e);
          if (wait) await new Promise((r) => setTimeout(r, wait));
        }
      }, steps);
      const p = [a.x, a.y], q = [a.x + 30, a.y];
      await touches([["touchstart", [p]], ["touchstart", [p, q]], ["touchend", [], 50]]);
      await touches([["touchstart", [p]], ["touchmove", [[a.x, a.y - 40]]], ["touchmove", [[a.x, a.y - 80]]], ["touchend", []]]);
      await touches([["touchstart", [p], 600], ["touchend", []]]);
      await pause(page, 200);
      const got = mice().slice(n1);
      check("AC-06 touch: a tap is a click, a two-finger tap a right click, a swipe the wheel, a long press nothing",
        got.slice(0, 4).join() === "press:0@3,2,release:0@3,2,press:2@3,2,release:2@3,2" && stepsIn(got.slice(4)) >= 2
          && got.slice(4).every((e) => e.startsWith("wheeldown:0@3,2")), JSON.stringify(got));
    }
    frame({ alt: true, mouse: false });
    await pause(page);
    const n2 = mice().length, c = await at(5, 0);
    await page.mouse.click(c.x, c.y);
    await pause(page);
    check("AC-06 mouse: a program that did not ask for the mouse gets no events", mice().length === n2);

    // a pane wider than the view: waiting for this view's size, Fit; one this page does not size
    // (a session of the user's own tmux only looked at, AC-89): text re-flows at the chosen size,
    // a frame is Fit, or Grid past 7 px, and stays one while a third of its rows still reach past
    const wide = (cols, txt) => frame({ alt: true, mouse: false, cols, ch: ROWS.map((_, i) => [i, line(txt(i, cols))]) });
    const shown = () => page.evaluate(() => {
      const t = document.getElementById("term"), cs = getComputedStyle(t);
      return { wrap: t.classList.contains("wrap"), px: parseFloat(cs.fontSize), size: parseFloat(cs.getPropertyValue("--size")), sideways: t.scrollWidth > t.clientWidth + 1 };
    });
    const look = async (cols, txt) => { wide(cols, txt); await pause(page, 200); return shown(); };
    const text = (i) => `• Ran cat plugin/src/vpatch-frozen.ts ${i}`, box = (i, cols) => `│${String(i).padEnd(cols - 2, "─")}│`;
    const waiting = await look(200, text);
    f.say({ t: "layout", groups: [{ ...LAYOUT[0], attached: "default" }, ...LAYOUT.slice(1)] });
    await pause(page);
    const reflowed = await look(210, text), framed = await look(120, box);
    const third = await look(120, (i, cols) => (i < 2 ? box(i, cols) : text(i))), less = await look(120, (i, cols) => (i < 1 ? box(i, cols) : text(i)));
    const huge = await look(600, box);
    check("AC-05 Screen mode: a wider pane waiting for this view's size is Fit; one it does not size shows text re-flowed at the chosen size, a frame as Fit (Grid under 7 px)",
      !waiting.wrap && reflowed.wrap && reflowed.px === reflowed.size && !framed.wrap
        && (phone ? framed.px === framed.size && framed.sideways : framed.px < framed.size && framed.px >= 7)
        && !huge.wrap && huge.px === huge.size && huge.sideways, JSON.stringify({ waiting, reflowed, framed, huge }));
    check("AC-05 Screen mode: a frame stays one while a third of its rows reach past the view, and re-flows under that",
      !third.wrap && less.wrap, JSON.stringify({ third, less }));
    f.say({ t: "layout", groups: LAYOUT });
    frame({ alt: true, mouse: false });

    // sized for another screen: said, and drawn as a wider pane, until this page types
    f.say({ t: "sized", sid: SID, cols: 180, rows: 50, mine: false });
    await pause(page);
    const sized = await page.evaluate(() => ({ text: document.getElementById("sized").textContent, shown: !document.getElementById("sized").hidden,
      wrap: document.getElementById("term").classList.contains("wrap") }));
    await page.evaluate(() => document.getElementById("kbd").focus());
    await page.keyboard.type("x");
    await pause(page);
    check("AC-13 sized: another screen's size is said, its text re-flowed, until this page types",
      sized.shown && sized.text === "Sized for another screen (180×50)" && sized.wrap && await page.isHidden("#sized"), JSON.stringify(sized));
    f.say({ t: "sized", sid: SID, cols: 180, rows: 50, mine: true, error: "size too small" });
    await pause(page);
    const refused = await page.evaluate(() => !document.getElementById("sized").hidden && document.getElementById("sized").textContent);
    f.say({ t: "sized", sid: SID, cols: 40, rows: 20, mine: true });
    await pause(page);
    check("AC-13 sized: a refused resize is said with tmux's reason; this page's own size clears it",
      refused === "Sized for another screen (180×50): size too small" && await page.isHidden("#sized"), JSON.stringify(refused));

    // too small for Screen mode: the last size stays
    const n3 = sizes().length;
    await page.setViewportSize({ width: 300, height: device.viewport.height });
    await page.click("#menubtn");
    for (let i = 0; i < 16; i++) await page.click("#bigger");
    await page.keyboard.press("Escape");
    await pause(page, 600);
    const small = await page.evaluate(() => document.getElementById("scrlabel").textContent);
    check("AC-05 Screen mode: a view under 20×5 says \"Too small for Screen mode\" and keeps the last size",
      small === "Too small for Screen mode" && sizes().length === n3, JSON.stringify({ small, more: sizes().slice(n3) }));
    check("AC-05 Screen mode: no page errors", !f.errors.length, f.errors.join(" | "));
    await f.close();
  }
  runningIn(name);
}
