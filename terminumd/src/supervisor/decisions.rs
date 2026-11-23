// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-240: owner choices grounded in a register problem and its recorded evidence.

use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::roles::supervisor::author;
use crate::tasks::{comments, ops::{self, NewTask, rows, sql}};

use super::{register, signals};

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Kind { FixOptions, StandingRule, Budget, DesignSignoff, Hold, OpenBreaker, Routine }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Proposal {
    problem: i64,
    kind: Kind,
    why: String,
    recommend_why: String,
    waiting: String,
    cost: f64,
    recommended: String,
    options: Vec<OptionLabel>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OptionLabel { id: String, label: String }

#[derive(Serialize)]
pub(super) struct Decision {
    id: i64,
    pub(super) problem: i64,
    pub(super) question: String,
    pub(super) task: Option<String>,
    cost: f64,
}

pub(super) fn pending(tx: &Transaction, project: &str) -> Result<Vec<Decision>, String> {
    rows(tx, "SELECT d.id, d.problem, d.question, t.slug, d.cost
        FROM supervisor_decisions d JOIN supervisor_problems p ON p.id = d.problem
        LEFT JOIN tasks t ON t.id = d.task AND t.project = d.project
        WHERE d.project = ?1 AND d.published AND p.state NOT IN ('resolved', 'accepted')
          AND d.id = (SELECT max(latest.id) FROM supervisor_decisions latest WHERE latest.problem = d.problem AND latest.kind = d.kind
            AND latest.published)
          AND (d.task IS NULL OR (t.status = 'WAIT FOR HUMAN' AND t.archived_at IS NULL
            AND NOT EXISTS (SELECT 1 FROM events e WHERE e.task = t.id AND e.author = 'human' AND e.kind = 'comment'
              AND e.rowid > (SELECT max(q.rowid) FROM events q WHERE q.task = t.id AND json_extract(q.data, '$.questions') = 1))))
        ORDER BY d.cost DESC, d.id LIMIT 100", [project],
        |r| Ok(Decision { id: r.get(0)?, problem: r.get(1)?, question: r.get(2)?, task: r.get(3)?, cost: r.get(4)? }))
}

fn prose(text: &str) -> Result<String, String> {
    let clean = crate::plans::rules::clean_with(text, false);
    if clean.is_empty() || clean.chars().count() > 600 {
        return Err("Give each decision explanation in 1–600 characters.".into());
    }
    Ok(clean)
}

pub(super) fn propose(tx: &Transaction, project: &str, run: Option<&str>, draft: Proposal, at: i64) -> Result<Value, String> {
    let p = register::get(tx, project, draft.problem, at)?;
    let scope: Option<String> = tx.query_row("SELECT fingerprint FROM supervisor_rounds
        WHERE run = ?1 AND project = ?2 AND failure IS NULL",
        (run, project), |r| r.get(0)).optional().map_err(sql)?;
    if !p.active() || !p.yardstick.iter().any(|y| Some(&y.fingerprint) == scope.as_ref()) {
        return Err("A decision requires this round's signal on an open register problem.".into());
    }
    if draft.options.len() < 2 || !draft.cost.is_finite() || draft.cost < 0. {
        return Err("Give at least two options and a nonnegative cost of waiting.".into());
    }
    let choice = json!({"kind": "strategic", "recommended": draft.recommended, "options": draft.options});
    let choice = comments::normalized_question(&choice)?;
    let recommended = draft.options.iter().find(|o| o.id == draft.recommended).ok_or("Choose a recommended option.")?;
    let kind = serde_json::to_value(draft.kind).map_err(|e| e.to_string())?;
    let kind = kind.as_str().ok_or("Unknown decision kind.")?;
    let mut evidence = Vec::new();
    for yardstick in &p.yardstick {
        let s = signals::get(tx, project, &yardstick.fingerprint)?;
        evidence.push((s.fingerprint, s.evidence));
    }
    evidence.sort_by(|a, b| a.0.cmp(&b.0));
    let encoded = serde_json::to_vec(&evidence).map_err(|e| e.to_string())?;
    let hash = crate::auth::crypto::hex(&Sha256::digest(encoded));
    let previous: Option<(i64, Option<String>)> = tx.query_row("SELECT id,
        (SELECT slug FROM tasks WHERE id = d.task AND project = d.project)
        FROM supervisor_decisions d WHERE problem = ?1 AND kind = ?2 AND evidence = ?3
        AND (d.published OR d.source_run = ?4 OR EXISTS (
            SELECT 1 FROM supervisor_rounds q WHERE q.run = d.source_run AND q.failure IS NULL AND q.finding IS NOT NULL))",
        (p.id, kind, &hash, run), |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(sql)?;
    if let Some((id, task)) = previous {
        return Ok(json!({"decision": id, "task": task, "already_recorded": true}));
    }
    let newer: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM supervisor_decisions WHERE problem = ?1 AND kind = ?2)",
        (p.id, kind), |r| r.get(0)).map_err(sql)?;
    let others = draft.options.iter().filter(|o| o.id != recommended.id)
        .map(|o| prose(&o.label)).collect::<Result<Vec<_>, _>>()?.join("; ");
    let evidence_ids = evidence.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>().join(", ");
    let question = format!("Why you are needed:\n{}\n\nRecommended:\n1. {}. {} Other options: {others}.\n\nDetails:\nRegister problem {}. {}Evidence: {evidence_ids}. Cost of waiting: {}.",
        prose(&draft.why)?, prose(&recommended.label)?, prose(&draft.recommend_why)?, p.id,
        if newer { "New evidence is recorded for this decision. " } else { "" }, prose(&draft.waiting)?);
    let id: i64 = tx.query_row("INSERT INTO supervisor_decisions (project, problem, kind, evidence, question, cost, source_run, choices)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        ON CONFLICT (problem, kind, evidence) DO UPDATE SET question = excluded.question,
            cost = excluded.cost, source_run = excluded.source_run, choices = excluded.choices RETURNING id",
        (project, p.id, kind, &hash, &question, draft.cost, run, choice.to_string()), |r| r.get(0)).map_err(sql)?;
    Ok(json!({"decision": id, "recorded": true, "awaiting_finding": true}))
}

pub(super) fn advance(tx: &Transaction, project: &str, at: i64) -> Result<Vec<String>, String> {
    if !register::advisor_enabled(tx, project)? { return Ok(Vec::new()); }
    let ready = rows(tx, "SELECT d.id, d.problem, d.kind, d.question, d.choices
        FROM supervisor_decisions d JOIN supervisor_rounds q ON q.run = d.source_run
        WHERE d.project = ?1 AND q.project = d.project AND NOT d.published
            AND q.finding IS NOT NULL AND q.failure IS NULL ORDER BY d.id LIMIT 20",
        [project], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?,
            r.get::<_, String>(3)?, super::signals::decode::<Value>(r, 4)?)))?;
    let mut changed = Vec::new();
    for (id, problem, kind, words, choices) in ready {
        let p = register::get(tx, project, problem, at)?;
        let card = if kind != "routine" && p.impact >= 2 && p.action_ready && p.state != "suspected" && p.active() {
            Some(open(tx, project, problem, &kind, &words, &choices, at)?)
        } else { None };
        tx.execute("UPDATE supervisor_decisions SET task = ?2, published = 1 WHERE id = ?1",
            (id, card.as_ref().map(|t| t.key.as_str()))).map_err(sql)?;
        changed.extend(card.map(|t| t.id));
    }
    Ok(changed)
}

fn open(tx: &Transaction, project: &str, problem: i64, kind: &str, words: &str, choices: &Value, at: i64) -> Result<ops::Task, String> {
    let by = author(tx, project)?;
    let prior: Option<String> = tx.query_row("SELECT t.slug FROM supervisor_decisions d JOIN tasks t ON t.id = d.task AND t.project = d.project
        WHERE d.problem = ?1 AND d.kind = ?2 AND t.status = 'WAIT FOR HUMAN' AND t.archived_at IS NULL
        AND NOT EXISTS (SELECT 1 FROM events e WHERE e.task = t.id AND e.kind = 'comment' AND e.author = 'human'
            AND e.rowid > (SELECT max(q.rowid) FROM events q WHERE q.task = t.id AND json_extract(q.data, '$.questions') = 1))
        ORDER BY d.id DESC LIMIT 1", (problem, kind), |r| r.get(0)).optional().map_err(sql)?;
    let task = if let Some(id) = prior {
        ops::edit(tx, &by, &id, ops::Edit { text: Some(words.into()), ..Default::default() }, at)?;
        ops::get(tx, &id)?
    } else {
        ops::create(tx, &by, NewTask { title: format!("Supervisor decision for problem {problem}"),
            text: words.into(), project: Some(project.into()), severity: Some("P1".into()), ..NewTask::default() }, at)?
    };
    super::task_policy::mark_card(tx, &task)?;
    comments::wait_for_owner(tx, &by, &task, words, choices, at)?;
    Ok(task)
}

pub(super) fn charter_due(tx: &Transaction, project: &str, at: i64) -> Result<(), String> {
    let charter = super::settings::read(tx, project)?.settings.charter;
    for (field, due) in [("priorities", charter.priority_review_at), ("allocation", charter.allocation_review_at)] {
        let Some(due) = due.filter(|due| *due <= at) else { continue };
        let asked: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM supervisor_charter_questions WHERE project = ?1 AND field = ?2 AND due = ?3)",
            (project, field, due), |r| r.get(0)).map_err(sql)?;
        if asked { continue; }
        let by = author(tx, project)?;
        let words = format!("Why you are needed:\nThe charter's review date for {field} has passed. The owner sets it.\n\nRecommended:\n1. Keep the current {field} if they still fit. Otherwise update them in Supervisor settings.\n\nDetails:\nIf this waits, the current {field} continue to guide the project.");
        let task = ops::create(tx, &by, NewTask { title: format!("Review the charter's {field}"), text: words.clone(),
            project: Some(project.into()), severity: Some("P1".into()), ..NewTask::default() }, at)?;
        super::task_policy::mark_card(tx, &task)?;
        comments::wait_for_owner(tx, &by, &task, &words, &Value::Null, at)?;
        tx.execute("INSERT INTO supervisor_charter_questions VALUES (?1, ?2, ?3, ?4)",
            (project, field, due, task.key)).map_err(sql)?;
    }
    Ok(())
}

/// Transcript mining requires an owner's action. Use the counter subjects and their figures only.
pub(super) fn retro(tx: &Transaction, project: &str, at: i64) -> Result<Option<String>, String> {
    if !register::advisor_enabled(tx, project)? { return Ok(None); }
    let active = rows(tx, "SELECT subject_id, max(json_extract(scores, '$.impact')),
        max(json_extract(measure, '$.value')), max(json_extract(measure, '$.threshold'))
        FROM supervisor_signals WHERE project = ?1 AND detector = 'drift.loop' AND subject_kind = 'session'
        AND state IN ('new','raised','recurred') AND last_seen >= ?2 AND json_extract(scores, '$.impact') >= 1
        GROUP BY subject_id ORDER BY subject_id LIMIT 12", (project, at - super::WEEK),
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, u8>(1)?, r.get::<_, f64>(2)?, r.get::<_, f64>(3)?)))?;
    if active.len() < 2 { return Ok(None); }
    let subjects = serde_json::to_string(&active.iter().map(|(s, _, _, _)| s).collect::<Vec<_>>()).map_err(|e| e.to_string())?;
    let existed: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM supervisor_drift_questions WHERE project = ?1 AND subjects = ?2)",
        (project, &subjects), |r| r.get(0)).map_err(sql)?;
    if existed { return Ok(None); }
    let evidence = active.iter().map(|(session, _, value, threshold)| {
        format!("- {} measured {value} against a drift threshold of {threshold}.", crate::plans::rules::clean_with(session, false))
    }).collect::<Vec<_>>().join("\n");
    let words = format!("Why you are needed:\nTwo or more sessions exceeded their drift threshold. Only you can choose to review their conversations.\n\nRecommended:\n1. Run `/retro` in the named sessions if the drift warrants a deeper look. Otherwise keep monitoring.\n\nDetails:\nIf this waits, the cause remains unexamined. The Supervisor used counters.\n{evidence}");
    let task = if active.iter().any(|(_, impact, _, _)| *impact >= 2) {
        let by = author(tx, project)?;
        let task = ops::create(tx, &by, NewTask { title: "Review repeated session drift".into(),
            text: words.clone(), project: Some(project.into()), severity: Some("P1".into()), ..NewTask::default() }, at)?;
        super::task_policy::mark_card(tx, &task)?;
        comments::wait_for_owner(tx, &by, &task, &words, &Value::Null, at)?;
        Some(task)
    } else { None };
    tx.execute("INSERT INTO supervisor_drift_questions VALUES (?1, ?2, ?3, ?4, ?5)",
        (project, subjects, words, task.as_ref().map(|t| &t.key), at)).map_err(sql)?;
    Ok(task.map(|t| t.id))
}
