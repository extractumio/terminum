// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The page's local sign-in (AC-09). The page served at /open/<code> carries the sign-in in a
// meta tag: it moves into this origin's storage (127.0.0.1:47900 only) and leaves the page and
// its address. Every same-origin request then carries it as X-TM-Session, an image's URL as
// ?t=, the WebSocket as its subprotocol; but only after the server proved it knows it
// (/api/hello), so another program on the port while terminumd is down never sees it. A page
// of the web listener holds none: its HttpOnly cookie goes by itself.
// Imported first by app.js, before anything fetches.

const KEY = "tm.session";
const enc = new TextEncoder();
let secret = null;
{
  const meta = document.querySelector('meta[name="tm-session"]');
  if (meta) {
    secret = meta.content || null;
    meta.remove();
    try { if (secret) localStorage.setItem(KEY, secret); } catch { /* private mode: kept for this page only */ }
    history.replaceState(null, "", "/");
  } else {
    try { secret = localStorage.getItem(KEY); } catch { secret = null; }
  }
}

/** A signed-out sign-in is dropped: it is never sent again. */
export function forget() {
  secret = null;
  proof = null;
  try { localStorage.removeItem(KEY); } catch { /* nothing kept */ }
}

const hex = (bytes) => [...new Uint8Array(bytes)].map((b) => b.toString(16).padStart(2, "0")).join("");
async function hmac(key, text) {
  const k = await crypto.subtle.importKey("raw", key, { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  return new Uint8Array(await crypto.subtle.sign("HMAC", k, enc.encode(text)));
}

/** The server's proof that it knows the secret: "ok", "bad" (another program answered, or
 *  nothing it could prove with), "unknown" (terminumd does not know this sign-in: signed out),
 *  or "down" (nothing answered). */
async function proveNow(s) {
  if (!crypto.subtle) return "bad";
  const n = hex(crypto.getRandomValues(new Uint8Array(16)));
  const id = hex(await crypto.subtle.digest("SHA-256", enc.encode(s))).slice(0, 16);
  let answer;
  try {
    const r = await realFetch(`/api/hello?n=${n}&id=${id}`, { cache: "no-store" });
    answer = await r.json().catch(() => null);
    if (r.status === 404 && answer?.error === "unknown_signin") return "unknown";
  } catch { return "down"; }
  const key = await hmac(enc.encode(s), "tm-proof-key");
  const want = hex(await hmac(key, `tm-hello-v1:${n}`));
  return typeof answer?.proof === "string" && answer.proof.toLowerCase() === want ? "ok" : "bad";
}

// One proof per connection: asked before the secret first goes out, and again after the
// connection dropped (invalidate), as another program may hold the port now.
let proof = null;
/** The proof for the current connection (asked when there is none): proveNow's answer. */
export function prove() {
  const s = secret;
  if (!s) return Promise.resolve("ok");
  if (!proof) {
    const mine = proof = proveNow(s).catch(() => "bad");
    mine.then((r) => { if (r !== "ok" && proof === mine) proof = null; });   // asked again next time
  }
  return proof;
}
/** The connection dropped: prove the server again before the secret goes out. */
export function invalidate() { proof = null; }

const realFetch = window.fetch.bind(window);
let refused = () => {};
/** fn(known) when a request of the page's is refused as signed out: a 401 (known: false, ask
 *  /auth/check why) or a sign-in terminumd does not know (known: true). */
export function onRefused(fn) { refused = fn; }

// Every same-origin request carries the sign-in, once the server proved itself; a refused one
// is reported. Requests elsewhere (none: CSP connect-src 'self') get nothing.
window.fetch = async (input, init = {}) => {
  const url = new URL(input instanceof Request ? input.url : String(input), location.href);
  if (url.origin !== location.origin) return realFetch(input, init);
  const s = secret;
  let headers = init.headers;
  if (s) {
    const p = await prove();
    if (p === "unknown") refused(true);
    if (p !== "ok") throw new TypeError(p === "unknown" ? "Signed out: run terminum open" : p === "down" ? "terminum's server is not running" : "terminum's server could not prove itself: run terminum open");
    headers = new Headers(init.headers ?? (input instanceof Request ? input.headers : undefined));
    headers.set("X-TM-Session", s);
  }
  let r;
  try { r = await realFetch(input, headers ? { ...init, headers } : init); }
  catch (e) { proof = null; throw e; }               // the server may be gone: prove the next one again
  if (r.status === 401 && url.pathname !== "/auth/check" && url.pathname !== "/auth/login") refused(false);
  return r;
};

/** A same-origin URL for an <img>, which sends no header: the sign-in as ?t= (AC-09), once the
 *  server proved itself; null when it did not (the image is not loaded). */
export async function signedUrl(url) {
  if (!secret) return url;
  const u = new URL(url, location.href);
  if (u.origin !== location.origin) return url;
  if ((await prove()) !== "ok" || !secret) return null;
  u.searchParams.set("t", secret);
  return u.pathname + u.search + u.hash;
}

/** The WebSocket's subprotocols: ["tm", secret] when the page holds one. */
export const protocols = () => (secret ? ["tm", secret] : undefined);
