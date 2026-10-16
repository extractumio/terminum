// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The web page's browser checks share these helpers: a check prints PASS or FAIL with the
// browser and size it ran in (e2e_all.mjs sets them), and results counts them per run.

export const results = [];
let where = "";
/** The browser and size the next checks run in, e.g. "webkit 390×844"; "" for none. */
export const runningIn = (label) => { where = label ? `[${label}] ` : ""; };
export const check = (name, ok, extra = "") => {
  results.push({ ok: !!ok, where });
  console.log(`${ok ? "PASS" : "FAIL"} ${where}${name}${extra ? " — " + extra : ""}`);
};
export const within = async (name, fn, ms = 5000) => {
  const t0 = Date.now();
  try { await fn(ms); check(name, true, `${Date.now() - t0} ms`); } catch (e) { check(name, false, e.message.split("\n")[0]); }
};
/** Nothing to stop: the web checks run against fakes in the browser (no server, no tmux). */
export const cleanup = () => {};
/** playwright-core, ui/'s devDependency (PLAYWRIGHT_CORE may name another copy). */
export const playwright = () => import(process.env.PLAYWRIGHT_CORE ?? new URL("../../ui/node_modules/playwright-core/index.mjs", import.meta.url).href);
