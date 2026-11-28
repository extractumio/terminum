// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Recorded choices on an agent's question and a human's answer (AC-230, AC-157).

use axum::{extract::{Path, State}, response::Response, routing::post, Json, Router};
use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{api, comments, ops::{self, sql, Task}, store::HUMAN};
use crate::app::Shared;

pub(crate) const OPTIONS_MOST: usize = 12;
pub(crate) const ID_MOST: usize = 32;
pub(crate) const LABEL_MOST: usize = 240;
pub(crate) const KINDS: [&str; 2] = ["operational", "strategic"];
const STALE: &str = "That question is no longer waiting for an answer.";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Choice {
    id: String,
    label: String,
}

#[derive(Serialize)]
pub(super) struct Question {
    #[serde(skip_serializing_if = "Option::is_none")]
    kind: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    options: Vec<Choice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    recommended: Option<String>,
}

fn option_id(id: &str) -> bool {
    id.len() <= ID_MOST && id.as_bytes().first().is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'))
        && crate::sent::redact(id) == id
}

pub(super) fn question(args: &Value) -> Result<Option<Question>, String> {
    let kind = match &args["kind"] {
        Value::Null => None,
        Value::String(kind) if KINDS.contains(&kind.as_str()) => Some(kind.clone()),
        _ => return Err("kind must be operational or strategic.".into()),
    };
    let options: Vec<Choice> = match &args["options"] {
        Value::Null => Vec::new(),
        Value::Array(options) if options.len() <= OPTIONS_MOST => Vec::deserialize(&args["options"])
            .map_err(|_| "Each option must have a string id and label.")?,
        _ => return Err(format!("options must be an array of up to {OPTIONS_MOST} options.")),
    };
    let recommended = match &args["recommended"] {
        Value::Null => None,
        Value::String(id) if options.iter().any(|o| &o.id == id) => Some(id.clone()),
        _ => return Err("recommended must be an ID of a listed option.".into()),
    };
    if !options.is_empty() && (kind.is_none() || recommended.is_none()) {
        return Err("Questions with options require kind and recommended.".into());
    }
    let mut checked: Vec<Choice> = Vec::with_capacity(options.len());
    for mut option in options {
        if !option_id(&option.id) {
            return Err(format!("Option IDs must be 1–{ID_MOST} characters, starting with a lowercase letter or digit and using only lowercase letters, digits, hyphens or underscores."));
        }
        if option.label.trim().chars().take(LABEL_MOST + 1).count() > LABEL_MOST {
            return Err(format!("Option labels must be 1–{LABEL_MOST} characters."));
        }
        if option.label.chars().any(crate::termtext::stray) {
            return Err("Option labels must be plain, single-line text.".into());
        }
        option.label = crate::plans::rules::clean_with(&option.label, false);
        if option.label.is_empty() || option.label.chars().count() > LABEL_MOST {
            return Err(format!("Option labels must be 1–{LABEL_MOST} characters."));
        }
        if checked.iter().any(|c| c.id == option.id || c.label == option.label) {
            return Err("Option IDs and labels must be distinct.".into());
        }
        checked.push(option);
    }
    Ok((kind.is_some() || !checked.is_empty()).then_some(Question { kind, options: checked, recommended }))
}

/// The question the task still waits on (terminum-dispatcher.md AC-157, AC-285): its agent's last questions, the
/// task in WAIT FOR HUMAN since, neither moved nor answered by a key; its event, and whether it recorded options.
pub(super) fn asked(tx: &Transaction, t: &Task) -> Result<Option<(String, bool)>, String> {
    if t.status != "WAIT FOR HUMAN" || t.archived_at.is_some() {
        return Ok(None);
    }
    let Some(row) = comments::last_question(tx, &t.key)? else { return Ok(None) };
    tx.query_row("SELECT id, coalesce(json_type(data, '$.question.options') = 'array', 0) FROM events
        WHERE rowid = ?2 AND NOT EXISTS (SELECT 1 FROM events WHERE task = ?1 AND rowid > ?2
            AND (kind IN ('project', 'answer') OR (kind = 'move' AND json_extract(data, '$.to') <> 'WAIT FOR HUMAN')))",
        (&t.key, row), |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(sql)
}

/// The current structured question, if the task is still waiting for that question.
fn current(tx: &Transaction, t: &Task) -> Result<Option<String>, String> {
    Ok(asked(tx, t)?.filter(|(_, options)| *options).map(|(id, _)| id))
}

/// The question the task still waits on, with recorded options or not (terminum-dispatcher.md AC-285).
pub(crate) fn waiting(tx: &Transaction, t: &Task) -> Result<Option<String>, String> {
    Ok(asked(tx, t)?.map(|(id, _)| id))
}

/// `question` still waits, else the refusal every answer to it says (AC-230, AC-285).
pub(crate) fn still_waits(tx: &Transaction, t: &Task, question: &str) -> Result<(), String> {
    if waiting(tx, t)?.as_deref() == Some(question) { Ok(()) } else { Err(STALE.into()) }
}

/// The conversation that asked `question`, when its hooks said it.
pub(crate) fn asked_in(tx: &Transaction, question: &str) -> Result<Option<String>, String> {
    tx.query_row("SELECT json_extract(data, '$.asked_in') FROM events WHERE id = ?1", [question], |r| r.get(0))
        .optional().map(Option::flatten).map_err(sql)
}

/// Its recommended option, when `question` recorded options (AC-230).
pub(crate) fn recommended(tx: &Transaction, question: &str) -> Result<Option<String>, String> {
    tx.query_row("SELECT json_extract(data, '$.question.recommended') FROM events WHERE id = ?1", [question], |r| r.get(0))
        .optional().map(Option::flatten).map_err(sql)
}

/// The human's option `option` on `question` of `task`, kept as their comment: its event, and whether it is new.
pub(crate) fn answer(tx: &Transaction, task: &str, question: &str, option: &str, at: i64) -> Result<(String, bool), String> {
    let t = ops::get(tx, task)?;
    if current(tx, &t)?.as_deref() != Some(question) {
        return Err(STALE.into());
    }
    record(tx, &t, question, option, at)
}

/// `option` on `question` of `t`, kept as the human's comment, the question judged waiting by the caller: by `answer`,
/// or by a press that went with it (terminum-dispatcher.md AC-285). Its event, and whether it is new.
pub(crate) fn record(tx: &Transaction, t: &Task, question: &str, option: &str, at: i64) -> Result<(String, bool), String> {
    let listed: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM events,
        json_each(data, '$.question.options') o WHERE events.id = ?1 AND json_extract(o.value, '$.id') = ?2)",
        (question, option), |r| r.get(0)).map_err(sql)?;
    if !listed {
        return Err("That option is not on the question.".into());
    }
    let previous: Option<(String, String)> = tx.query_row("SELECT id, json_extract(data, '$.choice.option')
        FROM events WHERE task = ?1 AND kind = 'comment' AND json_extract(data, '$.choice.question') = ?2",
        (&t.key, question), |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(sql)?;
    if let Some((event, previous)) = previous {
        return if previous == option { Ok((event, false)) } else { Err("An option was already chosen for this question.".into()) };
    }
    // The ordinary RUN and comment notice paths can carry these words. Only the validated ID comes from the agent.
    crate::supervisor::config_changes::owner_choice(tx, t, question, option, at)?;
    let text = format!("Selected option: {option}.");
    let data = json!({"choice": {"question": question, "option": option}});
    let event = comments::comment_with(tx, HUMAN, &t.id, &text, Some(&data), at)?;
    Ok((event, true))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pick {
    option: String,
}

/// A choice newly recorded on task `id`: its session hears of it, every page, and the Supervisor reads it (AC-230).
pub(crate) fn announce(app: &Shared, id: &str, event: &str) {
    super::notify::comment_added(app, id, event, None);
    api::told(app, id);
    app.supervisor.wake();
}

async fn choose(State(app): State<Shared>, Path((id, question)): Path<(String, String)>, Json(body): Json<Pick>) -> Response {
    let task = id.clone();
    let done = api::store(&app, move |tx| answer(tx, &task, &question, &body.option, ops::now())).await;
    if let Ok((event, true)) = &done {
        announce(&app, &id, event);
    }
    api::answer(done.map(|(event, _)| json!({"event": event})))
}

pub(super) fn routes() -> Router<Shared> {
    Router::new().route("/api/tasks/{id}/comments/{question}/choice", post(choose))
}

#[cfg(test)]
#[path = "choices_tests.rs"]
mod tests;
