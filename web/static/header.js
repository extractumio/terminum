// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// The header (AC-39): the profile's bar beside two lines: the agent's state, the title and the
// pencil; then one line: the pane's folder (its coding agent's, following it: follow.js) with a button that copies its full path, where the
// pane is in tmux (session:window, as tmux names a target), then what runs there (host, program,
// profile, titles). That last part gives way first, then the folder, down to a few letters each.
import { agentMark, el, folderBar, metaLine, shownState, stateIcon } from "./nav.js";
import { icon } from "./icons.js";
import { textForm } from "./render.js";
import { following } from "./follow.js";

const $ = (id) => document.getElementById(id);
const span = (cls, text) => el("span", cls, text);

/** The second line's parts, in order: the folder (and its full path, to copy), the window, the rest;
 *  `follow`: the folder its coding agent works in when it has one of its own (AC-51). */
export function headerParts(group, item, follow = following()) {
  const sub = item.sub ?? [], at = sub.findIndex((p) => /^[~/]/.test(p));
  const session = group.session ?? group.label;   // a host's group names its tmux session apart
  const where = item.index ? `${session}:${item.index}` : session;   // tmux's own target: session:window
  const rest = [...(item.host ? [item.host] : []), ...sub.filter((_, i) => i !== at)];
  if (item.profile) rest.splice(item.host ? 2 : 1, 0, item.profile);   // after the program, before the titles
  if (follow && item.agentDir) return { folder: item.agentFolder || item.agentDir, path: item.agentDir, where, rest: rest.filter(Boolean) };
  return { folder: at >= 0 ? sub[at] : null, path: item.dir || (at >= 0 ? sub[at] : null), where, rest: rest.filter(Boolean) };
}

/** copy(text): the page's Copy (input.js), the selection kept. Returns show(found). */
export function pageHeader({ copy, renaming, setAgent }) {
  function show(found) {
    $("current").querySelector(".t").textContent = found ? textForm(found.item.title) : "Choose a session";
    $("curstate").replaceChildren(...[found && stateIcon(shownState(found.item))].filter(Boolean));
    setAgent(found?.item.agent ?? null);
    if (!found) { $("cursub").replaceChildren(); $("curbar").replaceChildren(); $("curagent").replaceChildren(); }
    else {
      const { group: g, item: it } = found, { folder, path, where, rest } = headerParts(g, it), parts = [];
      if (folder) {
        const b = document.createElement("button");
        b.type = "button";
        b.className = "copydir";
        b.title = b.ariaLabel = `Copy ${path}`;
        b.append(icon("copy", 13));
        b.addEventListener("click", (e) => { e.stopPropagation(); copy(path); });   // not the header's own click (the list)
        parts.push([span("dir", folder), b]);
      }
      parts.push([span("where", where)]);
      if (rest.length) {                             // one part: it gives way as a whole, its words keep the list's colors
        const set = metaLine(rest, !!it.host);
        set.className = "set";
        parts.push([set]);
      }
      $("cursub").replaceChildren(...parts.flatMap((p, i) => (i ? [span("sep", "·"), ...p] : p)));
      $("curbar").replaceChildren(folderBar(it, true));   // still: the list's bar breathes while it works
      $("curagent").replaceChildren(agentMark(it, 16));
    }
    document.title = found ? `${found.item.title} – terminum` : "terminum";
    if (!renaming()) $("rename").hidden = !found;
  }
  return show;
}
