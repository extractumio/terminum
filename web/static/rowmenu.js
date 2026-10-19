// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The session list's menus: a row's on a right click or a long press (Clone, Add to Project,
// Remove from Project, Close Session, Move to the Top, to the Bottom, Up, Down; AC-07, AC-92), and a
// project's ⋯ (Rename, Ungroup). A move reorders the row among its group's: a project's
// sessions in the project, a tmux session's windows in tmux. Each is modal (a <dialog>): the
// page is shaded and takes no press while it is open; a press outside it closes it and does
// nothing else; Esc closes it.
import { askName, closeSession } from "./navtools.js";
import { icon } from "./icons.js";
import { projectColor } from "./nav.js";

const $ = (id) => document.getElementById(id);
const HOLD = 500;                // ms a finger rests before its menu opens

/** A finger resting on what `at(e)` finds in `el` (and not moving) for HOLD calls
 *  onHold(found, x, y); the click its lift makes is dropped. */
export function longPress(el, at, onHold) {
  let hold = null, held = false;
  el.addEventListener("pointerdown", (e) => {
    const found = e.pointerType === "touch" && at(e);
    if (!found) return;
    held = false;
    hold = { x: e.clientX, y: e.clientY, t: setTimeout(() => { held = true; onHold(found, hold.x, hold.y); }, HOLD) };
  });
  const cancel = () => { if (hold) clearTimeout(hold.t); hold = null; };
  el.addEventListener("pointermove", (e) => { if (hold && Math.hypot(e.clientX - hold.x, e.clientY - hold.y) > 10) cancel(); });
  for (const ev of ["pointerup", "pointercancel", "pointerleave"]) el.addEventListener(ev, cancel);
  el.addEventListener("click", (e) => { if (held) { held = false; e.preventDefault(); e.stopImmediatePropagation(); } }, true);
}

/** A list's new order (`ul`'s row ids): a project's sessions (AC-92), else a tmux session's windows. */
export function saveOrder(send, ul, ids) {
  if (ul.dataset.project) send({ t: "project", action: "members", project: ul.dataset.project, keys: ids });
  else if (ul.dataset.wid) send({ t: "reorder", group: ul.dataset.wid, ids });
}

/** list: #sessions; item(id): the row's layout item; projects(): terminumd's; clone(item). */
export function rowMenus({ list, send, item, projects, clone }) {
  const box = $("rowmenu");
  const hide = () => {
    if (!box.open) return;
    box.close();
    if (back?.isConnected && box.contains(document.activeElement)) back.focus({ preventScroll: true });
  };
  let openedAt = -Infinity, back = null;           // when a finger opened it; what had the focus
  const button = (label, action, run, enabled = true) => {
    const b = document.createElement("button");
    b.type = "button"; b.textContent = label; b.dataset.action = action; b.disabled = !enabled;
    b.setAttribute("role", "menuitem");
    // the lift of the finger that opened it is no press of what is under it now
    b.onclick = (e) => { e.stopPropagation(); if (performance.now() - openedAt > 350) run(); };
    return b;
  };
  /** The menu's items under a head: what it is for, and × to close it. */
  function fill(title, ...items) {
    const head = document.createElement("div");
    head.className = "mhead";
    const t = document.createElement("span");
    t.textContent = title;
    const x = button("×", "x", hide);
    x.title = "Close this menu";
    x.setAttribute("aria-label", "Close this menu");
    x.removeAttribute("role");
    head.append(t, x);
    box.replaceChildren(head, ...items);
  }
  function open(x, y, byFinger = false) {
    if (!box.open) { back = document.activeElement; box.showModal(); }
    openedAt = byFinger ? performance.now() : -Infinity;
    const w = box.offsetWidth, h = box.offsetHeight;
    box.style.left = `${Math.max(8, Math.min(x, innerWidth - w - 8))}px`;
    box.style.top = `${Math.max(8, Math.min(y, innerHeight - h - 8))}px`;
    box.querySelector("button:not(:disabled):not([data-action=x])")?.focus({ preventScroll: true });
  }

  /** The rows beside `li` in its list, and where its order goes. */
  function siblings(li) {
    const ul = li.parentElement;
    const ids = [...ul.children].map((c) => c.querySelector(":scope > .item")?.dataset.id).filter(Boolean);
    const save = (order) => saveOrder(send, ul, order);
    return { ids, save, movable: !!(ul.dataset.project || ul.dataset.wid) };
  }

  function rowMenu(li, x, y, byFinger = false) {
    const id = li.querySelector(":scope > .item")?.dataset.id, it = id && item(id);
    if (!it) return;
    const mine = projects().find((p) => p.members.includes(id));
    const { ids, save, movable } = siblings(li), i = ids.indexOf(id), last = ids.length - 1;
    const move = (to) => { const o = ids.filter((k) => k !== id); o.splice(to, 0, id); save(o); hide(); };
    fill(it.title,
      button("Clone", "clone", () => { hide(); clone(it); }),
      button("Add to Project…", "project", () => projectMenu(it, mine, x, y)),
      ...(mine ? [button(`Remove from ${mine.name}`, "unproject", () => { hide(); send({ t: "project", action: "remove", id }); })] : []),
      document.createElement("hr"),
      button("Move to the Top", "top", () => move(0), movable && i > 0),
      button("Move Up", "up", () => move(i - 1), movable && i > 0),
      button("Move Down", "down", () => move(i + 1), movable && i < last),
      button("Move to the Bottom", "bottom", () => move(last), movable && i < last),
      document.createElement("hr"),
      button("Close Session", "close", () => {
        hide();
        closeSession(it, send);
      }));
    box.dataset.for = id;
    open(x, y, byFinger);
  }

  function projectMenu(it, mine, x, y) {
    const pick = (project) => { hide(); send({ t: "project", action: "add", id: it.id, ...(project ? { project } : {}) }); };
    // a new project is named at once (its suggestion: the lowest "Group N" free)
    const named = async () => {
      hide();
      const taken = new Set(projects().map((p) => p.name));
      let n = 1;
      while (taken.has(`Group ${n}`)) n++;
      const name = await askName("Name the new project", `Group ${n}`, "Create");
      if (name) send({ t: "project", action: "add", id: it.id, name });
    };
    const others = projects().filter((p) => p !== mine).map((p) => {
      const b = button(p.name, "join", () => pick(p.id));
      const mark = icon("group", 15);
      mark.classList.add("picon");
      mark.style.setProperty("--proj", projectColor(p));
      b.prepend(mark);
      return b;
    });
    const again = () => { const li = list.querySelector(`.item[data-id="${CSS.escape(it.id)}"]`)?.closest("li"); if (li) rowMenu(li, x, y); else hide(); };
    fill("Add to Project", button("‹ Back", "back", again),
      document.createElement("hr"), ...others, button("New project…", "newproject", named));
    open(x, y);
  }

  async function rename(p) {
    const name = await askName(`Rename ${p.name}`, p.name);
    if (name && name !== p.name) send({ t: "project", action: "rename", project: p.id, name });
  }

  /** A project's ⋯: Rename, Ungroup. */
  function headMenu(p, b) {
    if (box.open && box.dataset.for === p.id) return hide();
    fill(p.name,
      button("Rename…", "rename", () => { hide(); rename(p); }),
      button("Ungroup", "ungroup", () => { hide(); send({ t: "project", action: "delete", project: p.id }); }));
    box.dataset.for = p.id;
    const r = b.getBoundingClientRect();
    open(r.right - 210, r.bottom + 4);
  }

  // a right click, the keyboard's menu key or ⇧F10 on a row (at the row), or a finger resting
  // on it (below the finger; its click that follows is dropped)
  list.addEventListener("contextmenu", (e) => {
    const li = e.target.closest(".item:not(.child)")?.closest("li");
    if (!li) return;
    e.preventDefault();
    const r = li.getBoundingClientRect(), keys = !e.clientX && !e.clientY;
    rowMenu(li, keys ? r.left + 24 : e.clientX, keys ? r.bottom : e.clientY);
  });
  box.addEventListener("keydown", (e) => {          // ↑ ↓ through its items, as a menu
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const items = [...box.querySelectorAll("button:not(:disabled)")], i = items.indexOf(document.activeElement);
    items[(i + (e.key === "ArrowDown" ? 1 : items.length - 1)) % items.length]?.focus();
  });
  longPress(list, (e) => !e.target.closest(".grip, .clone") && e.target.closest(".item:not(.child)")?.closest("li"),
    (li, x, y) => rowMenu(li, x - 24, y + 28, true));
  // a press elsewhere closes it (pointerdown: iOS sends no click for a tap on a plain area)
  // a press on the shade (the dialog itself, outside its box) closes it, and reaches nothing
  // under it; the lift of the finger that opened it does not
  box.addEventListener("click", (e) => {
    if (e.target !== box) return;
    const r = box.getBoundingClientRect(), inside = e.clientX >= r.left && e.clientX <= r.right && e.clientY >= r.top && e.clientY <= r.bottom;
    if (!inside && performance.now() - openedAt > 350) hide();
  });
  box.addEventListener("cancel", (e) => { e.preventDefault(); hide(); });   // Esc: the focus goes back too
  /** A menu of `items` ([label, run]) at x, y (the input's Paste, Undo and Redo); byFinger: a
   *  finger still down opened it. */
  function show(items, x, y, byFinger = false) {
    fill("Edit", ...items.map(([label, run]) => button(label, label.toLowerCase(), () => { hide(); run(); })));
    box.dataset.for = "";
    open(x, y, byFinger);
  }
  return { headMenu, hide, show, rename };
}
