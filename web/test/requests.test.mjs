// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// AC-02, AC-07, AC-32: dropped, failed, reordered and cancelled direct requests settle only their owners.
import assert from "node:assert/strict";
import { test } from "node:test";
import { request, Requests, answer } from "../static/requests.js";
import { Folds } from "../static/folds.js";

test("success and errors answer only their own request, once", async () => {
  const sent = [], send = (m) => sent.push(m);
  const a = request(send, { t: "more" }), b = request(send, { t: "sentget" });
  const failed = assert.rejects(a, /refused/);
  assert.notEqual(sent[0].ref, sent[1].ref);
  assert.equal(answer({ t: "layout" }), false);
  answer({ t: "sentone", ref: sent[1].ref, n: 2 });
  assert.equal((await b).n, 2);
  answer({ t: "error", ref: sent[0].ref, msg: "refused" });
  await failed;
  assert.equal(answer({ t: "sentone", ref: sent[1].ref }), true, "a late reply never falls through to another owner");
});

test("timeout, offline and cancellation release all owners; retry has a new ref", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const sent = [], reads = new Requests((m) => sent.push(m));
  const a = reads.ask({ t: "more" }), failed = assert.rejects(a, /15 s/);
  t.mock.timers.tick(15001);
  await failed;
  assert.equal(reads.size, 0);
  const b = reads.ask({ t: "more" });
  const cancelled = assert.rejects(b, { name: "AbortError" });
  assert.notEqual(sent[0].ref, sent[1].ref);
  answer({ t: "hist", ref: sent[0].ref });
  reads.cancel();
  await cancelled;
  assert.equal(reads.size, 0);
  await assert.rejects(request(() => false, { t: "more" }), /Not connected/);
});

test("cancelling after receipt but before its continuation prevents stale work", async () => {
  const sent = [], reads = new Requests((m) => sent.push(m));
  const p = reads.ask({ t: "more" });
  const cancelled = assert.rejects(p, { name: "AbortError" });
  answer({ t: "hist", ref: sent[0].ref });
  reads.cancel();
  await cancelled;
});

test("fold copies share a capture and every copy settles on success, reset, timeout and error", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const sent = [], notices = [], fills = [];
  const folds = new Folds({ view: { refill: (...a) => fills.push(a) }, send: (m) => sent.push(m), notify: (s) => notices.push(s) });
  folds.records = () => []; folds.textOf = () => "the fold"; folds.apply = () => {};
  const f = { n: 10, to: 15 };
  const copy = () => folds.ask(f, "copy");
  const a = copy(), b = copy();
  assert.equal(sent.length, 1);
  answer({ t: "lines", ref: sent[0].ref, first: 10, lines: ["line"] });
  assert.deepEqual(await Promise.all([a, b]), ["the fold", "the fold"]);
  await Promise.resolve();
  const c = copy(), d = copy();
  const reset = Promise.all([assert.rejects(c, { name: "AbortError" }), assert.rejects(d, { name: "AbortError" })]);
  folds.reset();
  await reset;
  answer({ t: "lines", ref: sent[1].ref, first: 10, lines: ["stale"] });
  assert.equal(fills.length, 1);
  const e = copy(), timed = assert.rejects(e, /15 s/);
  t.mock.timers.tick(15001); await timed;
  await Promise.resolve();
  const g = copy(), failed = assert.rejects(g, /capture failed/);
  answer({ t: "lines", ref: sent.at(-1).ref, error: "capture failed" });
  await failed;
  assert.ok(notices.some((s) => s.includes("capture failed")));
});

test("fold completion waits for placement; reset invalidates deferred work and settles its copy", async () => {
  const sent = [], deferred = [], fills = [];
  const folds = new Folds({ view: { refill: (...args) => fills.push(args) }, send: (m) => sent.push(m), notify: () => {}, keepPlace: (fn) => deferred.push(fn) });
  folds.records = () => []; folds.textOf = () => "ready"; folds.apply = () => {};
  let copied = false;
  const copy = folds.ask({ n: 10, to: 20 }, "copy").then(() => { copied = true; });
  const cancelled = assert.rejects(copy, { name: "AbortError" });
  answer({ t: "lines", ref: sent[0].ref, first: 10, lines: ["x"] });
  await new Promise(setImmediate);
  assert.equal(deferred.length, 1);
  assert.equal(copied, false);
  folds.reset();
  await cancelled;
  deferred[0]();
  assert.equal(fills.length, 0, "a deferred response cannot refill another pane");
});

test("a fold which grew while a copy waits gets a separate capture of its complete range", async () => {
  const sent = [], folds = new Folds({ view: { refill() {} }, send: (m) => sent.push(m), notify: () => {} });
  folds.records = () => []; folds.textOf = (f) => `through ${f.to}`; folds.apply = () => {};
  const copy = (to) => folds.ask({ n: 10, to }, "copy");
  const a = copy(20), b = copy(30);
  assert.equal(sent.length, 2);
  answer({ t: "lines", ref: sent[1].ref, first: 10, lines: [] });
  answer({ t: "lines", ref: sent[0].ref, first: 10, lines: [] });
  assert.deepEqual(await Promise.all([a, b]), ["through 20", "through 30"]);
  assert.equal(folds.open.size, 0, "Copy leaves the fold closed");
});
