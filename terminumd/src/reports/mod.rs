// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Reports and the Inbox (`docs/specs/terminum-reports.md`): what agents, routines and the Supervisor give a person to
//! read, kept apart from tasks, read like mail on the board.

pub mod api;
pub mod inbox;
pub mod migrate;
pub mod record;
pub mod sources;
pub mod tools;

use crate::app::Shared;

/// A report changed: the tasks' marks are read again, whose Inbox revision and critical lines every page hears
/// (AC-267, AC-271).
pub fn changed(app: &Shared) {
    crate::tasks::marks::read_later(app);
}

/// Every hour, deleted reports past their 30 days go with their files (AC-269).
pub async fn sweeping(app: Shared) {
    loop {
        let a = app.clone();
        let purged = tokio::task::spawn_blocking(move || {
            let files = a.tasks.with(|tx| record::purge(tx, crate::tasks::ops::now()))?;
            crate::tasks::files::drop_bytes(&a.cfg.dir, &files);
            Ok::<_, String>(files.len())
        }).await;
        if let Ok(Err(e)) = purged {
            tracing::warn!(event = "reports.purge", error = %e);
        }
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }
}

#[cfg(test)]
#[path = "reports_tests.rs"]
mod tests;
