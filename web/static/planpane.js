// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The Plan pane (AC-97, AC-98): the shown session's plan and its notes for the user, modal as the
// reminders' pane (fabpane.js). Opened by the header's progress (#curplan, its button) or a row's
// percent (a "planopen" event after the row's session is shown). It asks terminumd {"t":"plan"} on
// opening and again on {"t":"plannew"} of its key; it closes when another session is shown. Every
// text the agent wrote goes in as text (el's textContent): never HTML or Markdown.
import { fabPane } from "./fabpane.js";
import { icon } from "./icons.js";
import { armed, el, INSERT } from "./marks.js";
import { ageText, clock, etaLine, headerText, noteChat, orderNotes, rowTitle, sourceText, statusIcon } from "./plans.js";

const $ = (id) => document.getElementById(id);
const now = () => Date.now() / 1000;
const STATUS = { completed: "completed", in_progress: "in progress", pending: "pending", blocked: "blocked" };

/** send(msg) to terminumd; sid(): the shown pane's key; toChat(text): pasted into its input, no Enter. */
export function planPane({ send, sid, toChat }) {
  const fab = $("curplan");
  let of = null, shown = null, waiting = false;      // the key the pane shows; the shown row (layout); asked, no answer yet
  const pane = fabPane({ fab, pane: $("planpane"), close: $("planclose"), onToggle: (on) => {
    of = on ? sid() : null;
    if (on) { $("planbody").replaceChildren(el("p", "plmeta", "Loading…")); ask(); place(); }
  } });
  const ask = () => { if (of) { waiting = true; send({ t: "plan", key: of }); } };
  fab.addEventListener("click", (e) => e.stopPropagation());     // not the header's own click (the list)
  document.addEventListener("planopen", () => { if (!pane.open()) pane.toggle(true); else ask(); });

  /** The header's progress for the shown row (item: a layout row or null). */
  function header(item) {
    shown = item;
    const plan = item?.plan ?? null, notes = item?.notes ?? 0;
    if (pane.open() && sid() !== of) pane.toggle(false);
    else if (pane.open() && waiting) ask();          // asked while terminumd was away (a restart): its next layout asks again
    fab.hidden = !plan && !notes;                    // notes alone still open the pane
    if (fab.hidden) return;
    fab.classList.toggle("stale", !!plan?.stale);
    const said = notes ? `${notes} note${notes > 1 ? "s" : ""}` : "";
    $("curplant").textContent = plan ? headerText(plan, now()) : said;
    fab.title = plan ? `Plan: ${rowTitle(plan, now())}${said && ` · ${said}`}` : `Notes: ${said}`;
    fab.setAttribute("aria-label", fab.title);
    fab.style.setProperty("--pct", `${Math.max(0, Math.min(100, plan?.pct ?? 0))}%`);
  }
  setInterval(() => { if (shown) header(shown); }, 60_000);   // the age moves on without a message

  function onMessage(m) {
    if (!pane.open() || m.key !== of) return;
    if (m.t === "plannew") return void ask();
    waiting = false;
    draw(m.plan, orderNotes(m.notes));
  }

  function draw(p, notes) {
    const body = [];
    if (!p) body.push(el("p", "plmeta", "No plan for this session."));
    else {
      if (p.title) body.push(el("h3", "pltitle", p.title));
      if (p.doing) body.push(el("p", "pldoing", p.doing));
      const ul = el("ul", "plitems");
      for (const it of p.items ?? []) {
        const li = el("li", `plitem ${it.status ?? "pending"}`);
        const mark = el("span", "plmark", statusIcon(it.status));
        mark.title = STATUS[it.status] ?? "pending";
        mark.setAttribute("aria-label", mark.title);
        li.append(mark, el("span", "pltext", it.text));
        ul.append(li);
      }
      body.push(ul);
      if (p.partial) body.push(el("p", "plpartial", "partial: some updates were missed"));
      const meta = [`${p.done} of ${p.total} · ${Math.round(p.pct ?? 0)}%`, etaLine(p) && `ETA ${etaLine(p)}`,
        `updated ${ageText(p.updated, now())} (${clock(p.updated)})`, sourceText(p.source)].filter(Boolean);
      for (const line of meta) body.push(el("p", "plmeta", line));
    }
    body.push(el("h3", "plnoteshead", notes.length ? `Notes (${notes.filter((n) => n.status === "open").length} open)` : "No notes"));
    const list = el("ul", "plnotes");
    for (const n of notes) list.append(noteItem(n));
    body.push(list);
    const held = $("planpane").contains(document.activeElement);   // a pressed Done goes with the redraw: the pane keeps the keys (Esc)
    $("planbody").replaceChildren(...body);
    if (held && !$("planpane").contains(document.activeElement)) $("planpane").focus({ preventScroll: true });
    place();
  }

  function noteItem(n) {
    const li = el("li", `plnote ${n.status === "open" ? "open" : "done"}`);
    li.dataset.id = String(n.id);
    const head = el("div", "plnotehead");
    head.append(el("span", `plkind ${n.kind}`, n.kind), el("span", "plwhen", ageText(n.at, now())));
    li.append(head, el("p", "plnotetext", n.text));
    if (n.file) li.append(el("p", "plfile", n.file));
    const acts = el("div", "rmacts placts");
    const act = (label, fn, cls = "") => { const b = el("button", cls, label); b.type = "button"; b.onclick = fn; acts.append(b); return b; };
    act(INSERT, () => { pane.toggle(false); toChat(noteChat(n)); }, "tochat");
    const del = el("button", "del", "Delete");                  // asks again: a second press deletes it
    del.type = "button";
    armed(del, () => ["Delete"], () => send({ t: "note", key: of, id: n.id, act: "delete" }));
    acts.append(del);
    if (n.status === "open") act("Done", () => send({ t: "note", key: of, id: n.id, act: "done" }), "primary");
    else act("Reopen", () => send({ t: "note", key: of, id: n.id, act: "open" }));
    li.append(acts);
    return li;
  }

  $("planclose").replaceChildren(icon("x", 16));
  return { header, onMessage, place };
  /** Placed by fabpane; as a phone's sheet, its top at the header's bottom. */
  function place() {
    pane.place();
    if ("sheet" in $("planpane").dataset) $("planpane").style.setProperty("--plan-top", `${Math.round($("bar").getBoundingClientRect().bottom)}px`);
  }
}
