// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-88 against a real private terminumd (e2e_live.mjs): the quick shell opens under the shown
// pane in its folder, takes the keys, is listed under its pane, resizes at its edge, moves to
// the left, comes back with its pane, and closes with its ×.
import { check, within } from "./check.mjs";

const quickText = (page) => page.frameLocator("#quickframe").locator("#screen").innerText().catch(() => "");

export async function liveQuick(page, { tmux, work }) {
  const panes = () => tmux("list-panes", "-a", "-F", "#{pane_id} #{@tm-quick} #{window_width}x#{window_height}").split("\n").filter(Boolean);
  await page.click("#quickbtn");
  await within("AC-88 the quick shell opens below the terminal", () => page.waitForSelector("#quick:not([hidden])", { timeout: 5000 }));
  const marked = panes().find((l) => /^%\d+ %\d+ /.test(l));
  check("AC-88 it is a tmux window of its own, marked with its pane", !!marked, panes().join(" | "));
  await within("AC-88 its shell draws in the drawer", () => page.waitForFunction(() => {
    const d = document.getElementById("quickframe").contentDocument;
    return d?.querySelectorAll("#screen .ln").length > 1 && d.body.classList.contains("embed");
  }, null, { timeout: 8000 }));
  await page.waitForTimeout(300);
  await page.keyboard.type("pwd; echo quick-$((7*6))");
  await page.keyboard.press("Enter");
  await within("AC-88 the keys go to the quick shell, which starts in the pane's folder", async () => {
    for (let i = 0; i < 50; i++) {
      const t = await quickText(page);
      if (t.includes("quick-42") && t.includes(work)) return;
      await page.waitForTimeout(100);
    }
    throw new Error(await quickText(page));
  });
  const main = await page.evaluate(() => document.getElementById("screen").innerText);
  check("AC-88 nothing typed there reached the pane", !main.includes("quick-42"));
  const listed = await page.evaluate(() => [...document.querySelectorAll("#sessions .kids .item")].map((b) => b.innerText).join(" | "));
  check("AC-88 the list shows it under its pane as quick shell", listed.includes("quick shell"), listed);
  const share = () => page.evaluate(() => {
    const r = document.getElementById("termrow").getBoundingClientRect(), q = document.getElementById("quick").getBoundingClientRect();
    return Math.round((q.height / r.height) * 100);
  });
  const first = await share();
  const grip = await page.locator("#quickgrip").boundingBox();
  await page.mouse.move(grip.x + grip.width / 2, grip.y + 2);
  await page.mouse.down();
  await page.mouse.move(grip.x + grip.width / 2, grip.y - 150, { steps: 5 });
  await page.mouse.up();
  const dragged = await share();
  check("AC-88 it opens at a third and grows when its edge is dragged; the share is remembered",
    Math.abs(first - 33) <= 3 && dragged > first + 10 && await page.evaluate(() => localStorage.getItem("quickShare.bottom") !== null), JSON.stringify({ first, dragged }));
  await page.waitForTimeout(2500);                 // past a resize's second and the view's settling
  const sized = await page.evaluate(() => ({ banner: !document.getElementById("sized").hidden, text: document.getElementById("sized").textContent }));
  check("AC-88 the pane's page still sizes its pane: no \"Sized for another screen\" after the split opened, was typed in and dragged",
    !sized.banner, JSON.stringify({ ...sized, panes: panes() }));
  await page.click("#quickflip");
  await within("AC-88 ⇄ moves it to the left", () => page.waitForFunction(() => document.getElementById("termrow").classList.contains("left"), null, { timeout: 5000 }));
  const quickRow = await page.$('#sessions .kids .item:has-text("quick shell")');
  await page.reload();
  await within("AC-88 after a reload it shows again with its pane, where it was", () => page.waitForFunction(() =>
    !document.getElementById("quick").hidden && document.getElementById("termrow").classList.contains("left"), null, { timeout: 8000 }));
  check("AC-88 the quick shell's row existed", !!quickRow);
  await page.waitForTimeout(500);
  await page.click("#quickclose");
  await within("AC-88 × closes it: the window goes, the drawer too", async () => {
    await page.waitForFunction(() => document.getElementById("quick").hidden, null, { timeout: 5000 });
    for (let i = 0; i < 30 && panes().some((l) => /^%\d+ %\d+ /.test(l)); i++) await page.waitForTimeout(100);
    if (panes().some((l) => /^%\d+ %\d+ /.test(l))) throw new Error(panes().join(" | "));
  });
  await page.click("#term");
}
