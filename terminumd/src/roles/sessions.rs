// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A manager's hand on its project's sessions (`terminum-agents.md` AC-179): it lists and reads them, types into
//! another one as a paste, and closes one. Only the project's own panes, never the manager's own; text reaches tmux
//! as a buffer, never on its command line, and holds no control characters.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::{arg, number, Boss, DATA};
use crate::app::Shared;
use crate::dispatcher::runs;
use crate::tasks::api::store;
use crate::tmux::Server;

/// The lines of a screen get_session gives when none is asked for, and at most.
const LINES: i64 = 60;
const LINES_MOST: i64 = 500;
/// Between a paste and its Enter: an agent's box takes the paste first.
const BEFORE_ENTER: Duration = Duration::from_millis(250);

/// The project's panes, as the session list holds them.
fn members(app: &Shared, boss: &Boss) -> Vec<String> {
    app.projects.lock().by_id(&boss.project).map(|p| p.members.clone()).unwrap_or_default()
}

/// Another session of the manager's project, by the name list_sessions gave it: its key, server and pane. Never a
/// manager's: one the human opened is theirs to type into and close, not an agent's (AC-179).
async fn other(app: &Shared, boss: &Boss, args: &Value, to: &str) -> Result<(String, Arc<Server>, String), String> {
    let key = arg(args, "session")?.to_string();
    if key == boss.key {
        return Err(format!("That is your own session: {to} another one."));
    }
    let k = key.clone();
    if store(app, move |tx| Ok(super::manages(tx, &k)?.is_some())).await? {
        return Err(format!("{key} is a manager's session, the human's: you cannot {to} it. Read it with get_session; run_task starts a worker you can."));
    }
    let (key, (server, pane)) = of_project(app, boss, args)?;
    Ok((key, server, pane))
}

fn of_project(app: &Shared, boss: &Boss, args: &Value) -> Result<(String, (Arc<Server>, String)), String> {
    let key = arg(args, "session")?.to_string();
    if !members(app, boss).contains(&key) {
        return Err(format!("Your project has no session {key:?}: list_sessions shows them."));
    }
    let place = app.tmux.resolve(&key).ok_or("That session is gone.")?;
    Ok((key, place))
}

pub async fn list(app: &Shared, boss: &Boss) -> Result<String, String> {
    let keys = members(app, boss);
    let (held, managers) = store(app, move |tx| {
        let held = runs::live(tx)?.into_iter().filter(|r| r.holds_slot()).filter_map(|r| r.pane_key.clone().map(|k| (k, r))).collect::<std::collections::HashMap<_, _>>();
        let managers = keys.into_iter().filter_map(|k| super::manages(tx, &k).map(|m| m.map(|_| k)).transpose()).collect::<Result<std::collections::HashSet<_>, _>>()?;
        Ok((held, managers))
    }).await?;
    let now = crate::auth::store::unix_now();
    let layout = app.layout.read();
    let sessions: Vec<Value> = members(app, boss).iter().filter(|k| app.tmux.resolve(k).is_some()).map(|key| {
        let working = app.hooks.working(key);
        let plan = app.plans.details(key, working, now);
        let run = held.get(key);
        json!({
            "session": key,
            "title": crate::page::marks::find_item(&layout, key).and_then(|row| row["title"].as_str()).unwrap_or(""),
            "you": *key == boss.key,
            "agent": app.hooks.session(key).map(|(agent, _)| agent),
            "working": working,
            "role": super::name(managers.contains(key)),
            "run": run.map(|r| json!({"tasks": r.ids, "agent": r.agent_name, "state": r.state.label(), "asked_a_human": r.asked})),
            "plan": plan.map(|d| json!({"doing": d.row.doing, "done": d.row.done, "total": d.row.total})),
        })
    }).collect();
    Ok(format!("{DATA}\n{}", json!(sessions)))
}

pub async fn get(app: &Shared, boss: &Boss, args: &Value) -> Result<String, String> {
    let (key, (server, pane)) = of_project(app, boss, args)?;
    let lines = number(args, "lines", 1..=LINES_MOST)?.unwrap_or(LINES) as usize;
    let screen = server.run(format!("capture-pane -p -J -t {pane} -S -{lines}")).await.map_err(|e| format!("tmux does not answer: {e}"))?;
    let shown = crate::lastlines::tail(&[screen.iter().map(|l| crate::termtext::clean(l, false)).collect()], lines);
    let plan = crate::agentapi::plan_text(&app.plans, &key, app.hooks.working(&key), crate::auth::store::unix_now());
    Ok(format!("{DATA}\nIts plan:\n{plan}\n\nIts screen's last lines:\n{shown}"))
}

pub async fn send(app: &Shared, boss: &Boss, args: &Value) -> Result<String, String> {
    let (key, server, pane) = other(app, boss, args, "type into").await?;
    let text = args["text"].as_str().unwrap_or("").trim_end_matches(['\n', '\r']);
    if text.trim().is_empty() {
        return Err("text is required".into());
    }
    // what a pane shows of it is what runs: as a RUN's message (dispatcher/api.rs)
    if text.split(['\n', '\t']).any(crate::termtext::has_control) {
        return Err("Not typed: the text has control or invisible characters.".into());
    }
    let enter = args["enter"] != "no";
    let done = typed(app, &server, &pane, text, enter).await;
    let project = app.projects.lock().by_id(&boss.project).map(|p| (p.id.clone(), p.name.clone()));
    let words = if enter && done.is_ok() { format!("{text}<Enter>") } else { text.to_string() };
    crate::inputs::wrote(app, crate::inputs::Who::manager("manager.send", &boss.name), "prompt", &server, &pane, &words, done.as_ref().err().cloned(), project.as_ref().map(|(i, n)| (i.as_str(), n.as_str())));
    done?;
    Ok(format!("Typed into {key}{}. get_session shows what it does with it.", if enter { ", with Enter" } else { ", no Enter" }))
}

/// `text` pasted into `pane`, then Enter: behind the pane's own queue of typing, never into a tmux mode.
async fn typed(app: &Shared, server: &Arc<Server>, pane: &str, text: &str, enter: bool) -> Result<(), String> {
    let _held = server.typing(pane).lock_owned().await;
    let mode = server.run(format!("display -p -t {pane} '#{{pane_in_mode}}'")).await.map_err(|e| format!("tmux does not answer: {e}"))?;
    if mode.first().map(String::as_str) != Some("0") {
        return Err("Not typed: the pane is in a tmux mode.".into());
    }
    let client = server.client(pane).ok_or("tmux does not answer")?;
    crate::page::typing::paste_buffer(app, &client, server, pane, text).await?;
    if enter {
        tokio::time::sleep(BEFORE_ENTER).await;
        client.run(format!("send-keys -t {pane} Enter")).await.map_err(|e| format!("tmux did not take Enter: {e}"))?;
    }
    Ok(())
}

pub async fn close(app: &Shared, boss: &Boss, args: &Value) -> Result<String, String> {
    let (key, ..) = other(app, boss, args, "close").await?;
    crate::page::sessions::kill(app, &key).await?;
    tracing::info!(event = "manager.closed", key = %key, by = %boss.name);
    Ok(format!("{key} is closed: its program ended. A task its agent ran is no longer held by a session: list_runs and get_task say where it stands."))
}
