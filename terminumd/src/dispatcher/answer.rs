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
use crate::tasks::ops;
use crate::tasks::store::HUMAN;
use crate::tasks::{choices, comments};
use crate::tmux::PaneState;

/// Continue with Recommended's comment, the human's answer as RUN carries it (AC-122).
pub(crate) const RECOMMENDED: &str = "Go ahead with the recommended answer(s).";
const NOT_READ: &str = "Its session's server is not read now: answer in its window, or once that server answers.";
/// Its pane holds another conversation than the one that asked, or none its hooks said: a pane number tmux gave
/// again after a restart may hold another agent (CLAUDE.md §2).
const NOT_THE_ASKER: &str = "Its session no longer holds the conversation that asked: answer in its window, or unlink the session and answer again.";

#[derive(Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub(super) enum Choice {
    Answer,
    Recommended,
    Archive,
}

impl Choice {
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
    let data = json!({"choice": a.choice, "question": a.question, "from": "WAIT FOR HUMAN", "way": way});
    ops::log(tx, &key, HUMAN, "answer", &a.choice.said(&ops::user_name(tx, HUMAN)?), Some(&data), ops::now()).map(drop)
}

/// A RUN pressed for an answer (api.rs): refused on a question no longer waiting; Continue with Recommended's option
/// recorded first, as RUN keeps its comment whatever follows (AC-122); once it went or waits for room, kept in the
/// history.
pub(super) async fn going(app: &Shared, id: &str, answered: Option<Answered>, run: impl Future<Output = Result<Value, String>>) -> Result<Value, String> {
    let Some(a) = answered else { return run.await };
    let (wanted, q, choice) = (id.to_string(), a.question.clone(), a.choice);
    store(app, move |tx| {
        choices::still_waits(tx, &ops::get(tx, &wanted)?, &q)?;
        recommended_choice(tx, &wanted, &q, choice)
    }).await?;
    let done = run.await?;
    if done["full"].is_null() {
        let wanted = id.to_string();
        store(app, move |tx| keep(tx, &wanted, &a, "run")).await?;
    }
    Ok(done)
}

/// Where the answer goes: Archive here; a live session the human opened, typed; else the page goes on with RUN, its
/// comment the answer (`{run, comment}`), and RUN keeps what it went with (`going`).
async fn pressed(app: &Shared, id: &str, body: Press) -> Result<Value, String> {
    let Press { answered: a, text, files } = body;
    let text = text.trim().to_string();
    if gate::has_stray(&text) {
        return Err("The answer has control or invisible characters.".to_string());
    }
    if a.choice == Choice::Answer && text.is_empty() {
        return Err("Write the answer first.".to_string());
    }
    let (wanted, q) = (id.to_string(), a.question.clone());
    if a.choice == Choice::Archive {
        return store(app, move |tx| archive(tx, &wanted, &a, &text, &files)).await;
    }
    // a session linked to it that no run of its holds is one the human opened
    let found = store(app, move |tx| {
        let t = ops::get(tx, &wanted)?;
        choices::still_waits(tx, &t, &q)?;
        let last = runs::last_of(tx, &t.key)?;
        let Some(key) = t.session.clone().filter(|s| last.is_none_or(|r| r.pane_key.as_ref() != Some(s))) else { return Ok(None) };
        Ok(Some(Theirs { asker: choices::asked_in(tx, &q)?, task: t, key }))
    }).await?;
    let words = if a.choice == Choice::Recommended { RECOMMENDED.to_string() } else { text };
    let Some(theirs) = found else { return Ok(json!({"run": true, "comment": words})) };
    match linked_panes(app, id, &[]).await(&theirs.key) {
        PaneState::Open => typed(app, theirs, a, words, files).await,
        PaneState::Unread | PaneState::Unwatched => Err(NOT_READ.to_string()),
        PaneState::Closed => Ok(json!({"run": true, "comment": words})),
    }
}

/// A task linked to a session the human opened, and the conversation that asked it there.
struct Theirs {
    task: ops::Task,
    key: String,
    asker: Option<String>,
}

/// Continue with Recommended on a question with recorded options carries its recommended one as the human's choice
/// (terminum-supervisor.md AC-230).
fn recommended_choice(tx: &rusqlite::Transaction, id: &str, question: &str, choice: Choice) -> Result<(), String> {
    if choice == Choice::Recommended && let Some(option) = choices::recommended(tx, question)? {
        choices::answer(tx, id, question, &option, ops::now())?;
    }
    Ok(())
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
        comments::keep_human_comment(tx, id, text, attached, ops::now())?;
    }
    keep(tx, id, a, "archive")?;
    let now = ops::now();
    ops::move_marked(tx, HUMAN, id, "FAILED", Some(("archived", "archived, not wanted")), now)?;
    ops::archive(tx, HUMAN, id, true, now)?;
    Ok(json!({"archived": true}))
}

/// The answer into a live session the human opened (CLAUDE.md §2), only while it holds the conversation that asked:
/// Answer's text is kept first, as the agent reads it by get_task once the fixed words go in; a press tmux took no
/// paste of keeps nothing.
async fn typed(app: &Shared, theirs: Theirs, a: Answered, text: String, attached: Vec<String>) -> Result<Value, String> {
    let since = std::time::Instant::now();
    let Theirs { task, key, asker } = theirs;
    if asker.is_none() || app.hooks.session(&key).map(|(_, s)| s) != asker {
        return Err(NOT_THE_ASKER.to_string());
    }
    let (server, pane) = app.tmux.resolve(&key).ok_or("Its session is gone.")?;
    let _held = super::Held::take(app, |d| &d.pressing, &format!("task:{}", task.key)).ok_or("A Run is already being prepared.")?;
    // the task's own ID, never the route's spelling of it
    let (id, q, session, choice) = (task.id.clone(), a.question.clone(), key.clone(), a.choice);
    let kept = store(app, move |tx| {
        choices::still_waits(tx, &ops::get(tx, &id)?, &q)?;
        ops::check_free(tx, &session, &task.key)?;
        if choice != Choice::Answer { return Ok(None) }
        let (event, new) = comments::keep_human_comment(tx, &id, &text, &attached, ops::now())?;
        Ok(Some((event, new)))
    }).await?;
    if let Err(gate::Undelivered { miss, pasted }) = gate::deliver_theirs(app, &server, &pane, &task.id, &choice.typed(&task.id), since).await {
        // pasted, it may still be sent: the agent then reads the answer it was told of
        if let Some((event, true)) = kept.clone().filter(|_| !pasted) {
            store(app, move |tx| comments::retract(tx, &event)).await?;
        }
        return Err(match miss {
            Miss::NotEmpty(_) => "The session is not at an empty input box: answer there first".to_string(),
            Miss::Other(why) => format!("Not delivered: {why}"),
        });
    }
    let id = task.id.clone();
    store(app, move |tx| {
        recommended_choice(tx, &id, &a.question, a.choice)?;
        keep(tx, &id, &a, "typed")?;
        ops::move_marked(tx, HUMAN, &id, "IN PROGRESS", Some(("answered", "the human answered on the board")), ops::now())
    }).await?;
    if let Some((event, true)) = kept {
        crate::tasks::notify::comment_added(app, &task.id, &event, None);
    }
    Ok(json!({"typed": key}))
}

#[cfg(test)]
#[path = "answer_tests.rs"]
mod tests;
