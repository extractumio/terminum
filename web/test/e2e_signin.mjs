// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-09 checks of the page's sign-in against a fake terminumd (fake.mjs): the sign-in of
// /open/<code> moves into the origin's storage and out of the page and its address; every
// request carries it as X-TM-Session and an image's URL as ?t=, the WebSocket as its
// subprotocol, but only after the server proved it knows it; a server that cannot prove it
// never sees it; a signed-out page says so once and stops; the web listener's page holds none.

import crypto from "node:crypto";
import { check } from "./check.mjs";
import { drawn, openPage, pause } from "./fake.mjs";

const SECRET = "a".repeat(32) + "0123456789abcdef".repeat(2);
const carries = (r) => r.headers["x-tm-session"] === SECRET || r.query.includes(SECRET);

export async function webSignin(browser) {
  // a page opened by `terminum open`
  const f = await openPage(browser, { url: "/open/c0de", meta: SECRET, secret: SECRET });
  await drawn(f.page);
  const state = await f.page.evaluate(() => ({ meta: !!document.querySelector('meta[name="tm-session"]'),
    stored: localStorage.getItem("tm.session"), path: location.pathname + location.search }));
  await f.page.evaluate(() => fetch("/fb/api/state").catch(() => null));
  const img = await f.page.evaluate(() => import("/signin.js").then((m) => m.signedUrl("/fb/api/raw?path=%2Ftmp%2Fa.png")));
  const hello = f.requests.find((r) => r.path === "/api/hello"), first = f.requests.findIndex(carries);
  const q = new URLSearchParams(hello?.query ?? "");
  check("AC-09 sign-in: the page of /open/<code> moves its sign-in into storage and out of the page and its address",
    !state.meta && state.stored === SECRET && state.path === "/", JSON.stringify(state));
  check("AC-09 sign-in: the server proves itself first (/api/hello with a 32-hex nonce and the sign-in's id)",
    !!hello && /^[0-9a-f]{32}$/.test(q.get("n")) && q.get("id") === crypto.createHash("sha256").update(SECRET).digest("hex").slice(0, 16)
      && f.requests.indexOf(hello) < first, JSON.stringify({ hello, first }));
  check("AC-09 sign-in: requests carry X-TM-Session, the WebSocket the subprotocols tm and the secret, an image ?t=",
    f.requests.some((r) => r.path === "/fb/api/state" && r.headers["x-tm-session"] === SECRET)
      && JSON.stringify(f.sockets[0]?.protocols) === JSON.stringify(["tm", SECRET]) && img === `/fb/api/raw?path=%2Ftmp%2Fa.png&t=${SECRET}`,
    JSON.stringify({ protocols: f.sockets[0]?.protocols, img }));
  check("AC-09 sign-in: no page errors", !f.errors.length, f.errors.join(" | "));

  // signed out (revoked, expired): said once; nothing retried, nothing reloaded
  let auth = 204;
  const out = await openPage(browser, { stored: SECRET, secret: SECRET, auth: () => auth });
  await drawn(out.page);
  auth = { login: "code" };
  await out.page.evaluate(() => { window.kept = true; });
  out.sockets[0].close();
  await pause(out.page, 600);
  const said = await out.page.evaluate(() => document.getElementById("status").textContent);
  const n = out.requests.length;
  await pause(out.page, 4500);
  const after = await out.page.evaluate(() => ({ kept: window.kept === true, stored: localStorage.getItem("tm.session") }));
  check("AC-09 sign-in: a signed-out page says \"Signed out: run terminum open\", stops retrying and stays (no reload)",
    said === "Signed out: run terminum open" && out.sockets.length === 1 && out.requests.length === n && after.kept && after.stored === null,
    JSON.stringify({ said, sockets: out.sockets.length, more: out.requests.length - n, after }));
  await out.close();

  // the web listener: its password form when the WebSocket is refused
  const web = await openPage(browser, { auth: { login: "password" } });
  await drawn(web.page);
  web.sockets[0].close();
  await web.page.waitForSelector("#login:not([hidden])", { timeout: 3000 }).catch(() => {});
  check("AC-10 sign-in: a page of the web listener holds no secret (no proof, no header, no subprotocol); refused, it shows the password form",
    !web.requests.some((r) => r.path === "/api/hello" || r.headers["x-tm-session"]) && !web.sockets[0].protocols.length
      && await web.page.isVisible("#login") && !(await web.page.isVisible("#app")));
  await web.close();
  await f.close();

  // another program on the port: it cannot prove the secret, so it never gets it
  const evil = await openPage(browser, { stored: SECRET, secret: "not-the-secret" });
  await evil.page.waitForFunction(() => document.getElementById("status").textContent.length > 0, null, { timeout: 5000 }).catch(() => {});
  await pause(evil.page, 2500);                    // a retry or two
  const why = await evil.page.evaluate(() => document.getElementById("status").textContent);
  const evilImg = await evil.page.evaluate(() => import("/signin.js").then((m) => m.signedUrl("/fb/api/raw?path=%2Ftmp%2Fa.png")));
  check("AC-09 sign-in: a server that cannot prove itself never sees the secret: no header, no image URL with it, no WebSocket; the page says why",
    !evil.requests.some(carries) && evil.sockets.length === 0 && evilImg === null && why === "terminum's server could not prove itself: run terminum open"
      && evil.requests.filter((r) => r.path === "/api/hello").length >= 2, JSON.stringify({ why, sockets: evil.sockets.length, asked: evil.requests.map((r) => r.path) }));
  await evil.close();

  // a sign-in terminumd does not know (revoked, another install): signed out, nothing sent
  const gone = await openPage(browser, { stored: SECRET, secret: "unknown_signin" });
  await gone.page.waitForFunction(() => document.getElementById("status").textContent.length > 0, null, { timeout: 5000 }).catch(() => {});
  const asked = gone.requests.length;
  await pause(gone.page, 2500);
  const left = await gone.page.evaluate(() => ({ status: document.getElementById("status").textContent, stored: localStorage.getItem("tm.session") }));
  check("AC-09 sign-in: a sign-in /api/hello does not know (404 unknown_signin) is signed out once: nothing sent, nothing retried",
    left.status === "Signed out: run terminum open" && left.stored === null && !gone.requests.some(carries) && gone.sockets.length === 0
      && gone.requests.length === asked, JSON.stringify({ left, more: gone.requests.slice(asked).map((r) => r.path) }));
  await gone.close();
}
