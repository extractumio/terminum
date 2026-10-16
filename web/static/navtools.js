// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The session list's tools (AC-07, AC-52, AC-81): its width, "New session" and "Add host" at its
// foot, a host group's menu (retry, off, on, remove), the line for a tmux server that does not
// answer, the pencil that names the shown pane, and the page's confirmation dialog.

const $ = (id) => document.getElementById(id);

/** Asks `text` with a dialog: true when the user pressed `yes` (else Cancel, Esc). */
export function askConfirm(text, yes) {
  const box = $("confirmbox");
  $("confirmtext").textContent = text;
  $("confirmok").textContent = yes;
  box.returnValue = "";
  box.showModal();
  $("confirmok").focus();
  return new Promise((resolve) => box.addEventListener("close", () => resolve(box.returnValue === "ok"), { once: true }));
}

/** Wires the tools; `current()` is the shown pane's {group, item} or null, `onResize()` refits the terminal. */
export function navTools({ send, store, current, onResize, onRenamed }) {
  // The session list's width, dragged at its border (wide screens), remembered per browser.
  const NAV_MIN = 200, NAV_MAX = 640;
  function setNavWidth(px, save) {
    const w = Math.round(Math.min(NAV_MAX, Math.max(NAV_MIN, px)));
    document.documentElement.style.setProperty("--nav-w", `${w}px`);
    $("navgrip").setAttribute("aria-valuenow", String(w));
    if (save) store.set("navWidth", w);
    onResize();
  }
  {
    const grip = $("navgrip");
    const saved = store.get("navWidth", null);
    if (saved) setNavWidth(saved, false);
    grip.addEventListener("pointerdown", (e) => {
      e.preventDefault();
      grip.setPointerCapture(e.pointerId);
      const left = $("nav").getBoundingClientRect().left;
      const move = (ev) => setNavWidth(ev.clientX - left, false);
      const up = (ev) => {
        setNavWidth(ev.clientX - left, true);
        cancel();
      };
      const cancel = () => {
        grip.removeEventListener("pointermove", move);
        grip.removeEventListener("pointerup", up);
        grip.removeEventListener("pointercancel", cancel);
      };
      grip.addEventListener("pointermove", move);
      grip.addEventListener("pointerup", up);
      grip.addEventListener("pointercancel", cancel);
    });
    grip.addEventListener("keydown", (e) => {
      const step = { ArrowLeft: -16, ArrowRight: 16 }[e.key];
      if (step) { e.preventDefault(); setNavWidth($("nav").getBoundingClientRect().width + step, true); }
    });
  }

  // Inline forms at the list's foot: one open at a time; Esc or a second press closes it. The
  // form waits for terminumd: its error shows in the form, a new session closes it.
  let pending = null;                              // the form whose request is on its way
  function openForm(id, button) {
    for (const [f, b] of [["newsess", "newsessbtn"], ["addhost", "addhostbtn"]]) {
      const on = f === id && $(f).hidden;
      $(f).hidden = !on;
      $(b).setAttribute("aria-expanded", String(on));
      if (on) { $(`${f}err`).textContent = ""; $(f).querySelector("input").focus(); }
    }
    if (button && $(id).hidden) pending = null;
  }
  const closeForm = (id) => { $(id).hidden = true; $(`${id}btn`).setAttribute("aria-expanded", "false"); if (pending === id) pending = null; };
  $("newsessbtn").onclick = () => openForm("newsess", true);
  $("addhostbtn").onclick = () => openForm("addhost", true);
  for (const f of ["newsess", "addhost"]) {
    $(f).addEventListener("keydown", (e) => { if (e.key === "Escape") { e.stopPropagation(); closeForm(f); } });
    $(`${f}cancel`).onclick = () => closeForm(f);
  }
  $("newsess").addEventListener("submit", (e) => {
    e.preventDefault();
    const name = $("newsessname").value.trim();
    $("newsesserr").textContent = "";
    pending = "newsess";
    send({ t: "newsession", name });
  });
  // Enter on the tick sends the form too (WebKit submits only from a text field)
  $("addhostfiles").addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); $("addhost").requestSubmit(); } });
  $("addhost").addEventListener("submit", (e) => {
    e.preventDefault();
    $("addhosterr").textContent = "";
    pending = "addhost";
    send({ t: "addhost", name: $("addhostname").value.trim(), recipe: $("addhostssh").value.trim(), files: $("addhostfiles").checked });
  });
  /** An `error` from terminumd: true when it answers a form's request (shown there). */
  function formError(msg) {
    if (!pending) return false;
    $(`${pending}err`).textContent = msg;
    pending = null;
    return true;
  }
  /** A new layout: the session or host asked for is there, and its form closes. */
  function layoutCame(groups) {
    const has = (name) => groups.some((g) => g.wid === name || g.label === name || g.host === name);
    if (pending === "newsess" && has($("newsessname").value.trim())) { closeForm("newsess"); $("newsessname").value = ""; }
  }
  /** terminumd added the host (its group follows in a layout): the form closes. */
  function hostAdded() {
    if (pending !== "addhost") return;
    closeForm("addhost");
    $("addhostname").value = $("addhostssh").value = "";
    $("addhostfiles").checked = false;
  }

  // A host group's menu (AC-81): retry now, off or on, remove (asked first).
  function hostMenu(g, button) {
    const box = $("hostmenu");
    if (!box.hidden && box.dataset.host === g.host) return hideHostMenu();
    box.dataset.host = g.host;
    const act = (label, action, ask) => {
      const b = document.createElement("button");
      b.type = "button"; b.textContent = label; b.dataset.action = action;
      b.onclick = async () => {
        hideHostMenu();
        if (ask && !(await askConfirm(ask, label))) return;
        send({ t: "hostaction", name: g.host, action });
      };
      return b;
    };
    box.replaceChildren(act("Retry now", "retry"), g.state === "off" ? act("On", "on") : act("Off", "off"),
      act("Remove", "remove", `Remove ${g.host}? Its tmux session on the host keeps running; terminum stops showing it.`));
    const r = button.getBoundingClientRect();
    box.style.top = `${r.bottom + 4}px`;
    box.style.left = `${Math.max(8, r.right - 180)}px`;
    box.hidden = false;
    button.setAttribute("aria-expanded", "true");
  }
  function hideHostMenu() {
    $("hostmenu").hidden = true;
    for (const b of document.querySelectorAll("#sessions .hostbtn")) b.setAttribute("aria-expanded", "false");
  }
  document.addEventListener("click", (e) => { if (!$("hostmenu").hidden && !e.target.closest("#hostmenu")) hideHostMenu(); });

  // ---------- a tmux server not answering (AC-03, AC-14) ----------
  // Screens, keys and histories wait for it: say so, with the time it has not answered.
  const stalls = new Map();                        // server -> since (performance.now())
  let stallTimer = 0;
  function showStall(m) {
    if (m.ok) stalls.delete(m.server ?? "local");
    else if (!stalls.has(m.server ?? "local")) stalls.set(m.server ?? "local", performance.now() - (m.ms ?? 0));
    clearInterval(stallTimer);
    $("stall").hidden = !stalls.size;
    if (!stalls.size) return;
    const say = () => {
      $("stall").textContent = [...stalls].map(([server, since]) =>
        `tmux on ${server === "local" ? "this Mac" : server} is not responding (${Math.round((performance.now() - since) / 1000)} s)`).join(". ");
    };
    say();
    stallTimer = setInterval(say, 1000);
  }

  // The pencil names the shown pane's window (and the pane's title).
  function startRename() {
    const found = current();
    if (!found) return;
    $("renamename").value = found.item.title;
    $("current").hidden = $("rename").hidden = true;
    $("renamebox").hidden = false;
    $("renamename").focus();
    $("renamename").select();
  }
  // Done or cancelled: the keys go back to the terminal (on a touch screen the keyboard closes).
  function endRename(refocus = false) {
    $("renamename").blur();
    $("renamebox").hidden = true;
    $("current").hidden = false;
    $("rename").hidden = !current();
    if (refocus) onRenamed();
  }
  $("rename").onclick = (e) => { e.stopPropagation(); startRename(); };   // not the header's own click
  $("renamecancel").onclick = () => endRename(true);
  $("renamebox").addEventListener("submit", (e) => {
    e.preventDefault();
    send({ t: "rename", id: current()?.item.id, name: $("renamename").value });
    endRename(true);
  });
  $("renamename").addEventListener("keydown", (e) => { if (e.key === "Escape") { e.stopPropagation(); endRename(true); } });

  return { showStall, formError, layoutCame, hostAdded, hostMenu, endRename, renaming: () => !$("renamebox").hidden };
}
