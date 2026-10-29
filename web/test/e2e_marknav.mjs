// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-33 and AC-98 for e2e_all.mjs: the bookmarks' pane as a navigator (its own by line, an agent's
// mark, a bell; the chosen one's content beside the list, ↑ ↓ to choose, Enter shows it in the
// terminal, Done and Delete with Undo for 5 s, the bar's Done, Quote in chat), and an agent's
// record: found where its call was printed (by its title), its text as Markdown.
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

  // the navigator: the first one shown at once; a press or ↓ shows another, nothing jumps; an agent's record as Markdown
  const view = () => page.evaluate(() => {
    const body = document.querySelector("#markpane .mkvbody")?.shadowRoot?.querySelector(".body");
    return { open: !document.getElementById("markpane").hidden, chosen: document.querySelector("#markpane li.chosen")?.dataset.id,
      title: document.querySelector("#markpane .mkvtitle")?.textContent, pre: body?.querySelector("pre")?.textContent.trim() ?? null,
      strong: body?.querySelector("strong")?.textContent ?? null, items: body ? body.querySelectorAll("li").length : 0,
      acts: [...document.querySelectorAll("#markpane .mkvacts button")].map((b) => b.textContent.trim()),
      // Show in terminal at the actions' left edge; Resolve and Quote in chat together at their right edge
      place: (() => { const box = document.querySelector("#markpane .mkvacts"), [go, done, chat] = box ? [...box.querySelectorAll("button")].map((b) => b.getBoundingClientRect()) : [];
        if (!chat) return {}; const pad = parseFloat(getComputedStyle(box).paddingLeft), r = box.getBoundingClientRect();
        return { left: Math.abs(go.left - r.left - pad) < 1.5, right: Math.abs(r.right - parseFloat(getComputedStyle(box).paddingRight) - chat.right) < 1.5 && chat.left - done.right < 16 && done.left - go.right > 16 }; })(),
      wrap: body?.querySelector("pre") ? getComputedStyle(body.querySelector("pre")).whiteSpace : null,
      tall: Math.round(document.getElementById("term").getBoundingClientRect().height - document.getElementById("markpane").getBoundingClientRect().height) };
  });
  let from = sent.length;
  await page.waitForTimeout(400);
  const shown = await view();
  await page.click('#markpane li[data-id="A"] .mkitem');
  await page.keyboard.press("ArrowDown");
  await page.waitForTimeout(500);
  const next = await view();
  check(`AC-33 ${at} the pane, as tall as the terminal less 28 px, shows the first one's content at once beside the list (a bookmark: the terminal's text exactly, its long lines wrapped); a press or ↓ shows another, an agent's record as Markdown; nothing jumps; three actions`,
    shown.open && shown.chosen === "B" && shown.title === "first by line" && shown.pre === "out 100 lorem ipsum" && next.chosen === "R" && next.title.includes("Final report: retry loop fixed")
    && next.strong === "Fixed" && next.items === 2 && !sent.slice(from).some((m) => m.t === "locate") && shown.acts.join() === "Show in terminal,Resolve,Quote in chat"
    && shown.place.left && shown.place.right
    && shown.wrap === "pre-wrap" && shown.tall === 28, JSON.stringify({ shown, next }));

  // Enter: shown in the terminal, found by its title where its call was printed; the pane closes, the terminal has the keys
  await page.keyboard.press("Enter");
  await page.waitForFunction(() => document.getElementById("markrail") && document.querySelector(".marked, .wavy, #term [data-marked]") !== undefined, null, { timeout: 3000 }).catch(() => {});
  await page.waitForTimeout(800);
  const rec = await page.evaluate(() => {
    const rows = [...document.querySelectorAll("#hist .ln, #screen .ln")], t = rows.find((r) => r.textContent.includes("add_notes_for_user"));
    const view = document.getElementById("term").getBoundingClientRect(), r = t?.getBoundingClientRect();
    return { inView: !!r && r.top >= view.top && r.bottom <= view.bottom, bar: document.getElementById("mbnote").textContent, why: document.getElementById("mbwhy").hidden,
      open: !document.getElementById("markpane").hidden, keys: document.activeElement?.id };
  });
  check(`AC-98 ${at} Enter (Show in terminal) on an agent's record closes the pane and brings the line where its call was printed into view`,
    sent.slice(from).some((m) => m.t === "locate" && m.id === "R") && rec.inView && rec.bar === "Final report: retry loop fixed" && rec.why && !rec.open && rec.keys === "kbd", JSON.stringify(rec));
  await page.click("#mbdone");
  const barUndo = await page.evaluate(() => document.getElementById("mbdone").textContent);
  await page.click("#mbdone");
  const barBack = await page.evaluate(() => document.getElementById("mbdone").textContent);
  check(`AC-33 ${at} the bar's ✓ Resolve turns into Undo, and back`, barUndo === "Undo" && barBack === "Resolve", JSON.stringify({ barUndo, barBack }));
  from = sent.length;
  await page.click("#mbsend");
  await page.waitForTimeout(200);
  const pasted = sent.slice(from).filter((m) => m.t === "in").map((m) => m.data).join("");
  check(`AC-98 ${at} the bar's Quote in chat pastes an agent's title and, under it, its text quoted`,
    pasted.includes("Final report: retry loop fixed") && pasted.includes("> **Fixed** the retry loop."), JSON.stringify(pasted));
  await page.click("#mbclose");

  // Done: at once in the list with Undo, terminumd told only after 5 s; Undo takes it back
  await page.click("#marksfab");
  await page.waitForSelector('#markpane li[data-id="A"]');
  from = sent.length;
  await page.click('#markpane li[data-id="A"] .mkitem');
  await page.click("#markpane .mkvdone");
  await page.waitForTimeout(300);
  const pending = await page.evaluate(() => { const l = document.querySelector('#markpane li[data-id="A"]'); return { pending: l.classList.contains("pending"), said: l.querySelector(".mkdonetext")?.textContent, undo: !!l.querySelector(".mkundo"), done: document.querySelector("#markpane .mkvdone").textContent }; });
  await page.click('#markpane li[data-id="A"] .mkundo');
  await page.waitForTimeout(5300);
  const undone = await page.evaluate(() => !document.querySelector('#markpane li[data-id="A"]').classList.contains("pending"));
  check(`AC-33 ${at} Done marks a bookmark deleted at once with Undo (in its row and in the view); Undo takes it back and nothing is sent`,
    pending.pending && pending.said === "Deleted" && pending.undo && pending.done === "Undo" && undone && !sent.slice(from).some((m) => m.t === "unmark"), JSON.stringify({ pending, undone, sent: sent.slice(from) }));
  from = sent.length;
  await page.click('#markpane li[data-id="C"] .mkitem');
  await page.click("#markpane .mkvdone");
  await page.click('#markpane li[data-id="A"] .mkitem');      // chosen: Delete on it
  await page.keyboard.press("Delete");
  await page.waitForTimeout(1000);
  const early = sent.slice(from).filter((m) => m.t === "unmark" || m.t === "remind").length;
  await page.waitForTimeout(4600);
  const told = sent.slice(from).filter((m) => m.t === "unmark" || m.t === "remind");
  check(`AC-33 ${at} after 5 s a reminder is resolved and a bookmark deleted (Done, and Delete on the chosen one); nothing before`,
    early === 0 && told.some((m) => m.t === "remind" && m.id === "C" && m.action === "resolve") && told.some((m) => m.t === "unmark" && m.id === "A"), JSON.stringify({ early, told }));
  for (const m of told) say(m.t === "unmark" ? { t: "unmark", id: m.id } : { t: "mark", item: { ...ITEMS.find((i) => i.id === m.id), due: false, resolved: at0 + 50 } });

  // the view's Quote in chat: the chosen one pasted quoted, the pane closed
  await page.waitForTimeout(300);
  await page.click('#markpane li[data-id="B"] .mkitem');
  from = sent.length;
  await page.click("#markpane .mkvacts .tochat");
  await page.waitForTimeout(200);
  const quoted = sent.slice(from).filter((m) => m.t === "in").map((m) => m.data).join("");
  const after = await page.evaluate(() => !document.getElementById("markpane").hidden);
  check(`AC-33 ${at} the view's Quote in chat pastes the note and its text quoted, no Enter; the pane closes`,
    quoted.includes("first by line") && quoted.includes("> out 100 lorem ipsum") && !after, JSON.stringify({ quoted, after }));

  // the reminders' pane: an agent's record with its mark, its text as Markdown, not a quote
  await page.click("#remindfab");
  await page.waitForSelector("#remindlist .rmitem");
  const pane = await page.evaluate(() => [...document.querySelectorAll("#remindlist .rmitem")].map((li) => ({ mark: !!li.querySelector(".rmwhere .agmark"),
    quote: !!li.querySelector("blockquote"), text: li.querySelector(".markdown").textContent.trim() })));
  check(`AC-98 ${at} the reminders' pane shows an agent's reminder with its mark, its text as Markdown (no quote)`,
    pane.some((p) => p.mark && !p.quote && p.text.includes("fetch_all or list_all?")), JSON.stringify(pane));
  check(`AC-33 ${at} no page errors`, f.errors.length === 0, f.errors.join(" | "));
  await page.close();
}
