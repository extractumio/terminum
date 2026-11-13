// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
// Direct page requests: one ref per attempt, bounded waits, and explicit cancellation (AC-02,
// AC-07, AC-32). Broadcasts still go through the page's normal message dispatch.
let next = 0;
const waiting = new Map();

export function request(send, msg, { ms = 15000, signal } = {}) {
  return new Promise((resolve, reject) => {
    const ref = `req-${++next}`;
    const finish = (m) => {
      waiting.delete(ref);
      clearTimeout(timer);
      signal?.removeEventListener("abort", cancel);
      const why = m.error || (m.t === "error" && m.msg);
      if (why) { const e = new Error(why); if (m.unsent) e.unsent = true; if (m.cancelled) e.name = "AbortError"; reject(e); } else resolve(m);
    };
    const cancel = () => finish({ error: "The request was cancelled.", cancelled: true });
    const timer = setTimeout(() => finish({ error: `The server did not answer within ${ms / 1000} s. Try again.` }), ms);
    if (signal?.aborted) return cancel();
    waiting.set(ref, finish);
    signal?.addEventListener("abort", cancel, { once: true });
    try { if (send({ ...msg, ref }) === false) finish({ error: "Not connected. Try again when the connection is back.", unsent: true }); }
    catch (e) { finish({ error: e.message }); }
  });
}

/** Consumes direct replies, including expired ones: they never fall through to another owner. */
export function answer(m) {
  if (typeof m.ref !== "string" || !/^req-\d+$/.test(m.ref)) return false;
  waiting.get(m.ref)?.(m);
  return true;
}

/** Requests owned by one view or form; reset cancels and settles every waiter. */
export class Requests {
  constructor(send) { this.send = send; this.active = new Set(); }
  get size() { return this.active.size; }
  async ask(msg, options = {}) {
    const ctl = new AbortController();
    this.active.add(ctl);
    try {
      const m = await request(this.send, msg, { ...options, signal: ctl.signal });
      if (ctl.signal.aborted) throw Object.assign(new Error("The request was cancelled."), { name: "AbortError" });
      return m;
    }
    finally { this.active.delete(ctl); }
  }
  cancel() { for (const ctl of this.active) ctl.abort(); this.active.clear(); }
}
