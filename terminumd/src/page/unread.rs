// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Unread (AC-93): a pane whose coding agent ended a turn while no page in sight showed it
//! is unread until a page shows it, and the list marks its row. Kept while terminumd runs; a
//! change marks the list as last sent and tells the pages (no new read of the servers).

use serde_json::{json, Value};

use crate::app::Shared;

/// The panes whose agent ended a turn: those no page in sight shows are unread now.
pub fn ended(app: &Shared, keys: &[String]) {
    let looking: Vec<String> = app.pages.all().iter().filter_map(|p| {
        let st = p.state.lock();
        st.shown.as_ref().filter(|_| !st.paused).map(|s| s.key.clone())
    }).collect();
    let mut unread = app.unread.lock();
    let more: Vec<&String> = keys.iter().filter(|k| !looking.contains(k) && unread.insert((*k).clone())).collect();
    drop(unread);
    if !more.is_empty() {
        relist(app);
    }
}

/// The list as last sent takes the marks as they are now, and every page gets it.
fn relist(app: &Shared) {
    let told = {
        let mut layout = app.layout.write();
        if let Some(groups) = layout.as_array_mut() {
            mark(app, groups, false);
        }
        json!({"t": "layout", "groups": *layout}).to_string()
    };
    app.pages.tell(&told);
}

/// A page in sight shows pane `key`: it is read.
pub fn read(app: &Shared, key: &str) {
    if app.unread.lock().remove(key) {
        relist(app);
    }
}

/// The list's rows of unread panes say so; when the list is whole, panes gone are forgotten.
pub fn mark(app: &Shared, groups: &mut [Value], whole: bool) {
    let mut unread = app.unread.lock();
    let mut listed = std::collections::HashSet::new();
    for it in groups.iter_mut().flat_map(|g| g["items"].as_array_mut().into_iter().flatten()) {
        let Some(id) = it["id"].as_str() else { continue };
        let on = unread.contains(id);
        listed.insert(id.to_string());
        match it.as_object_mut() {
            Some(o) if on => { o.insert("unread".into(), json!(true)); }
            Some(o) => { o.remove("unread"); }
            None => {}
        }
    }
    if whole {
        unread.retain(|k| listed.contains(k));
    }
}
