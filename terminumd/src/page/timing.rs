// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A page's latency trace (AC-38) wired to its stream and its keys: the `trace` messages,
//! each key timed from its arrival to the frame that shows it, what woke each capture, a
//! tmux round trip every few seconds while recording.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::trace::KeyRec;
use super::Page;
use crate::app::Shared;

/// A `trace` message from the page.
pub fn handle(_app: &Shared, page: &Page, msg: &Value) {
    let replies = page.tracer.lock().handle(msg, crate::config::build_id());
    for r in replies {
        page.send_json(&r);
    }
}

/// An `in` message arrived: its key's record, when keys are timed.
pub fn arrived(page: &Page, k: Option<u64>) -> Option<KeyRec> {
    page.tracer.lock().key(k.map(|k| k as i64))
}

/// The keys went to tmux.
pub fn typed(page: &Page, recs: Vec<Option<KeyRec>>, via: &str) {
    let mut t = page.tracer.lock();
    for mut r in recs {
        t.step(&mut r, "to_tmux_ms", &[("via", json!(via))]);
        t.typed(r);
    }
}

/// What woke the stream: "key", "output" or "poll".
pub fn woken(page: &Page, how: &str) {
    page.tracer.lock().woken(how);
}

/// A capture that began at `began` ended.
pub fn captured(page: &Page, began: Instant) {
    page.tracer.lock().captured(began);
}

/// The `keys` field of the frame about to be sent.
pub fn frame(page: &Page, moved: bool) -> Option<Value> {
    // the page's trace.js and tracebuf.js read the keys as `tr.ks`, the encoding time as `tr.enc`
    page.tracer.lock().frame(moved).map(|keys| {
        let enc = keys.as_array().and_then(|a| a.first()).map_or(Value::Null, |k| k["encode_ms"].clone());
        json!({"tr": {"ks": keys, "enc": enc}})
    })
}

/// Every 2 s while the page is open: keys that never showed, the end of a trace, a round trip.
pub async fn follow(page: Arc<Page>) {
    loop {
        tokio::time::sleep(super::trace::EVERY).await;
        if page.is_closed() {
            return;
        }
        let replies = page.tracer.lock().tick();
        for r in replies {
            page.send_json(&r);
        }
        if !page.tracer.lock().probe_due() {
            continue;
        }
        let shown = page.state.lock().shown.as_ref().map(|s| (s.server.clone(), s.pane.clone()));
        let Some((server, pane)) = shown else { continue };
        let t0 = Instant::now();
        let r = tokio::time::timeout(Duration::from_secs(2), server.run_all_on(&pane, vec!["display -p x".into()])).await;
        let result = match r {
            Ok(a) if a.first().is_some_and(|x| x.is_ok()) => Ok(t0.elapsed()),
            Ok(_) => Err("error"),
            Err(_) => Err("timeout"),
        };
        page.tracer.lock().probed(result);
    }
}
