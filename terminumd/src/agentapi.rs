// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! POST /agent on the private socket (AC-97, AC-98): `terminum mcp`'s tool calls for the pane
//! tmux named in the agent's environment, validated as /hook validates its pane. Always 200
//! {"text", "error"}: the text is the tool's answer to the agent.

use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use crate::app::Shared;
use crate::notes::{New, Notes, Scope};
use crate::plans::{Plans, Update};

use crate::plans::rules::{self, NOT_IN_PANE, STATUSES};

/// The caller: its pane key, notes scope, agent and session.
pub struct Caller {
    pub key: String,
    pub scope: Scope,
    pub agent: String,
    pub session: Option<String>,
}

/// What a tool call changed, so the list is read again and the plans saved.
#[derive(Debug, PartialEq)]
pub enum Changed {
    Nothing,
    Plan,
    Notes,
}

pub async fn agent(State(app): State<Shared>, Json(body): Json<Value>) -> Json<Value> {
    let (socket, pane) = (body["socket"].as_str().unwrap_or(""), body["pane"].as_str().unwrap_or(""));
    let server = app.tmux.all().into_iter().find(|s| s.is_local() && s.socket == socket);
    let Some(server) = server.filter(|s| s.session_of(pane).is_some()) else { return answer(NOT_IN_PANE, true) };
    let key = server.key(pane);
    let hooked = app.hooks.session(&key);
    let session = hooked.as_ref().map(|(_, s)| s.clone()).filter(|s| !s.is_empty())
        .or_else(|| body["session"].as_str().map(|s| s.chars().filter(|c| !c.is_control()).take(100).collect()));
    let caller = Caller { scope: crate::notes::scope_of(&app, &key), agent: hooked.map_or(crate::plans::UNKNOWN, |(a, _)| a).to_string(), session, key };
    let (tool, args) = (body["tool"].as_str().unwrap_or("").to_string(), body["args"].clone());
    let (a, key, scope) = (app.clone(), caller.key.clone(), caller.scope.clone());
    let done = tokio::task::spawn_blocking(move || {
        let working = a.hooks.working(&caller.key);
        run(&a.plans, &a.notes, &caller, &tool, &args, working, crate::auth::store::unix_now())
    }).await;
    let Ok((text, error, changed)) = done else { return answer("terminum could not answer the call.", true) };
    match changed {
        Changed::Plan => {
            crate::plans::save(&app);
            crate::page::plans::tell(&app.pages, std::slice::from_ref(&key));
        }
        Changed::Notes => crate::page::plans::tell(&app.pages, &crate::page::plans::keys_of(&app, &scope)),
        Changed::Nothing => {}
    }
    if changed != Changed::Nothing {
        app.dirty.notify_one();
    }
    answer(&text, error)
}

fn answer(text: &str, error: bool) -> Json<Value> {
    Json(json!({"text": text, "error": error}))
}

/// One tool call: (answer, is an error, what changed).
pub fn run(plans: &Plans, notes: &Notes, c: &Caller, tool: &str, args: &Value, working: bool, now: u64) -> (String, bool, Changed) {
    let r = match tool {
        "update_session_plan" => update_plan(plans, c, args, now).map(|t| (t, Changed::Plan)),
        "get_session_plan" => Ok((plan_text(plans, &c.key, working, now), Changed::Nothing)),
        "add_notes_for_user" => add_notes(notes, c, args, now).map(|t| (t, Changed::Notes)),
        "update_notes_for_user" => update_note(notes, c, args, now).map(|t| (t, Changed::Notes)),
        other => Err(format!("unknown tool {other}: update_session_plan, get_session_plan, add_notes_for_user, update_notes_for_user")),
    };
    match r {
        Ok((t, ch)) => (t, false, ch),
        Err(e) => (e, true, Changed::Nothing),
    }
}

fn text_of(v: &Value, k: &str, max: usize) -> Result<Option<String>, String> {
    match v.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            let s = rules::clean(s);
            if s.chars().count() > max { Err(format!("{k} is over {max} characters")) } else { Ok(Some(s)) }
        }
        Some(_) => Err(format!("{k} must be a string")),
    }
}

fn update_plan(plans: &Plans, c: &Caller, args: &Value, now: u64) -> Result<String, String> {
    let items = args["items"].as_array().ok_or("items is required: send the whole plan, every item with its status")?;
    if items.is_empty() || items.len() > rules::ITEMS_MOST {
        return Err(format!("items has {} entries: send the whole plan, 1–{} items", items.len(), rules::ITEMS_MOST));
    }
    let mut list = Vec::new();
    for (i, it) in items.iter().enumerate() {
        let text = text_of(it, "text", rules::ITEM_TEXT).map_err(|e| format!("items[{i}].{e}"))?.filter(|t| !t.is_empty())
            .ok_or_else(|| format!("items[{i}].text is required"))?;
        let status = it["status"].as_str().filter(|s| STATUSES.contains(s))
            .ok_or_else(|| format!("items[{i}].status {} is not one of {}", it["status"], STATUSES.join(", ")))?;
        list.push((text, status.to_string()));
    }
    let eta_minutes = match &args["eta_minutes"] {
        Value::Null => None,
        v => Some(rules::eta_minutes(v).ok_or_else(|| format!("eta_minutes {v} is not a whole number of minutes from 1 to {}", rules::ETA_MOST))?),
    };
    let u = Update { items: Some(list), doing: text_of(args, "doing", rules::DOING)?, eta_minutes, title: text_of(args, "title", rules::TITLE)? };
    let took = plans.mcp(&c.key, &c.agent, c.session.as_deref(), u, now);
    let d = plans.details(&c.key, false, now).ok_or("The plan was not stored.")?;
    let eta = d.row.eta.map(|at| format!(", ETA ~{}", clock(at))).unwrap_or_default();
    let (done, total) = (d.row.done, d.row.total);
    Ok(if took {
        format!("Plan stored: {done} of {total} completed ({}%){eta}.", d.row.pct)
    } else {
        format!("Doing and ETA stored; items follow your own plan tool ({done} of {total} completed), so the items sent were not taken{eta}.")
    })
}

/// `at` (unix s) as local HH:MM.
fn clock(at: u64) -> String {
    let t = at as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: localtime_r writes into a valid tm
    unsafe { libc::localtime_r(&t, &mut tm) };
    format!("{:02}:{:02}", tm.tm_hour, tm.tm_min)
}

/// The stored plan as lines: "[x] / [>] / [ ] / [!]" items, then doing, ETA and updated.
pub fn plan_text(plans: &Plans, key: &str, working: bool, now: u64) -> String {
    let Some(d) = plans.details(key, working, now).filter(|d| !d.items.is_empty() || d.doing.is_some()) else { return "No plan yet.".into() };
    let mut out: Vec<String> = d.title.iter().cloned().collect();
    for it in &d.items {
        let mark = match it.status.as_str() { "completed" => "[x]", "in_progress" => "[>]", "blocked" => "[!]", _ => "[ ]" };
        out.push(format!("{mark} {}", it.text));
    }
    if let Some(doing) = &d.doing {
        out.push(format!("Doing: {doing}"));
    }
    if let Some(eta) = d.row.eta {
        out.push(format!("ETA: ~{}", clock(eta)));
    }
    out.push(format!("Updated {} min ago; {} of {} completed.", now.saturating_sub(d.row.updated) / 60, d.row.done, d.row.total));
    out.join("\n")
}

fn add_notes(notes: &Notes, c: &Caller, args: &Value, now: u64) -> Result<String, String> {
    let list = args["notes"].as_array().ok_or("notes is required: a list of 1–20 notes")?;
    let mut new = Vec::new();
    for (i, n) in list.iter().enumerate() {
        let s = |k: &str| n[k].as_str().map(str::to_string);
        new.push(New { kind: s("kind").ok_or_else(|| format!("notes[{i}].kind is required"))?,
                       text: s("text").ok_or_else(|| format!("notes[{i}].text is required"))?, file: s("file") });
    }
    let ids = notes.add(&c.scope, c.session.as_deref().unwrap_or(""), new, now)?;
    let ids: Vec<String> = ids.iter().map(u64::to_string).collect();
    Ok(format!("Notes stored for the user, ids: {}.", ids.join(", ")))
}

fn update_note(notes: &Notes, c: &Caller, args: &Value, now: u64) -> Result<String, String> {
    let id = match &args["id"] {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }.ok_or("id is required: a note's id as add_notes_for_user answered it")?;
    let (status, text, kind) = (args["status"].as_str(), args["text"].as_str(), args["kind"].as_str());
    if status.is_none() && text.is_none() && kind.is_none() {
        return Err("nothing to change: give status (open, done), text or kind".into());
    }
    notes.update(&c.scope, id, status, text, kind, now)?;
    Ok(format!("Note {id} updated."))
}

#[cfg(test)]
#[path = "agentapi_tests.rs"]
mod tests;
