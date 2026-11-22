// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-233–235: eligibility, case files and the durable investigation ledger.

use axum::{Router, extract::{Path, State}, response::Response, routing::{get, post}};
use rusqlite::{OptionalExtension, Transaction};
use serde_json::{Value, json};

use crate::app::Shared;
use crate::tasks::{api::{answer, store}, ops::{now, rows, sql}};

use super::{Evidence, Signal, signals, settings};

const TIME_LIMIT: i64 = 15 * 60;
const EXCERPT: usize = 1_000;
const CASE_EVIDENCE: usize = 128;

pub(crate) fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/task-projects/{project}/supervisor/signals/{key}/investigate", post(signal))
        .route("/api/supervisor/incidents/{incident}/investigate", post(incident))
        .route("/api/task-projects/{project}/supervisor/rounds/{case}", get(saved_case))
}

async fn saved_case(State(app): State<Shared>, Path((project, case)): Path<(String, String)>) -> Response {
    answer(crate::http::off(move || app.tasks.snapshot(super::SCHEMA, |tx| {
        let raw: String = tx.query_row("SELECT json_object('case', json(case_file), 'finding', json(finding),
            'failure', failure, 'refused', refused, 'ended_at', ended_at) FROM supervisor_rounds
            WHERE id = ?1 AND project = ?2", (case, project), |r| r.get(0)).optional().map_err(sql)?
            .ok_or("No such investigation in this project.")?;
        serde_json::from_str::<Value>(&raw).map_err(|e| e.to_string())
    })).await)
}

async fn signal(State(app): State<Shared>, Path((project, key)): Path<(String, String)>) -> Response {
    answer(store(&app, move |tx| request(tx, &project, &key)).await)
}

async fn incident(State(app): State<Shared>, Path(id): Path<i64>) -> Response {
    answer(store(&app, move |tx| {
        let (project, key): (String, String) = tx.query_row("SELECT i.project, e.fingerprint
            FROM supervisor_incidents i JOIN supervisor_incident_events e ON e.incident = i.id
            JOIN supervisor_signals s ON s.fingerprint = e.fingerprint AND s.project = i.project
            WHERE i.id = ?1 ORDER BY e.level DESC, e.id DESC LIMIT 1", [id],
            |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(sql)?
            .ok_or("This incident has no retained signal to investigate.")?;
        request(tx, &project, &key)
    }).await)
}

fn request(tx: &Transaction, project: &str, key: &str) -> Result<Value, String> {
    let sig = signals::get(tx, project, key)?;
    enabled(tx, project)?.ok_or("Enable the project's Supervisor to investigate.")?;
    if let Some(id) = active_id(tx, key)? {
        return Ok(json!({"message": format!("Investigation {id} is already open.")}));
    }
    let saved = settings::read(tx, project)?.settings;
    check_room(&saved, &sig)?;
    if crate::tasks::ops::project_place(tx, project)?.1.is_some() {
        return Err("Supervisor investigations require a local pool agent for pane identity checks.".into());
    }
    tx.execute("INSERT INTO supervisor_round_requests (fingerprint, owner_requested)
        VALUES (?1, 1) ON CONFLICT (fingerprint) DO UPDATE SET owner_requested = 1, reason = NULL", [key]).map_err(sql)?;
    Ok(json!({"message": "Investigation requested. It will start when its slot is free."}))
}

pub(crate) fn enabled(tx: &Transaction, project: &str) -> Result<Option<i64>, String> {
    tx.query_row("SELECT generation FROM project_supervisor WHERE project = ?1 AND enabled = 1",
        [project], |r| r.get(0)).optional().map_err(sql)
}

fn active_id(tx: &Transaction, key: &str) -> Result<Option<String>, String> {
    tx.query_row("SELECT id FROM supervisor_rounds WHERE fingerprint = ?1 AND ended_at IS NULL", [key],
        |r| r.get(0)).optional().map_err(sql)
}

pub(super) fn status(tx: &Transaction, key: &str) -> Result<Option<String>, String> {
    let latest: Option<(String, bool, bool, bool, Option<String>)> = tx.query_row(
        "SELECT id, ended_at IS NULL, finding IS NOT NULL, signal_revision = s.revision, failure
         FROM supervisor_rounds JOIN supervisor_signals s USING (fingerprint) WHERE fingerprint = ?1
         ORDER BY supervisor_rounds.rowid DESC LIMIT 1", [key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
        .optional().map_err(sql)?;
    if let Some((id, true, _, _, _)) = &latest {
        return Ok(Some(format!("Investigation {id} is open.")));
    }
    let waiting: Option<(bool, Option<String>)> = tx.query_row("SELECT owner_requested, reason
        FROM supervisor_round_requests WHERE fingerprint = ?1", [key], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional().map_err(sql)?;
    if let Some((requested, reason)) = waiting {
        if reason.is_some() { return Ok(reason); }
        if requested { return Ok(Some("Investigation requested.".into())); }
    }
    Ok(latest.filter(|(_, _, _, same, _)| *same).map(|(id, _, found, _, why)| match why {
        Some(why) => format!("Investigation {id} ended: {why}"),
        None if found => format!("Investigation {id} recorded its finding."),
        None => format!("Investigation {id} ended."),
    }))
}

fn check_room<'a>(settings: &'a settings::Settings, signal: &Signal) -> Result<&'a str, String> {
    if !settings.investigation_room() {
        return Err("The Supervisor's judgment budget has no room for an investigation.".into());
    }
    let (tier, agent) = settings.tiers.investigation(signal.level);
    agent.ok_or_else(|| format!("Choose a {tier}-tier pool agent to investigate."))
}

pub(crate) fn waiting(tx: &Transaction, at: i64, key: Option<&str>) -> Result<Vec<(String, String)>, String> {
    let week = at.div_euclid(super::WEEK);
    rows(tx, "WITH eligible AS (
        SELECT s.project, s.fingerprint, s.first_seen,
            max(json_extract(s.scores, '$.impact'), json_extract(s.scores, '$.blast'),
                json_extract(s.scores, '$.reversibility')) AS level,
            json_extract(s.scores, '$.impact') AS impact
        FROM project_supervisor p JOIN supervisor_signals s USING (project)
        LEFT JOIN supervisor_round_requests q USING (fingerprint)
        WHERE p.enabled = 1 AND (?2 IS NULL OR s.fingerprint = ?2) AND NOT EXISTS (
            SELECT 1 FROM supervisor_rounds r WHERE r.fingerprint = s.fingerprint AND r.ended_at IS NULL)
        AND (q.owner_requested = 1 OR (
            s.state IN ('new', 'raised', 'recurred', 'held') AND s.detector <> 'self.round_failed'
            AND NOT EXISTS (SELECT 1 FROM supervisor_rounds r WHERE r.fingerprint = s.fingerprint
                AND (r.signal_revision = s.revision OR (level = 1 AND r.finding IS NOT NULL)))
            AND (level >= 2 OR (level = 1 AND 3 <= (
                SELECT count(*) FROM supervisor_signal_weeks w
                WHERE w.fingerprint = s.fingerprint AND w.week BETWEEN ?1 - 7 AND ?1)))))
        )
        SELECT project, fingerprint FROM eligible ORDER BY impact DESC, level DESC, first_seen, fingerprint",
        (week, key), |r| Ok((r.get(0)?, r.get(1)?)))
}

pub(crate) struct Candidate {
    signal: Signal,
    generation: i64,
    revision: i64,
    pub agent: String,
}

pub(crate) fn candidate(tx: &Transaction, project: &str, key: &str) -> Result<Candidate, String> {
    let generation = enabled(tx, project)?.ok_or("The Supervisor is off.")?;
    if let Some(id) = active_id(tx, key)? { return Err(format!("Investigation {id} is already open.")); }
    let signal = signals::get(tx, project, key)?;
    let saved = settings::read(tx, project)?.settings;
    let agent = check_room(&saved, &signal)?.to_owned();
    let revision = tx.query_row("SELECT revision FROM supervisor_signals WHERE fingerprint = ?1",
        [key], |r| r.get(0)).map_err(sql)?;
    Ok(Candidate { signal, generation, revision, agent })
}

pub(crate) fn waiting_reason(tx: &Transaction, key: &str, reason: &str) -> Result<(), String> {
    tx.execute("INSERT INTO supervisor_round_requests (fingerprint, reason)
        SELECT fingerprint, ?2 FROM supervisor_signals WHERE fingerprint = ?1
        ON CONFLICT (fingerprint) DO UPDATE SET reason = excluded.reason WHERE reason IS NOT excluded.reason",
        (key, reason)).map_err(sql)?;
    Ok(())
}

fn excerpt(text: String) -> String {
    crate::sent::redact(&text).chars().take(EXCERPT).collect()
}

fn task_data(tx: &Transaction, signal: &Signal) -> Result<Vec<Value>, String> {
    let mut tasks = std::collections::BTreeSet::new();
    if signal.subject_kind == "task" { tasks.insert(signal.subject_id.clone()); }
    for id in signal.evidence.iter().take(CASE_EVIDENCE) {
        match id {
            Evidence::Tasks(id) => { tasks.insert(id.clone()); }
            Evidence::Events(id) => {
                let task = tx.query_row("SELECT e.task FROM events e JOIN tasks t ON t.id = e.task
                    WHERE e.id = ?1 AND t.project = ?2", (id, &signal.project), |r| r.get::<_, String>(0))
                    .optional().map_err(sql)?;
                tasks.extend(task);
            }
            _ => (),
        }
    }
    tasks.into_iter().take(4).filter_map(|task| {
        tx.query_row("SELECT slug, title, body FROM tasks WHERE id = ?1 AND project = ?2", (&task, &signal.project),
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
            .optional().transpose().map(|row| {
                let (id, title, text) = row.map_err(sql)?;
                let comments = rows(tx, "SELECT text FROM events WHERE task = ?1 AND kind = 'comment' AND text IS NOT NULL
                    ORDER BY at DESC, rowid DESC LIMIT 3", [&task], |r| r.get::<_, String>(0))?;
                Ok(json!({"task": id, "title_data": excerpt(title), "description_data": excerpt(text),
                    "comments_data": comments.into_iter().map(excerpt).collect::<Vec<_>>() }))
            })
    }).collect()
}

pub(crate) fn save(tx: &Transaction, run: &str, candidate: Candidate, at: i64, clock_scale: i64) -> Result<String, String> {
    let Candidate { mut signal, generation, revision, .. } = candidate;
    let id = format!("case_{run}");
    let verify = signal.level >= 2 && signal.scores.confidence < 0.7;
    let saved = settings::read(tx, &signal.project)?.settings;
    let charter = saved.investigation_charter().map(|lines| excerpt(lines.join("\n")));
    let deadline = at + (TIME_LIMIT / clock_scale).max(1);
    let evidence_count = signal.evidence.len();
    signal.evidence.truncate(CASE_EVIDENCE);
    let mut evidence = Vec::new();
    for id in &signal.evidence {
        evidence.push(json!({"reference": id, "row": super::round_evidence::resolve(tx, &signal.project, id)?}));
    }
    let case = json!({
        "id": id, "kind": if verify { "verification" } else { "investigation" },
        "signal": signal, "deadline": deadline, "budget_seconds": TIME_LIMIT,
        "charter_data": {"never": charter[0], "priorities": charter[1]},
        "task_data": task_data(tx, &signal)?, "evidence_rows": evidence, "evidence_count": evidence_count,
        "question": if verify { "Does the named measurement and its evidence confirm the signal?" }
            else { "What caused this signal, what would disprove the cause, and what would address it?" },
        "tools": super::round_tools::NAMES,
        "verification_hold": verify.then_some("No authorized hold with a lower measured cost than waiting."),
    });
    tx.execute("INSERT INTO supervisor_rounds (id, run, project, generation, fingerprint, signal_revision, case_file, deadline)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)", rusqlite::params![id, run, signal.project, generation,
            signal.fingerprint, revision, case.to_string(), deadline]).map_err(sql)?;
    tx.execute("DELETE FROM supervisor_round_requests WHERE fingerprint = ?1", [&signal.fingerprint]).map_err(sql)?;
    Ok(id)
}

// Callers bind ?2 to the server clock. A lost mark never acquires another role.
pub(crate) const ACTIVE: &str = "q.deadline > ?2 AND q.ended_at IS NULL AND q.failure IS NULL AND q.finding IS NULL
    AND EXISTS (SELECT 1 FROM project_supervisor p WHERE p.project = q.project
        AND p.enabled = 1 AND p.generation = q.generation)";

/// A staged investigation must still be active before starting the agent and typing.
pub(crate) fn check_first_prompt(tx: &Transaction, run: &str) -> Result<(), String> {
    let active = tx.prepare_cached(&format!("SELECT EXISTS (SELECT 1 FROM supervisor_rounds q
        WHERE q.run = ?1 AND ({ACTIVE}))")).map_err(sql)?
        .query_row((run, now()), |r| r.get::<_, bool>(0)).map_err(sql)?;
    if active { Ok(()) } else { Err("The Supervisor investigation is no longer active.".into()) }
}

pub(crate) fn mark(tx: &Transaction, run: &str, key: &str) -> Result<(), String> {
    check_first_prompt(tx, run)?;
    let changed = tx.execute("INSERT INTO supervisor_panes (pane_key, project, generation, round)
        SELECT ?1, project, generation, id FROM supervisor_rounds WHERE run = ?2", (key, run)).map_err(sql)?;
    if changed == 1 { Ok(()) } else { Err("The Supervisor investigation is gone.".into()) }
}

pub(crate) fn failure(tx: &Transaction, run: &str, why: &str, refused: bool) -> Result<(), String> {
    let (id, project, count, previous): (String, String, i64, Option<String>) = tx.query_row(
        "SELECT id, project, refused, failure FROM supervisor_rounds WHERE run = ?1", [run],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(sql)?;
    if !refused && previous.is_some() { return Ok(()); }
    if refused {
        tx.execute("UPDATE supervisor_rounds SET refused = refused + 1 WHERE run = ?1", [run]).map_err(sql)?;
    } else {
        tx.execute("UPDATE supervisor_rounds SET failure = ?2 WHERE run = ?1", (run, why)).map_err(sql)?;
    }
    if crate::tasks::ops::check_project(tx, &project).is_err() { return Ok(()); }
    let (metric, value) = if refused { ("refused_findings", (count + 1) as f64) } else { ("failed_rounds", 1.) };
    signals::round_failed(tx, &project, &id, metric, value, now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ac233_waiters_use_distinct_weeks_and_impact() {
        let (root, app) = crate::testutil::app_with_tasks("round-waiters");
        let week = 100;
        let at = week * crate::supervisor::WEEK;
        app.tasks.with(|tx| {
            tx.execute("INSERT INTO project_supervisor (project, enabled, generation, enabled_at)
                VALUES ('default', 1, 1, ?1)", [at]).map_err(sql)?;
            for (key, impact, blast) in [("low", 1, 0), ("repeat", 1, 0), ("critical", 3, 0), ("medium", 2, 0)] {
                let scores = json!({"impact": impact, "blast": blast, "reversibility": 0, "confidence": 1.0});
                tx.execute("INSERT INTO supervisor_signals (fingerprint, project, detector, subject_kind, subject_id,
                    metric, evidence, measure, scores, first_seen, last_seen, count, state)
                    VALUES (?1, 'default', 'flow.failed_repeat', 'task', ?1, 'failed_runs', '[]', '{}', ?2, ?3, ?3, 99, 'new')",
                    (key, scores.to_string(), at)).map_err(sql)?;
            }
            for (key, bucket) in [("low", week - 8), ("low", week), ("repeat", week - 7), ("repeat", week - 2), ("repeat", week)] {
                tx.execute("INSERT INTO supervisor_signal_weeks VALUES (?1, ?2)", (key, bucket)).map_err(sql)?;
            }
            let keys = waiting(tx, at, None)?.into_iter().map(|(_, key)| key).collect::<Vec<_>>();
            assert_eq!(keys, ["critical", "medium", "repeat"]);
            Ok(())
        }).unwrap();
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }
}
