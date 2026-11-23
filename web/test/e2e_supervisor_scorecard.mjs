// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
import { check } from "./check.mjs";
import { shot } from "./fake.mjs";

export async function outsideSettings(page, panel, suffix, phone) {
  const indicators = panel.locator('details[data-section="Indicators"]');
  await indicators.locator("summary").click();
  const add = indicators.getByRole("button", { name: "Add outside indicator" });
  await add.click();
  const title = indicators.getByLabel("Indicator", { exact: true });
  check("AC-262 adding an outside indicator focuses its name", await title.evaluate((node) => node === document.activeElement));
  await title.fill("Downloads");
  await indicators.getByLabel("Probe ID", { exact: true }).fill("store");
  await indicators.getByLabel("Metric ID", { exact: true }).fill("downloads");
  await indicators.getByLabel("Guard metric ID", { exact: true }).fill("errors");
  await indicators.getByLabel("Guard direction", { exact: true }).selectOption("up");
  await title.scrollIntoViewIfNeeded();
  await shot(page, `supervisor-outside-settings-${suffix}`);
  await panel.getByText("Feedback probes", { exact: true }).click();
  const addFeedback = panel.getByRole("button", { name: "Add feedback probe" });
  await addFeedback.click();
  await panel.getByLabel("Feedback probe ID", { exact: true }).fill("feedback");
  await panel.getByLabel("HTTPS reference base", { exact: true }).fill("https://example.com/issues/");
  await addFeedback.scrollIntoViewIfNeeded();
  check("AC-262 the feedback probe form fits alongside its settings",
    await addFeedback.evaluate((node, touch) => {
      const box = node.getBoundingClientRect();
      return box.left >= 0 && box.right <= innerWidth && (!touch || box.height >= 40);
    }, phone));
  await shot(page, `supervisor-feedback-settings-${suffix}`);
}

export async function checkOutsideSave(panel, settings) {
  check("AC-222 the review date survives a save in a timezone west of UTC",
    await panel.getByLabel("Review priorities on").inputValue() === "2027-01-15");
  const downloads = settings.charter.outside_indicators.Downloads;
  check("AC-262 the settings save sends the metric guard and approved probe references",
    downloads.probe === "store" && downloads.metric === "downloads" && downloads.higher_is_better
    && downloads.guard === "errors" && downloads.guard_higher_is_better
    && settings.probes.feedback.feedback === "https://example.com/issues/");
}

export const SMALL_RATE = { sample: 12, value: .25, interval_95: [.09, .53], can_gate: false, missing: null };
export const MISSING_RATE = { sample: 0, value: null, interval_95: null, can_gate: false, missing: "Project integration is off." };

export function scorecardFixture(DATE, LITERAL) {
  return {
    scorecard: {
      resolved: { count: 3, watching: 1, recurred: 0, rate: SMALL_RATE },
      decisions: { right: 3, pending: 1, rate: SMALL_RATE }, by_rule: [], delays: [], calibration: [],
      misses: { count: 1, sample: 2, recent: [{ source: "owner_bug", subject: "API-12",
        first_evidence_at: DATE, signal_state: "shadow" }] },
      overrides: { count: 0, rate: MISSING_RATE }, refusals: { count: 1, sample: 2 },
      usefulness: { count: 3, rate: SMALL_RATE }, judgment: { tokens: 600, sample: 3, per_resolution: 600 },
      fixes: { dollars: 2, sample: 1, unpriced: 1, per_resolution: null }, weekly_resolved: 1,
      owner_load: { cards: 2, pages: 1, presses: 1, timed: 1, verdict_minutes: 2.5 },
    },
    audit: { latest: { at: DATE, task: "S_API-9", report: `  Audit evidence\n\n  ${LITERAL}\n` },
      status: "No eligible slot for the latest audit.", last_due: DATE + 90 * 86400 },
    outside_indicators: [{ name: "Downloads", probe: "store", verdict: "worse", guard: "Guard metric · failures",
      metrics: [{ name: "downloads", unit: "count", verdict: "better", missing: null, before: 10, current: 20, noise: 0 },
        { name: "failures", unit: "count", verdict: "worse", missing: null, before: 1, current: 3, noise: 0 }] }],
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
  await shot(page, `supervisor-audit-${suffix}`);
  const scorecard = page.locator("#sv-scorecard");
  await scorecard.getByText("Latest misses").click();
  check("AC-255 AC-256 the scorecard shows sample sizes, uncertainty, missing samples and earlier suppressed evidence",
    (await scorecard.textContent()).includes("95% interval")
    && await scorecard.getByText("Unavailable · n=0", { exact: true }).isVisible()
    && await scorecard.getByText("API-12", { exact: false }).isVisible());
  await scorecard.scrollIntoViewIfNeeded();
  await shot(page, `supervisor-scorecard-${suffix}`);
  const outside = page.locator("#sv-score details").filter({ hasText: "Downloads · store — worse" });
  await outside.locator("summary").click();
  await outside.getByText("Guard metric · failures", { exact: true }).waitFor();
  check("AC-262 the scoreboard names the outside probe and shows its guard and units",
    await outside.getByText("Guard metric · failures", { exact: true }).isVisible()
    && await outside.getByText("downloads (count)", { exact: true }).isVisible()
    && await outside.evaluate((node) => node.getBoundingClientRect().right <= innerWidth));
  await shot(page, `supervisor-outside-scoreboard-${suffix}`);
}
