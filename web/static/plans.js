// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Plans and notes (AC-97, AC-98): the pure texts of a plan's row, header and pane. Times are
// terminumd's unix seconds; `now` is the page's (seconds). Nothing here touches the DOM.

const ICONS = { completed: "✓", in_progress: "▶", pending: "○", blocked: "⚠" };

/** The mark of an item's status; an unknown status reads as pending. */
export const statusIcon = (status) => ICONS[status] ?? ICONS.pending;

/** "43%" from terminumd's pct (never computed here). */
export const pctText = (plan) => `${Math.round(plan.pct ?? 0)}%`;

/** "just now", "2 min ago", "3 h ago", "2 d ago". */
export function ageText(at, now) {
  const s = Math.max(0, Math.floor(now - at));
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return `${Math.floor(s / 86400)} d ago`;
}

/** "17:40" in the page's time zone. */
export const clock = (at) => new Date(at * 1000).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", hour12: false });

/** "~17:40", or "" without an ETA. */
export const etaText = (plan) => (plan.eta ? `~${clock(plan.eta)}` : "");

/** The row's hover: "3 of 7 · ETA ~17:40 · updated 2 min ago[ · partial][ · stale]". */
export function rowTitle(plan, now) {
  const eta = etaText(plan);
  return [`${plan.done} of ${plan.total}`, eta && `ETA ${eta}`, `updated ${ageText(plan.updated, now)}`,
    plan.partial && "partial", plan.stale && "stale"].filter(Boolean).join(" · ");
}

/** The header's: "3/7 · 43% · ETA ~17:40 · 2 min ago". */
export function headerText(plan, now) {
  const eta = etaText(plan);
  return [`${plan.done}/${plan.total}`, pctText(plan), eta && `ETA ${eta}`, ageText(plan.updated, now)].filter(Boolean).join(" · ");
}

/** The pane's ETA: "~17:40 (agent's estimate, given 16:55)". */
export function etaLine(plan) {
  if (!plan.eta) return "";
  return `${etaText(plan)} (agent's estimate${plan.eta_given ? `, given ${clock(plan.eta_given)}` : ""})`;
}

/** Where the plan came from. */
export const sourceText = (source) => (source === "mcp" ? "reported through terminum's MCP" : "from the agent's plan tool");

/** Open notes first, newest first (terminumd sends them so; kept if it does not). */
export const orderNotes = (notes) => [...(notes ?? [])].sort((a, b) => Number(b.status === "open") - Number(a.status === "open") || b.at - a.at);

/** What Insert into chat pastes for a note: its kind, then its text (no Enter). */
export const noteChat = (note) => `${note.kind}: ${note.text}${note.file ? `\n(${note.file})` : ""}`;
