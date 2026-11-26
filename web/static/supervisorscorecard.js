// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
import { $, stamp } from "./kanbanutil.js";
import { el } from "./listparts.js";
import { levelText, levelTip, part, stack } from "./supervisorfields.js";

const decimal = new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 });
const money = new Intl.NumberFormat(undefined, { style: "currency", currency: "USD" });
const counted = (n, noun, plural = noun + "s") => `${n} ${n === 1 ? noun : plural}`;
const percent = value => `${decimal.format(value * 100)}%`;
const text = value => value == null ? "Unavailable" : decimal.format(value);
const rate = ({ count, rate: { sample, value, interval_95 } }) => value == null
  ? "Unavailable · n=0"
  : `${count} of ${sample} · ${percent(value)} (95% interval ${interval_95.map(percent).join("–")})`;

function reading(label, value) {
  return part("div", "sv-reading", el("dt", null, label), el("dd", null, value));
}

function scoreDetails(s) {
  const sections = [];
  if (s.by_rule.length) sections.push(part("dl", "sv-readings", ...s.by_rule.map(d => reading(
    [d.kind, d.detector, d.response].filter(Boolean).join(" · "),
    `${rate({ count: d.right, rate: d.rate })} · ${d.pending} awaiting judgment`))));
  const delays = s.delays.map(d => levelTip(reading(`Time to detect · ${levelText(d)}`, `${text(d.seconds / 60)} min average · n=${d.sample}`), d));
  sections.push(part("dl", "sv-readings", ...(delays.length ? delays : [reading("Time to detect", "No dated evidence · n=0")])));
  if (s.calibration.length) sections.push(part("dl", "sv-readings", ...s.calibration.map(c =>
    reading(`Cause confidence · ${percent(c.confidence)}`,
      c.rate ? `${percent(c.rate.value)} held · n=${c.sample}` : `Insufficient evidence · n=${c.sample}`))));
  if (!sections.length) return null;
  return part("details", "sv-panel", el("summary", "sv-touch", "Decision and detection detail"), stack(...sections));
}

function missDetails(misses) {
  if (!misses.length) return null;
  const labels = { owner_bug: "Owner Bug", target_red: "Red target", watchdog: "Watchdog" };
  const items = misses.map(m => el("li", "wrap", `${labels[m.source]} · ${m.subject} · first evidence ${stamp(m.first_evidence_at)} · ${m.signal_state ?? "No earlier signal"}`));
  return part("details", "sv-panel", el("summary", "sv-touch", "Latest misses"), part("ul", "sv-evidence", ...items));
}

export function drawScorecard(s) {
  const target = $("sv-scorecard");
  if (!s) { target.replaceChildren(); return; }
  const { resolved, decisions, misses, overrides, refusals, judgment, fixes, owner_load: load } = s;
  const spend = (sample, unit, value, per, missing = 0) =>
    `${sample ? value : "Unavailable"} · ${counted(sample, `measured ${unit}`)}${missing ? ` · ${missing} without complete pricing` : ""}${per ? ` · ${per} per resolution (n=${s.weekly_resolved})` : ""}`;
  target.replaceChildren(
    part("dl", "sv-readings",
      reading("Stayed resolved", `${rate(resolved)} · ${resolved.watching} incomplete · ${resolved.recurred} recurred`),
      reading("Decisions right", `${rate({ count: decisions.right, rate: decisions.rate })} · ${decisions.pending} awaiting judgment`),
      reading("Misses", `${counted(misses.count, "miss", "misses")} · ${counted(misses.sample, "independent observation")}`),
      reading("Overridden within 24 hours", rate(overrides)),
      reading("Findings refused", `${refusals.count} refused · ${counted(refusals.sample, "submission")}`),
      reading("Owner verdicts marked useful", rate(s.usefulness)),
      reading("Judgment spend · last 7 days", spend(judgment.sample, "run", `${text(judgment.tokens)} tokens`,
        judgment.per_resolution == null ? null : `${text(judgment.per_resolution)} tokens`)),
      reading("Fix spend · last 7 days", spend(fixes.sample, "run", money.format(fixes.dollars),
        fixes.per_resolution == null ? null : money.format(fixes.per_resolution), fixes.unpriced)),
      reading("Owner load · last 7 days",
        `${counted(load.cards, "card")} · ${counted(load.pages, "page", "pages")} delivered · ${counted(load.presses, "verdict press", "verdict presses")} · ${text(load.verdict_minutes)} verdict minutes (${load.timed} timed)`)),
    el("p", "msay", "Stayed resolved measures recurrence after a fix landed. Rates below 30 observations are advisory."),
    ...[scoreDetails(s), missDetails(misses.recent)].filter(Boolean));
}
