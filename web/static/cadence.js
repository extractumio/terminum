// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A schedule's When (terminum-schedules.md AC-160): the kinds of cadence its window offers, each written as five
// cron fields; fields of a kind's shape open in that kind, any other in Cron. terminumd checks what is kept,
// says the next runs and the cadence in words: here only the picker's sums.

export const KINDS = [
  ["minutes", "Every N minutes"],
  ["hours", "Every N hours"],
  ["daily", "Every day"],
  ["weekdays", "Weekdays"],
  ["weekly", "Every week on"],
  ["monthly", "Every month on day"],
  ["once", "Once at"],
  ["cron", "Cron"],
];
export const MINUTE_STEPS = [1, 5, 10, 15, 20, 30, 45];
export const DAY_NAMES = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

const int = (s) => (/^\d+$/.test(s) ? Number(s) : null);
const hhmm = (h, m) => `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}`;

/** The picker's state as cron fields; Once has none (its date and time go as they are). */
export function toCron(s) {
  const [h, m] = (s.time ?? "09:00").split(":").map(Number);
  switch (s.kind) {
    case "minutes": return `*/${s.every} * * * *`;
    case "hours": return `${s.minute ?? 0} */${s.every} * * *`;
    case "daily": return `${m} ${h} * * *`;
    case "weekdays": return `${m} ${h} * * 1-5`;
    case "weekly": return `${m} ${h} * * ${[...s.days].sort((a, b) => a - b).join(",") || "1"}`;
    case "monthly": return `${m} ${h} ${s.day} * *`;
    case "once": return null;
    default: return (s.cron ?? "").trim().split(/\s+/).join(" ");
  }
}

/** The picker's state for kept cron fields, or a moment kept for Once (unix seconds). */
export function fromCron(cron, onceAt = null) {
  const base = { every: 30, minute: 0, time: "09:00", days: [1], day: 1, cron: cron ?? "", once: onceAt };
  if (onceAt != null) return { ...base, kind: "once" };
  const f = (cron ?? "").trim().split(/\s+/);
  if (f.length !== 5) return { ...base, kind: "cron" };
  const [m, h, d, mo, w] = f, step = (x) => (x.startsWith("*/") ? int(x.slice(2)) : null);
  if (mo !== "*") return { ...base, kind: "cron" };
  if (step(m) && MINUTE_STEPS.includes(step(m)) && h === "*" && d === "*" && w === "*") return { ...base, kind: "minutes", every: step(m) };
  if (int(m) != null && step(h) && d === "*" && w === "*") return { ...base, kind: "hours", every: step(h), minute: int(m) };
  if (int(m) == null || int(h) == null || int(m) > 59 || int(h) > 23) return { ...base, kind: "cron" };
  const time = hhmm(int(h), int(m));
  if (d === "*" && w === "*") return { ...base, kind: "daily", time };
  if (d === "*" && w === "1-5") return { ...base, kind: "weekdays", time };
  const days = w.split(",").map(int);
  if (d === "*" && days.every((x) => x != null && x <= 7)) return { ...base, kind: "weekly", time, days: [...new Set(days.map((x) => x % 7))] };
  if (w === "*" && int(d) != null && int(d) >= 1 && int(d) <= 31) return { ...base, kind: "monthly", time, day: int(d) };
  return { ...base, kind: "cron" };
}

/** A date input's and a time input's local values as unix seconds; none when either is empty. */
export function localSeconds(date, time) {
  if (!date || !time) return null;
  const t = new Date(`${date}T${time}`).getTime();
  return Number.isNaN(t) ? null : Math.floor(t / 1000);
}
