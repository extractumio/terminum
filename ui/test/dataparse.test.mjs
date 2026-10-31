// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// JSON, YAML, CSV and TSV read for their rendered view (dataparse.ts, AC-94).
import assert from "node:assert/strict";
import { test } from "node:test";
import { load } from "./bundle.mjs";
const { dataKind, parseDelimited, parseJson, parseYaml, scalar } = await load("src/dataparse.ts");

test("JSON: a value, or why not and where", () => {
  assert.deepEqual(parseJson('{"a": [1, true, null]}'), { ok: true, value: { a: [1, true, null] } });
  const bad = parseJson('{\n  "a": 1,\n  "b": }');
  assert.equal(bad.ok, false);
  assert.deepEqual([bad.line, bad.col], [3, 8], JSON.stringify(bad));
  assert.ok(!bad.error.includes('"a"'), "the engine's quote of the text left out: " + bad.error);
});

test("YAML: mappings, sequences, flow, quotes, block scalars, anchors, several documents", () => {
  const text = `a: 1\nb: [x, "y\\tz", {k: v}]\nc:\n  - 1.5\n  - two: 2\n    three: null\n    four: ~\nd: |\n  line1\n  line2\ne: >-\n  folded\n  text\nf: &an hi\ng: *an\nh: 'it''s'\ni: true\nj: 0x1F\nk: "07"\n---\n- doc2\n`;
  const r = parseYaml(text);
  assert.equal(r.ok, true, JSON.stringify(r));
  assert.deepEqual(r.value[0], { a: 1, b: ["x", "y\tz", { k: "v" }], c: [1.5, { two: 2, three: null, four: null }], d: "line1\nline2\n",
    e: "folded text", f: "hi", g: "hi", h: "it's", i: true, j: 31, k: "07" });
  assert.deepEqual(r.value[1], ["doc2"]);
});

test("YAML: a syntax error says where", () => {
  const r = parseYaml("a: 1\nb: [1, 2\nc: 3\n");
  assert.equal(r.ok, false);
  assert.ok(r.line >= 2, JSON.stringify(r));
});

test("YAML: aliases stay within their document and must follow a defined anchor", () => {
  const same = parseYaml("a: &value {x: 1}\nb: *value\n---\na: &value 2\nb: *value\n");
  assert.deepEqual(same, { ok: true, value: [{ a: { x: 1 }, b: { x: 1 } }, { a: 2, b: 2 }] });
  for (const [text, line, col] of [
    ["a: *missing\n", 1, 4],
    ["a: *later\nb: &later 1\n", 1, 4],
    ["a: &old 42\n---\nb: *old\n", 3, 4],
  ]) {
    const r = parseYaml(text);
    assert.deepEqual([r.ok, r.line, r.col], [false, line, col], JSON.stringify(r));
    assert.match(r.error, /anchor/i);
  }
});

test("YAML: explicit block indentation preserves extra spaces at every nesting level", () => {
  const r = parseYaml("top: |2\n    indented\nnested:\n  text: |2-\n      more\nlist:\n  - |2\n      item\ncompact:\n  - text: |2\n        value\n");
  assert.deepEqual(r, { ok: true, value: [{ top: "  indented\n", nested: { text: "  more" }, list: ["  item\n"], compact: [{ text: "  value\n" }] }] });
  assert.deepEqual(parseYaml("a: >2-\n    keep\n    space\n"), { ok: true, value: [{ a: "  keep\n  space" }] });
});

test("YAML: escaped quoted line endings join without a space, literal backslashes still fold", () => {
  for (const newline of ["\n", "\r\n"]) {
    const continued = `a: "line\\${newline}  continued"${newline}`;
    assert.deepEqual(parseYaml(continued), { ok: true, value: [{ a: "linecontinued" }] });
    const backslash = `a: "line\\\\${newline}  continued"${newline}`;
    assert.deepEqual(parseYaml(backslash), { ok: true, value: [{ a: "line\\ continued" }] });
  }
});

test("YAML scalars by the core schema", () => {
  for (const [t, v] of [["", null], ["~", null], ["NULL", null], ["True", true], ["false", false], ["12", 12], ["-3.5e2", -350], ["0o17", 15], [".inf", Infinity], ["1_000", 1000], ["yes", "yes"], ["1.2.3", "1.2.3"]]) assert.deepEqual(scalar(t), v, t);
});

test("CSV and TSV by RFC 4180: quotes, separators and line ends inside, CRLF; an unclosed quote said", () => {
  assert.deepEqual(parseDelimited('a,b,c\r\n1,"x, ""y""\nz",3\n', ","), { ok: true, value: [["a", "b", "c"], ["1", 'x, "y"\nz', "3"]] });
  assert.deepEqual(parseDelimited("a\tb\n1\t\n", "\t"), { ok: true, value: [["a", "b"], ["1", ""]] });
  const bad = parseDelimited('a,b\n1,"open\n2,3\n', ",");
  assert.deepEqual([bad.ok, bad.line, bad.col], [false, 2, 3]);
});

test("YAML: keys named as Object's own (__proto__, constructor, toString) are members like any", () => {
  const r = parseYaml("record:\n  __proto__: {visible: 1}\n  regular: 2\n__proto__: 3\nconstructor: x\ntoString: y\n");
  assert.equal(r.ok, true, JSON.stringify(r));
  const [doc] = r.value;
  assert.deepEqual(Object.keys(doc), ["record", "__proto__", "constructor", "toString"]);
  assert.deepEqual(Object.keys(doc.record), ["__proto__", "regular"]);
  assert.equal(JSON.stringify(doc.record), '{"__proto__":{"visible":1},"regular":2}', "Copy keeps it");
  assert.equal(Object.getPrototypeOf(doc.record), Object.prototype, "the prototype untouched");
});

test("YAML: a mapping's key twice is an error where the second stands; the same key in two mappings is not", () => {
  const twice = parseYaml("name: a\nport: 8080\nport: 9000\n");
  assert.deepEqual([twice.ok, twice.line, twice.col], [false, 3, 1], JSON.stringify(twice));
  assert.match(twice.error, /"port".*twice/);
  const flow = parseYaml("a: {x: 1, x: 2}\n");
  assert.equal(flow.ok, false, JSON.stringify(flow));
  assert.equal(parseYaml("1: a\n'1': b\n").ok, false, "two keys YAML tells apart (a number, a string) but one name in the tree: said, not one dropped");
  assert.equal(parseYaml("a: {x: 1}\nb: {x: 2}\n").ok, true);
});

test("YAML: an escape past Unicode is an error where its string stands, not a throw; the last character and an astral one read", () => {
  const bad = parseYaml('a: 1\nname: "\\UFFFFFFFF"\n');
  assert.deepEqual([bad.ok, bad.line, bad.col], [false, 2, 7], JSON.stringify(bad));
  assert.deepEqual(parseYaml('a: "\\U0010FFFF\\U0001F600\\x41"\n'), { ok: true, value: [{ a: "\u{10FFFF}\u{1F600}A" }] });
  assert.equal(parseYaml('a: "\\U00110000"\n').ok, false);
});

test("CSV: an empty quoted field is a record, with or without a last line end; an empty text has none", () => {
  assert.deepEqual(parseDelimited('header\n""', ","), { ok: true, value: [["header"], [""]] });
  assert.deepEqual(parseDelimited('header\n""\n', ","), { ok: true, value: [["header"], [""]] });
  assert.deepEqual(parseDelimited('""', ","), { ok: true, value: [[""]] });
  assert.deepEqual(parseDelimited("a,\n1,", ","), { ok: true, value: [["a", ""], ["1", ""]] });
  assert.deepEqual(parseDelimited("", ","), { ok: true, value: [] });
});

test("the kind of data by its name", () => {
  assert.deepEqual(["a.json", "b.YAML", "c.yml", "d.csv", "e.tsv", "f.geojson", "g.md", "h.jsonc"].map(dataKind), ["json", "yaml", "yaml", "csv", "tsv", "json", null, null]);
});

test("YAML: escaped line continuations keep subsequent blank lines", () => {
  for (const newline of ["\n", "\r\n"]) for (const blanks of [1, 2]) {
    const text = 'a: "foo\\' + newline.repeat(blanks + 1) + '  bar"' + newline;
    assert.deepEqual(parseYaml(text), { ok: true, value: [{ a: "foo" + "\n".repeat(blanks) + "bar" }] });
  }
});
