// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-09: live board requests own the current document's sign-in, including while its module
// is still loading after navigation. Never fall through to an unsigned write or retry a refusal.
import { boardKit, nap } from "./boardkit.mjs";
import { check, until } from "./check.mjs";

export async function liveBoardSignin(page, opts) {
  const { j } = boardKit(page, opts);
  let release, requested = false, posts = [];
  const gate = new Promise((r) => { release = r; });
  const hold = async (route) => { requested = true; await gate; await route.continue(); };
  const record = (r) => {
    if (new URL(r.url()).pathname === "/api/tasks" && r.method() === "POST") posts.push(!!r.headers()["x-tm-session"]);
  };
  page.on("request", record);
  await page.route("**/signin.js", hold);
  try {
    // Commit deliberately exposes the startup interval: default goto's load wait usually hides it.
    await page.goto(`${opts.base}/kanban/`, { waitUntil: "commit" });
    await page.waitForSelector("#newtask");   // static markup is not sign-in readiness
    if (!await until(() => requested)) throw new Error("the board never requested its sign-in module");
    check("AC-09 the renewed context keeps its sign-in while the new document waits for its module",
      await page.evaluate(() => !!localStorage.getItem("tm.session") && /\[native code\]/.test(String(fetch))));
    const made = j("/api/tasks", "POST", { title: "Signed in after navigation" }).then((value) => ({ value }), (error) => ({ error: error.message }));
    // Longer than the former 100 × 100ms polling bound: it must not send an unsigned fallback.
    await nap(11000);
    check("AC-09 a live helper sends no task write while sign-in installation is pending", posts.length === 0, `posts=${posts.length}`);
    release();
    const result = await made;
    check("AC-09 once sign-in is installed the live helper creates the task once, with its header",
      result.value?.code === 200 && !!result.value.id && posts.length === 1 && posts[0], result.error ?? `posts=${posts.length}`);
    await page.waitForLoadState("load");
  } finally {
    release();
    await page.unroute("**/signin.js", hold);
    page.off("request", record);
  }

  // The board redirects when a background read is refused. A scriptless document keeps this
  // real API refusal about the helper; restore its sign-in for the next context's storage.
  const kept = await page.evaluate(() => localStorage.getItem("tm.session"));
  const url = `${opts.base}/__test_unsigned_helper`;
  const empty = (route) => route.fulfill({ contentType: "text/html", body: "<!doctype html><title>Unsigned helper check</title>" });
  await page.route(url, empty);
  posts = [];
  page.on("request", record);
  try {
    await page.goto(url);
    await page.evaluate(async () => (await import("/signin.js")).forget());
    const why = await j("/api/tasks", "POST", { title: "Must be refused" }).then(() => "", (e) => e.message);
    check("AC-09 a genuinely unsigned live task write is refused once and fails the helper",
      /POST \/api\/tasks refused as signed out: Missing or expired sign-in/.test(why) && posts.length === 1 && !posts[0], `posts=${posts.length}; ${why}`);
  } finally {
    page.off("request", record);
    await page.unroute(url, empty);
    await page.evaluate(async (s) => (await import("/signin.js")).adopt(s), kept);
  }
}
