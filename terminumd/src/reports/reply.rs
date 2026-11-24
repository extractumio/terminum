// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A human's reply to a report (terminum-reports.md AC-270): it goes on in the conversation of the task the report was
//! made in, as a human's RUN of that task does (terminum-dispatcher.md AC-122), through RUN's press and its §9 gate,
//! its words fixed but for the human's text; nothing of the report is typed. Kept below the report only once the press
//! is accepted. A report of no task Dispatcher ran says where to answer instead.

use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::post;
use axum::{Json, Router};
use rusqlite::{OptionalExtension, Transaction};
use serde::Deserialize;
use serde_json::json;

use super::record;
use crate::app::Shared;
use crate::tasks::api::{answer, store};
use crate::tasks::ops::{self, sql};
use crate::tasks::store::HUMAN;

pub fn routes() -> Router<Shared> {
    Router::new().route("/api/reports/{id}/reply", post(reply))
}

/// The task a reply to report `key` goes on in, or why none: in words the pane shows where the field would stand.
pub fn target(tx: &Transaction, key: &str) -> Result<Result<String, String>, String> {
    let (id, task, session, incident, deleted): (String, Option<String>, Option<String>, Option<i64>, bool) = tx.query_row(
        "SELECT slug, task, session_key, incident, deleted_at IS NOT NULL FROM reports WHERE id = ?1", [key],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).map_err(sql)?;
    if deleted {
        return Ok(Err(format!("{id} is deleted: restore it to reply.")));
    }
    // an incident has its own way to a session, Talk: one subject, one session (terminum-supervisor.md AC-244)
    if let Some(n) = incident {
        return Ok(Err(format!("Answer through Talk on incident {n}.")));
    }
    let Some(task) = task else {
        return Ok(Err(match session {
            Some(_) => format!("{id} was made in a session terminum did not start: answer in that session."),
            None => format!("{id} has no agent to answer: a digest, or a report moved from the board."),
        }));
    };
    // only the conversation the report was made in: its task's newest run is the run that made it, so a report a
    // manager, the Supervisor or another agent wrote on a task never sends the human's words into that task's pane
    // (or a later run of the same conversation, a reply's resume among them)
    let made_in: Option<String> = tx.query_row("SELECT run FROM reports WHERE id = ?1", [key], |r| r.get(0)).map_err(sql)?;
    let made_in = made_in.map(|id| crate::dispatcher::runs::by_id(tx, &id)).transpose().ok().flatten();
    let last = crate::dispatcher::runs::last_of(tx, &task)?;
    let same = match (&made_in, &last) {
        (Some(m), Some(l)) => m.id == l.id || (m.agent_session.is_some() && m.agent_session == l.agent_session),
        _ => false,
    };
    let (slug, archived): (String, bool) = tx.query_row("SELECT slug, archived_at IS NOT NULL FROM tasks WHERE id = ?1", [&task],
        |r| Ok((r.get(0)?, r.get(1)?))).map_err(sql)?;
    Ok(if archived {
        Err(format!("{slug} is archived: bring it back to reply."))
    } else if !same {
        Err(format!("{id} was not made in {slug}'s last run: answer in its window, with RUN."))
    } else {
        Ok(slug)
    })
}

/// What the agent is told, fixed but for the reply: the report by its ID only, never its text (AC-270).
pub fn words(task: &str, report: &str, reply: &str) -> String {
    format!("{task}: the human replied to your report {report}. Answer below it by execute_tool(\"append_to_report\", {{\"id\": \"{report}\", \"text\": ...}}), or, when they ask for a new report, by execute_tool(\"create_report\", {{\"task\": \"{task}\", \"follows\": \"{report}\", \"title\": ..., \"summary\": ..., \"body\": ...}}); then call finish_task on {task} as before.\n\n{reply}")
}

#[derive(Deserialize)]
struct Reply {
    text: String,
}

/// Send: RUN's press of the report's task with the fixed words; the reply kept below the report once it took.
async fn reply(State(app): State<Shared>, Path(id): Path<String>, Json(body): Json<Reply>) -> Response {
    let text = body.text.trim().to_string();
    let wanted = id.clone();
    let found = store(&app, move |tx| {
        let key = record::key_of(tx, &wanted)?;
        let slug: String = tx.query_row("SELECT slug FROM reports WHERE id = ?1", [&key], |r| r.get(0)).map_err(sql)?;
        let to = target(tx, &key)?;
        Ok((key, slug, to))
    }).await;
    let (key, slug, task) = match found {
        Ok((key, slug, Ok(task))) if !text.is_empty() => (key, slug, task),
        Ok((_, _, Ok(_))) => return answer::<()>(Err("Write a reply first.".into())),
        Ok((_, _, Err(why))) | Err(why) => return answer::<()>(Err(why)),
    };
    // RUN's own checks and gate decide; a refusal keeps nothing here, the reply stays in its field
    let pressed = crate::dispatcher::api::press_reply(&app, &task, words(&task, &slug, &text)).await;
    if let Err(why) = pressed {
        return answer::<()>(Err(why));
    }
    let kept = store(&app, move |tx| {
        let at = ops::now();
        record::note(tx, &key, HUMAN, "reply", &record::check_note(&text)?, at)?;
        tx.execute("UPDATE reports SET replied_at = ?1 WHERE id = ?2", (at, &key)).map_err(sql)?;
        // answering it is seeing it: its incident is acknowledged as reading would (AC-276)
        record::acknowledge_by_reading(tx, &key, at).map(drop)
    }).await;
    crate::tasks::api::told(&app, &task);
    super::changed(&app);
    answer(kept.map(|()| json!({"task": task})))
}

/// Report `p`'s reply has no answer yet: no agent's comment since it, no report following it (AC-270, AC-277).
pub(crate) const UNANSWERED: &str = "NOT EXISTS (SELECT 1 FROM report_notes n JOIN users u ON u.id = n.author WHERE n.report = p.id AND u.kind = 'agent' AND n.kind = 'comment' AND n.at >= p.replied_at)
          AND NOT EXISTS (SELECT 1 FROM reports f WHERE f.follows = p.id AND f.created_at >= p.replied_at)";

/// Report `key`'s last reply still unanswered: the agent answering it now, else why no answer comes.
pub fn answer_state(tx: &Transaction, key: &str) -> Result<(Option<String>, Option<String>), String> {
    let got: Option<(String, String, Option<String>, bool)> = tx.query_row(&format!("SELECT t.slug, t.status,
            (SELECT r.agent_name FROM run_tasks m JOIN runs r ON r.id = m.run WHERE m.task = t.id ORDER BY r.rowid DESC LIMIT 1),
            EXISTS (SELECT 1 FROM run_waits w WHERE w.task = t.id)
        FROM reports p JOIN tasks t ON t.id = p.task
        WHERE p.id = ?1 AND p.replied_at IS NOT NULL AND {UNANSWERED}"), [key],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).optional().map_err(sql)?;
    let Some((task, status, agent, waits)) = got else { return Ok((None, None)) };
    let agent = agent.unwrap_or_else(|| "Its agent".into());
    Ok(if status == "IN PROGRESS" || waits { (Some(agent), None) } else { (None, Some(format!("No answer from {agent}: {task} is {status}."))) })
}
