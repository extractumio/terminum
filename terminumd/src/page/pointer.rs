// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The page's `mouse` message for a full-screen program that asked for the mouse (AC-06).

use std::sync::Arc;

use serde_json::Value;

use super::Page;
use crate::app::Shared;
use crate::tmux::mouse;

/// A click, drag or wheel for a full-screen program, as the mouse report a terminal would send
/// (AC-06): queued with the keys (typing.rs), never awaited here.
pub(super) fn event(app: &Shared, page: &Arc<Page>, msg: &Value) {
    let Some((server, pane, key, f)) = (|| {
        let st = page.state.lock();
        let s = st.shown.as_ref()?;
        Some((s.server.clone(), s.pane.clone(), s.key.clone(), s.ctl.fields.lock().clone()?))
    })() else {
        return;
    };
    if !f.mouse {
        return; // the program did not ask for the mouse: nothing is sent
    }
    let Some(bytes) = from_message(msg, f.mouse_sgr) else { return };
    // a left release to a full-screen program: what it copies of its selection is this page's (AC-06)
    let arm = f.alt && msg["action"] == "release" && msg["button"] == 0;
    super::typing::mouse(app, page, server, pane, key, bytes, arm);
}

/// The page's `mouse` message as the bytes of a report.
fn from_message(msg: &Value, sgr: bool) -> Option<Vec<u8>> {
    let (button, action) = mouse::from_page(msg["button"].as_u64().unwrap_or(0).min(255) as u8, msg["action"].as_str().unwrap_or(""))?;
    let m = &msg["mods"];
    let mods = mouse::Mods { shift: m["shift"].as_bool().unwrap_or(false), meta: m["alt"].as_bool().unwrap_or(false), ctrl: m["ctrl"].as_bool().unwrap_or(false) };
    let coord = |k: &str| msg[k].as_u64().filter(|v| *v < 10_000).map(|v| v as u16);
    let one = mouse::report(coord("x")?, coord("y")?, button, action, mods, sgr)?;
    // the wheel's steps of one frame of the page come as one message with their count
    let n = if msg["action"].as_str().is_some_and(|a| a.starts_with("wheel")) { msg["count"].as_u64().unwrap_or(1).clamp(1, WHEEL_MAX) as usize } else { 1 };
    Some(one.repeat(n))
}

/// Most wheel steps one message carries.
const WHEEL_MAX: u64 = 100;
