// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
import { check } from "./check.mjs";
import { shot } from "./fake.mjs";

export const SMALL_RATE = { sample: 12, value: .25, interval_95: [.09, .53], can_gate: false, missing: null };
export const MISSING_RATE = { sample: 0, value: null, interval_95: null, can_gate: false, missing: "Project integration is off." };

export function scorecardFixture(DATE, LITERAL) {
  return {
    scorecard: {
      resolved: { count: 1, watching: 1, recurred: 0, rate: SMALL_RATE },
      decisions: { right: 1, pending: 1, rate: SMALL_RATE }, by_rule: [], delays: [], calibration: [],
      misses: { count: 1, sample: 2, recent: [{ source: "owner_bug", subject: "API-12",
        first_evidence_at: DATE, signal_state: "shadow" }] },
      overrides: { count: 0, rate: MISSING_RATE }, refusals: { count: 1, sample: 2 },
      usefulness: { count: 1, rate: SMALL_RATE }, judgment: { tokens: 600, sample: 3, per_resolution: 600 },
      fixes: { dollars: 2, sample: 1, unpriced: 1, per_resolution: null }, weekly_resolved: 1,
      owner_load: { cards: 2, pages: 1, presses: 1, timed: 1, verdict_minutes: 2.5 },
    },
    audit: { latest: { at: DATE, task: "S_API-9", report: `  Audit evidence\n\n  ${LITERAL}\n` },
      status: "No eligible slot for the latest audit.", last_due: DATE + 90 * 86400 },
  };
}

export async function checkScorecard(page, reports, suffix, LITERAL) {
  await shot(page, `supervisor-reports-${suffix}`);
  await reports.getByText("Quarterly audit ·", { exact: false }).click();
  const quarterly = reports.locator("pre");
  check("AC-259 the audit shows its exact text and the latest firing's failure",
    await quarterly.textContent() === `  Audit evidence\n\n  ${LITERAL}\n`
    && await reports.getByText("No eligible slot", { exact: false }).isVisible()
    && await reports.locator("img").count() === 0
    && await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  const scorecard = page.locator("#sv-scorecard");
  await scorecard.getByText("Latest misses").click();
  check("AC-255 AC-256 the scorecard shows sample sizes, uncertainty, missing samples and earlier suppressed evidence",
    (await scorecard.textContent()).includes("95% interval")
    && await scorecard.getByText("Unavailable · n=0", { exact: true }).isVisible()
    && await scorecard.getByText("API-12", { exact: false }).isVisible());
  await scorecard.scrollIntoViewIfNeeded();
  await shot(page, `supervisor-scorecard-${suffix}`);
}
