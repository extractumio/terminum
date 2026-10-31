// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// JSON, YAML, CSV and TSV read for their rendered view (AC-94). Pure: no DOM. YAML through the
// Lezer grammar the editor highlights it with (already in the bundle): its tree turned into
// values by YAML 1.2's core schema; anchors and aliases resolved, tags left out.

import { parser } from "@lezer/yaml";
import type { SyntaxNode } from "@lezer/common";

export type Value = null | boolean | number | string | Value[] | { [key: string]: Value };
/** Parsed, or why not and where (1-based). */
export type Parsed<T> = { ok: true; value: T } | { ok: false; error: string; line: number; col: number };

/** Line and column (1-based) of offset `at` in `text`. */
export function lineCol(text: string, at: number) {
  const before = text.slice(0, Math.max(0, at)).split("\n");
  return { line: before.length, col: before.at(-1)!.length + 1 };
}

export function parseJson(text: string): Parsed<Value> {
  try { return { ok: true, value: JSON.parse(text) }; } catch (e: any) {
    const msg = String(e.message ?? e).replace(/^JSON\.parse: |^JSON Parse error: /, "").replace(/, (\.\.\.)?"[\s\S]*" is not valid JSON$/, "");
    return { ok: false, error: msg, ...lineCol(text, jsonErrorAt(text)) };
  }
}

/** Where a JSON text stops being JSON (engines' messages do not always say): a scan of its grammar. */
export function jsonErrorAt(text: string): number {
  let i = 0;
  const ws = () => { while (/[ \t\n\r]/.test(text[i] ?? "")) i++; };
  const fail = () => { throw i; };
  const value = (): void => {
    ws();
    const c = text[i];
    if (c === "{") {
      i++; ws();
      if (text[i] === "}") { i++; return; }
      for (;;) { ws(); if (text[i] !== '"') fail(); string(); ws(); if (text[i] !== ":") fail(); i++; value(); ws(); if (text[i] === ",") { i++; continue; } if (text[i] === "}") { i++; return; } fail(); }
    }
    if (c === "[") {
      i++; ws();
      if (text[i] === "]") { i++; return; }
      for (;;) { value(); ws(); if (text[i] === ",") { i++; continue; } if (text[i] === "]") { i++; return; } fail(); }
    }
    if (c === '"') return string();
    const m = text.slice(i).match(/^(-?(0|[1-9]\d*)(\.\d+)?([eE][-+]?\d+)?|true|false|null)/);
    if (!m) fail();
    i += m![0].length;
  };
  const string = () => {
    i++;
    while (i < text.length && text[i] !== '"') {
      if (text[i] === "\\") { if (!/["\\/bfnrtu]/.test(text[i + 1] ?? "")) { i++; fail(); } i += text[i + 1] === "u" ? 6 : 2; continue; }
      if (text.charCodeAt(i) < 32) fail();
      i++;
    }
    if (text[i] !== '"') fail();
    i++;
  };
  try { value(); ws(); return i < text.length ? i : text.length; } catch (at) { return at as number; }
}

// ── YAML ─────────────────────────────────────────────────────────────────────

const NULL = /^(~|null|Null|NULL)?$/, TRUE = /^(true|True|TRUE)$/, FALSE = /^(false|False|FALSE)$/;
const INT = /^[-+]?(0|[1-9][0-9_]*)$/, OCT = /^0o[0-7]+$/, HEX = /^0x[0-9a-fA-F]+$/, FLOAT = /^[-+]?(\.[0-9]+|[0-9][0-9_]*(\.[0-9]*)?)([eE][-+]?[0-9]+)?$/;

/** A plain scalar by the core schema: null, a boolean, a number, else the text. */
export function scalar(t: string): Value {
  if (NULL.test(t)) return null;
  if (TRUE.test(t)) return true;
  if (FALSE.test(t)) return false;
  if (INT.test(t) || FLOAT.test(t)) return Number(t.replace(/_/g, ""));
  if (OCT.test(t)) return parseInt(t.slice(2), 8);
  if (HEX.test(t)) return parseInt(t.slice(2), 16);
  if (/^[-+]?\.(inf|Inf|INF)$/.test(t)) return t.startsWith("-") ? -Infinity : Infinity;
  if (/^\.(nan|NaN|NAN)$/.test(t)) return NaN;
  return t;
}

/** What makes a YAML text no data though its grammar holds: why, and the offset where. */
class YamlError { constructor(readonly error: string, readonly at: number) {} }

const ESC: Record<string, string> = { n: "\n", t: "\t", r: "\r", "0": "\0", '"': '"', "\\": "\\", "/": "/", b: "\b", f: "\f", e: "\x1b", " ": " ", N: "\u0085", _: " " };
/** A quoted scalar's text (`at`: its offset, for an escape past Unicode). */
function quoted(t: string, at: number): string {
  const fold = (s: string) => s.replace(/[ \t]*\n[ \t]*/g, "\n").replace(/([^\n])\n(?!\n)/g, "$1 ").replace(/\n(\n+)/g, "$1");
  const body = t.slice(1, -1).replace(/\r\n?/g, "\n");
  if (t.startsWith("'")) return fold(body).replace(/''/g, "'");
  // Consume escapes in pairs: an escaped backslash before a line end is not a continuation.
  const continued = body.replace(/\\(?:\n[ \t]*(?:\n[ \t]*)*|[^\n])/g, (s) =>
    s[1] === "\n" ? "\\n".repeat(s.split("\n").length - 2) : s);
  return fold(continued).replace(/\\(x[0-9a-fA-F]{2}|u[0-9a-fA-F]{4}|U[0-9a-fA-F]{8}|.)/g, (_, c: string) => {
    if (c.length === 1) return ESC[c] ?? c;
    const code = parseInt(c.slice(1), 16);
    if (code > 0x10ffff) throw new YamlError(`\\${c} is past the last Unicode character`, at);
    return String.fromCodePoint(code);
  });
}

/** A block scalar: "|" keeps its line ends, ">" folds lines into spaces; "-" strips the last line
 *  end, "+" keeps every one. */
function block(header: string, content: string, baseIndent: number): string {
  const lines = content.split("\n");
  const explicit = header.match(/[1-9]/)?.[0];
  const indent = explicit ? baseIndent + Number(explicit) : Math.min(...lines.filter((l) => l.trim()).map((l) => l.match(/^ */)![0].length));
  const body = lines.map((l) => l.slice(Number.isFinite(indent) ? indent : 0)).join("\n");
  const trailing = body.match(/\n*$/)![0], core = body.slice(0, body.length - trailing.length);
  const text = header.includes(">") ? core.replace(/([^\n])\n(?=[^\n ])/g, "$1 ") : core;
  return text + (header.includes("-") ? "" : header.includes("+") ? "\n" + trailing : "\n");
}

/** Every document of a YAML stream, or the first error (a syntax error, a mapping's key twice,
 *  an escape past Unicode). */
export function parseYaml(text: string): Parsed<Value[]> {
  const tree = parser.parse(text);
  let bad: SyntaxNode | null = null;
  tree.iterate({ enter: (n) => { if (!bad && n.type.isError) bad = n.node; return !bad; } });
  if (bad) return { ok: false, error: "Not valid YAML here", ...lineCol(text, (bad as SyntaxNode).from) };
  const anchors = new Map<string, Value>();
  const src = (n: SyntaxNode) => text.slice(n.from, n.to);
  const kids = (n: SyntaxNode, name: string) => { const out: SyntaxNode[] = []; for (let c = n.firstChild; c; c = c.nextSibling) if (c.name === name) out.push(c); return out; };
  const valueOf = (n: SyntaxNode | null): Value => {
    if (!n) return null;
    switch (n.name) {
      case "Literal": return scalar(src(n).trim());
      case "QuotedLiteral": return quoted(src(n), n.from);
      case "BlockLiteral": {
        let parent = n.parent;
        while (parent && !/^(Pair|Item|Document)$/.test(parent.name)) parent = parent.parent;
        const at = parent?.from ?? 0, line = text.lastIndexOf("\n", at - 1) + 1;
        const before = text.slice(line, at);
        const base = parent?.name === "Item" ? Math.max(0, before.lastIndexOf("-")) : parent?.name === "Document" ? 0 : before.length;
        return block(src(n.getChild("BlockLiteralHeader")!), n.getChild("BlockLiteralContent") ? src(n.getChild("BlockLiteralContent")!) : "", base);
      }
      case "Alias": {
        const name = src(n).slice(1);
        if (!anchors.has(name)) throw new YamlError(`No earlier anchor named ${JSON.stringify(name)} in this document`, n.from);
        return anchors.get(name)!;
      }
      case "Anchored": case "Tagged": {
        let inner: SyntaxNode | null = n.firstChild;
        while (inner && (inner.name === "Anchor" || inner.name === "Tag")) inner = inner.nextSibling;
        const v = valueOf(inner);
        const anchor = n.getChild("Anchor");
        if (anchor) anchors.set(src(anchor).slice(1), v);
        return v;
      }
      case "BlockSequence": case "FlowSequence": return kids(n, "Item").map((i) => valueOf(i.firstChild));
      case "BlockMapping": case "FlowMapping": {
        const out: { [k: string]: Value } = {};
        for (const pair of kids(n, "Pair")) {
          const key = pair.getChild("Key");
          const k = String(key ? valueOf(key.firstChild) : "");
          if (Object.hasOwn(out, k)) throw new YamlError(`The key ${JSON.stringify(k)} is in this mapping twice`, (key ?? pair).from);
          let v: SyntaxNode | null = pair.getChild(":")?.nextSibling ?? null;
          while (v && v.name === "Comment") v = v.nextSibling;
          // its own member whatever its name: out["__proto__"] = … would set the prototype
          Object.defineProperty(out, k, { value: valueOf(v), enumerable: true, writable: true, configurable: true });
        }
        return out;
      }
      default: return valueOf(n.firstChild);
    }
  };
  try {
    const docs = tree.topNode.getChildren("Document").map((d) => {
      anchors.clear();
      let c = d.firstChild;
      while (c && /^(DirectiveEnd|DocEnd|Comment|Directive)$/.test(c.name)) c = c.nextSibling;
      return valueOf(c);
    });
    return { ok: true, value: docs.length ? docs : [null] };
  } catch (e) {
    if (e instanceof YamlError) return { ok: false, error: e.error, ...lineCol(text, e.at) };
    throw e;
  }
}

// ── CSV and TSV ──────────────────────────────────────────────────────────────

/** Rows of a CSV (",") or TSV ("\t") text, RFC 4180: a quoted field holds the separator, quotes
 *  ("" for one) and line ends; a line end is \n or \r\n; a last empty line is none. */
export function parseDelimited(text: string, sep: "," | "\t"): Parsed<string[][]> {
  const rows: string[][] = [];
  let row: string[] = [], field = "", i = 0, quotedAt = -1, quotedRow = false;   // quotedRow: a quoted field began in this row ("" is one)
  const n = text.length;
  while (i < n) {
    const c = text[i];
    if (quotedAt >= 0) {
      if (c === '"') {
        if (text[i + 1] === '"') { field += '"'; i += 2; continue; }
        quotedAt = -1; i++; continue;
      }
      field += c; i++; continue;
    }
    if (c === '"' && field === "") { quotedAt = i; quotedRow = true; i++; continue; }
    if (c === sep) { row.push(field); field = ""; i++; continue; }
    if (c === "\n" || c === "\r") {
      row.push(field); rows.push(row); row = []; field = ""; quotedRow = false;
      i += c === "\r" && text[i + 1] === "\n" ? 2 : 1;
      continue;
    }
    field += c; i++;
  }
  if (quotedAt >= 0) return { ok: false, error: "A quoted field is not closed", ...lineCol(text, quotedAt) };
  if (field !== "" || row.length || quotedRow) { row.push(field); rows.push(row); }
  return { ok: true, value: rows };
}

/** The kind of data a path holds, or null. */
export function dataKind(path: string): "json" | "yaml" | "csv" | "tsv" | null {
  const m = path.toLowerCase().match(/\.(json|geojson|webmanifest|ya?ml|csv|tsv|tab)$/);
  if (!m) return null;
  return /^(json|geojson|webmanifest)$/.test(m[1]) ? "json" : /^ya?ml$/.test(m[1]) ? "yaml" : m[1] === "csv" ? "csv" : "tsv";
}
