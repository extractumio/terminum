// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The integration queue's engine (`terminum-integration.md`): batches built, checked and published by terminumd,
//! one batch in work on the machine at a time, a step each beat off the async workers.

pub mod agent;
pub mod batch;
pub mod engine;
pub mod git;
pub mod keys;
pub mod runner;
pub mod stages;
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
        start_agents(&app).await;
        crate::page::projects::tell(&app);
        tokio::time::sleep(BEAT).await;
    }
}

/// Each red batch's integration agent started on its task, with its budget (AC-194); a press refused leaves the
/// batch to a human, saying why.
async fn start_agents(app: &Shared) {
    use crate::tasks::api::store;
    let Ok(wanted) = store(app, |tx| agent::take_wanted(tx, crate::tasks::ops::now())).await else { return };
    for (batch, id, who) in wanted {
        if let Err(why) = crate::dispatcher::press_integration(app, &id, &who).await {
            let _ = store(app, move |tx| {
                let b = store::by_id(tx, batch)?;
                store::wait_for_human(tx, &b, &format!("Batch {}'s integration agent did not start: {why}", b.number), crate::tasks::ops::now())
            }).await;
        }
    }
}
