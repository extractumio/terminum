// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Runs with a retained case and an absolute deadline (terminum-supervisor.md AC-233 and AC-234).

use std::time::Duration;

use crate::app::Shared;
use crate::http::off;
use crate::supervisor::rounds;
use crate::tasks::ops::{self, now, rows, sql};
use crate::tmux::Server;

use super::{Held, firstprompt, places, pool, procs, runs::{self, Phase, Run}, scale, start, store};

const MEMORY_READ: Duration = Duration::from_secs(5);

pub(crate) async fn watch(app: Shared) {
    let mut clock = tokio::time::interval(Duration::from_secs(2));
    clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        clock.tick().await;
        if let Err(error) = beat(&app).await {
            tracing::warn!(event = "supervisor.rounds", %error);
        }
    }
}

struct Active {
    run: String,
    expired: bool,
    disabled: bool,
    finished: bool,
    failed: bool,
}

async fn beat(app: &Shared) -> Result<(), String> {
    let a = app.clone();
    let (active, waiting, designs_due) = off(move || a.tasks.snapshot(crate::tasks::store::SCHEMA_VERSION, |tx| {
        let at = now();
        let active = rows(tx, "SELECT r.run, r.deadline <= ?1,
            NOT EXISTS (SELECT 1 FROM project_supervisor p WHERE p.project = r.project
                AND p.enabled = 1 AND p.generation = r.generation),
            coalesce(r.found_at <= ?1 - 5, false), r.failure IS NOT NULL OR r.budget_overrun_at IS NOT NULL
            FROM supervisor_rounds r WHERE r.ended_at IS NULL", [at],
            |row| Ok(Active { run: row.get(0)?, expired: row.get(1)?, disabled: row.get(2)?,
                finished: row.get(3)?, failed: row.get(4)? }))?;
        Ok((active, rounds::waiting(tx, at, None)?, crate::supervisor::design_reviews::due(tx, at)?))
    })).await?;
    if designs_due {
        let changed = store(app, |tx| crate::supervisor::design_reviews::advance(tx, now())).await?;
        for id in changed { crate::tasks::api::told(app, &id); }
    }
    // A launch can link its pane and finish between two awaits. Exclude launches before reading tmux,
    // so a round that just finished opening cannot be paired with an older list.
    let active: Vec<_> = {
        let starting = app.dispatcher.starting.lock();
        active.into_iter().filter(|round| !starting.contains(&round.run)).collect()
    };
    let panes = if active.is_empty() { Vec::new() } else {
        app.tmux.local.run("list-panes -a -F '#{pane_id}\t#{@terminum-run}\t#{@terminum-exit}\t#{pane_pid}'").await?
    };
    let panes = std::sync::Arc::new(panes);
    for r in active {
        let a = app.clone();
        let Some(hold) = Held::take(app, |d| &d.ending, &r.run) else { continue };
        let panes = panes.clone();
        tokio::spawn(async move {
            if let Err(error) = check(&a, r, &panes).await {
                tracing::warn!(event = "supervisor.round_end", %error);
            }
            drop(hold);
        });
    }
    for (project, key) in waiting {
        if let Err(why) = admit(app, &project, &key).await {
            store(app, move |tx| rounds::waiting_reason(tx, &key, &why)).await?;
        }
    }
    Ok(())
}

async fn admit(app: &Shared, project: &str, key: &str) -> Result<(), String> {
    let (p, k) = (project.to_owned(), key.to_owned());
    let (desired, terms) = store(app, move |tx| {
        let candidate = rounds::candidate(tx, &p, &k)?;
        if ops::project_place(tx, &p)?.1.is_some() {
            return Err("Supervisor investigations need a local pool agent for pane identity checks.".into());
        }
        let agent = crate::tasks::agents::chosen(tx, &candidate.agent, &p)?;
        if let Some(full) = pool::Slots::read(tx)?.check_in(tx, &agent, &p, Some(pool::Slot::Supervisor), None)? {
            return Err(full.words);
        }
        let terms = candidate.memory_terms();
        Ok((candidate.agent, terms))
    }).await?;
    let (agent, host, folder, server) = places::place(app, project, Some(desired)).await?;
    // AC-234: the project's own memory, read off the store's write turn and bounded in time, so a stuck file
    // never holds the rounds' beat; admission below checks the folder is unchanged.
    let memory = match terms {
        Some(terms) => {
            let dir = crate::tasks::integration::local_dir(&folder);
            let read = off(move || Ok(crate::supervisor::case_memory::snapshot(&dir, &terms)));
            Some(tokio::time::timeout(MEMORY_READ, read).await
                .unwrap_or_else(|_| Ok(serde_json::json!({"unavailable": "Reading the project's memory timed out."})))?)
        }
        None => None,
    };
    let like = Run::of(&agent, host, folder.clone(), project.to_owned(), None);
    let key = key.to_owned();
    let (run, saved) = store(app, move |tx| {
        let at = now();
        let mut candidate = rounds::candidate(tx, &like.project, &key)?;
        candidate.memory = memory;
        let (_, current_host, current_folder) = ops::project_place(tx, &like.project)?;
        if candidate.agent != agent.id || current_host.is_some() || current_folder != like.folder
            || crate::tasks::agents::in_project(tx, &agent.id, &like.project)?.is_none_or(|current| current.line != agent.line || current.at_once != agent.at_once || current.kind != agent.kind)
            || rounds::waiting(tx, at, Some(&key))?.is_empty()
        {
            return Err("The investigation changed during admission.".into());
        }
        if let Some(full) = pool::Slots::read(tx)?.check_in(tx, &agent, &like.project, Some(pool::Slot::Supervisor), None)? {
            return Err(full.words);
        }
        let id = runs::add(tx, &like, &[], runs::Started::of(runs::How::Supervisor), at)?;
        let saved = rounds::save(tx, &id, candidate, at, scale())?;
        Ok((id, saved))
    }).await?;
    let rounds::Saved { prompt, held, nudges } = saved;
    crate::supervisor::holds_api::nudged(app, &nudges);
    if let Some(task) = held { crate::supervisor::holds_api::interim_changed(app, &task); }
    let hold = Held::take(app, |d| &d.starting, &run).ok_or("Its start is already watched.")?;
    let (a, project) = (app.clone(), project.to_owned());
    tokio::spawn(async move {
        let result = launch(&a, &server, &project, &folder, &run, prompt).await;
        if let Err(why) = result
            && let Err(error) = store(&a, move |tx| rounds::failure(tx, &run, &why, false)).await
        {
            tracing::warn!(event = "supervisor.round_start", %error);
        }
        drop(hold);
    });
    Ok(())
}

async fn launch(app: &Shared, server: &std::sync::Arc<Server>, project: &str, folder: &str, run: &str, text: String) -> Result<(), String> {
    let name = places::project_named(app, project);
    let id = run.to_owned();
    let r = store(app, move |tx| runs::by_id(tx, &id)).await?;
    let opening = start::Opening { server, project: name.as_ref().map(|(id, name)| (id.as_str(), name.as_str())),
        folder, run, tasks: &[], command: &r.command, program: &r.kind, agent: &r.agent_id };
    start::open_supervisor(app, opening).await?;
    firstprompt::type_once(app, run, &text, firstprompt::Scope::Supervisor).await
}

async fn check(app: &Shared, active: Active, panes: &[String]) -> Result<(), String> {
    let id = active.run;
    let wanted = id.clone();
    let run = store(app, move |tx| runs::by_id(tx, &wanted)).await?;
    let mut why = if active.disabled {
        Some("The Supervisor was disabled.")
    } else if active.expired && !active.finished {
        Some("The investigation reached its deadline without a checked finding.")
    } else if run.is_starting() && !active.failed {
        Some("Dispatcher restarted before the round took its prompt.")
    } else {
        None
    };
    let mut gone = run.pane_key.is_none();
    if let Some((server, pane)) = run.place(app) {
        let found = panes.iter().find_map(|line| {
            let mut fields = line.split('\t');
            if fields.next()? != pane { return None; }
            Some((fields.next()?, fields.next()?, fields.next()?))
        });
        match found {
            Some((mark, exit, pid)) if mark == id => {
                if !exit.is_empty() && !active.finished {
                    why.get_or_insert("The agent exited without a checked finding.");
                }
                if !active.finished && !active.failed && why.is_none() { return Ok(()); }
                let _lock = server.typing(&pane).lock_owned().await;
                if exit.is_empty() {
                    let targets = procs::under_root(app, &server, pid).await?;
                    let current = server.run(format!("display -p -t {pane} '#{{@terminum-run}}\t#{{@terminum-exit}}\t#{{pane_pid}}'")).await?;
                    if current.first().is_some_and(|line| line == &format!("{id}\t\t{pid}")) {
                        procs::term(app, &server, &targets).await;
                        procs::kill_left(app, &server, pid, &targets, Duration::from_millis(500)).await;
                    }
                }
                start::marked_command(&server, &pane, &id, &format!("kill-pane -t {pane}")).await?;
                gone = true;
            }
            None => {
                let current = server.run("list-panes -a -F '#{pane_id}'").await?;
                if current.contains(&pane) { return Ok(()); }
                gone = true;
            }
            _ => (),
        }
    }
    if !gone {
        why.get_or_insert("The pane no longer carries the investigation's mark.");
    } else if why.is_none() && !active.finished && !active.failed {
        why = Some("The round's marked pane is gone.");
    }
    let held = store(app, move |tx| {
        let found: bool = tx.query_row("SELECT finding IS NOT NULL FROM supervisor_rounds WHERE run = ?1",
            [&id], |r| r.get(0)).map_err(sql)?;
        if let Some(why) = why.filter(|_| !found) {
            rounds::failure(tx, &id, why, false)?;
        }
        if gone {
            tx.execute("DELETE FROM supervisor_panes WHERE round = (SELECT id FROM supervisor_rounds WHERE run = ?1)",
                [&id]).map_err(sql)?;
        }
        runs::change(tx, &id, |r| r.state = Phase::Closed)?;
        tx.execute("UPDATE supervisor_rounds SET ended_at = ?2 WHERE run = ?1", (&id, now())).map_err(sql)?;
        crate::supervisor::interim::round_ended(tx, &id, now())
    }).await?;
    if let Some(task) = held { crate::supervisor::holds_api::interim_changed(app, &task); }
    app.dispatcher.soon.notify_one();
    Ok(())
}
