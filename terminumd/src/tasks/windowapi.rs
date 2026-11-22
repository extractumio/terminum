// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A task's window as the page reads it (`terminum-kanban.md` AC-107, AC-199).

use axum::extract::{Path, State};
use axum::response::Response;
use serde_json::json;

use super::api::{answer, card, store, text_rev};
use super::{files, ops, query};
use crate::app::Shared;
use crate::dispatcher::runs;

/// A task's window: its fields, text, comments and history, files.
pub(super) async fn window(State(app): State<Shared>, Path(id): Path<String>) -> Response {
    let done = store(&app, move |tx| {
        let t = ops::get(tx, &id)?;
        let held = runs::holding(tx, &t.key)?.map(|r| r.names());
        let agent = runs::card_agents(tx, &[&t.key])?.remove(&t.key);
        let waits = crate::dispatcher::waits::on_cards(tx, &[&t.key])?.remove(&t.key);
        let kids = query::subtasks(tx, &t.key)?;
        let from = super::schedules::of_task(tx, &t.key)?.map(|s| json!({"key": s.key, "title": s.title}));
        let runs_on = super::auto::agent_of(tx, &t.key)?;
        let lists = super::attrs::lists_of(tx, &t.key)?;
        let report = super::report::waits_on(tx, &t.key)?;
        Ok((ops::text_of(tx, &id)?, ops::history(tx, &id)?, files::of_task(tx, &id)?, (held, waits, from, runs_on, lists), agent, kids, (t, report)))
    }).await;
    answer(done.map(|(body, history, attached, (held, waits, from, runs_on, lists), agent, kids, (t, report))| {
        let mut v = card(&app, &t);
        v["report"] = json!(report);                  // it waits with its report: Done reads it (terminum-dispatcher.md AC-264)
        v["lists"] = lists;
        v["children"] = json!(kids);
        v["text_rev"] = json!(text_rev(&body));
        v["text"] = json!(body);
        let mut history = history;
        super::notify::marks(&mut history, &app.task_lines.waiting_comments(&t.id));   // each comment says whether its session was told (AC-209)
        v["history"] = history;
        v["files"] = json!(attached);
        v["run_holds"] = json!(held);
        if let Some(wait) = waits { wait.mark(&mut v); }
        v["schedule"] = json!(from);
        v["runs_on"] = json!(runs_on);                // the agent a human chose to run it (terminum-dispatcher.md AC-170)
        if let Some(agent) = agent {
            v["agent"] = agent;
        }
        v
    }))
}
