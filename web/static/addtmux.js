// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// "Attach tmux sessions" (AC-89): sessions of the user's own tmux on this Mac or on a host, attached by
// name. "On" picks the place (a server id: `local:default`, `<host>:default`); the sessions
// running there that are not attached are ticks, the name field takes one that does not run
// yet; one message attaches them all. It opens from the list's foot (this Mac), a host group's
// import icon or menu, and by itself once a host just added is attached with other sessions.

const $ = (id) => document.getElementById(id);
const OFFER_FOR = 30000;                           // how long a host just added may take to attach

/** `groups()`: the layout's groups; `openForm(id)` toggles a foot form, `closeForm(id)` closes it. */
export function addTmux({ send, groups, openForm, closeForm, setPending }) {
  let offer = null;                                // {host, until}: a host just added, to offer its sessions
  const form = $("addtmux"), on = $("addtmuxon"), ticks = $("addtmuxticks");

  /** This Mac and every attached host, by server id. */
  function places() {
    const hosts = groups().filter((g) => g.host && !g.attached && g.state === "attached").map((g) => g.host);
    return [["local:default", "This Mac"], ...[...new Set(hosts)].map((h) => [`${h}:default`, h])];
  }
  function ask() {
    ticks.replaceChildren(Object.assign(document.createElement("p"), { className: "hint", textContent: "Looking for running sessions…" }));
    send({ t: "tmuxsessions", server: on.value });
  }
  /** Opens the form on `server` (kept open when it is), asking what runs there. */
  function open(server = "local:default") {
    on.replaceChildren(...places().map(([id, label]) => Object.assign(document.createElement("option"), { value: id, textContent: label })));
    on.value = server;
    if (on.value !== server) on.value = "local:default";
    if (form.hidden) openForm("addtmux");
    $("addtmuxerr").textContent = "";
    ask();
  }
  on.addEventListener("change", ask);

  /** What runs at the place asked (an answer for another place is late: ignored). A session tmux
   *  destroys with its last client (`ends`: destroy-unattached) says so: once terminum lets go of
   *  it (detach, terminum stops, a host's link drops) after the user's own terminal left, it is gone. */
  function choices(m) {
    if (form.hidden || m.server !== on.value) return;
    const ends = new Set(m.ends ?? []);
    const rows = m.names.map((name) => {
      const l = document.createElement("label");
      l.className = "tick";
      const box = Object.assign(document.createElement("input"), { type: "checkbox", value: name });
      l.append(box, document.createTextNode(` ${name}`));
      if (ends.has(name)) {
        l.classList.add("ends");
        l.append(Object.assign(document.createElement("span"), { textContent: "destroy-unattached is on: tmux closes it when terminum is its last client and lets go" }));
      }
      return l;
    });
    ticks.replaceChildren(...(rows.length ? rows
      : [Object.assign(document.createElement("p"), { className: "hint", textContent: "No other session runs there: name one to attach when it starts." })]));
  }

  // Safari's Enter in a field sends the form only from a text field: the ticks send it too
  form.addEventListener("keydown", (e) => {
    if (e.key === "Enter" && !e.isComposing && e.target.matches("input")) { e.preventDefault(); form.requestSubmit(); }
  });
  form.addEventListener("submit", (e) => {
    e.preventDefault();
    const names = [...ticks.querySelectorAll("input:checked")].map((b) => b.value);
    const typed = $("addtmuxname").value.trim();
    if (typed) names.push(typed);
    $("addtmuxerr").textContent = "";
    setPending("addtmux");
    send({ t: "addtmux", server: on.value, sessions: names });
  });

  /** terminumd attached them: the form closes. */
  function added() {
    closeForm("addtmux");
    $("addtmuxname").value = "";
    ticks.replaceChildren();
  }

  /** Add host added `name`: once it is attached, its other sessions are offered. */
  function hostAdded(name) {
    offer = name ? { host: name, until: Date.now() + OFFER_FOR } : null;
  }
  /** A new layout: the host just added is attached (offer its sessions, if it has others), failed, or too late. */
  function layoutCame(gs) {
    if (!offer) return;
    const g = gs.find((x) => x.host === offer.host && !x.attached);
    if (g?.state === "attached") {
      const free = ["newsess", "addhost", "addtmux"].every((f) => $(f).hidden);
      if (g.others > 0 && free) open(`${offer.host}:default`);
      offer = null;
    } else if (g?.state === "failed" || Date.now() > offer.until) offer = null;
  }

  return { open, choices, added, hostAdded, layoutCame };
}
