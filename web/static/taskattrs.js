// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A task's Details (terminum-kanban.md AC-199), folded until opened: its type, a link to an outside tracker, the
// release it is meant for, and the tasks that block it and follow it, each with its reason. Each list takes a task
// by + and a search over the project's tasks, and lets one go by ×; nothing is typed as a list of IDs. terminumd
// decides what stands: a refusal is said and the pane shows what is kept.
import { el, iconButton } from "./listparts.js";
import { api, button, statusTag } from "./kanbanutil.js";

const TYPES = ["Feature", "Bug", "Debt", "Story"];
export const LISTS = [["blocked_by", "Blocked by", "Why it waits for it"], ["follow_ups", "Follow-ups", "Why it follows"]];
/** Found tasks a search shows at most. */
const FOUND_MOST = 12;

/** What the summary says of a folded pane: what is set. */
function summaryOf(v) {
  const said = [v.type, v.target && `target ${v.target}`, v.external_link && "link",
    ...LISTS.map(([list, name]) => (v[list]?.length ? `${name.toLowerCase()} ${v[list].length}` : ""))].filter(Boolean);
  return said.length ? `Details: ${said.join(", ")}` : "Details";
}

/** A web address a press may open in a new tab; anything else opens nothing. */
const webLink = (s) => /^https?:\/\/\S+$/i.test(s ?? "");

/** The tasks of `project` whose ID, title or text has `text`, without `left` (IDs), newest first. */
async function findTasks(project, text, left) {
  const q = new URLSearchParams({ project, q: text, subtasks: "1", sort: "recent", limit: String(FOUND_MOST + left.length) });
  const { tasks } = await api(`/api/tasks?${q}`);
  return tasks.filter((t) => !left.includes(t.id)).slice(0, FOUND_MOST);
}

/** `node`'s change goes to terminumd by `send(value)`; refused, it shows `was` again (and `after()` runs). */
function saves(node, send, was, after = () => {}) {
  node.addEventListener("change", async () => { if (!await send(node.value.trim())) { node.value = was; after(); } });
}

/** The pane. `v`: {type, external_link, target, blocked_by, follow_ups: [{id, title, status, reason}]}; `self`: the
 *  task's own ID, none while it is made; `project()`: where its lists' tasks are found; `open`: whether it shows
 *  unfolded, `onToggle(open)`; changes go by `set(field, value)`, `link(list, id, reason)`, `unlink(list, id)`, each
 *  a promise of whether terminumd took it; `openTask(id)` opens a listed task, when there is a window to open it. */
export function detailsPane({ v, self = null, project, open = false, onToggle = () => {}, set, link, unlink, openTask = null }) {
  const pane = el("details", "more taskattrs");
  pane.open = open;
  const summary = el("summary", null, summaryOf(v));
  pane.append(summary);
  pane.addEventListener("toggle", () => onToggle(pane.open));

  const grid = el("div", "fieldgrid three");
  const typeRow = el("label", "fieldrow", "Type");
  const type = el("select", "field");
  type.append(new Option("None", ""), ...TYPES.map((t) => new Option(t, t)));
  type.value = v.type ?? "";
  saves(type, (value) => set("type", value), v.type ?? "");
  typeRow.append(type);

  const linkRow = el("label", "fieldrow", "External link");
  const linkLine = el("span", "attrline");
  const address = el("input", "field");
  Object.assign(address, { type: "url", value: v.external_link ?? "", placeholder: "https://…", autocomplete: "off", spellcheck: false });
  const go = el("a", "attrgo", "Open");
  const showGo = () => {
    go.hidden = !webLink(address.value.trim());
    if (!go.hidden) Object.assign(go, { href: address.value.trim(), target: "_blank", rel: "noopener noreferrer", title: `Open ${address.value.trim()} in a new tab` });
  };
  showGo();
  address.addEventListener("input", showGo);
  saves(address, (value) => set("external_link", value), v.external_link ?? "", showGo);
  linkLine.append(address, go);
  linkRow.append(linkLine);

  const targetRow = el("label", "fieldrow", "Target");
  const target = el("input", "field");
  Object.assign(target, { type: "text", value: v.target ?? "", maxLength: 64, placeholder: "A release, such as v1.4", autocomplete: "off" });
  saves(target, (value) => set("target", value), v.target ?? "");
  targetRow.append(target);
  grid.append(typeRow, linkRow, targetRow);
  pane.append(grid);

  for (const [list, name, why] of LISTS) pane.append(listPart({ list, name, why, items: v[list] ?? [], self, project, link, unlink, openTask }));
  return pane;
}

/** One list: its tasks, each with its reason and ×, then + to find another. */
function listPart({ list, name, why, items, self, project, link, unlink, openTask }) {
  const wrap = el("div", "fieldrow attrlist");
  const head = el("div", "attrhead");
  head.append(el("span", null, name));
  const rows = el("ul", "attrrows");
  rows.setAttribute("aria-label", name);
  for (const it of items) {
    const li = el("li", "attrrow");
    const id = openTask ? button("linklike", it.id, () => openTask(it.id)) : el("span", "attrid", it.id);
    const title = el("span", "attrtitle", it.title ?? "");
    title.title = it.title ?? "";
    const reason = el("input", "field attrreason");
    Object.assign(reason, { type: "text", value: it.reason ?? "", placeholder: why, autocomplete: "off" });
    reason.setAttribute("aria-label", `${why}: ${it.id}`);
    saves(reason, (value) => link(list, it.id, value), it.reason ?? "");
    const gone = iconButton("icon", "x", `Take ${it.id} out of ${name}`, `Take ${it.id} out of ${name}`, () => unlink(list, it.id));
    li.append(id, title, ...(it.status ? [statusTag(it)] : []), reason, gone);
    rows.append(li);
  }
  const add = iconButton("icon", "plus", `Add a task to ${name}`, `Add a task to ${name}`, () => {
    add.hidden = true;
    const left = [self, ...items.map((i) => i.id)].filter(Boolean);
    wrap.append(picker({ name, project, left, done: () => { add.hidden = false; add.focus(); }, pick: (t) => link(list, t.id, "", t) }));
  });
  head.append(add);
  wrap.append(head, rows);
  return wrap;
}

/** A search box over the project's tasks, its finds under it: arrows move, Enter or a press picks, Esc lets go. */
function picker({ name, project, left, done, pick }) {
  const box = el("div", "attrpick");
  const field = el("input", "field");
  Object.assign(field, { type: "search", placeholder: "Find a task by its ID, title or text", autocomplete: "off" });
  field.setAttribute("aria-label", `Find a task for ${name}`);
  const found = el("ul", "attrfound");
  found.setAttribute("role", "listbox");
  const note = el("p", "attrnote");
  note.setAttribute("role", "status");
  let shown = [], active = -1, asked = 0, timer = 0;
  const end = () => { clearTimeout(timer); box.remove(); done(); };
  const mark = (i) => {
    if (i === active) return;
    active = i;
    [...found.children].forEach((c, n) => { c.classList.toggle("active", n === i); c.setAttribute("aria-selected", String(n === i)); });
    found.children[i]?.scrollIntoView({ block: "nearest" });
  };
  const take = async (t) => { if (await pick(t)) end(); };
  async function search() {
    const mine = ++asked;
    const where = project();
    if (!where) { note.textContent = "Choose the project first."; return; }
    try {
      const tasks = await findTasks(where, field.value.trim(), left);
      if (mine !== asked) return;
      shown = tasks;
      found.replaceChildren(...tasks.map((t, n) => {
        const li = el("li", "attrfind");
        li.setAttribute("role", "option");
        li.append(el("span", "attrid", t.id), el("span", "attrtitle", t.title), statusTag(t));
        li.addEventListener("pointerdown", (e) => e.preventDefault());   // the field keeps the focus
        li.addEventListener("click", () => take(t));
        li.addEventListener("pointermove", () => mark(n));
        return li;
      }));
      note.textContent = tasks.length ? "" : "No task of this project has that.";
      active = -2;                                // the rows are new: mark the first anew
      mark(tasks.length ? 0 : -1);
    } catch (e) { if (mine === asked) note.textContent = e.message; }
  }
  field.addEventListener("input", () => { clearTimeout(timer); timer = setTimeout(search, 150); });
  field.addEventListener("keydown", (e) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") { e.preventDefault(); if (shown.length) mark((active + (e.key === "ArrowDown" ? 1 : shown.length - 1)) % shown.length); }
    else if (e.key === "Enter") { e.preventDefault(); e.stopPropagation(); if (shown[active]) take(shown[active]); }
    else if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); end(); }
  });
  const close = iconButton("icon", "x", "Close the search", "Close the search (Esc)", end);
  const line = el("div", "attrline");
  line.append(field, close);
  box.append(line, found, note);
  requestAnimationFrame(() => field.focus());
  search();
  return box;
}
