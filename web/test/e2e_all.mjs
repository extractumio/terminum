// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Every browser check of the web page (web/static), against fakes of terminumd played in the
// browser (no server, no tmux), in Chromium and in WebKit; the checks that depend on the size
// run at a wide (1400×900) and a phone (390×844, touch) size themselves.
//
//   node web/test/e2e_all.mjs [chromium|webkit] [check…]     (needs `cd ui && npm ci && npm run build`)

import { check, playwright, results, runningIn } from "./check.mjs";
import { webApp } from "./e2e_app.mjs";
import { webBoxNav } from "./e2e_boxnav.mjs";
import { webClip } from "./e2e_clip.mjs";
import { webCodex } from "./e2e_codex.mjs";
import { webCtxTrace } from "./e2e_ctxtrace.mjs";
import { webEdit } from "./e2e_edit.mjs";
import { webLook } from "./e2e_look.mjs";
import { webMarks } from "./e2e_marks.mjs";
import { webMotion } from "./e2e_motion.mjs";
import { webMove } from "./e2e_move.mjs";
import { webScreen } from "./e2e_screen.mjs";
import { webSignin } from "./e2e_signin.mjs";
import { webSent } from "./e2e_sent.mjs";
import { webSplit } from "./e2e_split.mjs";
import { webTerm } from "./e2e_term.mjs";
import { webTmux } from "./e2e_tmux.mjs";
import { webTouch } from "./e2e_touch.mjs";
import { webTrace } from "./e2e_trace.mjs";
import { webWidgets } from "./e2e_widgets.mjs";
import { webIdeKeys } from "./e2e_idekeys.mjs";
import { webScreenFolds } from "./e2e_screenfolds.mjs";
import { webProjects } from "./e2e_projects.mjs";

const CHECKS = { webTerm, webApp, webMove, webTrace, webCtxTrace, webClip, webWidgets, webMarks, webTouch, webEdit, webBoxNav,
  webSignin, webTmux, webScreen, webLook, webMotion, webCodex, webSplit, webSent, webIdeKeys, webScreenFolds, webProjects };
const BROWSERS = ["chromium", "webkit"];
// Checks that drive the page with Chromium's own tools: a CDP touch screen, synthetic Touch events.
const CHROMIUM_ONLY = { webClip: "synthetic Touch events", webTouch: "a CDP touch screen" };

const args = process.argv.slice(2);
const browsers = args.filter((a) => BROWSERS.includes(a));
const only = args.filter((a) => !BROWSERS.includes(a));
const pw = await playwright();
for (const name of browsers.length ? browsers : BROWSERS) {
  const browser = await pw[name].launch();
  try {
    for (const [which, run] of Object.entries(CHECKS)) {
      if (only.length && !only.includes(which)) continue;
      if (name !== "chromium" && CHROMIUM_ONLY[which]) { console.log(`SKIP [${name}] ${which}: needs ${CHROMIUM_ONLY[which]}`); continue; }
      runningIn(name);
      try { await run(browser, name); } catch (e) { check(`${which} ran to its end`, false, String(e.stack ?? e).split("\n").slice(0, 3).join(" | ")); }
      for (const ctx of browser.contexts()) await ctx.close();   // a page a failed check left open
    }
  } finally { await browser.close(); }
}
runningIn("");
const by = new Map();
for (const r of results) {
  const k = r.where.trim() || "-";
  const n = by.get(k) ?? { pass: 0, fail: 0 };
  n[r.ok ? "pass" : "fail"]++;
  by.set(k, n);
}
for (const [k, n] of by) console.log(`${k}: ${n.pass} passed, ${n.fail} failed`);
const failed = results.filter((r) => !r.ok).length;
console.log(failed ? `FAIL (${failed} of ${results.length})` : `PASS (${results.length} checks)`);
process.exit(failed ? 1 : 0);
