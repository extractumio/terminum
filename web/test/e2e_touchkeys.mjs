// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// terminum.md AC-18 against a fake terminumd (fake.mjs), on a phone and a tablet: the keyboard's button is lit only
// while the on-screen keyboard is up (the emulated view never shrinks: none is), so the terminal having the keys
// lights nothing; a key that comes with none up is a hardware keyboard's, which hides the keyboard's and the hot
// keys' buttons until the terminal lets the keys go; the cursor of a terminal without the keys is a dim block on a
// touch screen, an outline on a computer.
import { check, runningIn, until } from "./check.mjs";
import { drawn, openPage, PHONE, TABLET, WIDE } from "./fake.mjs";

const keys = (page) => page.evaluate(() => {
  const seen = (id) => { const b = document.getElementById(id); return !!b && b.getBoundingClientRect().width > 0 && getComputedStyle(b).display !== "none"; };
  return { kbd: seen("kbdbtn"), hot: seen("keysfab"), lit: document.getElementById("kbdbtn").getAttribute("aria-pressed"), hw: document.body.classList.contains("hwkbd") };
});
const cursor = (page) => page.evaluate(() => {
  const c = document.querySelector("#screen .cur");
  if (!c) return null;
  const st = getComputedStyle(c);
  return { outline: st.outlineStyle, bg: st.backgroundColor };
});

export async function webTouchKeys(browser, name) {
  for (const [size, device] of [["390×844", PHONE], ["1180×820 touch", TABLET]]) {
    const at = `(${name} ${size})`;
    runningIn(`${name} ${size}`);
    const f = await openPage(browser, { device });
    const { page } = f;
    await drawn(page);
    const cur = await cursor(page);
    check(`AC-18 ${at} the cursor without the keys is a dim block, no hollow box`, cur && cur.outline === "none" && !/rgba\(0, 0, 0, 0\)|transparent/.test(cur.bg), JSON.stringify(cur));
    const rest = await keys(page);
    check(`AC-18 ${at} at rest: the keyboard's button unlit`, rest.kbd && rest.lit === "false" && !rest.hw, JSON.stringify(rest));
    await page.focus("#kbd");
    const typing = await keys(page);
    check(`AC-18 ${at} the terminal has the keys, no on-screen keyboard up: the button stays unlit`, typing.kbd && typing.lit === "false", JSON.stringify(typing));
    await page.keyboard.press("ArrowUp");
    check(`AC-18 ${at} a key with no on-screen keyboard up: a hardware keyboard, its buttons go`, await until(async () => { const k = await keys(page); return k.hw && !k.kbd && !k.hot; }), JSON.stringify(await keys(page)));
    await page.evaluate(() => document.getElementById("kbd").blur());
    check(`AC-18 ${at} the keys let go: both buttons back, unlit`, await until(async () => { const k = await keys(page); return !k.hw && k.kbd && k.hot && k.lit === "false"; }), JSON.stringify(await keys(page)));
    check(`no page error ${at}`, f.errors.length === 0, f.errors.join("; "));
    await f.context.close();
  }
  runningIn(`${name} 1400×900`);
  const f = await openPage(browser, { device: WIDE });
  await drawn(f.page);
  await f.page.evaluate(() => document.getElementById("kbd").blur());
  const cur = await cursor(f.page);
  check(`AC-18 (${name} 1400×900) a computer's cursor without the keys stays an outline`, cur?.outline === "solid", JSON.stringify(cur));
  await f.context.close();
}
