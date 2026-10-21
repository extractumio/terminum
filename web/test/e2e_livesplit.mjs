// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-13 against a real private terminumd (e2e_live.mjs): a window split under the shown pane (as
// Claude Code's teammates do) shows a banner with this pane's width; Full width zooms it in tmux
// and the window takes the view's size; a new split unzooms it and nothing zooms it again;
// Other pane shows the window's next pane, round; the banner goes with the last other pane; docked
// Files follows the quick switches back to the pane's folder (AC-70).
import { check, within } from "./check.mjs";

const note = (page) => page.evaluate(() => { const b = document.getElementById("splitnote");
  return { shown: !b.hidden, text: b.querySelector("span").textContent, zoom: document.getElementById("splitzoom").textContent }; });

export async function liveSplit(page, { tmux, work }) {
  const panes = () => tmux("list-panes", "-a", "-F", "#{pane_id} #{window_id} #{pane_current_path}").split("\n").filter(Boolean).map((l) => l.split(" "));
  const [pane, win] = panes().find((p) => p[2] === work) ?? [];
  const of = (fmt) => tmux("display", "-p", "-t", pane, fmt);
  const wait = async (what, ok, ms = 3000) => within(what, async () => {
    for (let t = 0; t < ms; t += 100) { if (await ok()) return; await page.waitForTimeout(100); }
    throw new Error(JSON.stringify({ note: await note(page), tmux: of("#{window_panes} #{window_zoomed_flag} #{pane_width} #{window_width}") }));
  });
  const before = of("#{pane_width}");
  tmux("split-window", "-h", "-d", "-l", "60%", "-t", pane);
  const mate = panes().find((p) => p[1] === win && p[0] !== pane)?.[0];
  await wait("AC-13 a split under the shown pane: a banner says so and how wide the pane is", async () => {
    const n = await note(page), [w, ww] = of("#{pane_width} #{window_width}").split(" ");
    return n.shown && n.text === `This window is split: 2 panes, this one ${w} of ${ww} columns` && n.zoom === "Full width";
  });
  check("AC-13 the split window is not resized by the page (its layout is its maker's)", Number(of("#{window_width}")) === Number(before), `${before} → ${of("#{window_width}")}`);
  tmux("resize-window", "-t", win, "-x", String(Number(before) + 30));   // another client's size: wider than the view
  await page.waitForTimeout(300);
  await page.click("#splitzoom");
  await wait("AC-13 Full width: tmux zooms the pane, the banner says so, its window takes the view's size", async () => {
    const n = await note(page), [z, w, ww] = of("#{window_zoomed_flag} #{pane_width} #{window_width}").split(" ");
    return z === "1" && w === ww && ww === before && n.text === "This window is split: 2 panes, this one zoomed" && n.zoom === "Show split";
  });
  tmux("split-window", "-v", "-d", "-t", mate);          // an agent's next teammate: tmux unzooms the window
  await wait("AC-13 a new split unzooms it: the banner shows the split again", async () => (await note(page)).text.includes("3 panes, this one "));
  await page.waitForTimeout(1500);
  check("AC-13 nothing zooms it again by itself", of("#{window_zoomed_flag}") === "0");
  await page.click("#splitother");
  await page.waitForTimeout(800);
  await page.keyboard.type("echo other-$((5*9))");
  await page.keyboard.press("Enter");
  await wait("AC-13 Other pane shows the window's next pane, and the keys go there", () =>
    tmux("capture-pane", "-p", "-t", mate).includes("other-45") && !tmux("capture-pane", "-p", "-t", pane).includes("other-45"));
  for (let i = 0; i < 2; i++) { await page.click("#splitother"); await page.waitForTimeout(500); }   // round, back to the first pane
  await page.keyboard.type("echo back-$((5*9))");
  await page.keyboard.press("Enter");
  await wait("AC-13 Other pane goes round the window's panes, back to the first", () => tmux("capture-pane", "-p", "-t", pane).includes("back-45"));
  // a quick switch away and back: Files shows the pane's folder again (WebKit stayed blank)
  const files = () => page.evaluate(() => { const f = document.getElementById("files");
    return { moving: f.classList.contains("moving"), href: f.contentWindow?.location.href, text: f.contentDocument?.body?.innerText.slice(0, 120) ?? null }; });
  await within("AC-70 Files follows the pane back to its folder after quick switches", async () => {
    for (let t = 0; t < 8000; t += 200) { const f = await files(); if (f.text?.includes("alpha.txt") && !f.moving) return; await page.waitForTimeout(200); }
    throw new Error(JSON.stringify(await files()));
  });
  for (const [id, w] of panes()) if (w === win && id !== pane && id !== mate) tmux("kill-pane", "-t", id);
  tmux("kill-pane", "-t", mate);
  await wait("AC-13 the banner goes with the window's last other pane", async () => !(await note(page)).shown);
}
