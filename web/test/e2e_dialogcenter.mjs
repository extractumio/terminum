// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-37 every dialog in the middle of the window, across and down, whatever its size: as it opens,
// as the window is resized, as a field in it is drawn taller (the bookmarks window keeps View's place).

import { check } from "./check.mjs";
import { open } from "./e2e_widgets.mjs";

const SIZES = [{ width: 1400, height: 900 }, { width: 1000, height: 620 }, { width: 1800, height: 1100 }, { width: 390, height: 844 }];

// how far each open dialog's middle is from the window's, in px
const off = (page, id) => page.evaluate((i) => {
  const b = document.getElementById(i).getBoundingClientRect(), h = document.documentElement.clientHeight;
  return { dx: Math.round(b.left + b.width / 2 - innerWidth / 2), dy: Math.round(b.top + b.height / 2 - h / 2), top: Math.round(b.top), bottom: Math.round(b.bottom), h };
}, id);
const centered = (o) => Math.abs(o.dx) <= 1 && Math.abs(o.dy) <= 1 && o.top >= 0 && o.bottom <= o.h;
const settled = (page) => page.waitForTimeout(80);                // --app-h follows the viewport (a sheet's middle)

export async function webDialogCenter(browser, name = "") {
  const at = `AC-37 web app${name}:`;
  const { page, errors } = await open(browser, { device: { viewport: SIZES[0] } });
  const ids = await page.evaluate(() => [...document.querySelectorAll("dialog.dlg:not(#marksdlg)")].map((d) => d.id));
  check(`${at} the dialogs are found`, ids.length >= 7 && ids.includes("askbox"), ids.join());
  check(`${at} the list stands beside the terminal: the window's middle is not the terminal's`,
    await page.evaluate(() => document.getElementById("term").getBoundingClientRect().left > 0));
  for (const id of ids) {
    await page.setViewportSize(SIZES[0]);
    await page.evaluate((i) => document.getElementById(i).showModal(), id);
    const bad = [];
    for (const size of SIZES) {
      await page.setViewportSize(size);
      await settled(page);
      const o = await off(page, id);
      if (!centered(o)) bad.push({ ...size, ...o });
    }
    check(`${at} ${id} is in the window's middle at each size, resized while open`, !bad.length, JSON.stringify(bad));
    await page.evaluate((i) => document.getElementById(i).close(), id);
  }
  // Ask's field takes the room its dialog keeps for it, no handle (AC-173); a dialog as tall as its
  // fields that grows (an error said, a part opened) grows both ways; taller than the window it scrolls
  await page.setViewportSize(SIZES[0]);
  await page.evaluate(() => document.getElementById("askbox").showModal());
  const ask = await page.evaluate(() => getComputedStyle(document.getElementById("askq")).resize);
  await page.evaluate(() => document.getElementById("askbox").close());
  check(`${at} Ask's field fills its dialog, with no handle to draw it`, ask === "none", ask);
  await page.evaluate(() => document.getElementById("convertbox").showModal());
  const before = await off(page, "convertbox");
  const grow = (px) => page.evaluate((px) => { const e = document.getElementById("converterror"); e.hidden = false; e.style.height = `${px}px`; }, px);
  await grow(300);
  const taller = await off(page, "convertbox");
  check(`${at} a dialog that grows stays in the middle, growing up and down`, centered(taller) && taller.top < before.top && taller.bottom > before.bottom, JSON.stringify({ before, taller }));
  await grow(2000);
  const huge = await off(page, "convertbox");
  check(`${at} taller than the window: in the middle, inside the window`, centered(huge), JSON.stringify(huge));
  check(`${at} no page errors`, !errors.length, errors.join(" | "));
  await page.close();
}
