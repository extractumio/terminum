// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The header's menu (AC-07, AC-15, AC-17): its pane actions, Split right and below, Move window
// to session…, Close pane (asked first), Restore again after a restore report, the report's
// banner over the terminal, and the chip of a newer release with its own menu.
import { askConfirm } from "./navtools.js";

const $ = (id) => document.getElementById(id);

/** current(): the shown pane's {group, item} or null; groups(): the layout's; pick(id) opens a
 *  pane; signOut() for the web listener's Sign out; copy(text) to the clipboard. */
export function paneMenu({ send, current, groups, pick, signOut, copy, quick }) {
  function open() { syncItems(); $("menu").hidden = false; $("menubtn").setAttribute("aria-expanded", "true"); }
  function close() { $("menu").hidden = true; $("menubtn").setAttribute("aria-expanded", "false"); hideMoves(); }
  $("menubtn").onclick = (e) => { e.stopPropagation(); $("menu").hidden ? open() : close(); };
  document.addEventListener("click", (e) => { if (!$("menu").hidden && !e.target.closest("#menu")) close(); });

  // The pane's actions need a pane; a window moves only to another session of its tmux server.
  const targets = () => {
    const found = current();
    if (!found) return [];
    return groups().filter((g) => g.kind === "tmux" && g.wid && g.wid !== found.group.wid && (g.host ?? null) === (found.group.host ?? null) && g.server === found.group.server);
  };
  function syncItems() {
    const found = current();
    for (const id of ["splitright", "splitbelow", "quickbelow", "quickleft", "closepane"]) $(id).disabled = !found;
    $("movewin").hidden = !targets().length;
  }

  const paneSend = (m) => { const found = current(); close(); if (found) send({ ...m, id: found.item.id }); };
  $("splitright").onclick = () => paneSend({ t: "split", how: "right" });
  $("splitbelow").onclick = () => paneSend({ t: "split", how: "below" });
  $("quickbelow").onclick = () => { close(); quick("bottom"); };     // AC-88
  $("quickleft").onclick = () => { close(); quick("left"); };
  $("closepane").onclick = async () => {
    const found = current();
    close();
    if (!found) return;
    const prog = found.item.sub?.[0] || "a program";
    if (await askConfirm(`Close this pane? The program running in it (${prog}) ends.`, "Close")) send({ t: "kill", id: found.item.id });
  };

  function hideMoves() { $("movelist").hidden = true; $("movewin").setAttribute("aria-expanded", "false"); }
  $("movewin").onclick = (e) => {
    e.stopPropagation();
    if (!$("movelist").hidden) return hideMoves();
    $("movelist").replaceChildren(...targets().map((g) => {
      const b = document.createElement("button");
      b.type = "button"; b.className = "row"; b.textContent = g.label; b.dataset.session = g.wid;
      b.onclick = () => paneSend({ t: "move", session: g.wid });
      return b;
    }));
    $("movelist").hidden = false;
    $("movewin").setAttribute("aria-expanded", "true");
  };

  $("restoreagain").onclick = () => { close(); send({ t: "restore" }); };
  $("signout").onclick = () => { close(); signOut(); };

  // ---------- the restore report (AC-15) ----------
  // "Restored 2 sessions, 4 windows. 1 pane ran ssh devbox: start it again", its panes linked.
  function showRestore(report) {
    $("restoreagain").hidden = false;
    const box = $("restored"), text = document.createElement("span");
    if (typeof report === "string" || typeof report?.text === "string") text.textContent = report.text ?? report;
    else {
      const n = (k, one) => `${report?.[k] ?? 0} ${one}${report?.[k] === 1 ? "" : "s"}`;
      text.append(`Restored ${n("sessions", "session")}, ${n("windows", "window")}.`);
      const ran = report?.ran ?? [];
      if (ran.length) {
        text.append(` ${ran.length} pane${ran.length === 1 ? "" : "s"} ran `);
        ran.forEach((p, i) => {
          if (i) text.append(", ");
          const a = document.createElement("button");
          a.type = "button"; a.className = "link"; a.textContent = p.program ?? p.cmd ?? "a program";
          a.onclick = () => { if (p.id) pick(p.id); };
          text.append(a);
        });
        text.append(`: start ${ran.length === 1 ? "it" : "them"} again.`);
      }
      for (const f of report?.failed ?? []) text.append(` Not restored: ${f}.`);
    }
    const x = document.createElement("button");
    x.type = "button"; x.className = "x"; x.textContent = "×"; x.setAttribute("aria-label", "Close the report");
    x.onclick = () => { box.hidden = true; };
    box.replaceChildren(text, x);
    box.hidden = false;
  }

  // ---------- a newer release (AC-17) ----------
  // The header's chip "↑ v0.4.0"; its menu copies the upgrade command, skips the release or
  // stops the checks. Only a tag terminumd checked reaches here (vMAJOR.MINOR.PATCH).
  let latest = null;
  function showUpdate(tag) {
    latest = typeof tag === "string" && /^v\d+\.\d+\.\d+$/.test(tag) ? tag : null;
    $("updchip").hidden = !latest;
    $("updchip").textContent = latest ? `↑ ${latest}` : "";
    if (!latest) hideUpdate();
    else $("updskip").textContent = `Skip ${latest}`;
  }
  function hideUpdate() { $("updmenu").hidden = true; $("updchip").setAttribute("aria-expanded", "false"); }
  $("updchip").onclick = (e) => {
    e.stopPropagation();
    if (!$("updmenu").hidden) return hideUpdate();
    const r = $("updchip").getBoundingClientRect();
    $("updmenu").style.top = `${r.bottom + 6}px`;
    $("updmenu").style.right = `${Math.max(8, innerWidth - r.right)}px`;
    $("updmenu").hidden = false;
    $("updchip").setAttribute("aria-expanded", "true");
  };
  $("updcopy").onclick = () => { hideUpdate(); copy("terminum upgrade"); };
  $("updskip").onclick = () => { hideUpdate(); send({ t: "updatechoice", action: "skip", tag: latest }); };
  $("updoff").onclick = () => { hideUpdate(); send({ t: "updatechoice", action: "off" }); };
  document.addEventListener("click", (e) => { if (!$("updmenu").hidden && !e.target.closest("#updmenu")) hideUpdate(); });

  return { open, close, showRestore, showUpdate };
}
