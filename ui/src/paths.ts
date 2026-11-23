// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// POSIX path and URL helpers. No DOM and no token: modules tested under node import these.

export const isUnder = (p: string, dir: string) => p === dir || p.startsWith(dir === "/" ? "/" : dir + "/");
/** `p` after `from` was renamed to `to` (unchanged if not inside `from`). */
export const rebase = (p: string, from: string, to: string) => (isUnder(p, from) ? to + p.slice(from.length) : p);

export const basename = (p: string) => p.slice(p.lastIndexOf("/") + 1) || "/";
export const dirname = (p: string) => p.slice(0, Math.max(p.lastIndexOf("/"), 1));
export const join = (a: string, b: string) => (a === "/" ? "/" + b : a + "/" + b);

/** Resolve a relative link against a folder, POSIX-style. */
export function resolvePath(dir: string, rel: string): string {
  const parts = (rel.startsWith("/") ? rel : dir + "/" + rel).split("/");
  const out: string[] = [];
  for (const p of parts) {
    if (!p || p === ".") continue;
    if (p === "..") out.pop(); else out.push(p);
  }
  return "/" + out.join("/");
}

/** A URL with a scheme (`https:`, `javascript:`, `file:` …), as opposed to a relative path. */
export const hasScheme = (u: string) => /^[a-z][a-z0-9+.-]*:/i.test(u);

/** Decode a file URL while accepting a literal percent sign in ordinary filenames. */
export function decodePath(url: string): string {
  try { return decodeURIComponent(url); } catch { return url; }
}

/** A folder typed by the user (AC-275): a full path, `~` or `~/…` from the pane's home, or a
 *  path relative to `base`; `.` and `..` resolved (the page reads a printed path by the same rule:
 *  web/static/links.js absolutePath). Why not, when it cannot be one. */
export function typedFolder(text: string, base: string, home: string | null): { path: string } | { error: string } {
  const t = text.trim();
  if (!t) return { error: "Type a folder's path" };
  if (t === "~" || t.startsWith("~/")) {
    if (!home) return { error: "The pane's home is not known here: type a full path" };
    return { path: resolvePath(home, t.slice(2)) };
  }
  if (t.startsWith("~")) return { error: "Only your own home (~ or ~/…) is known here: type a full path" };
  return { path: resolvePath(base || "/", t) };
}
