// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Remote fixtures are synthetic browser responses; no SSH agents or real hosts are used. A host's
// pane is shown by loading the panel at its URL (`host=`), as the page does; the races of files
// that are renamed or rescanned while a request is pending are checked on one host each.
import assert from "node:assert/strict";

export async function hostRegressions({ page, test, focus, text, edit, deferred, SB }) {
  const A = "dev-a.example", B = "dev-b.example", path = `${SB}/shared.txt`, readme = `${SB}/remote.md`;
  const body = (host, p) => ({ path: p, size: 20, etag: host + "-etag", binary: false, mime: "text/plain", truncated: false, utf8: true, writable: true, text: p === readme ? "# Remote\n\n[shared](shared.txt)" : `${host} original` });
  let beforeFile = async () => {};
  await page.route("**/fb/api/ls?*", async (route) => {
    const host = route.request().headers()["x-tm-host"];
    if (!host) return route.continue();
    await route.fulfill({ json: { status: "ready", gen: 1, total: 2, writable: true, entries: [{ n: "remote.md", k: "f", s: 20 }, { n: "shared.txt", k: "f", s: 20 }] } });
  });
  await page.route("**/fb/api/file?*", async (route) => {
    const host = route.request().headers()["x-tm-host"];
    if (!host) return route.continue();
    if (await beforeFile(route, host)) return;
    const p = new URL(route.request().url()).searchParams.get("path");
    await route.fulfill({ json: route.request().method() === "PUT" ? { etag: host + "-saved" } : body(host, p) });
  });
  const onHost = (host, tabs = [path]) => focus(`review-${host}`, SB, tabs, host);
  await test("Overwrite follows an external rename while the initial save was pending", async () => {
    await onHost(A); await text(A); await edit(" pending rename save");
    const started = deferred(), release = deferred(), targets = [], renamed = `${SB}/renamed-during-save.txt`;
    beforeFile = async (route, host) => {
      if (host !== A || route.request().method() !== "PUT") return;
      targets.push(new URL(route.request().url()).searchParams.get("path"));
      if (targets.length === 1) {
        started.resolve(); await release.promise;
        await route.fulfill({ status: 409, json: { error: "conflict", message: "File moved during save" } });
      } else await route.fulfill({ json: { etag: "renamed-saved" } });
      return true;
    };
    await page.keyboard.press("Meta+s"); await started.promise;
    await page.evaluate(({ from, to, host, dir }) => window.reviewSocket.dispatchEvent(new MessageEvent("message", { data: JSON.stringify({ event: "fs-change", data: { dirs: [dir], files: [], moved: [{ from, to }], host } }) })), { from: path, to: renamed, host: A, dir: SB });
    await page.locator('.tab.active .tname:text("renamed-during-save.txt")').waitFor();
    release.resolve(); await page.click('.modal button[data-id="overwrite"]');
    await page.locator(".tab.active:not(.dirty)").waitFor();
    assert.deepEqual(targets, [path, renamed]);
    assert.ok((await page.textContent(".cm-content")).includes("pending rename save"));
    beforeFile = async () => {};
  });
  await test("old-path save success cannot mark the renamed buffer clean", async () => {
    await onHost(A); await text(A); await edit(" unconfirmed renamed save");
    const started = deferred(), release = deferred(), renamed = `${SB}/success-renamed.txt`;
    beforeFile = async (route, host) => {
      if (host !== A || route.request().method() !== "PUT") return;
      started.resolve(); await release.promise; await route.fulfill({ json: { etag: "old-path-saved" } }); return true;
    };
    await page.keyboard.press("Meta+s"); await started.promise;
    await page.evaluate(({ from, to, host, dir }) => window.reviewSocket.dispatchEvent(new MessageEvent("message", { data: JSON.stringify({ event: "fs-change", data: { dirs: [dir], files: [], moved: [{ from, to }], host } }) })), { from: path, to: renamed, host: A, dir: SB });
    await page.locator('.tab.active .tname:text("success-renamed.txt")').waitFor(); release.resolve();
    await page.waitForFunction(() => document.querySelector("#toast")?.textContent.includes("file was renamed"));
    assert.equal(await page.locator(".tab.active.dirty").count(), 1);
    assert.ok((await page.textContent(".cm-content")).includes("unconfirmed renamed save"));
    beforeFile = async () => {};
  });
  await test("conflict Reload refreshes the renamed document on screen", async () => {
    await onHost(A); await text(A); await edit(" discard renamed edits");
    const started = deferred(), release = deferred(), renamed = `${SB}/reload-renamed.txt`;
    beforeFile = async (route, host) => {
      if (host !== A || route.request().method() !== "PUT") return;
      started.resolve(); await release.promise;
      await route.fulfill({ status: 409, json: { error: "conflict", message: "File moved before save" } }); return true;
    };
    await page.keyboard.press("Meta+s"); await started.promise;
    await page.evaluate(({ from, to, host, dir }) => window.reviewSocket.dispatchEvent(new MessageEvent("message", { data: JSON.stringify({ event: "fs-change", data: { dirs: [dir], files: [], moved: [{ from, to }], host } }) })), { from: path, to: renamed, host: A, dir: SB });
    await page.locator('.tab.active .tname:text("reload-renamed.txt")').waitFor(); release.resolve();
    await page.click('.modal button[data-id="reload"]');
    await page.waitForFunction(() => document.querySelector(".cm-content") && !document.querySelector(".cm-content").textContent.includes("discard renamed edits"));
    assert.equal(await page.locator(".tab.active.dirty").count(), 0);
    beforeFile = async () => {};
  });
  await test("a read of the old pathname cannot replace renamed file metadata", async () => {
    await onHost(A); await text(A);
    const started = deferred(), release = deferred(), renamed = `${SB}/read-renamed.txt`, targets = [];
    beforeFile = async (route, host) => {
      if (host !== A) return;
      const p = new URL(route.request().url()).searchParams.get("path");
      if (route.request().method() === "GET" && p === path) {
        started.resolve(); await release.promise;
        await route.fulfill({ json: { ...body(A, path), etag: "stale-old-name", text: "Stale old pathname" } }); return true;
      }
      if (route.request().method() === "PUT") targets.push(p);
    };
    await page.evaluate((host) => window.reviewSocket.dispatchEvent(new MessageEvent("message", { data: JSON.stringify({ event: "rescan", data: { host } }) })), A);
    await started.promise;
    await page.evaluate(({ from, to, host, dir }) => window.reviewSocket.dispatchEvent(new MessageEvent("message", { data: JSON.stringify({ event: "fs-change", data: { dirs: [dir], files: [], moved: [{ from, to }], host } }) })), { from: path, to: renamed, host: A, dir: SB });
    await page.locator('.tab.active .tname:text("read-renamed.txt")').waitFor(); release.resolve(); await page.waitForTimeout(150);
    await edit(" current renamed edit"); await page.keyboard.press("Meta+s"); await page.locator(".tab.active:not(.dirty)").waitFor();
    assert.deepEqual(targets, [renamed]);
    beforeFile = async () => {};
  });
  const rescan = (host) => page.evaluate((host) => window.reviewSocket.dispatchEvent(new MessageEvent("message", { data: JSON.stringify({ event: "rescan", data: { host } }) })), host);
  await test("rescan marks a changed dirty file and preserves its edits", async () => {
    await onHost(B); await text(B); await edit(" B unsaved");
    let reads = 0;
    beforeFile = async (route, host) => {
      if (host === B && route.request().method() === "GET") {
        reads++;
        await route.fulfill({ json: { ...body(B, path), etag: "changed", text: "B changed on disk" } }); return true;
      }
    };
    await rescan(A); await page.waitForTimeout(100); assert.equal(reads, 0);
    await rescan(B); await page.locator('.banner:has-text("Changed on disk")').waitFor();
    assert.ok((await page.textContent(".cm-content")).includes("B unsaved"));
    assert.equal(await page.locator(".tab.active.dirty").count(), 1);
  });
  await test("rescan reloads a clean tab and disconnected agents do not mark it deleted", async () => {
    await onHost(A, [readme]); await page.locator(".md-body h1").waitFor();
    beforeFile = async (route, host) => {
      if (host === A && route.request().method() === "GET") {
        await route.fulfill({ json: { ...body(A, readme), etag: "new-readme", text: "# Rescanned clean tab" } }); return true;
      }
    };
    await rescan(A); await page.locator('.md-body h1:text("Rescanned clean tab")').waitFor();
    beforeFile = async (route, host) => {
      if (host === A) { await route.fulfill({ status: 404, json: { error: "no_agent", message: "Agent disconnected" } }); return true; }
    };
    await rescan(A); await page.waitForTimeout(150);
    assert.equal(await page.locator('.banner:has-text("Deleted on disk")').count(), 0);
  });
}
