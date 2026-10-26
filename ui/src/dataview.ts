// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// JSON, YAML, CSV and TSV rendered (AC-94): a tree in the editor's syntax colors, open as far as
// OPEN rows allow, that folds and unfolds; a table whose first row is the head. Children and rows
// are drawn as they are shown, a page at a time.

import { copyText, esc } from "./api";
import { dataKind, parseDelimited, parseJson, parseYaml, type Value } from "./dataparse";

const OPEN = 2000;        // rows open at first: level by level, one that would pass them stays folded
const ALL = 20000;        // rows Expand all opens at most
const CHILDREN = 1000;    // a folder's children drawn at once
const ROWS = 5000;        // a table's rows drawn at once

const isBox = (v: Value): v is Value[] | { [k: string]: Value } => v !== null && typeof v === "object";
const entries = (v: Value[] | { [k: string]: Value }): [string, Value][] => (Array.isArray(v) ? v.map((x, i) => [String(i), x]) : Object.entries(v));
const count = (v: Value) => (isBox(v) ? (Array.isArray(v) ? `[${v.length}]` : `{${Object.keys(v).length}}`) : "");

function scalarHtml(v: Value): string {
  if (v === null) return '<span class="tok-atom">null</span>';
  if (typeof v === "boolean") return `<span class="tok-bool">${v}</span>`;
  if (typeof v === "number") return `<span class="tok-number">${Number.isNaN(v) ? ".nan" : Number.isFinite(v) ? v : v > 0 ? ".inf" : "-.inf"}</span>`;
  return `<span class="tok-string">${esc(JSON.stringify(v))}</span>`;
}

/** The deepest level to open so that at most `budget` rows show (0: only the top level's). */
export function openDepth(roots: Value[], budget: number): number {
  let level = roots.filter(isBox) as (Value[] | { [k: string]: Value })[], depth = 0, shown = roots.length;
  for (;;) {
    const next: Value[] = level.flatMap((b) => entries(b).map(([, x]) => x));
    if (!next.length || shown + Math.min(next.length, CHILDREN * level.length) > budget) return depth;
    shown += next.length;
    level = next.filter(isBox) as (Value[] | { [k: string]: Value })[];
    depth++;
  }
}

/** Renders a tree of `docs` (one per YAML document) into `box`. */
function tree(box: HTMLElement, docs: Value[]) {
  const values = new WeakMap<HTMLElement, Value>();   // a redrawn tree's old rows go with their values
  const row = (key: string | null, v: Value, depth: number, open: number) => {
    const item = document.createElement("div");
    item.className = "dn";
    const head = document.createElement("div");
    head.className = "dr";
    head.style.setProperty("--d", String(depth));
    head.setAttribute("role", "treeitem");
    const k = key === null ? "" : `<span class="tok-propertyName">${esc(key)}</span><span class="tok-punctuation">: </span>`;
    head.innerHTML = `<span class="tw">${isBox(v) ? "▸" : ""}</span>${k}${isBox(v) ? `<span class="cnt tok-punctuation">${count(v)}</span>` : `<span class="sv">${scalarHtml(v)}</span>`}` +
      '<button class="dcopy" type="button" title="Copy its value as JSON">Copy</button>';
    item.append(head);
    values.set(head, v);
    if (isBox(v)) {
      head.setAttribute("aria-expanded", "false");
      if (depth < open) unfold(item, head, open);
    }
    return item;
  };
  // rows at `depth` of `list` (a number: a YAML document's heading), CHILDREN at a time, the next
  // drawn where their button stood
  const page = (list: ([string | null, Value] | number)[], depth: number, open: number, from = 0): HTMLElement[] => {
    const out: HTMLElement[] = list.slice(from, from + CHILDREN).map((x) => (typeof x === "number" ? heading(x) : row(x[0], x[1], depth, open)));
    if (from + CHILDREN < list.length) {
      const more = document.createElement("button");
      more.className = "dmore";
      more.type = "button";
      more.style.setProperty("--d", String(depth));
      more.textContent = `… ${(list.length - from - CHILDREN).toLocaleString()} more`;
      more.onclick = () => more.replaceWith(...page(list, depth, open, from + CHILDREN));
      out.push(more);
    }
    return out;
  };
  const heading = (i: number) => { const h = document.createElement("div"); h.className = "ddoc"; h.textContent = `Document ${i + 1}`; return h; };
  const kids = (item: HTMLElement, v: Value[] | { [k: string]: Value }, depth: number, open: number) => {
    const box = document.createElement("div");
    box.className = "dc";
    item.append(box);
    box.append(...page(entries(v), depth + 1, open));
  };
  function unfold(item: HTMLElement, head: HTMLElement, open: number) {
    head.setAttribute("aria-expanded", "true");
    head.querySelector(".tw")!.textContent = "▾";
    const box = item.querySelector<HTMLElement>(":scope > .dc");
    if (box) { box.hidden = false; return; }
    kids(item, values.get(head) as Value[] | { [k: string]: Value }, Number(head.style.getPropertyValue("--d")), open);
  }
  function draw(budget: number) {
    const open = openDepth(docs.length === 1 && isBox(docs[0]) ? entries(docs[0]).map(([, x]) => x) : docs, budget);
    // the top level paged as a folder's children: a root of 100,000 values draws CHILDREN rows
    const top = docs.flatMap((d, i): ([string | null, Value] | number)[] => [...(docs.length > 1 ? [i] : []), ...(isBox(d) ? entries(d) : [[null, d] as [null, Value]])]);
    box.querySelector<HTMLElement>(".dtree")!.replaceChildren(...page(top, 0, open));
  }
  box.innerHTML = '<div class="dbar"><button type="button" class="btn" data-all="open">Expand all</button><button type="button" class="btn" data-all="close">Collapse all</button></div><div class="dtree" role="tree"></div>';
  box.addEventListener("click", (e) => {
    const t = e.target as HTMLElement;
    const all = t.closest<HTMLElement>("[data-all]");
    if (all) return draw(all.dataset.all === "open" ? ALL : 0);
    const head = t.closest<HTMLElement>(".dr");
    if (!head) return;
    if (t.closest(".dcopy")) { const v = values.get(head) ?? null; return void copyText(typeof v === "string" ? v : JSON.stringify(v, null, 2), "Copied its value"); }
    if (!isBox(values.get(head) ?? null)) return;
    const item = head.parentElement!;
    if (head.getAttribute("aria-expanded") === "true") {
      head.setAttribute("aria-expanded", "false");
      head.querySelector(".tw")!.textContent = "▸";
      item.querySelector<HTMLElement>(":scope > .dc")!.hidden = true;
    } else unfold(item, head, 0);
  });
  draw(OPEN);
}

/** Renders `rows` (the first the head) as a table into `box`. */
function table(box: HTMLElement, rows: string[][]) {
  const [head = [], ...body] = rows;
  const width = body.reduce((w, r) => Math.max(w, r.length), head.length);   // every row's: a later one may be wider
  const numeric = Array.from({ length: width }, (_, c) => body.slice(0, ROWS).every((r) => !r[c] || /^[-+]?(\d[\d,_]*)?(\.\d+)?([eE][-+]?\d+)?%?$/.test(r[c].trim())) && body.some((r) => r[c]));
  const cell = (t: string | undefined, c: number) => `<td${numeric[c] ? ' class="num"' : ""}>${esc(t ?? "")}</td>`;
  box.innerHTML = `<table class="dtab"><thead><tr><th class="rn">#</th>${Array.from({ length: width }, (_, c) => `<th>${esc(head[c] ?? "")}</th>`).join("")}</tr></thead><tbody></tbody></table>`;
  const tbody = box.querySelector("tbody")!;
  const page = (from: number) => {
    tbody.insertAdjacentHTML("beforeend", body.slice(from, from + ROWS).map((r, i) =>
      `<tr><td class="rn">${from + i + 1}</td>${Array.from({ length: width }, (_, c) => cell(r[c], c)).join("")}</tr>`).join(""));
    if (from + ROWS < body.length) {
      const more = document.createElement("button");
      more.className = "dmore";
      more.type = "button";
      more.textContent = `… ${(body.length - from - ROWS).toLocaleString()} more rows`;
      more.onclick = () => { more.remove(); page(from + ROWS); };
      box.append(more);
    }
  };
  page(0);
}

/** Renders the data file `text` at `path` into `box`; a file that does not parse says why and where. */
export function renderData(box: HTMLElement, text: string, path: string) {
  const kind = dataKind(path);
  const parsed = kind === "json" ? parseJson(text) : kind === "yaml" ? parseYaml(text) : parseDelimited(text, kind === "csv" ? "," : "\t");
  box.classList.add("dataview");
  if (!parsed.ok) {
    box.innerHTML = `<div class="dataerr">Line ${parsed.line}, column ${parsed.col}: ${esc(parsed.error)}. Source shows the text.</div>`;
    return;
  }
  if (kind === "csv" || kind === "tsv") return table(box, parsed.value as string[][]);
  tree(box, kind === "yaml" ? (parsed.value as Value[]) : [parsed.value as Value]);
}
