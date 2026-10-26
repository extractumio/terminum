// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A session row's plan (AC-97): "43%" right of its title, a thin track under its text in the row's
// own color (planpane.css). A press on the percent shows the session and opens its Plan pane.
import { pctText, rowTitle } from "./plans.js";

const span = (cls, text = "") => { const e = document.createElement("span"); e.className = cls; e.textContent = text; return e; };

/** The percent; onPick(id) shows the row's session first (the pane is the shown session's). */
export function planPct(item, onPick) {
  const p = span("plpct" + (item.plan.stale ? " stale" : "") + (item.plan.partial ? " partial" : ""), pctText(item.plan));
  p.title = rowTitle(item.plan, Date.now() / 1000);
  p.addEventListener("click", (e) => {               // inside the row's button: no button of its own
    e.stopPropagation();
    onPick(item.id);
    document.dispatchEvent(new Event("planopen"));
  });
  return p;
}

export function planTrack(plan) {
  const t = span("pltrack" + (plan.stale ? " stale" : ""));
  t.append(span("plfill"));
  t.style.setProperty("--pct", `${Math.max(0, Math.min(100, plan.pct ?? 0))}%`);
  t.setAttribute("aria-hidden", "true");
  return t;
}
