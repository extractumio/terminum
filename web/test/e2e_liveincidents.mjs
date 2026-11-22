// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-224–226, AC-165: a real watchdog signal and owner keys on a private terminumd.
import path from "node:path";
import { boardKit, intoStore } from "./boardkit.mjs";
import { check, playwright, until } from "./check.mjs";
import { NO_SIGNALS } from "../../ui/test/process-stop.mjs";

const FEED = "/api/supervisor/incidents";
const BAR = ".incident-bar:popover-open";

export async function liveIncidents(page, opts) {
  const { base, app, signIn, shots } = opts;
  const { j, open } = boardKit(page, opts);
  const made = await j("/api/task-projects", "POST", { name: "Incident drill" });
  if (!made.id) throw new Error(made.error);
  const project = made.id;
  const api = `/api/task-projects/${project}/supervisor`;
  const ordinary = await j("/api/tasks", "POST", { project, title: "Ordinary waiting task" });
  const moved = await j(`/api/tasks/${ordinary.id}`, "PATCH", { status: "WAIT FOR HUMAN" });
  if (moved.code !== 200) throw new Error(moved.error);
  await open(`?project=${project}`);

  const wideName = page.context().browser().browserType().name();
  const phoneName = wideName === "chromium" ? "webkit" : "chromium";
  const pw = await playwright();
  const other = await pw[phoneName].launch(NO_SIGNALS);
  const filesContext = await page.context().browser().newContext();
  const files = await filesContext.newPage();
  let stalled = false;
  try {
    const context = await other.newContext({ viewport: { width: 390, height: 844 }, hasTouch: true,
      isMobile: phoneName === "chromium", reducedMotion: "reduce" });
    const phone = await context.newPage();
    await signIn(phone);
    await phone.goto(`${base}/kanban/?project=${project}`);
    await phone.locator("#newtask").click();
    await phone.locator("#newtitle").fill("Draft beside the drill");
    await signIn(files);
    await files.goto(`${base}/fb/`);
    await j(api, "PUT", { enabled: true });
    if (!await until(async () => Boolean((await j(api)).last_ok), 30000)) throw new Error("The drill's collector did not start.");

    const start = Date.now();
    intoStore(app, `CREATE TRIGGER stall_incident_collector BEFORE UPDATE OF last_ok ON project_supervisor
      WHEN NEW.project = '${project}' BEGIN SELECT RAISE(ABORT, 'test collector stopped'); END;`);
    stalled = true;
    await Promise.all([page, phone, files].map((p) => p.locator(BAR).getByText("Incident drill", { exact: false })
      .waitFor({ timeout: 60000 })));
    check("AC-225 the live Level 3 bar reached both signed-in browsers and standalone Files within one minute",
      Date.now() - start < 60000, `${Math.round((Date.now() - start) / 1000)} seconds · ${wideName} and ${phoneName}`);

    await page.locator('#colswitch button[data-status="WAIT FOR HUMAN"]').click();
    const pinned = page.locator('.col[data-status="WAIT FOR HUMAN"] .cards > :first-child .incident-card');
    const id = await pinned.getAttribute("data-incident");
    const timeline = `${FEED}/${id}`;
    check("AC-225 the incident is pinned before an ordinary waiting task",
      Boolean(id) && await page.locator('.col[data-status="WAIT FOR HUMAN"] .cards > .card').count() === 1);
    const material = page.locator(BAR).locator(`[data-incident="${id}"]`);
    check("AC-225 fixed words give the measured failure, hold, requested action and disposition in order",
      /missed \d+ heartbeats.*No hold in effect\. Acknowledge when seen\. No automatic action/s.test(await material.innerText()));

    const buttons = phone.locator(BAR).getByRole("button");
    const ink = await phone.locator(BAR).locator(".incident-brief").evaluate((brief) =>
      getComputedStyle(brief).color === getComputedStyle(brief.closest(".incident-bar")).color);
    check("AC-225 the critical text keeps its contrast inside a phone dialog", ink);

    const position = await buttons.evaluateAll((keys) => keys.map((key) => {
      const r = key.getBoundingClientRect();
      return r.width >= 40 && r.height >= 40 && r.left >= 0 && r.right <= innerWidth;
    }));
    check("AC-225 the two phone keys fit in the viewport and each has a touch-size hit area",
      position.length === 2 && position.every(Boolean), JSON.stringify(position));
    check("AC-165 arrival leaves exactly one dialog and keeps the draft's focus",
      await phone.locator("dialog[open]").count() === 1 && await phone.locator("#newtitle").evaluate((f) => f === document.activeElement));
    if (shots) {
      await page.screenshot({ path: path.join(shots, `incident-${wideName}-wide.png`) });
      await phone.screenshot({ path: path.join(shots, `incident-${phoneName}-phone.png`) });
    }

    const unsigned = await fetch(`${base}${timeline}`, { method: "POST",
      headers: { "Content-Type": "application/json" }, body: JSON.stringify({ action: "acknowledge" }) });
    check("AC-226 the unsigned action is refused without an audit entry",
      unsigned.status === 401 && !(await j(timeline)).rows.some((e) => e.actor === "human"));

    const signedOut = await files.evaluate(async () => {
      const r = await fetch("/auth/logout", { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" });
      dispatchEvent(new Event("pageshow"));
      return r.status;
    });
    await files.locator(BAR).waitFor({ state: "hidden", timeout: 15000 });
    check("AC-226 revoked sign-in clears the existing notice in standalone Files", signedOut === 204);

    const screen = await page.context().newPage();
    await screen.goto(`${base}${await pinned.getByRole("link", { name: "Timeline" }).getAttribute("href")}`);
    const entry = screen.locator(`#incident-${id}`);
    await entry.locator(".incident-events li").filter({ hasText: /Level 3.*missed \d+ heartbeats/ }).waitFor();
    check("AC-225 the Supervisor screen opens the same incident's timeline above the scoreboard",
      await entry.isVisible() && await screen.locator("#sv-content > section").first().getByRole("heading", { level: 2 }).textContent() === "Incidents");

    await phone.locator(BAR).getByRole("button", { name: "Snooze 1 hour" }).click();
    const snooze = await until(async () => {
      const primary = await j(timeline);
      const event = primary.rows.find((e) => e.kind === "snoozed");
      return event?.actor === "human" && event.at > 0;
    });
    check("AC-226 phone Snooze keeps the bar and records who and when",
      snooze && await phone.locator(BAR).getByText(/Repeats snoozed until/).isVisible());
    await phone.locator(BAR).getByRole("button", { name: "Acknowledge" }).click();
    await Promise.all([page, phone, screen].map((p) => p.locator(BAR).waitFor({ state: "hidden", timeout: 15000 })));
    const owner = (await j(timeline)).rows.filter((e) => e.actor === "human");
    check("AC-226 phone Acknowledge clears the bar across pages and records one owner action",
      owner.map((e) => e.kind).join() === "acknowledged,snoozed" && owner[0].at >= owner[1].at && await pinned.isVisible());
    check("AC-165 owner actions leave the draft dialog and typed text intact",
      await phone.locator("dialog[open]").count() === 1 && await phone.inputValue("#newtitle") === "Draft beside the drill");
    await phone.keyboard.press("Escape");
    await phone.locator("#newbox").waitFor({ state: "hidden" });
    await phone.locator("#newtask").click();
    check("AC-165 Escape closes just the draft and reopening restores it",
      await phone.locator("dialog[open]").count() === 1 && await phone.inputValue("#newtitle") === "Draft beside the drill");

    intoStore(app, "DROP TRIGGER stall_incident_collector");
    stalled = false;
    await entry.getByText(/All-clear\. Every linked signal cleared\./).waitFor({ timeout: 30000 });
    check("AC-224 collector recovery appends the all-clear to the same timeline and unpins the acknowledged card",
      await entry.isVisible() && await until(async () => !(await pinned.count()), 15000));
    await screen.close();
  } finally {
    if (stalled) intoStore(app, "DROP TRIGGER stall_incident_collector");
    await Promise.all([filesContext.close(), other.close(), j(api, "PUT", { enabled: false }),
      j(`/api/tasks/${ordinary.id}`, "PATCH", { status: "DONE" })]);
  }
}
