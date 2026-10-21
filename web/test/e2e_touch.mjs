// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-52 checks for e2e_panel.mjs: typing on a touch screen (the page against a fake bridge, a
// phone's size): a held delete has something to delete (iOS repeats it only then), the keyboard
// field lies over a coding agent's whole input box, a finger that rests and then slides moves the
// cursor, and one held still 2.5 s there opens Paste, Undo and Redo. Alone: node web/test/e2e_touch.mjs.

import { fileURLToPath } from "node:url";
import { check, cleanup, playwright, results } from "./check.mjs";
import { open } from "./e2e_widgets.mjs";

const line = (text, extra = {}) => ({ r: [[text, null, null, 0]], e: true, ...extra });
const RULE = "─".repeat(60);

export async function webTouch(browser) {
  const at = "AC-52 web app on a touch screen:";
  const { page, sent, say } = await open(browser, { agent: true, history: [line("$ claude"), ...Array.from({ length: 80 }, (_, i) => line(`⏺ step ${i} of the agent's work`))], screen: ["…"],
    device: { viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true } });
  // a coding agent's screen: its answer, then its input box between two rules, the cursor in it
  const rows = [line("⏺ Done."), line(""), line(RULE), { r: [["> fix the ", null, null, 0], [" ", null, null, 256]], e: true }, line(RULE), line("  ? for shortcuts")];
  say({ t: "screen", sid: "S1", full: true, n: rows.length, cols: 60, rows: rows.length, ch: rows.map((r, i) => [i, r]) });
  await page.waitForFunction(() => document.querySelector("#screen .blk.inbox"), null, { timeout: 5000 }).catch(() => {});
  await page.waitForTimeout(200);
  const cover = await page.evaluate(() => {
    const k = document.getElementById("kbd").getBoundingClientRect(), b = document.querySelector("#screen .blk.inbox").getBoundingClientRect();
    return { k: [Math.round(k.top), Math.round(k.bottom)], box: [Math.round(b.top), Math.round(b.bottom)] };
  });
  check(`${at} the keyboard field lies over the coding agent's whole input box (a finger held there gets Paste, Undo and Redo)`,
    cover.k[0] === cover.box[0] && cover.k[1] === cover.box[1], JSON.stringify(cover));

  // only a tap on the input opens the keyboard: one on the answer above it leaves it closed
  const tapAt = async (sel) => {
    const r = await page.evaluate((s) => { const b = document.querySelector(s).getBoundingClientRect(); return { x: b.left + 40, y: b.top + b.height / 2 }; }, sel);
    await page.touchscreen.tap(r.x, r.y);
    await page.waitForTimeout(100);
    return page.evaluate(() => document.activeElement?.id === "kbd");
  };
  const offInput = await tapAt("#screen .ln"), onInput = await tapAt("#kbd");
  check(`${at} a tap on the terminal outside the input leaves the keyboard closed; a tap on the input opens it`, !offInput && onInput, JSON.stringify({ offInput, onInput }));
  await page.evaluate(() => document.getElementById("kbd").blur());

  const shape = await page.evaluate(() => { const cs = getComputedStyle(document.getElementById("kbdbtn")); return [cs.borderTopLeftRadius, cs.borderTopRightRadius]; });
  check(`${at} the floating keyboard buttons are nearly round blobs, not circles`, shape[0] !== shape[1] && !shape.includes("50%"), JSON.stringify(shape));
  // every other button a little uneven: bowed edges (neighbors differ), a border as a ring of that shape, a round one a blob
  const uneven = await page.evaluate(() => {
    const cs = (el, p) => getComputedStyle(el, p), tabs = [...document.querySelectorAll("#views button")];
    const key = document.querySelector("#hotkeys .key"), down = cs(document.getElementById("bottom"));
    return { tabs: tabs.map((b) => cs(b).clipPath.startsWith("shape(")), differ: cs(tabs[0]).clipPath !== cs(tabs[1]).clipPath,
      save: cs(document.querySelector("#markbox .primary")).clipPath.startsWith("shape("),
      key: [cs(key).borderTopWidth, cs(key, "::after").backgroundColor, cs(key, "::after").clipPath.startsWith("shape(evenodd")],
      down: [down.borderTopLeftRadius, down.borderTopRightRadius, down.clipPath] };
  });
  check(`${at} buttons are a little uneven: the tabs' and the bookmark dialog's bowed (neighbors differently), a hot key a fill of that shape with no ring (the design code: no outlines), Scroll to the bottom a blob`,
    uneven.tabs.every(Boolean) && uneven.differ && uneven.save && uneven.key[0] === "0px" && /^rgba\(0, 0, 0, 0\)$|transparent/.test(uneven.key[1]) && uneven.key[2]
    && uneven.down[0] !== uneven.down[1] && !uneven.down.includes("50%") && uneven.down[2] === "none", JSON.stringify(uneven));
  await page.evaluate(() => document.getElementById("kbd").focus());
  const kept = await page.evaluate(() => document.getElementById("kbd").value);
  const before = sent.length;
  for (let i = 0; i < 3; i++) await page.keyboard.press("Backspace");
  await page.keyboard.type("ab");
  await page.waitForTimeout(100);
  const typed = sent.slice(before).filter((m) => m.t === "in").map((m) => m.data);
  const keptWord = await page.evaluate(() => document.getElementById("kbd").value);
  check(`${at} the field keeps characters to delete on both sides of its caret, each delete is one backspace, typing is as before, the typed word kept for the keyboard`,
    kept === "\u200b".repeat(32) + "\u200b".repeat(32) && JSON.stringify(typed) === JSON.stringify(["\x7f", "\x7f", "\x7f", "a", "b"])
    && keptWord === "\u200b".repeat(32) + "ab" + "\u200b".repeat(32), JSON.stringify({ kept, typed, keptWord }));

  // a kept letter deleted ("ab" → "a"), then iOS's correction of the word to "the": the pane gets the
  // difference (a backspace and "the"); a line end empties the field
  const fix = sent.length;
  await page.keyboard.press("Backspace");
  await page.evaluate((pad) => {
    const k = document.getElementById("kbd");
    k.value = pad + "the" + pad; k.setSelectionRange(pad.length + 3, pad.length + 3);
    k.dispatchEvent(new InputEvent("input", { inputType: "insertReplacementText", data: "the", bubbles: true }));
  }, "\u200b".repeat(32));
  await page.keyboard.press("Enter");
  await page.waitForTimeout(100);
  const fixed = sent.slice(fix).filter((m) => m.t === "in").map((m) => m.data);
  const emptied = await page.evaluate(() => document.getElementById("kbd").value);
  check(`${at} a kept letter's delete is one backspace; iOS's correction of the word sends the difference; Enter empties the field`,
    JSON.stringify(fixed) === JSON.stringify(["\x7f", "\x7fthe", "\r"]) && emptied === "\u200b".repeat(64), JSON.stringify({ fixed, emptied: emptied.length }));

  // a key of the bar over the keyboard (Tab: the shell completes) drops what was kept: iOS's later
  // correction of the word sends no backspace for letters before the cursor's move; an emoji is one character
  const tab = sent.length;
  await page.keyboard.type("gi");
  await page.evaluate(() => [...document.querySelectorAll("#hotkeys button.key")].find((b) => b.dataset.action === "\t").dispatchEvent(new MouseEvent("click", { bubbles: true })));
  await page.evaluate((pad) => {
    const k = document.getElementById("kbd");
    k.value = pad + "his" + pad; k.setSelectionRange(pad.length + 3, pad.length + 3);
    k.dispatchEvent(new InputEvent("input", { inputType: "insertReplacementText", data: "his", bubbles: true }));
    k.value = pad + "his\u{1F600}" + pad; k.setSelectionRange(pad.length + 5, pad.length + 5);
    k.dispatchEvent(new InputEvent("input", { inputType: "insertText", data: "\u{1F600}", bubbles: true }));
    k.value = pad + "his" + pad; k.setSelectionRange(pad.length + 3, pad.length + 3);
    k.dispatchEvent(new InputEvent("input", { inputType: "deleteContentBackward", bubbles: true }));
  }, "\u200b".repeat(32));
  await page.waitForTimeout(100);
  const barred = sent.slice(tab).filter((m) => m.t === "in").map((m) => m.data);
  check(`${at} a key of the bar drops what the field kept (no backspace for letters before the cursor moved); an emoji typed and deleted is one character`,
    JSON.stringify(barred) === JSON.stringify(["g", "i", "\t", "his", "\u{1F600}", "\x7f"]), JSON.stringify(barred));

  // the space bar's trackpad moves the caret: as many arrows; a jump to an end (an up or down move) sends none
  await page.evaluate(() => { const k = document.getElementById("kbd"); k.blur(); k.focus(); });   // focused again: an empty field
  await page.waitForTimeout(700);                                // a caret moved this soon after a finger on the field is the finger's
  const pad = sent.length;
  await page.evaluate((n) => { const k = document.getElementById("kbd"); k.setSelectionRange(n - 2, n - 2); }, 32);
  await page.waitForTimeout(50);
  await page.evaluate((n) => { const k = document.getElementById("kbd"); k.setSelectionRange(n + 1, n + 1); }, 32);
  await page.waitForTimeout(50);
  await page.evaluate(() => document.getElementById("kbd").setSelectionRange(0, 0));
  await page.waitForTimeout(50);
  const moved = sent.slice(pad).filter((m) => m.t === "in").map((m) => m.data);
  check(`${at} the space bar's trackpad: the caret's moves are sent as ← and →, a jump to the field's end as nothing`,
    JSON.stringify(moved) === JSON.stringify(["\x1b[D\x1b[D", "\x1b[C"]), JSON.stringify(moved));

  // the caret before the kept character (iOS's space-bar trackpad): a delete still goes; a cut sends nothing
  const edge = sent.length;
  await page.evaluate(() => document.getElementById("kbd").setSelectionRange(0, 0));
  await page.keyboard.press("Backspace");
  await page.evaluate(() => {
    const k = document.getElementById("kbd");
    k.dispatchEvent(new InputEvent("input", { inputType: "deleteByCut", bubbles: true }));
    k.dispatchEvent(new InputEvent("input", { inputType: "deleteCompositionText", bubbles: true }));
  });
  await page.waitForTimeout(100);
  const edged = sent.slice(edge).filter((m) => m.t === "in").map((m) => m.data);
  check(`${at} a delete with the caret before the kept character still goes; a cut or an IME's own delete sends nothing`,
    JSON.stringify(edged) === JSON.stringify(["\x7f"]), JSON.stringify(edged));

  // rest a finger on the input box, then slide right 3 characters and back 1
  const cdp = await page.context().newCDPSession(page);
  const geo = await page.evaluate(() => {
    const k = document.getElementById("kbd").getBoundingClientRect(), t = document.getElementById("term");
    const probe = document.createElement("span"); probe.textContent = "0".repeat(10); probe.style.cssText = "position:absolute;visibility:hidden;white-space:pre"; t.append(probe);
    const w = probe.getBoundingClientRect().width / 10; probe.remove();
    return { x: k.left + 60, y: k.top + k.height / 2, w };
  });
  // each touch carries its own time (the page judges a finger by it): the gesture, not this browser's pace
  let clock = Date.now() / 1000;
  const after = (ms) => (clock += ms / 1000);
  const touch = (type, x, ms = 16) => cdp.send("Input.dispatchTouchEvent", { type, timestamp: after(ms), touchPoints: type === "touchEnd" ? [] : [{ x, y: geo.y }] });
  // in an agent's input box the cursor follows the finger onto the character under it (held as
  // long as the finger likes first); the moves are ← and → only, and add up to there
  const net = (ms) => ms.join("").match(/\x1b\[[A-D]/g)?.reduce((n, k) => n + ({ C: 1, D: -1 }[k[2]] ?? 100), 0) ?? 0;
  const curX = await page.evaluate(() => document.querySelector("#screen .cur").getBoundingClientRect().left);
  const slide = async (rest, path) => {
    const n = sent.length;
    await touch("touchStart", geo.x);
    after(rest);                                                  // the finger rests
    for (const dx of path) await touch("touchMove", geo.x + dx);
    await touch("touchEnd");
    await page.waitForTimeout(100);
    return sent.slice(n).filter((m) => m.t === "in").map((m) => m.data);
  };
  const boxMoves = await slide(1200, [0.5, 1, 1.5, 2, 2.5, 3].map((c) => c * geo.w));
  const want = Math.floor((geo.x + 3 * geo.w - curX) / geo.w);
  check(`${at} in an agent's input box a finger held (however long) and slid takes the cursor onto the character under it`,
    boxMoves.length > 0 && Math.abs(net(boxMoves) - want) <= 1, JSON.stringify({ boxMoves, net: net(boxMoves), want }));

  // at a shell's prompt it moves along the line a character a step, both ways
  say({ t: "layout", groups: [{ kind: "window", label: "Window 1", where: "", host: null, wid: "W1", new: "New tab", pool: "",
    items: [{ id: "S1", title: "One", sub: ["zsh", "~/p"], index: "", host: null, profile: "Default", agent: null, shell: true, state: "" }] }] });
  const prompt = [line("$ echo hello"), { r: [["$ git commit -m 'x'", null, null, 0], [" ", null, null, 256]], e: true }];
  say({ t: "screen", sid: "S1", full: true, n: prompt.length, cols: 60, rows: prompt.length, ch: prompt.map((r, i) => [i, r]) });
  await page.waitForFunction(() => !document.querySelector("#screen .blk.inbox") && document.getElementById("screen").textContent.includes("git commit"), null, { timeout: 3000 }).catch(() => {});
  await page.waitForTimeout(150);
  Object.assign(geo, await page.evaluate(() => { const k = document.getElementById("kbd").getBoundingClientRect(); return { x: k.left + 60, y: k.top + k.height / 2 }; }));
  const moves = await slide(350, [...[1, 2, 3, 4, 5, 6, 7].map((k) => (geo.w * 3.5 * k) / 7), geo.w * 2.5]);
  check(`${at} a finger that rests and then slides moves the cursor a character a step, both ways, and taps nothing`,
    JSON.stringify(moves) === JSON.stringify(["\x1b[C", "\x1b[C", "\x1b[C", "\x1b[D"]), JSON.stringify(moves));

  // a finger held still on the input: at 2.5 s the page's menu, Paste, Undo and Redo (iOS shows none
  // for the invisible field); lifted before then, none; Undo sends the shell's Ctrl+_
  const hold = async (ms) => {
    await touch("touchStart", geo.x);
    await page.waitForTimeout(ms);
    await touch("touchEnd", geo.x, ms);
    await page.waitForTimeout(150);
    return page.evaluate(() => { const m = document.getElementById("rowmenu"); return m.open ? [...m.querySelectorAll("button[data-action]:not([data-action=x])")].map((b) => b.textContent) : null; });
  };
  const early = await hold(1200);
  const menuFrom = sent.length, held = await hold(2700);
  await page.waitForTimeout(300);
  await page.click("#rowmenu button[data-action=undo]");
  await page.waitForTimeout(150);
  const undone = sent.slice(menuFrom).filter((m) => m.t === "in").map((m) => m.data);
  check(`${at} a finger held still on the input for 2.5 s opens the menu of Paste, Undo and Redo (none when lifted sooner, no tap on the lift); its Undo sends Ctrl+_`,
    early === null && JSON.stringify(held) === JSON.stringify(["Paste", "Undo", "Redo"]) && JSON.stringify(undone) === JSON.stringify(["\x1f"])
    && await page.evaluate(() => !document.getElementById("rowmenu").open), JSON.stringify({ early, held, undone }));

  // a quick swipe is not a slide
  const swipeFrom = sent.length;
  await touch("touchStart", geo.x);
  for (let k = 1; k <= 5; k++) await touch("touchMove", geo.x + geo.w * k);
  await touch("touchEnd");
  await page.waitForTimeout(100);
  check(`${at} a quick swipe moves nothing`, !sent.slice(swipeFrom).some((m) => m.t === "in" && /\x1b\[[CD]/.test(m.data)));

  // a swipe up or down that starts on the input box scrolls the terminal
  await page.evaluate(() => { const t = document.getElementById("term"); t.scrollTop = t.scrollHeight; });
  const top0 = await page.evaluate(() => document.getElementById("term").scrollTop);
  const vertical = (type, y) => cdp.send("Input.dispatchTouchEvent", { type, timestamp: after(16), touchPoints: type === "touchEnd" ? [] : [{ x: geo.x, y }] });
  await vertical("touchStart", geo.y);
  for (let k = 1; k <= 6; k++) await vertical("touchMove", geo.y + k * 20);
  await vertical("touchEnd");
  await page.waitForTimeout(100);
  const top1 = await page.evaluate(() => document.getElementById("term").scrollTop);
  check(`${at} a swipe down that starts on the input box scrolls the terminal`, top1 < top0, JSON.stringify({ top0, top1 }));
  await page.close();
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const pw = await playwright();
  const browser = await pw.chromium.launch();
  try { await webTouch(browser); } finally { await browser.close(); cleanup(); }
  const failed = results.filter((r) => !r.ok).length;
  console.log(`${results.length - failed}/${results.length} passed`);
  process.exit(failed ? 1 : 0);
}
