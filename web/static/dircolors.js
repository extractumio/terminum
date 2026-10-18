// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// A folder's color (AC-01): every working folder of the list gets a pastel hue of its own. The
// folders not inside one another share the hue circle evenly, so they lie as far apart as their
// count allows; a folder inside another listed folder takes a hue near its parent's, nearer the
// deeper it lies, between its siblings'. "~" and "/" are nobody's parent: everything is in them.
// The lightness and chroma are the page's (--dir-l, --dir-c: style.css), the hue is the folder's.

const START = 250;                   // the first hue: a calm blue
const NO_PARENT = new Set(["~", "/"]);

/** A row's folder: the first part of its meta line that is a path; keyed with its host. */
export function folderKey(item) {
  const dir = item.sub?.find((p) => /^[~/]/.test(p));
  return dir ? `${item.host ?? ""}\n${dir.length > 1 ? dir.replace(/\/+$/, "") : dir}` : null;
}

const depth = (key) => key.split("/").length;
const inside = (key, parent) => !NO_PARENT.has(parent.split("\n")[1]) && key.startsWith(parent + "/");

/** Map of folder key → hue (degrees) for every distinct folder in `keys`. */
export function folderHues(keys) {
  const dirs = [...new Set(keys.filter(Boolean))].sort();
  const kids = new Map(), roots = [];
  for (const d of dirs) {
    let parent = null;                         // the deepest listed folder it is inside
    for (const p of dirs) if (p !== d && inside(d, p) && (!parent || p.length > parent.length)) parent = p;
    if (parent) kids.set(parent, [...(kids.get(parent) ?? []), d]);
    else roots.push(d);
  }
  const hues = new Map();
  // a folder owns `room` degrees around its hue; its children share half of it, less the deeper they lie
  const place = (d, hue, room) => {
    hues.set(d, ((hue % 360) + 360) % 360);
    const list = kids.get(d) ?? [];
    list.forEach((k, i) => {
      const span = room / 2 / (depth(k) - depth(d));
      const step = span / (list.length + 1);
      const side = i % 2 ? -1 : 1, n = Math.floor(i / 2) + 1;   // +1, −1, +2, −2 … steps: never the parent's own hue
      place(k, hue + side * n * step * (list.length > 1 ? 1 : 0.5), step);
    });
  };
  const gap = 360 / Math.max(roots.length, 1);
  roots.forEach((r, i) => place(r, START + i * gap, gap));
  return hues;
}
