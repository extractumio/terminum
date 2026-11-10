// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A schedule's When picker (terminum-schedules.md AC-160): each kind writes its cron fields and opens again as itself.
import { test } from "node:test";
import assert from "node:assert/strict";
import { fromCron, localSeconds, toCron } from "../static/cadence.js";

test("AC-160 each kind writes its five fields", () => {
  assert.equal(toCron({ kind: "minutes", every: 15 }), "*/15 * * * *");
  assert.equal(toCron({ kind: "hours", every: 2, minute: 5 }), "5 */2 * * *");
  assert.equal(toCron({ kind: "daily", time: "03:00" }), "0 3 * * *");
  assert.equal(toCron({ kind: "weekdays", time: "09:30" }), "30 9 * * 1-5");
  assert.equal(toCron({ kind: "weekly", time: "18:05", days: [5, 1, 3] }), "5 18 * * 1,3,5");
  assert.equal(toCron({ kind: "monthly", time: "08:00", day: 1 }), "0 8 1 * *");
  assert.equal(toCron({ kind: "once" }), null);
  assert.equal(toCron({ kind: "cron", cron: "  0  4 * *   0 " }), "0 4 * * 0");
});

test("AC-160 fields of a kind's shape open in that kind, any other in Cron", () => {
  for (const s of [{ kind: "minutes", every: 30 }, { kind: "hours", every: 3, minute: 15 }, { kind: "daily", time: "03:00" }, { kind: "weekdays", time: "09:30" },
    { kind: "weekly", time: "18:05", days: [1, 3, 5] }, { kind: "monthly", time: "08:00", day: 28 }]) {
    const back = fromCron(toCron(s));
    assert.equal(back.kind, s.kind, toCron(s));
    assert.equal(toCron(back), toCron(s));
  }
  assert.equal(fromCron("*/7 * * * *").kind, "cron", "a step the picker does not offer");
  assert.equal(fromCron("0 9 1 6 *").kind, "cron", "a month named");
  assert.equal(fromCron("0 9 1-7 * 1").kind, "cron", "a range of days");
  assert.equal(fromCron("bad").kind, "cron");
  assert.deepEqual(fromCron("0 0 * * 7").days, [0], "7 is Sunday");
  assert.equal(fromCron(null, 1800000000).kind, "once");
});

test("AC-160 Once's date and time are local", () => {
  const at = localSeconds("2026-11-12", "09:00");
  const d = new Date(at * 1000);
  assert.deepEqual([d.getFullYear(), d.getMonth(), d.getDate(), d.getHours(), d.getMinutes()], [2026, 10, 12, 9, 0]);
  assert.equal(localSeconds("", "09:00"), null);
});
