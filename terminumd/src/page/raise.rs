// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A session whose coding agent took a message goes to the top of where the list shows it
//! (AC-92): of its project, kept in the store; else of its tmux group, whose order is its
//! windows' (AC-07), moved as a drag moves them. A session the user attached by name (AC-89),
//! on this Mac or a host, is left as it is.

use serde_json::Value;

use crate::app::Shared;

/// A pane's place in the list.
#[derive(Debug, PartialEq)]
pub(crate) struct Place {
    /// The row it stands under.
    pub row: String,
    /// Its tmux group's id.
    pub wid: String,
    /// That group's rows in order, each (its id, its window).
    pub rows: Vec<(String, String)>,
}

/// Pane `key`'s place in the list. A row is a window's first pane, not a quick shell; a
/// split's other panes and a quick shell go under one, as the page draws them (web/static/nav.js
/// `membership`: the same rule). None when the layout has no such pane.
pub(crate) fn place_of(layout: &Value, key: &str) -> Option<Place> {
    let is_row = |it: &Value| it["pane"].as_u64().unwrap_or(0) <= 1 && it["quickOf"].is_null();
    for g in layout.as_array()? {
        let items = g["items"].as_array().map_or(&[][..], Vec::as_slice);
        let Some(it) = items.iter().find(|it| it["id"] == key) else { continue };
        let row = match it["quickOf"].as_str() {
            Some(parent) => parent.to_string(),
            None => items.iter().find(|x| x["window"] == it["window"] && is_row(x))?["id"].as_str()?.to_string(),
        };
        let rows = items.iter().filter(|it| is_row(it))
            .filter_map(|it| Some((it["id"].as_str()?.to_string(), it["window"].as_str()?.to_string())))
            .collect();
        return Some(Place { row, wid: g["wid"].as_str()?.to_string(), rows });
    }
    None
}

/// Row `row`'s window and the windows of the group's `rows` its list shows (those in no project,
/// `in_project` says), in their order. None when `row` is not among them, or stands first already.
pub(crate) fn to_raise(rows: &[(String, String)], row: &str, in_project: impl Fn(&str) -> bool) -> Option<(String, Vec<String>)> {
    let free: Vec<&(String, String)> = rows.iter().filter(|(id, _)| !in_project(id)).collect();
    let at = free.iter().position(|(id, _)| id == row)?;
    (at > 0).then(|| (free[at].1.clone(), free.iter().map(|(_, w)| w.clone()).collect()))
}

/// Pane `key`'s agent took a message: its row goes to the top of its project, or of its tmux
/// group, every page told (AC-92).
pub async fn raise(app: &Shared, key: &str) {
    let found = place_of(&app.layout.read(), key);
    // a pane the layout does not list yet is its own row
    let row = found.as_ref().map_or(key, |p| p.row.as_str());
    if crate::page::projects::raise(app, row).await {
        return;
    }
    let Some(Place { row, wid, rows }) = &found else { return };
    let moving = { let ps = app.projects.lock(); to_raise(rows, row, |k| ps.of(k).is_some()) };
    let Some((top, free)) = moving else { return };
    let Some((server, session)) = super::sessions::group(app, wid) else { return };
    if server.users(&session) {
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
