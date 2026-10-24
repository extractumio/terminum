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

test("YAML scalars by the core schema", () => {
  for (const [t, v] of [["", null], ["~", null], ["NULL", null], ["True", true], ["false", false], ["12", 12], ["-3.5e2", -350], ["0o17", 15], [".inf", Infinity], ["1_000", 1000], ["yes", "yes"], ["1.2.3", "1.2.3"]]) assert.deepEqual(scalar(t), v, t);
});

test("CSV and TSV by RFC 4180: quotes, separators and line ends inside, CRLF; an unclosed quote said", () => {
  assert.deepEqual(parseDelimited('a,b,c\r\n1,"x, ""y""\nz",3\n', ","), { ok: true, value: [["a", "b", "c"], ["1", 'x, "y"\nz', "3"]] });
  assert.deepEqual(parseDelimited("a\tb\n1\t\n", "\t"), { ok: true, value: [["a", "b"], ["1", ""]] });
  const bad = parseDelimited('a,b\n1,"open\n2,3\n', ",");
  assert.deepEqual([bad.ok, bad.line, bad.col], [false, 2, 3]);
});

test("the kind of data by its name", () => {
  assert.deepEqual(["a.json", "b.YAML", "c.yml", "d.csv", "e.tsv", "f.geojson", "g.md", "h.jsonc"].map(dataKind), ["json", "yaml", "yaml", "csv", "tsv", "json", null, null]);
});
