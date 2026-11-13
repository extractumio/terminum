// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-51 header layout: dot, title and buttons on the first line; the mode badge left of the
// path on the second.
import { check } from "./e2e_harness.mjs";

export async function layoutTools(page) {
  const l = await page.evaluate(() => {
    const box = (id) => document.getElementById(id).getBoundingClientRect();
    const mode = box("mode"), crumbs = box("crumbs"), title = box("title");
    return {
      badgeLeftOfPath: mode.right <= crumbs.left + 1 && Math.abs((mode.top + mode.bottom) / 2 - (crumbs.top + crumbs.bottom) / 2) < 6,
      titleAbove: title.bottom <= crumbs.top + 1,
      badgeNotOnTop: document.getElementById("mode").closest(".top") === null,
      mode: document.getElementById("mode").textContent,
    };
  });
  check("AC-51 the mode badge sits left of the path, on the line below the title", l.badgeLeftOfPath && l.titleAbove && l.badgeNotOnTop, JSON.stringify(l));
  check("AC-51 a local pane's badge says tmux", l.mode === "tmux", JSON.stringify(l));
}

/** AC-52 the splitter's drag ends when the gesture is cancelled (the system took the pointer):
 *  later moves no longer change the split. Needs an open tab (the splitter shows with the viewer). */
export async function splitterCancel(page) {
  const split = () => page.$eval("#app", (a) => a.style.getPropertyValue("--split"));
  const box = await page.$eval("#splitter", (s) => { const r = s.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; });
  const before = await split();
  await page.mouse.move(box.x, box.y);
  await page.mouse.down();
  await page.mouse.move(box.x, box.y - 60, { steps: 3 });
  const dragged = await split();
  await page.dispatchEvent("#splitter", "pointercancel");
  await page.mouse.move(box.x, box.y + 120, { steps: 3 });
  const after = await split();
  await page.mouse.up();
  check("AC-52 a drag of the splitter moves it, and a cancelled drag leaves it where it was dropped", dragged !== before && after === dragged, JSON.stringify({ before, dragged, after }));
  await page.dispatchEvent("#splitter", "dblclick");   // back to half, as the later checks expect
}
