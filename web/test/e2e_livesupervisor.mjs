// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-222 and AC-223 through the signed-in page, the private store and its real collector.
import { boardKit } from "./boardkit.mjs";
import { check, until } from "./check.mjs";

const RULE = "flow.failed_no_reason";

export async function liveSupervisor(page, opts) {
  const { j, open } = boardKit(page, opts);

  async function putProject(name) {
    const made = await j("/api/task-projects", "POST", { name });
    if (!made.id) throw new Error(made.error);
    return { id: made.id, path: `/api/task-projects/${made.id}/supervisor` };
  }

  const api = await putProject("Supervisor API"), web = await putProject("Supervisor Web");
  const unsigned = await fetch(`${opts.base}${api.path}/screen`);
  check("AC-223 a signed-out read gets no project evidence", unsigned.status === 401, String(unsigned.status));
  const foreign = await j("/api/task-agents", "POST", { name: "Web agent", project: web.id, kind: "claude", command: "true" });
  const refused = await j(`${api.path}/settings`, "PUT", { revision: 0, settings: { tiers: { small: foreign.id } } });
  check("AC-222 the live settings API names the foreign tier without applying it",
    refused.code === 400 && /^Small:.*project's pool/.test(refused.error), refused.error);
  await j(`/api/task-agents/${foreign.id}`, "DELETE");

  await open(`?project=${api.id}`);
  await page.click("#projectcog");
  await page.click("#projectsupervisor");
  const panel = page.locator("#supervisorpanel");
  await panel.getByLabel("Level", { exact: true }).selectOption("observer");
  await panel.getByText("Charter and allocation", { exact: true }).click();
  await panel.getByLabel("Never list, one rule per line").fill("Never ship without review");
  await page.click("#supervisorok");
  check("AC-222 the live tab keeps settings in the correct project", await until(async () => {
    const a = await j(`${api.path}/settings`), b = await j(`${web.path}/settings`);
    return a.settings.level === "observer" && a.settings.charter.never[0] === "Never ship without review"
      && b.settings.level === "operator" && !b.settings.charter.never.length;
  }));

  await panel.getByText("Detectors and mutes", { exact: true }).click();
  const detector = panel.locator(`details[data-section="${RULE}"]`);
  await panel.getByText(RULE, { exact: true }).click();
  await detector.getByLabel("Threshold", { exact: true }).selectOption("absolute");
  await detector.getByLabel("Limit", { exact: true }).fill("0");
  await page.click("#supervisorok");
  check("AC-222 the live store keeps an explicit threshold", await until(async () =>
    (await j(`${api.path}/settings`)).settings.detectors[RULE]?.threshold.absolute === 0));

  await page.click("#supervisoron");
  check("AC-222 the live switch attaches its project independently",
    await until(async () => (await j(api.path)).enabled && !(await j(web.path)).enabled));

  const set = await j(`${web.path}/settings`, "PUT", { revision: 0, settings: { detectors: { [RULE]: { threshold: { absolute: 0 } } } } });
  if (set.code !== 200) throw new Error(set.error);
  await j(web.path, "PUT", { enabled: true });
  for (const project of [api, web]) {
    const task = await j("/api/tasks", "POST", { title: "Fixture failure", project: project.id });
    const moved = await j(`/api/tasks/${task.id}`, "PATCH", { status: "FAILED" });
    if (moved.code !== 200) throw new Error(moved.error);
  }

  let ours, theirs;
  check("AC-223 the real collector publishes the same detector in two projects with different evidence", await until(async () => {
    ours = (await j(`${api.path}/screen`)).signals.find((s) => s.detector === RULE);
    theirs = (await j(`${web.path}/screen`)).signals.find((s) => s.detector === RULE);
    return ours?.evidence.length && theirs?.evidence.length && ours.evidence[0].id !== theirs.evidence[0].id;
  }, 30000));
  if (!ours || !theirs) throw new Error("The fixture did not collect both signals.");

  await panel.getByRole("link", { name: "Open Supervisor screen" }).click();
  await page.getByRole("heading", { name: "Supervisor API · Supervisor" }).waitFor();
  await page.locator("#sv-signals summary").filter({ hasText: "reasonless_failures" }).click();
  await page.locator("#sv-signals details[open] .sv-evidence").waitFor();
  check("AC-223 the live screen shows the correct signal evidence and seven guarded indicators",
    await page.locator("#sv-score summary").count() === 7
    && await page.getByText(ours.evidence[0].id, { exact: false }).isVisible()
    && !await page.getByText(theirs.evidence[0].id, { exact: false }).count());
  await page.locator("#sv-score summary").filter({ hasText: "Release cadence" }).click();
  check("AC-223 the live scoreboard says which data is missing",
    await until(() => page.getByText("Not measured yet (release tags are unavailable)", { exact: true }).isVisible()));
  await j(api.path, "PUT", { enabled: false });
  await j(web.path, "PUT", { enabled: false });
}
