// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
/** A host's coding agents' progress (AC-99): the host menu's item, the dialog that says what is
 *  installed on the host, and terminumd's per-agent result lines in it (plain text). */
const $ = (id) => document.getElementById(id);

export function hostAgents(send) {
  let host = null;

  /** The host menu's item: enable (asked first), or turn off with the link's state ("up",
   *  "refused: why"). Offered on an attached host, or whenever it is on. */
  function item(g, hide) {
    const on = !!g.agents && g.agents !== "off";
    if (!on && g.state !== "attached") return [];
    const b = document.createElement("button");
    b.type = "button"; b.dataset.action = "agents";
    b.textContent = on ? `Turn off coding agents' progress on ${g.host}` : `Coding agents' progress on ${g.host}…`;
    if (on) {
      const s = document.createElement("span");
      s.className = "agentsstate"; s.textContent = ` (${g.agents})`;
      b.append(s);
    }
    b.onclick = () => { hide(); open(g.host, !on); };
    return [b];
  }

  function open(name, on) {
    host = name;
    $("agentstitle").textContent = on ? `Coding agents' progress on ${name}?` : `Turn off coding agents' progress on ${name}?`;
    $("agentstext").textContent = on
      ? `Installs terminum's hooks and MCP server for Claude Code and Codex, in ~/.claude and ~/.codex on ${name}; Codex asks to trust them in /hooks.`
      : `Takes terminum's hooks and MCP server out of ~/.claude and ~/.codex on ${name}.`;
    const lines = $("agentslines");
    lines.textContent = ""; lines.hidden = true; lines.classList.remove("bad");
    const ok = $("agentsok");
    ok.hidden = false; ok.disabled = false; ok.textContent = on ? "Enable" : "Turn off";
    $("agentscancel").textContent = "Cancel";
    ok.onclick = (e) => {
      e.preventDefault();
      ok.disabled = true; ok.textContent = on ? "Enabling…" : "Turning off…";
      send({ t: "hostagents", name, on });
    };
    $("agentsbox").showModal();
    ok.focus();
  }

  /** terminumd's answer: each agent's line, or why it failed; the dialog then only closes. */
  function answered(m) {
    if (m.name !== host || !$("agentsbox").open) return;
    const lines = $("agentslines");
    lines.textContent = (m.lines || []).join("\n");
    lines.hidden = false;
    lines.classList.toggle("bad", !m.ok);
    $("agentsok").hidden = true;
    $("agentscancel").textContent = "Close";
    $("agentscancel").focus();
  }

  return { item, answered };
}
