// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-240: owner choices grounded in a register problem and its recorded evidence.

use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::roles::supervisor::author;
use crate::tasks::{comments, ops::{self, NewTask, rows, sql}};

use super::{flow, register, signals};

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
    /// A standing rule's: the event ID of one asking of the recurring question it ends.
    #[serde(default)]
    recurring: Option<String>,
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

/// Deferred choices appear together within the weekly review.
pub(super) fn cards(tx: &Transaction, project: &str) -> Result<Vec<Decision>, String> {
    Ok(pending(tx, project)?.into_iter().filter(|decision| decision.task.is_some()).collect())
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
    let recurring = match (matches!(draft.kind, Kind::StandingRule), draft.recurring) {
        (true, Some(event)) => {
            let asking = super::Evidence::Events(event.clone());
            if !evidence.iter().any(|(_, ids)| ids.contains(&asking)) {
                return Err("Name a question event from this problem's signal evidence.".into());
            }
            Some(recurring_question(tx, project, &event, at)?)
        }
        (true, None) => return Err("A standing rule names the recurring question it ends: give recurring, the event ID of one asking.".into()),
        (false, Some(_)) => return Err("Only a standing rule names a recurring question.".into()),
        (false, None) => None,
    };
    // The recurring question joins the hash only where named, so other kinds keep their recorded identities.
    let encoded = match &recurring {
        Some(question) => serde_json::to_vec(&(&evidence, question)),
        None => serde_json::to_vec(&evidence),
    }.map_err(|e| e.to_string())?;
    let hash = crate::auth::crypto::sha256_hex(&encoded);
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
    let id: i64 = tx.query_row("INSERT INTO supervisor_decisions (project, problem, kind, evidence, question, cost, source_run, choices, recurring)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        ON CONFLICT (problem, kind, evidence) DO UPDATE SET question = excluded.question,
            cost = excluded.cost, source_run = excluded.source_run, choices = excluded.choices RETURNING id",
        (project, p.id, kind, &hash, &question, draft.cost, run, choice.to_string(), recurring), |r| r.get(0)).map_err(sql)?;
    Ok(json!({"decision": id, "recorded": true, "awaiting_finding": true}))
}

/// The text of a question asked in this project more than once, by one asking's event ID.
fn recurring_question(tx: &Transaction, project: &str, event: &str, at: i64) -> Result<String, String> {
    let asked: Option<(String, i64)> = tx.query_row(&format!("{}, asked AS (SELECT id, text FROM project_events
        WHERE kind = 'comment' AND json_extract(data, '$.questions') IS NOT NULL AND text IS NOT NULL)
        SELECT a.text, (SELECT count(*) FROM asked b WHERE b.text = a.text) FROM asked a WHERE a.id = ?3",
        flow::project_events()), (project, at, event), |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(sql)?;
    match asked {
        Some((text, n)) if n >= 2 => Ok(text),
        Some(_) => Err("That question was asked once. A standing rule ends a recurring question.".into()),
        None => Err("No question with that event ID in this project.".into()),
    }
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
    let question = comments::wait_for_owner(tx, &by, &task, words, choices, at)?;
    if let Some(s) = super::action_tasks::problem_signal(tx, project, problem)? {
        super::outcomes::record(tx, &s, super::outcomes::Kind::Card, &question, &task.key, "problem", at)?;
    }
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
        let question = comments::wait_for_owner(tx, &by, &task, &words, &Value::Null, at)?;
        super::outcomes::unattributed(tx, project, super::outcomes::Kind::Card, &question, &task.key, at)?;
        tx.execute("INSERT INTO supervisor_charter_questions VALUES (?1, ?2, ?3, ?4)",
            (project, field, due, task.key)).map_err(sql)?;
    }
    Ok(())
}

/// Transcript mining requires an owner's action. Use the counter subjects and their figures only.
/// Two or more sessions above their drift threshold open a card whatever their impact (hook counters size none);
/// one is a lesser decision for the weekly list. A session whose counters carry no figure is named, never counted.
pub(super) fn retro(tx: &Transaction, project: &str, at: i64) -> Result<Option<String>, String> {
    if !register::advisor_enabled(tx, project)? { return Ok(None); }
    let readings = rows(tx, "SELECT subject_id,
        CASE WHEN json_type(measure, '$.value') IN ('integer', 'real') THEN json_extract(measure, '$.value') END,
        CASE WHEN json_type(measure, '$.threshold') IN ('integer', 'real') THEN json_extract(measure, '$.threshold') END
        FROM supervisor_signals WHERE project = ?1 AND detector IN ('drift.loop', 'drift.subagents') AND subject_kind = 'session'
        AND state IN ('new','raised','recurred') AND last_seen >= ?2 ORDER BY subject_id LIMIT 48", (project, at - super::WEEK),
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<f64>>(1)?, r.get::<_, Option<f64>>(2)?)))?;
    let mut drifting = std::collections::BTreeMap::<String, (f64, f64)>::new();
    let mut unknown = std::collections::BTreeSet::new();
    for (session, value, threshold) in readings {
        match (value, threshold) {
            (Some(value), Some(threshold)) if value > threshold => {
                let best = drifting.entry(session).or_insert((value, threshold));
                if value - threshold > best.0 - best.1 { *best = (value, threshold); }
            }
            (Some(_), Some(_)) => {}
            _ => { unknown.insert(session); }
        }
    }
    unknown.retain(|session| !drifting.contains_key(session));
    if drifting.is_empty() { return Ok(None); }
    let subjects = serde_json::to_string(&drifting.keys().collect::<Vec<_>>()).map_err(|e| e.to_string())?;
    let existed: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM supervisor_drift_questions WHERE project = ?1 AND subjects = ?2)",
        (project, &subjects), |r| r.get(0)).map_err(sql)?;
    if existed { return Ok(None); }
    let name = |session: &String| format!("`{}`", crate::plans::rules::clean_with(session, false));
    let names = drifting.keys().map(name).collect::<Vec<_>>().join(", ");
    let mut evidence = drifting.iter().map(|(session, (value, threshold))| {
        format!("- {} measured {value} against a drift threshold of {threshold}.", name(session))
    }).collect::<Vec<_>>();
    if !unknown.is_empty() {
        evidence.push(format!("- Not counted, their counters carry no figure: {}.", unknown.iter().map(name).collect::<Vec<_>>().join(", ")));
    }
    let card = drifting.len() >= 2;
    let why = if card {
        format!("{} sessions exceeded their drift threshold. Only you can choose to review their conversations.", drifting.len())
    } else {
        "One session exceeded its drift threshold. Only you can choose to review its conversation.".into()
    };
    let words = format!("Why you are needed:\n{why}\n\nRecommended:\n1. Run `/retro` on {names} to find the cause. Otherwise keep monitoring.\n\nDetails:\nIf this waits, the cause remains unexamined. The Supervisor read counters, no transcript.\n{}",
        evidence.join("\n"));
    let task = if card {
        let by = author(tx, project)?;
        let task = ops::create(tx, &by, NewTask { title: "Review repeated session drift".into(),
            text: words.clone(), project: Some(project.into()), severity: Some("P1".into()), ..NewTask::default() }, at)?;
        super::task_policy::mark_card(tx, &task)?;
        let choices = json!({"kind": "operational", "recommended": "retro", "options": [
            {"id": "retro", "label": "Run /retro on the named sessions"}, {"id": "monitor", "label": "Keep monitoring"}]});
        let question = comments::wait_for_owner(tx, &by, &task, &words, &choices, at)?;
        super::outcomes::unattributed(tx, project, super::outcomes::Kind::Card, &question, &task.key, at)?;
        Some(task)
    } else { None };
    tx.execute("INSERT INTO supervisor_drift_questions VALUES (?1, ?2, ?3, ?4, ?5)",
        (project, subjects, words, task.as_ref().map(|t| &t.key), at)).map_err(sql)?;
    Ok(task.map(|t| t.id))
}
