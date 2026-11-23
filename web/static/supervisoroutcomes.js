// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
import { oneAtATime } from "./dialogs.js";
import { $, api, button, showError, stamp } from "./kanbanutil.js";
import { el, textKey } from "./listparts.js";
import { taskUrl } from "./tasklink.js";
import { actions, part, stack } from "./supervisorfields.js";

function config(c) {
  const [type, value] = Object.entries(c.threshold)[0];
  const threshold = typeof value === "number" ? value : Object.entries(value).map(([name, n]) => `${name} ${n}`).join(", ");
  const allocation = Object.entries(c.allocation).map(([kind, share]) => `${kind} ${Math.round(share * 100)}%`).join(", ");
  return [c.mode, `${c.window_days}-day window`, `${type} threshold ${threshold}`, c.muted && "muted", allocation].filter(Boolean).join(" · ");
}

function breaker(b, path, refresh) {
  const panel = part("details", "sv-panel", el("summary", "sv-touch", `${b.detector} / ${b.response} · Breaker open`));
  const error = el("p", "merr");
  error.setAttribute("role", "alert");
  error.hidden = true;
  const card = stack(), keys = actions();
  const url = `${path}/breakers/${b.id}`;
  keys.append(button("textkey", "Review rule change", oneAtATime(panel, async () => {
    showError(error, "");
    card.replaceChildren();
    try {
      const { review, proof } = await api(`${url}/review`, { method: "POST", body: {} });
      const restore = button("textkey", "Restore autonomy", oneAtATime(card, async () => {
        showError(error, "");
        try {
          await api(`${url}/restore`, { method: "POST", body: { proof } });
          await refresh();
          $("sv-outcomes-title").focus();
        } catch (e) { showError(error, e.message); }
      }));
      card.append(el("h3", null, "Rule change and replay"), part("dl", "sv-readings",
        part("div", "sv-reading", el("dt", null, "When the breaker opened"), el("dd", null, config(review.before))),
        part("div", "sv-reading", el("dt", null, "Current settings"), el("dd", null, config(review.after))),
        part("div", "sv-reading", el("dt", null, "Replay firings"),
          el("dd", null, `${review.before_firings} before · ${review.after_firings} after`))),
      el("p", "msay", `Retained history ${stamp(review.first)} to ${stamp(review.as_of)}.`), actions(restore));
      restore.focus();
    } catch (e) { showError(error, e.message); }
  })));
  const cards = b.cards.map((id) => {
    const link = el("a", "textkey", id);
    link.href = taskUrl(id);
    textKey(link);
    return link;
  });
  panel.append(stack(el("p", "msay", `Opened ${stamp(b.opened_at)}. The rule’s holds, runs and alerts now ask for a decision.`),
    ...(cards.length ? [part("div", "sv-actions", ...cards)] : []), keys, card, error));
  return panel;
}

export function drawRuleOutcomes(data, path, refresh) {
  const opened = data.breakers ?? [];
  $("sv-breakers").replaceChildren(...(opened.length ? opened.map((b) => breaker(b, path, refresh))
    : [el("p", "msay", "No open breakers.")]));
  const outcomes = data.outcomes ?? [];
  $("sv-outcomes").replaceChildren(...(outcomes.length ? [part("dl", "sv-readings", ...outcomes.map((s) =>
    part("div", "sv-reading", el("dt", null, s.response ? `${s.detector} / ${s.response}` : "Unattributed"),
      el("dd", null, `${s.right} right · ${s.wrong} wrong · ${s.undecided} undecided · ${s.pending} pending`))))]
    : [el("p", "msay", "No decisions recorded.")]));
}
