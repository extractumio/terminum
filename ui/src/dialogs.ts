// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Small modal dialog and context menu, styled by the panel theme.

import { esc } from "./api";

export interface Choice { id: string; label: string; primary?: boolean; danger?: boolean }

/** Modal with a message and buttons, × in its corner as the Page's dialogs. Resolves to the chosen
 *  id, or "cancel" on × and Esc. */
export function ask(title: string, message: string, choices: Choice[]): Promise<string> {
  return new Promise((resolve) => {
    const previous = document.activeElement as HTMLElement | null;
    const back = document.createElement("div");
    back.className = "modal-back";
    back.innerHTML =
      `<div class="modal box" role="dialog" aria-modal="true" aria-label="${esc(title)}"><div class="mhead"><div class="mtitle">${esc(title)}</div>` +
      `<button data-id="cancel" class="mx" title="Close (Esc)" aria-label="Close"><svg viewBox="0 0 24 24" width="16" height="16" fill="none" ` +
      `stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg></button></div>` +
      (message ? `<div class="mtext">${esc(message)}</div>` : "") +
      `<div class="mbtns">${choices.map((c) =>
        `<button data-id="${esc(c.id)}" class="btn${c.primary ? " primary" : ""}${c.danger ? " danger" : ""}">${esc(c.label)}</button>`).join("")}</div></div>`;
    const done = (id: string) => {
      back.remove(); document.removeEventListener("keydown", key, true);
      if (previous?.isConnected) previous.focus({ preventScroll: true });
      resolve(id);
    };
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); done("cancel"); }
      if (e.key === "Enter") {
        e.preventDefault(); e.stopPropagation();
        const button = document.activeElement?.closest<HTMLButtonElement>("button[data-id]");
        done(button && back.contains(button) ? button.dataset.id! : (choices.find((c) => c.primary) ?? choices[0]).id);
      }
      if (e.key === "Tab") {
        e.preventDefault(); e.stopPropagation();
        const buttons = [...back.querySelectorAll<HTMLButtonElement>("button[data-id]")];
        const i = buttons.indexOf(document.activeElement as HTMLButtonElement);
        buttons[(i + (e.shiftKey ? buttons.length - 1 : 1)) % buttons.length]?.focus();
      }
    };
    back.addEventListener("click", (e) => {
      const b = (e.target as HTMLElement).closest<HTMLElement>("button[data-id]");
      if (b) done(b.dataset.id!);
      else if (e.target === back) done("cancel");
    });
    document.addEventListener("keydown", key, true);
    document.body.append(back);
    back.querySelector<HTMLButtonElement>(".btn.primary, .btn")?.focus();
  });
}

export interface MenuItem {
  id: string; label: string; keys?: string; disabled?: boolean; danger?: boolean;
  /** a choice of a set: ✓ when true */
  checked?: boolean;
  /** a second level, opened beside the item (pointer over it, a tap, → or ↩) */
  sub?: MenuItem[];
}
export type MenuEntry = MenuItem | "-";

let shut: (() => void) | null = null;       // the menu open now: a new one closes it (its promise gets null)

/** Context menu at the mouse position, an item's sub menu beside it. Resolves to the chosen id or null. */
export function menu(x: number, y: number, entries: MenuEntry[]): Promise<string | null> {
  shut?.();
  return new Promise((resolve) => {
    const levels: { el: HTMLElement; items: HTMLElement[]; list: MenuItem[]; cur: number }[] = [];
    const item = (e: MenuItem) =>
      `<div class="mi${e.disabled ? " off" : ""}${e.danger ? " danger" : ""}" data-id="${esc(e.id)}" role="${e.checked === undefined ? "menuitem" : "menuitemradio"}"` +
      `${e.sub ? ' aria-haspopup="menu"' : ""}${e.checked === undefined ? "" : ` aria-checked="${e.checked}"`}>` +
      `<span class="lb">${e.checked === undefined ? "" : `<span class="ck">${e.checked ? "✓" : ""}</span>`}${esc(e.label)}</span>` +
      `${e.keys ? `<kbd>${esc(e.keys)}</kbd>` : ""}${e.sub ? '<span class="more" aria-hidden="true">›</span>' : ""}</div>`;
    /** A level of the menu: at (x, y), or beside its parent's item `by` (right, else left). */
    const open = (list: MenuEntry[], at: { x: number; y: number } | { by: HTMLElement }) => {
      const lv = levels.length, el = document.createElement("div");
      el.className = "ctx box";
      el.setAttribute("role", "menu");
      el.innerHTML = list.map((e) => (e === "-" ? '<div class="sep"></div>' : item(e))).join("");
      document.body.append(el);
      const r = el.getBoundingClientRect();
      let left: number, top: number;
      if ("by" in at) {
        const p = at.by.closest(".ctx")!.getBoundingClientRect(), b = at.by.getBoundingClientRect();
        left = p.right + r.width - 2 <= innerWidth - 4 ? p.right - 2 : p.left - r.width + 2;
        top = b.top - 4;                              // its first item level with the parent's
      } else ({ x: left, y: top } = at);
      el.style.left = `${Math.max(4, Math.min(left, innerWidth - r.width - 4))}px`;
      el.style.top = `${Math.max(4, Math.min(top, innerHeight - r.height - 4))}px`;
      levels.push({ el, items: [...el.querySelectorAll<HTMLElement>(".mi:not(.off)")], list: list.filter((e): e is MenuItem => e !== "-"), cur: -1 });
      el.addEventListener("click", (e) => {
        const m = (e.target as HTMLElement).closest<HTMLElement>(".mi");
        if (m && !m.classList.contains("off")) choose(lv, m);
      });
      el.addEventListener("pointermove", (e) => {
        const m = (e.target as HTMLElement).closest<HTMLElement>(".mi:not(.off)"), i = m ? levels[lv]?.items.indexOf(m) : -1;
        if (!m || i === undefined || i === levels[lv].cur) return;
        focus(lv, i);
        if (entry(lv, m)?.sub) openSub(lv, m); else close(lv + 1);
      });
    };
    const entry = (lv: number, m: HTMLElement) => levels[lv].list.find((e) => e.id === m.dataset.id);
    const focus = (lv: number, i: number) => { const l = levels[lv]; l.items.forEach((m, j) => m.classList.toggle("hot", j === i)); l.cur = i; };
    /** Levels from `lv` on closed. */
    const close = (lv: number) => { for (const l of levels.splice(lv)) l.el.remove(); };
    const openSub = (lv: number, m: HTMLElement) => {
      if (levels[lv + 1]?.el.dataset.of === m.dataset.id) return;
      close(lv + 1);
      open(entry(lv, m)!.sub!, { by: m });
      levels[lv + 1].el.dataset.of = m.dataset.id!;
    };
    const choose = (lv: number, m: HTMLElement) => {
      if (!entry(lv, m)?.sub) return done(m.dataset.id!);
      openSub(lv, m);
      focus(lv + 1, 0);
    };
    const done = (id: string | null) => {
      shut = null;
      close(0);
      document.removeEventListener("pointerdown", outside, true);
      document.removeEventListener("keydown", key, true);
      resolve(id);
    };
    const outside = (e: Event) => { if (!levels.some((l) => l.el.contains(e.target as Node))) done(null); };
    const key = (e: KeyboardEvent) => {
      e.preventDefault(); e.stopPropagation();
      // the keys act on the deepest level with an item in focus: a sub menu the pointer opened waits for →
      let lv = levels.length - 1;
      while (lv > 0 && levels[lv].cur < 0) lv--;
      const l = levels[lv], m = l.items[l.cur];
      if (e.key === "Escape" || e.key === "ArrowLeft") { if (lv) close(lv); else if (e.key === "Escape") done(null); }
      else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        focus(lv, (l.cur + (e.key === "ArrowDown" ? 1 : l.items.length - 1)) % l.items.length);
        close(lv + 1);
      }
      else if (m && (e.key === "Enter" || (e.key === "ArrowRight" && entry(lv, m)?.sub))) choose(lv, m);
    };
    open(entries, { x, y });
    shut = () => done(null);
    setTimeout(() => {
      document.addEventListener("pointerdown", outside, true);
      document.addEventListener("keydown", key, true);
    });
  });
}
