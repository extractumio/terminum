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

/// The current structured question, if the task is still waiting for that question.
pub(super) fn current(tx: &Transaction, t: &Task) -> Result<Option<String>, String> {
    if t.status != "WAIT FOR HUMAN" || t.archived_at.is_some() {
        return Ok(None);
    }
    let Some(row) = comments::last_question(tx, &t.key)? else { return Ok(None) };
    tx.query_row("SELECT id FROM events
        WHERE rowid = ?2 AND json_type(data, '$.question.options') = 'array'
        AND NOT EXISTS (SELECT 1 FROM events WHERE task = ?1 AND rowid > ?2
            AND (kind = 'project' OR (kind = 'move' AND json_extract(data, '$.to') <> 'WAIT FOR HUMAN')))",
        (&t.key, row), |r| r.get(0)).optional().map_err(sql)
}

fn answer(tx: &Transaction, task: &str, question: &str, option: &str, at: i64) -> Result<(String, bool), String> {
    let t = ops::get(tx, task)?;
    if current(tx, &t)?.as_deref() != Some(question) {
        return Err(STALE.into());
    }
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
    crate::supervisor::config_changes::owner_choice(tx, &t, question, option, at)?;
    let text = format!("Selected option: {option}.");
    let data = json!({"choice": {"question": question, "option": option}});
    let event = comments::comment_with(tx, HUMAN, task, &text, Some(&data), at)?;
    Ok((event, true))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pick {
    option: String,
}

async fn choose(State(app): State<Shared>, Path((id, question)): Path<(String, String)>, Json(body): Json<Pick>) -> Response {
    let task = id.clone();
    let done = api::store(&app, move |tx| answer(tx, &task, &question, &body.option, ops::now())).await;
    if let Ok((event, true)) = &done {
        super::notify::comment_added(&app, &id, event, None);
        api::told(&app, &id);
        app.supervisor.wake();
    }
    api::answer(done.map(|(event, _)| json!({"event": event})))
}

pub(super) fn routes() -> Router<Shared> {
    Router::new().route("/api/tasks/{id}/comments/{question}/choice", post(choose))
}

#[cfg(test)]
#[path = "choices_tests.rs"]
mod tests;
