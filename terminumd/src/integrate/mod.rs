// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The integration queue's engine (`terminum-integration.md`): batches built, checked and published by terminumd,
//! one batch in work on the machine at a time, a step each beat off the async workers.

pub mod batch;
pub mod engine;
pub mod git;
pub mod keys;
pub mod runner;
pub mod store;

use std::time::Duration;

use crate::app::Shared;
use crate::tmux::server::Link;

/// How often the engine looks.
const BEAT: Duration = Duration::from_secs(10);

/// The engine's loop: a step each beat; a failed step is said and the next beat tries again.
pub async fn run(app: Shared) {
    if app.cfg.agent {
        return;
    }
    let Link::Local { tmux, socket, conf } = &app.tmux.local.link else { return };
    let runner = std::sync::Arc::new(runner::Tmux { tmux: tmux.clone(), socket: socket.clone(), conf: conf.clone() });
    loop {
        let (store, r) = (app.tasks.clone(), runner.clone());
        match tokio::task::spawn_blocking(move || engine::step(&store, r.as_ref(), &crate::config::tm_home(), crate::tasks::ops::now())).await {
            Ok(Err(e)) => tracing::warn!(event = "integration.step", error = %e),
            Err(e) => tracing::error!(event = "integration.panic", error = %e),
            Ok(Ok(())) => {}
        }
        crate::page::projects::tell(&app);
        tokio::time::sleep(BEAT).await;
    }
}
