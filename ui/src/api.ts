// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Thin client for terminumd's Files API under /fb/api (terminum-files.md §9). The loopback
// page's sign-in comes from this origin's storage, where the page put it (AC 09); the web
// listener's page holds none: its HttpOnly cookie goes by itself.

/** This tab's storage, which survives in-page reloads; private mode may refuse it. */
export const session = {
  get: (k: string) => { try { return sessionStorage.getItem(k); } catch { return null; } },
  set: (k: string, v: string | null) => {
    try { if (v == null) sessionStorage.removeItem(k); else sessionStorage.setItem(k, v); } catch { /* private mode */ }
  },
};

/** The panel lives under /fb/ beside the page; its API is /fb/api/*. */
export const BASE = typeof location === "undefined" ? "" : location.pathname.replace(/\/[^/]*$/, "");
export const PROXIED = BASE !== "";
/** The Mac's own browser (the loopback listener answers only to 127.0.0.1): Finder, default
 *  apps and typing into the terminal are offered there only (terminumd refuses them elsewhere). */
export const AT_MAC = typeof location !== "undefined" && location.hostname === "127.0.0.1";

/** The loopback sign-in the page stored (null: the web listener's cookie signs in). */
const SECRET: string | null = (() => { try { return localStorage.getItem("tm.session"); } catch { return null; } })();
const utf8 = (s: string) => new TextEncoder().encode(s);
const hex = (b: ArrayBuffer | Uint8Array) => Array.from(new Uint8Array(b), (x) => x.toString(16).padStart(2, "0")).join("");
async function hmac(key: BufferSource, text: string): Promise<Uint8Array<ArrayBuffer>> {
  const k = await crypto.subtle.importKey("raw", key, { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  return new Uint8Array(await crypto.subtle.sign("HMAC", k, utf8(text)));
}
/** What terminumd answers to nonce `n` when it knows sign-in `secret` (terminumd/src/auth/signin.rs):
 *  HMAC-SHA256 keyed by the proof key HMAC-SHA256(secret, "tm-proof-key"). A sign-in is stored
 *  only on 127.0.0.1, a secure context, so crypto.subtle is there. */
const proof = async (secret: string, n: string) => hex(await hmac(await hmac(utf8(secret), "tm-proof-key"), `tm-hello-v1:${n}`));

/** terminumd proved, since the last time it was out of reach, that it knows the sign-in: only
 *  then does the secret go out. While terminumd is not running another program may hold its
 *  port; it cannot answer the proof, so it never sees the secret (AC 09). */
let proven: Promise<void> | null = null;
let isProven = false;
let refusal = "";  // why the last proof failed, as the user is told
export function proveServer(): Promise<void> {
  if (!SECRET) { isProven = true; return Promise.resolve(); }
  if (!proven) {
    const secret = SECRET;
    const n = hex(crypto.getRandomValues(new Uint8Array(16)));
    const mine: Promise<void> = crypto.subtle.digest("SHA-256", utf8(secret)).then(async (digest) => {
      const id = hex(digest).slice(0, 16);
      const res = await fetch(`/api/hello?n=${n}&id=${id}`, { cache: "no-store" });  // a network error: not reachable
      const body = await res.json().catch(() => null);
      if (res.status === 404 && body?.error === "unknown_signin") throw new ApiError(401, "signed_out", "Signed out: run terminum open", body);
      if (!res.ok || body?.proof !== await proof(secret, n)) {
        throw new ApiError(401, "unproven", "terminumd does not prove it knows this page's sign-in, or another program holds its port", body);
      }
      if (proven === mine) isProven = true;  // not a proof that was forgotten while it ran
    });
    mine.catch((e) => { if (e instanceof ApiError) refusal = e.message; if (proven === mine) forgetProof(); });
    proven = mine;
  }
  return proven;
}
/** terminumd went out of reach: prove it again before the secret goes out. */
export function forgetProof() { proven = null; isProven = false; }
/** Why the last proof failed, as the user is told. */
export const proofRefusal = () => refusal;
/** Names this panel in workspace events, so it can ignore the echo of its own writes. */
export const CLIENT = Math.random().toString(36).slice(2, 10);

export interface Row { n: string; k: "d" | "f" | "l" | "L"; s?: number }
export interface Page {
  status: "ready" | "loading" | "error"; total: number; writable: boolean;
  gen: number; entries: Row[]; error?: string; located?: number;
}
/** A changed file and its etag now (null: gone), as sent in fs-change events. */
export interface Stamp { path: string; etag: string | null }
export interface FsChange { dirs: string[]; files: Stamp[]; moved: { from: string; to: string }[]; host?: string }
export interface Tab { path: string; view: "auto" | "rendered" | "source" }
export interface Pane {
  rev: number; root: string; expanded: string[]; selected: string[]; scroll: number;
  tabs: Tab[]; active_tab: number | null; updated: number;
}
export interface Theme {
  bg?: string; fg?: string; sel?: string; selfg?: string; cursor?: string; link?: string;
  ansi?: string[]; font?: string | null; size?: number | null; dark?: boolean;
}
export interface TermState {
  version: number; bridge: boolean; key?: string; session?: string; title?: string;
  mode?: string; note?: string; job?: string; busy?: boolean; cwd?: string | null; stale?: boolean; theme?: Theme;
  build?: string;   // terminumd's build: another one than this page's means an upgrade (AC-16)
  host?: string;    // a remote pane's host: its files are reached through that host's agent (AC 82)
  /** a remote pane's host as offered to the user (AC 83): key = its name */
  remote?: {
    key: string; name: string; state: "ask" | "dismissed" | "enabling" | "connecting" | "updating" | "up" | "down";
    updated?: string;  // the build its helper was just updated to (AC-42)
    why?: string;      // why the enabling ticked in Add host failed (AC 83)
  };
}
export interface FileView {
  path: string; size: number; etag: string; binary: boolean; mime: string | null;
  truncated: boolean; utf8: boolean; writable: boolean; text?: string;
}
export interface Prefs { hidden: boolean; split: number }
export const DEFAULT_PREFS: Prefs = { hidden: true, split: 0.5 };

export class ApiError extends Error {
  constructor(public status: number, public code: string, message: string, public body: any) { super(message); }
}

type Query = Record<string, string | number | boolean | undefined>;

/** `host`: the machine the request is for (null: this Mac); default the shown pane's. An
 *  action that waited for the user (a dialog) passes the host it started on (AC-37). */
export async function api<T>(method: string, path: string, opts: { query?: Query; body?: unknown; headers?: Record<string, string>; retry?: boolean; host?: string | null } = {}): Promise<T> {
  const qs = new URLSearchParams();
  for (const [k, v] of Object.entries(opts.query ?? {})) if (v !== undefined) qs.set(k, String(v));
  const url = BASE + path + (qs.size ? `?${qs}` : "");
  const host = opts.host === undefined ? scope : opts.host;
  const headers: Record<string, string> = { ...(SECRET ? { "X-TM-Session": SECRET } : {}), "X-TM-Client": CLIENT, ...(host ? { "X-TM-Host": host } : {}), ...opts.headers };
  if (opts.body !== undefined) headers["Content-Type"] = "application/json";
  let res: Response;
  for (let waited = 0, step = 250; ; waited += step, step *= 2) {
    try {
      await proveServer();
      res = await fetch(url, { method, headers, body: opts.body === undefined ? undefined : JSON.stringify(opts.body) });
      break;
    } catch (e) {
      if (e instanceof ApiError) throw e;  // not proven: the secret is never sent, nor retried
      forgetProof();
      // reads ride out a backend restart (AC-34); writes fail at once, never repeated behind the user
      if (method !== "GET" || opts.retry === false || waited >= 3000) throw new ApiError(0, "network", "terminumd not reachable (restarting?)", null);
      await new Promise((r) => setTimeout(r, step));
    }
  }
  if (res.status === 204 || res.status === 304) return undefined as T;
  const body = await res.json().catch(() => null);
  if (!res.ok) throw new ApiError(res.status, body?.error ?? "http", body?.message ?? `HTTP ${res.status}`, body);
  return body as T;
}

/** The host whose files the panel shows (a remote pane, AC-37), or null for this Mac. Every
 *  file request names it; terminumd sends such a request to that host's agent and nowhere else. */
export let scope: string | null = null;
export const setScope = (host: string | null) => { scope = host || null; };

/** An image's URL carries the sign-in (an <img> sends no header): none until terminumd proved itself. */
export const rawUrl = (path: string) => !isProven ? "data:," :
  `${BASE}/api/raw?path=${encodeURIComponent(path)}${SECRET ? `&t=${encodeURIComponent(SECRET)}` : ""}${scope ? `&host=${encodeURIComponent(scope)}` : ""}`;
export const socketUrl = () => `${location.protocol === "https:" ? "wss" : "ws"}://${location.host}${BASE}/api/ws`;
/** The event socket's subprotocols carry the sign-in: ["tm", secret] (none with a cookie). */
export const socketProtocols = () => (SECRET ? ["tm", SECRET] : undefined);

/** Run an API call; on failure show its message and resolve to undefined. */
export const apiOrToast = <T>(...args: Parameters<typeof api<T>>) => api<T>(...args).catch((e: Error) => { toast(e.message); return undefined; });

export const isDirKind = (k: Row["k"]) => k === "d" || k === "L";
export * from "./paths";
export function fmtSize(n?: number | null): string {
  if (n == null) return "";
  if (n < 1024) return `${n} B`;
  if (n < 1048576) return `${(n / 1024).toFixed(n < 10240 ? 1 : 0)} K`;
  if (n < 1073741824) return `${(n / 1048576).toFixed(1)} M`;
  return `${(n / 1073741824).toFixed(1)} G`;
}

export const esc = (s: string) =>
  s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]!);

let toastTimer = 0;
/** Copy text, with the old copy command where the clipboard API is refused; then say `done`. */
export async function copyText(text: string, done = `Copied ${text}`) {
  try { await navigator.clipboard.writeText(text); }
  catch {
    const ta = Object.assign(document.createElement("textarea"), { value: text });
    document.body.append(ta); ta.select(); document.execCommand("copy"); ta.remove();
  }
  toast(done);
}

export function toast(msg: string) {
  const el = document.getElementById("toast")!;
  el.textContent = msg;
  el.classList.add("on");
  clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => el.classList.remove("on"), 3500);
}
