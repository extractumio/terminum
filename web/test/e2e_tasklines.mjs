// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// terminum-kanban.md AC-117 against a fake terminumd (fake.mjs): a task's line closes by terminumd, so every browser
// shows the same; the line hides while its close goes, comes back with terminumd's refusal, stays hidden through a
// `taskmarks` sent before the close was kept, and shows again under a new key (a new move into the column).
import { check, runningIn, until } from "./check.mjs";
import { drawn, openPage, SIZES } from "./fake.mjs";

const line = (key, words = "T-7 waits for you") => ({ id: "T-7", failed: false, words, key });
const shown = (page) => page.evaluate(() => [...document.querySelectorAll("#asks .taskline")].map((c) => c.textContent).join("|"));
const toast = (page) => page.evaluate(() => (document.getElementById("toast").hidden ? "" : document.getElementById("toast").textContent));

export async function webTaskLines(browser, name) {
  for (const [size, device] of SIZES) {
    const at = `(${name} ${size})`;
    runningIn(`${name} ${size}`);
    const f = await openPage(browser, { device });
    const { page } = f;
    await drawn(page);
    const posts = [];
    let answer = { status: 400, body: { error: "The tasks' store: locked" } };
    await page.route("**/api/tasks/*/seen", (route) => {
      posts.push({ path: new URL(route.request().url()).pathname, body: JSON.parse(route.request().postData() || "{}") });
      return route.fulfill({ status: answer.status, contentType: "application/json", body: JSON.stringify(answer.body) });
    });
    f.say({ t: "taskmarks", marks: {}, counts: {}, waiting: 1, failed: 0, lines: [line("5|T-7 waits for you")] });
    check(`AC-117 ${at} terminumd's line shows`, await until(async () => (await shown(page)).includes("T-7 waits for you")), await shown(page));

    await page.click("#asks .taskline .x");
    check(`AC-117 ${at} a close is sent to terminumd with the line's key`, await until(() => posts.length === 1) && posts[0].path === "/api/tasks/T-7/seen" && posts[0].body.key === "5|T-7 waits for you", JSON.stringify(posts));
    check(`AC-117 ${at} a refused close brings the line back and says why`, await until(async () => (await shown(page)) !== "" && /T-7's line: The tasks' store: locked/.test(await toast(page))), `${await shown(page)} / ${await toast(page)}`);

    answer = { status: 200, body: { ok: true } };
    await page.click("#asks .taskline .x");
    check(`AC-117 ${at} a kept close hides the line`, await until(async () => (await shown(page)) === "") && posts.length === 2, await shown(page));
    f.say({ t: "taskmarks", marks: {}, counts: {}, waiting: 1, failed: 0, lines: [line("5|T-7 waits for you")] });
    await page.waitForTimeout(200);
    check(`AC-117 ${at} a taskmarks sent before terminumd kept the close does not bring it back`, (await shown(page)) === "", await shown(page));
    f.say({ t: "taskmarks", marks: {}, counts: {}, waiting: 1, failed: 0, lines: [] });
    f.say({ t: "taskmarks", marks: {}, counts: {}, waiting: 1, failed: 0, lines: [line("9|T-7 waits for you")] });
    check(`AC-117 ${at} moved into the column again: its line again, same words`, await until(async () => (await shown(page)).includes("T-7 waits for you")), await shown(page));
    // the session page opens a task's window on the board, which says it was seen: no key, terminumd knows the line
    f.say({ t: "taskmarks", marks: {}, counts: {}, waiting: 1, failed: 0, lines: [] });
    check(`AC-117 ${at} a line that goes in terminumd goes here`, await until(async () => (await shown(page)) === ""), await shown(page));
    // the Kanban key's counts, as the board's cards: waiting for a human in yellow, failed in red, none at 0
    const badges = () => page.evaluate(() => {
      const key = document.querySelector('.nav-head .sections a[href^="/kanban/"]');
      if (!key) return null;
      const css = getComputedStyle(document.documentElement), k = key.getBoundingClientRect();
      return [...key.querySelectorAll(".taskneed")].map((b) => {
        const r = b.getBoundingClientRect(), st = getComputedStyle(b);
        return { kind: b.classList.contains("waiting") ? "waiting" : b.classList.contains("failed") ? "failed" : "", n: b.textContent, title: b.title, bg: st.backgroundColor, ink: st.color,
          onKey: r.left >= k.left - 6 && r.right <= k.right + 6 && r.bottom > k.top - 6 && r.top < k.bottom,
          yellow: css.getPropertyValue("--waiting").trim(), red: css.getPropertyValue("--danger-fill").trim() };
      });
    });
    const hex = (rgb) => "#" + (rgb.match(/\d+/g) ?? []).slice(0, 3).map((v) => Number(v).toString(16).padStart(2, "0")).join("");
    f.say({ t: "taskmarks", marks: {}, counts: {}, waiting: 2, failed: 3, lines: [] });
    await until(async () => (await badges())?.length === 2);
    const two = await badges();
    check(`AC-117 ${at} Kanban counts those waiting in yellow, those failed in red`, two?.length === 2 && two[0].kind === "waiting" && two[0].n === "2" && hex(two[0].bg) === two[0].yellow.toLowerCase() && two[1].kind === "failed" && two[1].n === "3" && hex(two[1].bg) === two[1].red.toLowerCase(), JSON.stringify(two));
    check(`AC-117 ${at} each says what it counts`, two?.[0]?.title === "2 tasks wait for you" && two?.[1]?.title === "3 tasks failed", JSON.stringify(two?.map((b) => b.title)));
    check(`AC-117 ${at} both on the key's corner, within its width`, two?.every((b) => b.onKey), JSON.stringify(two));
    f.say({ t: "taskmarks", marks: {}, counts: {}, waiting: 0, failed: 1, lines: [] });
    check(`AC-117 ${at} none waiting: only the red one`, await until(async () => JSON.stringify((await badges())?.map((b) => [b.kind, b.n])) === '[["failed","1"]]'), JSON.stringify(await badges()));
    f.say({ t: "taskmarks", marks: {}, counts: {}, waiting: 0, failed: 0, lines: [] });
    check(`AC-117 ${at} none at all: no count`, await until(async () => (await badges())?.length === 0), JSON.stringify(await badges()));
    const stored = await page.evaluate(() => Object.keys(localStorage).filter((k) => /taskseen/.test(k)).join());
    check(`AC-117 ${at} nothing of the lines is kept in this browser`, stored === "", stored);
    check(`no page error ${at}`, f.errors.length === 0, f.errors.join("; "));
    await f.context.close();
  }
}
