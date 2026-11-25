// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Answering a task its agent asked on (`terminum-dispatcher.md` AC-285): Answer, Continue with Recommended,
//! Archive, whoever started its session and whether it lives. Each press that goes is kept in the task's history.

use std::future::Future;

use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::gate::{self, Miss};
use super::places::linked_panes;
use super::{runs, store, waits};
use crate::app::Shared;
use crate::tasks::api::{answer, told};
use crate::tasks::ops::{self, sql};
use crate::tasks::store::HUMAN;
use crate::tasks::{choices, files};
use crate::tmux::PaneState;

/// Continue with Recommended's comment, the human's answer as RUN carries it (AC-122).
pub(crate) const RECOMMENDED: &str = "Go ahead with the recommended answer(s).";
const NOT_READ: &str = "Its session's server is not read now: answer in its window, or once that server answers.";

#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub(super) enum Choice {
    Answer,
    Recommended,
    Archive,
}

impl Choice {
    fn name(self) -> &'static str {
        match self {
            Choice::Answer => "answer",
            Choice::Recommended => "recommended",
            Choice::Archive => "archive",
        }
    }

    fn said(self, who: &str) -> String {
        match self {
            Choice::Answer => format!("{who} answered"),
            Choice::Recommended => format!("{who} chose Continue with Recommended"),
            Choice::Archive => format!("{who} archived it: not wanted"),
        }
    }

    /// The fixed words typed into a live session the human opened: the task's ID, never the human's text (CLAUDE.md §2).
    fn typed(self, id: &str) -> String {
        match self {
            Choice::Recommended => format!("{id}: the human chose your recommended answers. Go on with the task with them."),
            _ => format!("{id}: the human answered your questions on the board. Read the answer by execute_tool(\"get_task\", {{id}}) and go on with the task, taking it into account."),
        }
    }
}

/// The question a press answers and how, as the page sends it; RUN carries it when the answer goes as a RUN (api.rs).
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub(super) struct Answered {
    choice: Choice,
    question: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Press {
    #[serde(flatten)]
    answered: Answered,
    #[serde(default)]
    text: String,
    #[serde(default)]
    files: Vec<String>,
}

pub(super) fn routes() -> Router<Shared> {
    Router::new().route("/api/tasks/{id}/answer", post(press))
}

async fn press(State(app): State<Shared>, Path(id): Path<String>, Json(body): Json<Press>) -> Response {
    let done = pressed(&app, &id, body).await;
    if done.is_ok() {
        told(&app, &id);
    }
    answer(done)
}

/// The history's line of a press that went: its choice, the question, the column it left, how it went.
fn keep(tx: &rusqlite::Transaction, id: &str, a: &Answered, way: &str) -> Result<(), String> {
    let key = ops::key_of(tx, id)?;
    let data = json!({"choice": a.choice.name(), "question": a.question, "from": "WAIT FOR HUMAN", "way": way});
    ops::log(tx, &key, HUMAN, "answer", &a.choice.said(&ops::user_name(tx, HUMAN)?), Some(&data), ops::now()).map(drop)
}

/// The human's comment with its files: its event.
fn comment(tx: &rusqlite::Transaction, id: &str, text: &str, attached: &[String]) -> Result<String, String> {
    let event = ops::comment(tx, HUMAN, id, text, ops::now())?;
    files::attach(tx, attached, id, Some(&event))?;
    Ok(event)
}

/// A RUN pressed for an answer (api.rs): refused on a question no longer waiting; once it went or waits for room,
/// kept in the history.
pub(super) async fn going(app: &Shared, id: &str, answered: Option<Answered>, run: impl Future<Output = Result<Value, String>>) -> Result<Value, String> {
    let Some(a) = answered else { return run.await };
    let (wanted, q) = (id.to_string(), a.question.clone());
    store(app, move |tx| choices::still_waits(tx, &ops::get(tx, &wanted)?, &q)).await?;
    let done = run.await?;
    if done["full"].is_null() {
        let wanted = id.to_string();
        store(app, move |tx| keep(tx, &wanted, &a, "run")).await?;
    }
    Ok(done)
}

/// Where the answer goes: Archive here; a live session the human opened, typed; else the page goes on with RUN, its
/// comment the answer (`{run, comment}`).
async fn pressed(app: &Shared, id: &str, body: Press) -> Result<Value, String> {
    let Press { answered: a, text, files } = body;
    let text = text.trim().to_string();
    if gate::has_stray(&text) {
        return Err("The answer has control or invisible characters.".to_string());
    }
    if a.choice == Choice::Answer && text.is_empty() {
        return Err("Write the answer first.".to_string());
    }
    let (wanted, q, choice) = (id.to_string(), a.question.clone(), a.choice);
    if choice == Choice::Archive {
        return store(app, move |tx| archive(tx, &wanted, &a, &text, &files)).await;
    }
    let theirs = store(app, move |tx| {
        let t = ops::get(tx, &wanted)?;
        choices::still_waits(tx, &t, &q)?;
        // a question with recorded options carries its recommended one as the choice (terminum-supervisor.md AC-230)
        if choice == Choice::Recommended && let Some(option) = choices::recommended(tx, &q)? {
            choices::answer(tx, &wanted, &q, &option, ops::now())?;
        }
        let last = runs::last_of(tx, &t.key)?;
        Ok(t.session.clone().filter(|s| last.is_none_or(|r| r.pane_key.as_ref() != Some(s))))
    }).await?;
    let words = if choice == Choice::Recommended { RECOMMENDED.to_string() } else { text };
    let Some(key) = theirs else { return Ok(json!({"run": true, "comment": words})) };
    match linked_panes(app, id, &[]).await(&key) {
        PaneState::Closed => Ok(json!({"run": true, "comment": words})),
        PaneState::Open => typed(app, id, &key, a, words, files).await,
        PaneState::Unread | PaneState::Unwatched => Err(NOT_READ.to_string()),
    }
}

/// Archive: not wanted. What is written stays as the human's comment, the task fails and leaves the board, a RUN
/// waiting for room is called off; refused while a run holds it.
fn archive(tx: &rusqlite::Transaction, id: &str, a: &Answered, text: &str, attached: &[String]) -> Result<Value, String> {
    let t = ops::get(tx, id)?;
    choices::still_waits(tx, &t, &a.question)?;
    if runs::holding(tx, &t.key)?.is_some() {
        return Err("Its run holds it: give up on the run first.".to_string());
    }
    if waits::of_task(tx, &t.key)?.is_some() {
        waits::take_out(tx, id)?;
    }
    if !text.is_empty() {
        comment(tx, id, text, attached)?;
    }
    keep(tx, id, a, "archive")?;
    let now = ops::now();
    ops::move_marked(tx, HUMAN, id, "FAILED", Some(("archived", "archived, not wanted")), now)?;
    ops::archive(tx, HUMAN, id, true, now)?;
    Ok(json!({"archived": true}))
}

/// The answer into a live session the human opened (CLAUDE.md §2): Answer's text is kept first, as the agent reads it
/// by get_task once the fixed words go in; a press that does not go keeps nothing.
async fn typed(app: &Shared, id: &str, key: &str, a: Answered, text: String, attached: Vec<String>) -> Result<Value, String> {
    let since = std::time::Instant::now();
    let (server, pane) = app.tmux.resolve(key).ok_or("Its session is gone.")?;
    let (wanted, q, session) = (id.to_string(), a.question.clone(), key.to_string());
    let task_key = store(app, move |tx| ops::key_of(tx, &wanted)).await?;
    let _held = super::Held::take(app, |d| &d.pressing, &format!("task:{task_key}")).ok_or("A Run is already being prepared.")?;
    let (wanted, choice) = (id.to_string(), a.choice);
    let kept = store(app, move |tx| {
        let t = ops::get(tx, &wanted)?;
        choices::still_waits(tx, &t, &q)?;
        ops::check_free(tx, &session, &t.key)?;
        if choice == Choice::Answer { comment(tx, &wanted, &text, &attached).map(Some) } else { Ok(None) }
    }).await?;
    if let Err(miss) = gate::deliver_theirs(app, &server, &pane, id, &choice.typed(id), since).await {
        if let Some(event) = kept.clone() {
            store(app, move |tx| {
                tx.execute("UPDATE files SET task = NULL, event = NULL WHERE event = ?1", [&event]).map_err(sql)?;
                tx.execute("DELETE FROM events WHERE id = ?1", [&event]).map(drop).map_err(sql)
            }).await?;
        }
        return Err(match miss {
            Miss::NotEmpty(_) => "The session is not at an empty input box: answer there first".to_string(),
            Miss::Other(why) => format!("Not delivered: {why}"),
        });
    }
    let wanted = id.to_string();
    store(app, move |tx| {
        keep(tx, &wanted, &a, "typed")?;
        ops::move_marked(tx, HUMAN, &wanted, "IN PROGRESS", Some(("answered", "the human answered on the board")), ops::now())
    }).await?;
    if let Some(event) = kept {
        crate::tasks::notify::comment_added(app, id, &event, None);
    }
    Ok(json!({"typed": key}))
}

#[cfg(test)]
#[path = "answer_tests.rs"]
mod tests;
