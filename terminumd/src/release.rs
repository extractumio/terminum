// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The update notice (AC-17) wired to the pages: the daily release check (update.rs), the
//! chip's `update` message, and the page's choices (skip a release, stop checking).

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::json;

use crate::app::Shared;
use crate::update::{self, Updates};

static UPDATES: OnceLock<Arc<Mutex<Updates>>> = OnceLock::new();

fn message(latest: Option<String>) -> String {
    json!({"t": "update", "latest": latest}).to_string()
}

/// The check's loop; every page hears the notice when it changes.
pub async fn run(app: Shared) {
    let updates = UPDATES.get_or_init(|| Arc::new(Mutex::new(Updates::new(&app.cfg.dir, crate::config::build_id())))).clone();
    let pages = app.clone();
    update::follow(updates, Duration::from_secs(update::FIRST_S), Duration::from_secs(update::TICK_S), update::latest_tag, move |n| {
        pages.pages.tell(&message(n));
    })
    .await;
}

/// What a page learns when it connects.
pub fn hello(page: &crate::page::Page) {
    if let Some(n) = UPDATES.get().and_then(|u| u.lock().notice()) {
        page.send(message(Some(n)));
    }
}

/// The chip's menu (the Mac's page only): "skip" {tag}, "off", "on".
pub fn choose(app: &Shared, action: &str, tag: Option<&str>) -> Result<(), String> {
    let Some(u) = UPDATES.get() else { return Err("The update check is not running".into()) };
    u.lock().choose(action, tag)?;
    app.pages.tell(&message(u.lock().notice()));
    Ok(())
}
