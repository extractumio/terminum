// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-09 checks of the page's sign-in against a fake terminumd (fake.mjs): the password form
// gets a sign-in that goes into the origin's storage; every request carries it as
// X-TM-Session and an image's URL as ?t=, the WebSocket as its subprotocol, but only after the
// server proved it knows it; a server that cannot prove it never sees it; a signed-out page
// drops its sign-in and shows the form; while no password is set the form says how to set one.

import crypto from "node:crypto";
import { check } from "./check.mjs";
import { drawn, openPage, pause } from "./fake.mjs";

const SECRET = "a".repeat(32) + "0123456789abcdef".repeat(2);
const carries = (r) => r.headers["x-tm-session"] === SECRET || r.query.includes(SECRET);
const said = (page) => page.evaluate(() => document.getElementById("loginerr").textContent);

export async function webSignin(browser) {
  // a browser that never signed in: the form; the right password gets the sign-in
  let auth = { login: "password" }, tried = [];
  const login = (body) => {
    tried.push(body.password);
    if (body.password !== "correct horse") return [401, { reason: "wrong", retry: 0 }];
    auth = 204;
    return [200, { secret: SECRET }];
  };
  const f = await openPage(browser, { secret: SECRET, auth: () => auth, login });
  await f.page.waitForSelector("#login:not([hidden])", { timeout: 5000 });
  const before = { hello: f.requests.some((r) => r.path === "/api/hello"), protocols: f.sockets[0]?.protocols ?? [] };
  await f.page.fill("#password", "wrong guess");
  await f.page.press("#password", "Enter");
  await f.page.waitForFunction(() => document.getElementById("loginerr").textContent.length > 0, null, { timeout: 3000 });
  const wrong = await said(f.page);
  await f.page.fill("#password", "correct horse");
  await f.page.press("#password", "Enter");
  await drawn(f.page);
  const state = await f.page.evaluate(() => ({ stored: localStorage.getItem("tm.session"), path: location.pathname + location.search,
    form: !document.getElementById("login").hidden, typed: document.getElementById("password").value }));
  await f.page.evaluate(() => fetch("/fb/api/state").catch(() => null));
  const img = await f.page.evaluate(() => import("/signin.js").then((m) => m.signedUrl("/fb/api/raw?path=%2Ftmp%2Fa.png")));
  const hello = f.requests.find((r) => r.path === "/api/hello"), first = f.requests.findIndex(carries);
  const q = new URLSearchParams(hello?.query ?? "");
  check("AC-09 sign-in: a new browser gets the password form and holds nothing (no proof, no header, no subprotocol)",
    !before.hello && !before.protocols.length && !f.requests.slice(0, 3).some(carries), JSON.stringify(before));
  check("AC-09 sign-in: a wrong password says so; the right one's sign-in goes into storage, never the address, and the form goes",
    wrong === "Wrong password." && JSON.stringify(tried) === JSON.stringify(["wrong guess", "correct horse"])
      && state.stored === SECRET && state.path === "/" && !state.form && state.typed === "", JSON.stringify({ wrong, tried, state }));
  check("AC-09 sign-in: the server proves itself first (/api/hello with a 32-hex nonce and the sign-in's id)",
    !!hello && /^[0-9a-f]{32}$/.test(q.get("n")) && q.get("id") === crypto.createHash("sha256").update(SECRET).digest("hex").slice(0, 16)
      && f.requests.indexOf(hello) < first, JSON.stringify({ hello, first }));
  check("AC-09 sign-in: requests carry X-TM-Session, the WebSocket the subprotocols tm and the secret, an image ?t=",
    f.requests.some((r) => r.path === "/fb/api/state" && r.headers["x-tm-session"] === SECRET)
      && JSON.stringify(f.sockets.at(-1)?.protocols) === JSON.stringify(["tm", SECRET]) && img === `/fb/api/raw?path=%2Ftmp%2Fa.png&t=${SECRET}`,
    JSON.stringify({ protocols: f.sockets.at(-1)?.protocols, img }));

  // Sign out in the menu: this browser's sign-in ends, at terminumd and here
  await f.page.evaluate(() => document.getElementById("signout").click());
  await f.page.waitForSelector("#login:not([hidden])", { timeout: 3000 }).catch(() => {});
  const out = f.requests.find((r) => r.path === "/auth/logout");
  const left = await f.page.evaluate(() => localStorage.getItem("tm.session"));
  check("AC-09 sign-in: Sign out tells terminumd which sign-in ends, forgets it and shows the form",
    out?.headers["x-tm-session"] === SECRET && left === null && await f.page.isVisible("#login"), JSON.stringify({ out: !!out, left }));
  check("AC-09 sign-in: no page errors", !f.errors.length, f.errors.join(" | "));
  await f.close();

  // signed out elsewhere (terminum signout, a new password): the form, the sign-in dropped, the page kept
  let auth2 = 204;
  const gone = await openPage(browser, { stored: SECRET, secret: SECRET, auth: () => auth2 });
  await drawn(gone.page);
  auth2 = { login: "password" };
  await gone.page.evaluate(() => { window.kept = true; });
  gone.sockets[0].close();
  await gone.page.waitForSelector("#login:not([hidden])", { timeout: 3000 }).catch(() => {});
  const after = await gone.page.evaluate(() => ({ kept: window.kept === true, stored: localStorage.getItem("tm.session") }));
  check("AC-09 sign-in: a signed-out page says \"Signed out: sign in again.\" in the form, drops its sign-in and stays (no reload)",
    await said(gone.page) === "Signed out: sign in again." && after.kept && after.stored === null, JSON.stringify(after));
  await gone.close();

  // no password set yet: the form says how to set one and waits
  const none = await openPage(browser, { secret: SECRET, auth: { login: "none", message: "No password is set yet. On the Mac, run: terminum password" } });
  await none.page.waitForSelector("#login:not([hidden])", { timeout: 5000 });
  check("AC-09 sign-in: while no password is set the form says \"On the Mac, run: terminum password\" and cannot be sent",
    await said(none.page) === "No password is set yet. On the Mac, run: terminum password" && await none.page.isDisabled("#password"),
    await said(none.page));
  await none.close();

  // another program on the port: it cannot prove the secret, so it never gets it
  const evil = await openPage(browser, { stored: SECRET, secret: "not-the-secret" });
  await evil.page.waitForFunction(() => document.getElementById("status").textContent.length > 0, null, { timeout: 5000 }).catch(() => {});
  await pause(evil.page, 2500);                    // a retry or two
  const why = await evil.page.evaluate(() => document.getElementById("status").textContent);
  const evilImg = await evil.page.evaluate(() => import("/signin.js").then((m) => m.signedUrl("/fb/api/raw?path=%2Ftmp%2Fa.png")));
  check("AC-09 sign-in: a server that cannot prove itself never sees the secret: no header, no image URL with it, no WebSocket; the page says why",
    !evil.requests.some(carries) && evil.sockets.length === 0 && evilImg === null && why === "terminum's server could not prove itself"
      && evil.requests.filter((r) => r.path === "/api/hello").length >= 2, JSON.stringify({ why, sockets: evil.sockets.length, asked: evil.requests.map((r) => r.path) }));
  await evil.close();

  // a sign-in terminumd does not know (revoked, another install): dropped, nothing sent
  const unknown = await openPage(browser, { stored: SECRET, secret: "unknown_signin", auth: { login: "password" } });
  await unknown.page.waitForSelector("#login:not([hidden])", { timeout: 5000 }).catch(() => {});
  const asked = unknown.requests.length;
  await pause(unknown.page, 2500);
  const was = await unknown.page.evaluate(() => localStorage.getItem("tm.session"));
  check("AC-09 sign-in: a sign-in /api/hello does not know (404 unknown_signin) is dropped once: the form, nothing sent, nothing retried",
    await said(unknown.page) === "Signed out: sign in again." && was === null && !unknown.requests.some(carries) && unknown.sockets.length === 0
      && unknown.requests.length === asked, JSON.stringify({ was, more: unknown.requests.slice(asked).map((r) => r.path) }));
  await unknown.close();
}
