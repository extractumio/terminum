// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-33 and AC-98 for e2e_all.mjs: going through a session's bookmarks and reminders in the
// bookmarks' pane (its own by line, an agent's mark, a bell, ↑ ↓ and Enter, the pane kept open on
// a jump, ✓ and Delete with Undo for 5 s, the bar's ✓), and an agent's record: found where its call
// was printed (by its title), its text as Markdown, Insert into chat unquoted.
import { check } from "./check.mjs";
import { open } from "./e2e_widgets.mjs";

const line = (text) => ({ r: [[text, null, null, 0]], e: true });
const HIST = Array.from({ length: 400 }, (_, i) => line(`out ${i} lorem ipsum`));
HIST[300] = line('⏺ terminum - add_notes_for_user (MCP)(notes: [{"title": "Final report: retry loop fixed", "text": "…"}])');
const at0 = 1790000000;
const ITEMS = [
  { id: "A", sid: "S1", n: 200, text: "out 200 lorem ipsum", note: "second by line", title: "One", at: at0 + 9 },
  { id: "B", sid: "S1", n: 100, text: "out 100 lorem ipsum", note: "first by line", title: "One", at: at0 + 1 },
  { id: "C", sid: "S1", n: 150, text: "out 150 lorem ipsum", note: "check the deploy", title: "One", at: at0 + 2, remind: true, due: true },
  { id: "D", sid: "S1", n: 160, text: "out 160 lorem ipsum", note: "settled", title: "One", at: at0 + 3, remind: true, resolved: at0 + 4 },
  { id: "R", sid: "S1", n: -1, note: "Final report: retry loop fixed", text: "**Fixed** the retry loop.\n\n- `src/retry.rs`\n- next: add a test",
    title: "One", at: at0 + 5, by: "claude", kind: "report" },
  { id: "Q", sid: "S1", n: -1, note: "Decide the API name", text: "fetch_all or list_all?", title: "One", at: at0 + 6, by: "codex", remind: true },
];
const mod = process.platform === "darwin" ? "Meta" : "Control";

export async function webMarkNav(browser, name) {
  const at = `[${name}]`;
  const f = await open(browser, { history: HIST, first: 0, screen: ["done"], hook: (m, say) => {
    if (m.t !== "locate") return;
    const n = { A: 200, B: 100, C: 150, R: 300 }[m.id];
    say(n === undefined ? { t: "located", id: m.id, sid: "S1", n: null, why: "These lines are no longer in this session's history." }
      : { t: "located", id: m.id, sid: "S1", n, start: 0, size: m.id === "R" ? 26 : 12 });
  } });
  const { page, sent, say } = f;
  say({ t: "marks", items: ITEMS, error: null });
  await page.waitForFunction(() => !document.getElementById("marksfab").hidden, null, { timeout: 5000 }).catch(() => {});
  const fab = await page.evaluate(() => document.getElementById("marksfabn").textContent);
  await page.click("#marksfab");
  await page.waitForSelector("#markpane .mknav li");
  const list = () => page.evaluate(() => [...document.querySelectorAll("#markpane .mknav li")].map((l) => ({ id: l.dataset.id, cls: l.className,
    agent: !!l.querySelector(".note .agmark img"), bell: !!l.querySelector(".note .mkbell"), title: l.querySelector(".note").textContent })));
  const rows = await list();
  check(`AC-33 ${at} the bookmarks' pane lists the session's bookmarks and open reminders together, by line, an agent's (not found yet) after by time; the button counts them`,
    rows.map((r) => r.id).join() === "B,C,A,R,Q" && fab === "5", JSON.stringify({ rows, fab }));
  check(`AC-98 ${at} an agent's record has its agent's mark, a reminder a bell, a resolved reminder is not listed`,
    rows.find((r) => r.id === "R").agent && rows.find((r) => r.id === "Q").agent && rows.find((r) => r.id === "C").bell && !rows.some((r) => r.id === "D"), JSON.stringify(rows));
  const focus = await page.evaluate(() => document.activeElement?.id);
  check(`AC-33 ${at} on a computer the open pane has the keys`, focus === "markpane", String(focus));

  // a press jumps and keeps the pane open on it; ↓ goes on to the next; Enter closes it and the terminal has the keys
  let from = sent.length;
  await page.click('#markpane li[data-id="B"] .mkitem');
  await page.waitForTimeout(400);
  const first = await page.evaluate(() => ({ open: !document.getElementById("markpane").hidden, chosen: document.querySelector("#markpane li.chosen")?.dataset.id,
    bar: document.getElementById("mbnote").textContent }));
  await page.keyboard.press("ArrowDown");
  await page.waitForTimeout(400);
  const second = await page.evaluate(() => document.querySelector("#markpane li.chosen")?.dataset.id);
  const located = sent.slice(from).filter((m) => m.t === "locate").map((m) => m.id);
  check(`AC-33 ${at} a press brings it into view and the pane stays open on it; ↓ goes on to the next`,
    first.open && first.chosen === "B" && first.bar === "first by line" && second === "C" && located.join() === "B,C", JSON.stringify({ first, second, located }));
  await page.keyboard.press("Enter");
  await page.waitForTimeout(150);
  const closed = await page.evaluate(() => ({ open: !document.getElementById("markpane").hidden, keys: document.activeElement?.id }));
  check(`AC-33 ${at} Enter closes the pane and gives the terminal the keys`, !closed.open && closed.keys === "kbd", JSON.stringify(closed));

  // ✓: done at once in the list with Undo, terminumd told only after 5 s; Undo takes it back
  await page.click("#marksfab");
  await page.waitForSelector('#markpane li[data-id="A"]');
  from = sent.length;
  await page.click('#markpane li[data-id="A"] .mkdone');
  await page.waitForTimeout(300);
  const pending = await page.evaluate(() => { const l = document.querySelector('#markpane li[data-id="A"]'); return { pending: l.classList.contains("pending"), said: l.querySelector(".mkdonetext")?.textContent, undo: !!l.querySelector(".mkundo") }; });
  await page.click('#markpane li[data-id="A"] .mkundo');
  await page.waitForTimeout(5300);
  const undone = await page.evaluate(() => !document.querySelector('#markpane li[data-id="A"]').classList.contains("pending"));
  check(`AC-33 ${at} ✓ marks a bookmark deleted at once with Undo; Undo takes it back and nothing is sent`,
    pending.pending && pending.said === "Deleted" && pending.undo && undone && !sent.slice(from).some((m) => m.t === "unmark"), JSON.stringify({ pending, undone, sent: sent.slice(from) }));
  from = sent.length;
  await page.click('#markpane li[data-id="C"] .mkdone');
  await page.click('#markpane li[data-id="A"] .mkitem');      // chosen: Delete on it
  await page.keyboard.press("Delete");
  await page.waitForTimeout(1000);
  const early = sent.slice(from).filter((m) => m.t === "unmark" || m.t === "remind").length;
  await page.waitForTimeout(4600);
  const told = sent.slice(from).filter((m) => m.t === "unmark" || m.t === "remind");
  check(`AC-33 ${at} after 5 s a reminder is resolved and a bookmark deleted (✓, and Delete on the chosen one); nothing before`,
    early === 0 && told.some((m) => m.t === "remind" && m.id === "C" && m.action === "resolve") && told.some((m) => m.t === "unmark" && m.id === "A"), JSON.stringify({ early, told }));
  for (const m of told) say(m.t === "unmark" ? { t: "unmark", id: m.id } : { t: "mark", item: { ...ITEMS.find((i) => i.id === m.id), due: false, resolved: at0 + 50 } });

  // an agent's record: found by its title where its call was printed; the bar's ✓
  from = sent.length;
  await page.click('#markpane li[data-id="R"] .mkitem');
  await page.waitForFunction(() => document.getElementById("markrail") && document.querySelector(".marked, .wavy, #term [data-marked]") !== undefined, null, { timeout: 3000 }).catch(() => {});
  await page.waitForTimeout(800);
  const rec = await page.evaluate(() => {
    const rows = [...document.querySelectorAll("#hist .ln, #screen .ln")], t = rows.find((r) => r.textContent.includes("add_notes_for_user"));
    const view = document.getElementById("term").getBoundingClientRect(), r = t?.getBoundingClientRect();
    return { inView: !!r && r.top >= view.top && r.bottom <= view.bottom, bar: document.getElementById("mbnote").textContent, why: document.getElementById("mbwhy").hidden };
  });
  check(`AC-98 ${at} Open on an agent's record brings the line where its call was printed into view`,
    sent.slice(from).some((m) => m.t === "locate" && m.id === "R") && rec.inView && rec.bar === "Final report: retry loop fixed" && rec.why, JSON.stringify(rec));
  await page.keyboard.press("Escape");
  await page.click("#mbdone");
  const barUndo = await page.evaluate(() => document.getElementById("mbdone").textContent);
  await page.click("#mbdone");
  const barBack = await page.evaluate(() => document.getElementById("mbdone").textContent);
  check(`AC-33 ${at} the bar's ✓ Done turns into Undo, and back`, barUndo === "Undo" && barBack === "Done", JSON.stringify({ barUndo, barBack }));

  // Insert into chat: an agent's title and text, unquoted
  from = sent.length;
  await page.click("#mbsend");
  await page.waitForTimeout(200);
  const pasted = sent.slice(from).filter((m) => m.t === "in").map((m) => m.data).join("");
  check(`AC-98 ${at} Insert into chat pastes an agent's title and its text as it wrote it, not quoted`,
    pasted.includes("Final report: retry loop fixed") && pasted.includes("**Fixed** the retry loop.") && !pasted.includes("> **Fixed**"), JSON.stringify(pasted));

  // the reminders' pane: an agent's record with its mark, its text as Markdown, not a quote
  await page.click("#mbclose");
  await page.click("#remindfab");
  await page.waitForSelector("#remindlist .rmitem");
  const pane = await page.evaluate(() => [...document.querySelectorAll("#remindlist .rmitem")].map((li) => ({ mark: !!li.querySelector(".rmwhere .agmark"),
    quote: !!li.querySelector("blockquote"), text: li.querySelector(".markdown").textContent.trim() })));
  check(`AC-98 ${at} the reminders' pane shows an agent's reminder with its mark, its text as Markdown (no quote)`,
    pane.some((p) => p.mark && !p.quote && p.text.includes("fetch_all or list_all?")), JSON.stringify(pane));
  check(`AC-33 ${at} no page errors`, f.errors.length === 0, f.errors.join(" | "));
  await page.close();
}
