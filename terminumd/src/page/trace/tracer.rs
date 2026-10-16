// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! One page's part of the latency trace (AC-38): the WebSocket `trace` messages, its keys
//! joined to the frame that shows them, the pane it opens, the tmux round trip and slow work.

use super::*;

impl Tracer {
    pub fn new(rec: Arc<Recorder>) -> Tracer {
        Tracer {
            rec,
            id: CLIENTS.fetch_add(1, Ordering::Relaxed),
            joined: None,
            stages: false,
            keys: Vec::new(),
            woke: Some("first"),
            read_at: None,
            read_ms: 0.0,
            read_woke: "first",
            mode: "local",
            opened: None,
            probed: Instant::now(),
        }
    }

    /// This page records into the file that is on.
    pub fn live(&self) -> bool {
        self.joined.is_some() && self.rec.current() == self.joined
    }

    /// Keys are timed: for the file, or for the page's own trace (`?trace=2`).
    pub fn timing(&self) -> bool {
        self.live() || self.stages
    }

    fn write(&self, mut rec: Map<String, Value>, with_mode: bool) {
        rec.insert("c".into(), json!(self.id));
        if with_mode {
            rec.insert("mode".into(), json!(self.mode));
        }
        self.rec.write("terminumd", rec);
    }

    fn record(&self, v: Value) {
        if let Value::Object(m) = v {
            self.write(m, false);
        }
    }

    /// A `trace` message of the page: the replies to send back. `build` goes into the start record.
    pub fn handle(&mut self, msg: &Value, build: &str) -> Vec<Value> {
        if let Some(n) = msg.get("ping").filter(|p| p.is_number()) {
            return vec![json!({"t": "trace", "pong": n})];
        }
        if let Some(s) = msg.get("stages") {
            self.stages = s == true;
            return vec![json!({"t": "trace", "stages": self.stages})];
        }
        if let Some(on) = msg.get("on") {
            if on == true {
                if self.rec.current().is_none() {
                    if msg.get("resume") == Some(&Value::Bool(true)) {
                        return vec![json!({"t": "trace", "on": false})]; // the trace ended while the page was away
                    }
                    if let Err(e) = self.rec.start() {
                        tracing::warn!(event = "trace.failed", msg = %e);
                        return vec![json!({"t": "trace", "on": false, "error": format!("The trace cannot start: {e}")})];
                    }
                }
                self.joined = self.rec.current(); // a trace already on is joined, not restarted
                self.record(json!({"ev": "start", "build": build, "mode": self.mode}));
            } else {
                self.rec.stop();
                self.joined = None;
            }
            let file = self.rec.path().and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())).unwrap_or_default();
            return vec![json!({"t": "trace", "on": self.live(), "left": self.rec.left(), "file": file})];
        }
        if self.live()
            && let Some(evs) = msg.get("ev").and_then(Value::as_array)
        {
            for r in evs.iter().take(MAX_BATCH) {
                if let Some(mut c) = clean(r) {
                    c.insert("c".into(), json!(self.id));
                    c.insert("mode".into(), json!(self.mode));
                    self.rec.write("page", c);
                }
            }
        }
        Vec::new()
    }

    /// Every `EVERY`: keys that never showed are noted; when the trace this page joined has
    /// ended (time, size, a failed write, another page), the reply that tells the page.
    pub fn tick(&mut self) -> Vec<Value> {
        let now = Instant::now();
        let (stale, keep): (Vec<KeyRec>, Vec<KeyRec>) = self.keys.drain(..).partition(|r| now - r.t0 > STALE);
        self.keys = keep;
        if self.live() {
            for r in stale {
                self.record(json!({"ev": "noecho", "k": r.k}));
            }
        } else if self.joined.take().is_some() {
            return vec![json!({"t": "trace", "on": false})];
        }
        Vec::new()
    }

    /// Whether to time a tmux round trip now (every `PROBE_EVERY` while recording).
    pub fn probe_due(&mut self) -> bool {
        if !self.live() || self.probed.elapsed() < PROBE_EVERY {
            return false;
        }
        self.probed = Instant::now();
        true
    }

    /// The round trip's time, or the kind of its failure (a probe never stops the trace).
    pub fn probed(&self, result: Result<Duration, &str>) {
        if self.live() {
            self.record(match result {
                Ok(d) => json!({"ev": "tmux", "ms": ms(d)}),
                Err(kind) => json!({"ev": "tmux", "error": kind.chars().take(40).collect::<String>()}),
            });
        }
    }

    /// A message sent to the page: its kind (terminumd's own word), size and time to send.
    pub fn sent(&self, kind: &str, bytes: usize, began: Instant) {
        if self.live() {
            self.record(json!({"ev": "tx", "t": kind, "bytes": bytes, "ms": since(began)}));
        }
    }

    /// A page message handled: noted when it took longer than 20 ms.
    pub fn handled(&self, kind: &str, began: Instant) {
        let took = since(began);
        if self.live() && took > SLOW_MSG && kind != "trace" {
            let t = if ["sub", "in", "more", "fit", "files", "new", "size", "paste"].contains(&kind) { kind } else { "other" };
            self.record(json!({"ev": "msg", "t": t, "ms": took}));
        }
    }

    /// History sent (`mode`: reset|append|prepend).
    pub fn hist(&self, mode: &str, lines: usize, began: Instant) {
        if self.live() {
            let mode = if ["reset", "append", "prepend"].contains(&mode) { mode } else { "other" };
            self.record(json!({"ev": "hist", "mode": mode, "lines": lines, "ms": since(began)}));
        }
    }

    /// The page asked for this pane: keys on their way are dropped; its first frame is timed.
    pub fn opening(&mut self, sid: &str, remote: bool) {
        self.keys.clear();
        self.woke = Some("first");
        self.mode = if remote { "remote" } else { "local" };
        self.opened = if self.live() { pane_of(sid).map(|p| (p, Instant::now())) } else { None };
    }

    /// An `in` message's key number: its record, or None when nothing is timed.
    pub fn key(&self, k: Option<i64>) -> Option<KeyRec> {
        let k = k.filter(|_| self.timing())?;
        let t = Instant::now();
        Some(KeyRec { k, t0: t, m: t, fields: Map::new(), polls: 0, skip: 0 })
    }

    /// A stage of the key ended: `name` gets its time since the last stage, `fields` are
    /// terminumd's words (e.g. `("via", "keys")`, `("kind", "char")`), never typed text.
    pub fn step(&self, rec: &mut Option<KeyRec>, name: &str, fields: &[(&str, Value)]) {
        if let Some(r) = rec {
            let t = Instant::now();
            r.fields.insert(name.into(), json!(ms(t - r.m)));
            r.m = t;
            for (k, v) in fields {
                r.fields.insert((*k).into(), v.clone());
            }
        }
    }

    /// The key went to tmux: it waits for a frame that shows it.
    pub fn typed(&mut self, rec: Option<KeyRec>) {
        if let Some(r) = rec {
            let now = Instant::now();
            self.keys.retain(|k| now - k.t0 < STALE);
            self.keys.push(r);
        }
    }

    /// What woke the stream for its next capture: "key", "output" or "poll"; the first counts.
    pub fn woken(&mut self, how: &str) {
        if self.woke.is_none() {
            self.woke = ["key", "output", "poll"].into_iter().find(|w| *w == how);
        }
    }

    /// A capture that began at `began` ended: its time and cause; keys typed before it count a poll.
    pub fn captured(&mut self, began: Instant) {
        if self.timing() {
            self.read_at = Some(began);
            self.read_ms = since(began);
            self.read_woke = self.woke.unwrap_or("next");
            for r in &mut self.keys {
                if r.m <= began {
                    r.polls += 1;
                }
            }
        }
        self.woke = None;
    }

    /// The `keys` field of the frame about to be sent: the keys typed before its capture began,
    /// when the cursor moved or its row changed (`moved`: a spinner's frame answers no key).
    pub fn frame(&mut self, moved: bool) -> Option<Value> {
        if !self.timing() {
            return None;
        }
        if let Some((pane, t0)) = self.opened.take()
            && self.live()
        {
            self.record(json!({"ev": "open", "sid": pane, "ms": since(t0), "mode": self.mode}));
        }
        let read_at = self.read_at?;
        let (shown, waiting): (Vec<KeyRec>, Vec<KeyRec>) = self.keys.drain(..).partition(|r| r.m <= read_at);
        self.keys = waiting;
        if shown.is_empty() {
            return None;
        }
        if !moved {
            self.keys.extend(shown.into_iter().map(|mut r| {
                r.skip += 1;
                r
            }));
            return None;
        }
        let now = Instant::now();
        let encode = ((ms(now - read_at) - self.read_ms) * 100.0).round() / 100.0;
        let mut ks = Vec::new();
        for r in shown {
            let mut b = r.fields;
            b.insert("k".into(), json!(r.k));
            b.insert("woken".into(), json!(self.read_woke));
            b.insert("until_change_ms".into(), json!(ms(read_at.saturating_duration_since(r.m))));
            b.insert("polls".into(), json!(r.polls.saturating_sub(1)));
            b.insert("capture_ms".into(), json!(self.read_ms));
            b.insert("encode_ms".into(), json!(encode));
            b.insert("total_ms".into(), json!(ms(now - r.t0)));
            b.insert("skip".into(), json!(r.skip));
            b.insert("moved".into(), json!(true));
            if self.live() {
                let mut rec = b.clone();
                rec.insert("ev".into(), json!("key"));
                self.write(rec, true);
            }
            ks.push(Value::Object(b));
        }
        Some(Value::Array(ks))
    }
}
