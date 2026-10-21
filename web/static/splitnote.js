// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A split window (AC-13): a banner over the terminal says the shown pane's window holds several
// panes (Claude Code's teammates, the user's own tmux) and how wide this one is, with Full width
// (tmux zooms the pane, and its window then takes this view's size) or, zoomed, Show split, and
// Other pane. terminumd says the split with each frame that changes it (`split`: null or
// {panes, zoomed, wcols}).

const $ = (id) => document.getElementById(id);

/** The banner's words for `split` and the pane's `cols`; null for a window of one pane. */
export function splitWords(split, cols) {
  if (!split) return null;
  return split.zoomed ? `This window is split: ${split.panes} panes, this one zoomed`
    : `This window is split: ${split.panes} panes, this one ${cols} of ${split.wcols} columns`;
}

/** The pane after `sid` in its window (the list's order, round), or null: a window is known by
 *  its id on its tmux server (the key without the pane). */
export function nextPane(groups, sid) {
  const server = (id) => id.slice(0, id.lastIndexOf(":"));
  const items = groups.flatMap((g) => g.items ?? []), cur = items.find((it) => it.id === sid);
  if (!cur?.window) return null;
  const mates = items.filter((it) => it.window === cur.window && server(it.id) === server(sid));
  return mates.length > 1 ? mates[(mates.indexOf(cur) + 1) % mates.length].id : null;
}

/** send(msg) to terminumd; other(): shows the window's next pane. {seen(m)}: each `screen`. */
export function splitNote({ send, other }) {
  const box = $("splitnote"), words = box.querySelector("span"), zoom = $("splitzoom");
  let split = null, cols = 0, said = null;
  zoom.addEventListener("click", () => send({ t: "zoom", on: !split?.zoomed }));
  $("splitother").addEventListener("click", () => other());
  return {
    /** Each frame (30 a second): the page is changed only where it differs; hidden, nothing is read. */
    seen(m) {
      if ("split" in m) split = m.split;
      if (m.cols) cols = m.cols;
      const text = splitWords(split, cols);
      if (!text && !said) return;
      if (text) {
        // over the terminal's top left, clear of the marks' strip: the banners above may come and go
        const term = $("term"), top = `${term.offsetTop + 8}px`, left = `${term.offsetLeft + 24}px`;
        if (box.style.top !== top) box.style.top = top;
        if (box.style.left !== left) box.style.left = left;
      }
      if (text === said) return;
      said = text;
      box.hidden = !text;
      if (!text) return;
      words.textContent = text;
      zoom.textContent = split.zoomed ? "Show split" : "Full width";
      zoom.title = split.zoomed ? "Give the window back to its panes" : "Zoom this pane in tmux: it takes the whole window";
    },
  };
}
