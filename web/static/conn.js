// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The page's connection to terminumd (AC-09, AC-14, AC-16): the WebSocket and its reconnects,
// what a refused one means (signed out: run terminum open; the web listener: its password
// form), and the build check that reloads a page of another build once.
import * as signin from "./signin.js";

const $ = (id) => document.getElementById(id);
const RETRY = 2000;          // ms between tries while terminumd is down (AC-14)
const QUIET = 3000;          // ms down before the page says so: a restart takes less

/** onOpen(ws): a connection is up; onMessage(m): a message from terminumd; status(text): the
 *  status line; busy(): a Files or View frame has unsaved edits (no reload then). */
export function connection({ onOpen, onMessage, status, busy, parse }) {
  let ws = null, timer = 0, quiet = 0, downSince = 0, stopped = false, connecting = false, unproven = RETRY;

  function lost(text = "terminum's server is not running") {
    downSince ||= performance.now();
    // a restart is back within 3 s: the line says so only later
    clearTimeout(quiet);
    if (performance.now() - downSince >= QUIET) status(text);
    else quiet = setTimeout(() => { if (downSince && !stopped && ws?.readyState !== 1) status(text); }, QUIET);
    clearTimeout(timer);
    timer = setTimeout(connect, RETRY);
  }

  // One attempt at a time (the retry timer, a page back from memory, a sign-in); each proves the
  // server anew before the secret goes out (AC-09): another program may hold the port by now.
  async function connect() {
    clearTimeout(timer);
    if (stopped || connecting || ws?.readyState === 0 || ws?.readyState === 1) return;
    connecting = true;
    let p;
    try {
      signin.invalidate();
      p = await signin.prove();
    } finally { connecting = false; }
    if (stopped) return;
    if (p === "down") return lost();
    if (p === "unknown") return signedOut();
    if (p === "bad") {                             // asked again later, less and less often
      downSince ||= performance.now();
      status("terminum's server could not prove itself: run terminum open");
      timer = setTimeout(connect, unproven = Math.min(unproven * 2, 30000));
      return;
    }
    unproven = RETRY;
    const sock = ws = new WebSocket(`${location.protocol === "https:" ? "wss:" : "ws:"}//${location.host}/ws`, signin.protocols());
    sock.onopen = () => { downSince = 0; clearTimeout(quiet); status(""); signedIn(); onOpen(sock); };
    sock.onmessage = (ev) => onMessage(parse(ev.data));
    sock.onclose = async () => {
      if (ws !== sock || stopped) return;
      signin.invalidate();                          // another program may hold the port now
      const why = await authCheck();
      if (why === "password") return showLogin();
      if (why === "code") return signedOut();
      lost();
    };
  }

  /** null when signed in or terminumd does not answer, else how to sign in: "password" or "code". */
  async function authCheck() {
    const r = await fetch("/auth/check").catch(() => null);
    if (r?.status !== 401) return null;
    const m = await r.json().catch(() => ({}));
    return m.login === "password" ? "password" : "code";
  }

  // An expired or revoked sign-in: said once, nothing retried; the page stays as it is, unsaved
  // edits in View included, until the user runs terminum open (a new page).
  function signedOut() {
    stopped = true;
    clearTimeout(timer);
    signin.forget();
    status("Signed out: run terminum open");
  }
  signin.onRefused(async (known) => {
    if (stopped) return;
    if (known) return signedOut();
    const why = await authCheck();
    if (why === "password") showLogin();
    else if (why === "code") signedOut();
  });

  function send(m) {
    if (ws?.readyState === 1) { ws.send(JSON.stringify(m)); return true; }
    return false;
  }

  function showLogin(message = "") {
    stopped = true;
    $("app").hidden = true; $("login").hidden = false;
    $("loginerr").textContent = message;
    $("password").focus();
  }
  function signedIn() { $("login").hidden = true; $("app").hidden = false; }

  $("login").addEventListener("submit", async (e) => {
    e.preventDefault();
    $("loginerr").textContent = "";
    const r = await fetch("/auth/login", { method: "POST", headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ password: $("password").value }) }).catch(() => null);
    if (!r) return showLogin("terminum's server does not answer. Check that web access is on.");
    if (r.ok) { $("password").value = ""; stopped = false; return connect(); }
    const m = await r.json().catch(() => ({}));
    const why = { wrong: "Wrong password.", locked: "Too many wrong passwords." }[m.reason] ?? "Sign-in failed.";
    showLogin(why + (m.retry ? ` Try again in ${m.retry} s.` : ""));
  });

  async function signOut() {
    const was = ws; ws = null; was?.close();
    await fetch("/auth/logout", { method: "POST" }).catch(() => {});
    showLogin();
  }

  // A page Safari brings back from memory connects again, and so learns the current build.
  addEventListener("pageshow", (e) => { if (e.persisted && ws?.readyState !== WebSocket.OPEN) connect(); });

  // ---------- updates ----------
  // terminumd names the build of the page's files first on every connection. A page of another
  // build (open across an upgrade, or restored from memory) reloads once, but never while a
  // Files or View frame has unsaved edits: it waits for those.
  const BUILD = document.querySelector('meta[name="build"]').content;
  let updateTimer = 0;
  function checkBuild(id) {
    if (id === BUILD || updateTimer) return;
    if (sessionStorage.getItem("web.reloaded-for") === id) return status("An update is ready: reload the page.");   // no loop
    const attempt = () => {
      if (busy()) return status("An update is ready: the page reloads after you save.");
      clearInterval(updateTimer);
      try { sessionStorage.setItem("web.reloaded-for", id); } catch { /* private mode: at worst asks again */ }
      location.reload();
    };
    updateTimer = setInterval(attempt, 1000);
    attempt();
  }

  return { connect, send, signOut, checkBuild, open: () => ws?.readyState === 1 };
}
