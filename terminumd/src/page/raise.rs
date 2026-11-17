// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A session whose coding agent took a message goes to the top of where the list shows it
//! (AC-92): of its project, kept in the store; else of its tmux group, whose order is its
//! windows' (AC-07), moved as a drag moves them. The user's own tmux (AC-89) is left as it is.

use serde_json::Value;

use crate::app::Shared;
use crate::page::marks::layout_items;

/// One raise at a time: two prompts close together each move from the order the other left.
static TURN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Whether a layout row stands in the list itself: a window's first pane, not a quick shell
/// (a split's other panes and a quick shell go under such a row, as the page draws them).
fn is_row(it: &Value) -> bool {
    it["pane"].as_u64().unwrap_or(0) <= 1 && it["quickOf"].is_null()
}

/// The row pane `key` stands under in the list. None when the layout has no such pane.
pub(crate) fn row_of(layout: &Value, key: &str) -> Option<String> {
    for g in layout.as_array()? {
        let items = g["items"].as_array().map_or(&[][..], Vec::as_slice);
        let Some(it) = items.iter().find(|it| it["id"] == key) else { continue };
        if let Some(parent) = it["quickOf"].as_str() {
            return Some(parent.to_string());
        }
        let first = items.iter().find(|x| x["window"] == it["window"] && is_row(x));
        return first.and_then(|x| x["id"].as_str()).map(str::to_string);
    }
    None
}

/// Where row `row` moves in its tmux group: the group's id, its window, and the windows of the
/// group's rows the list shows (those in no project, `in_project` says). None when the group
/// shows no such row.
pub(crate) fn group_windows(layout: &Value, row: &str, in_project: impl Fn(&str) -> bool) -> Option<(String, String, Vec<String>)> {
    let g = layout.as_array()?.iter().find(|g| g["items"].as_array().is_some_and(|its| its.iter().any(|it| it["id"] == row)))?;
    let shown: Vec<&Value> = layout_items(std::slice::from_ref(g)).filter(|it| is_row(it) && it["id"].as_str().is_some_and(|id| !in_project(id))).collect();
    let top = shown.iter().find(|it| it["id"] == row)?["window"].as_str()?.to_string();
    let free = shown.iter().filter_map(|it| it["window"].as_str()).map(str::to_string).collect();
    Some((g["wid"].as_str()?.to_string(), top, free))
}

/// Pane `key`'s agent took a message: its row goes to the top of its project, or of its tmux
/// group, every page told (AC-92).
pub async fn raise(app: &Shared, key: &str) {
    let _turn = TURN.lock().await;
    // a pane the layout does not list yet is its own row
    let row = row_of(&app.layout.read(), key).unwrap_or_else(|| key.to_string());
    let Some(members) = crate::page::projects::raise(app, &row).await else { return };
    let found = group_windows(&app.layout.read(), &row, |k| members.contains(k));
    let Some((wid, top, free)) = found else { return };
    let Some((server, session)) = super::sessions::group(app, &wid) else { return };
    if server.is_users() {
        return;
    }
    match super::sessions::to_top(&server, &session, &top, &free).await {
        Ok(true) => { crate::lists::request(app, &server); }
        Ok(false) => {}
        Err(e) => tracing::warn!(event = "sessions.unraised", group = %wid, error = %e),
    }
}

#[cfg(test)]
#[path = "raise_tests.rs"]
mod tests;
