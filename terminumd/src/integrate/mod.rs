// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The integration queue's engine (`terminum-integration.md`): batches built, checked and published by terminumd,
//! one batch in work on the machine at a time, a step each beat off the async workers.

pub mod agent;
pub mod batch;
pub mod column;
pub mod engine;
pub mod git;
pub mod keys;
pub mod notices;
mod retry_notes;
pub mod runner;
pub mod stages;
pub mod store;

#[cfg(test)]
mod retry_live_tests;

use std::time::Duration;

use crate::app::Shared;
use crate::tmux::server::Link;

/// How often the engine looks.
const BEAT: Duration = Duration::from_secs(10);
/// How many beats between two walks over the trees in work (AC-192).
const WALK_EVERY: u32 = 6;

/// The engine's loop: a step each beat; a failed step is said and the next beat tries again.
pub async fn run(app: Shared) {
    if app.cfg.agent {
        return;
    }
    let Link::Local { tmux, socket, conf } = &app.tmux.local.link else { return };
    let runner = std::sync::Arc::new(runner::Tmux { tmux: tmux.clone(), socket: socket.clone(), conf: conf.clone() });
    // notices from the first look on reach sessions; older ones stand as comments (AC-192)
    let mut last: Option<i64> = None;
    let mut beat = 0u32;
    loop {
        let (store, r) = (app.tasks.clone(), runner.clone());
        match tokio::task::spawn_blocking(move || engine::step(&store, r.as_ref(), &crate::config::tm_home(), crate::tasks::ops::now())).await {
            Ok(Err(e)) => tracing::warn!(event = "integration.step", error = %e),
            Err(e) => tracing::error!(event = "integration.panic", error = %e),
            Ok(Ok(())) => {}
        }
        start_agents(&app).await;
        if beat.is_multiple_of(WALK_EVERY) {
            walk(&app).await;
        }
        beat = beat.wrapping_add(1);
        deliver(&app, &mut last).await;
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

/// The queue's new notices handed to their tasks' sessions (AC-192): a task returned to a run of Dispatcher's gets the
/// gate's fixed words while its agent idles; every other notice waits for its session's next hook. A session that
/// never takes one leaves it standing as its comment. Until where to start is read, nothing goes: no old notice is
/// ever typed again.
async fn deliver(app: &Shared, last: &mut Option<i64>) {
    let Some(after) = *last else {
        *last = crate::tasks::api::store(app, notices::last).await.ok();
        return;
    };
    let Ok((new, read)) = crate::tasks::api::store(app, move |tx| notices::since(tx, after)).await else { return };
    *last = Some(read);
    for n in new {
        if let Some(words) = notices::gate_words(&n) && crate::dispatcher::tell_task_run(app, &n.task, words).await {
            continue;
        }
        if let Some(key) = &n.session && app.task_lines.push(key, &n.id, notices::hook_line(&n)) {
            tracing::info!(event = "integration.notice", task = %n.id, "the oldest line waiting for its session went: at most 20 wait");
        }
    }
}

/// The trees of tasks in work, newest first: the folder its agent works in, as its hooks said, else its session's, on
/// this Mac; compared off the async workers.
async fn walk(app: &Shared) {
    let Ok(tasks) = crate::tasks::api::store(app, |tx| crate::tasks::ops::rows(tx, "SELECT t.id, t.slug, t.project, t.session_key FROM tasks t
        JOIN project_integration i ON i.project = t.project WHERE i.is_on = 1 AND t.status = 'IN PROGRESS' AND t.session_key IS NOT NULL AND t.archived_at IS NULL
        ORDER BY t.updated_at DESC", [], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?)))).await else { return };
    let trees: Vec<notices::Tree> = {
        let layout = app.layout.read();
        tasks.into_iter().filter_map(|(task, id, project, key)| {
            let dir = notices::tree_dir(crate::page::marks::find_item(&layout, &key)?)?;
            Some(notices::Tree { task, id, project, dir })
        }).collect()
    };
    let store = app.tasks.clone();
    match tokio::task::spawn_blocking(move || notices::walk(&store, &crate::config::tm_home(), &trees, crate::tasks::ops::now())).await {
        Ok(Err(e)) => tracing::warn!(event = "integration.walk", error = %e),
        Err(e) => tracing::error!(event = "integration.panic", error = %e),
        Ok(Ok(())) => {}
    }
}
