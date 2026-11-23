// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-251: design-first notices and durable review evidence on the task.

use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;

use crate::tasks::{attrs, comments, ops::{self, NewTask, Task, sql}, store::HUMAN};
use super::{settings, surfaces::{Match, Tier}};

pub(crate) const RECORD: &str = "record_design_review";
pub(crate) const BRIEF: &str = "request_design_review";
pub(super) const TIME_LIMIT: i64 = 20 * 60;

pub(super) struct Gate {
    pub key: String,
    pub project: String,
    pub tier: u8,
    pub evidence: String,
    pub existing_review: bool,
    pub brief: Option<String>,
    pub blocker: Option<String>,
    pub question: Option<String>,
    pub status: String,
    pub deadline: i64,
    scope: String,
}

pub(super) fn get(tx: &Transaction, key: &str) -> Result<Option<Gate>, String> {
    tx.query_row("SELECT task, project, tier, evidence, existing_review, brief, blocker,
        owner_question, status, deadline, scope FROM supervisor_design_gates WHERE task = ?1", [key], |r| Ok(Gate {
        key: r.get(0)?, project: r.get(1)?, tier: r.get(2)?, evidence: r.get(3)?, existing_review: r.get(4)?,
        brief: r.get(5)?, blocker: r.get(6)?, question: r.get(7)?, status: r.get(8)?, deadline: r.get(9)?, scope: r.get(10)?,
    })).optional().map_err(sql)
}

pub(crate) fn created(tx: &Transaction, author: &str, task: &Task, text: &str, at: i64) -> Result<(), String> {
    if task.status != "TODO" || author.starts_with("supervisor:") || !super::register::advisor_enabled(tx, &task.project)? {
        return Ok(());
    }
    let map = settings::read(tx, &task.project)?.settings.charter.surfaces;
    if let Some(hit) = map.text_match(&task.title, text) {
        observe(tx, task, hit, false, at)?;
    }
    Ok(())
}

/// Diff evidence supersedes a guess from text. A stronger observed tier retains earlier review evidence.
pub(super) fn observe(tx: &Transaction, task: &Task, hit: Match, from_diff: bool, at: i64) -> Result<(), String> {
    if hit.tier < Tier::T2 { return Ok(()); }
    if let Some(old) = get(tx, &task.key)?.filter(|old| old.project != task.project) {
        close(tx, &old, task, at)?;
        tx.execute("DELETE FROM supervisor_design_records WHERE task = ?1", [&task.key]).map_err(sql)?;
        tx.execute("DELETE FROM supervisor_design_gates WHERE task = ?1", [&task.key]).map_err(sql)?;
    }
    let tier = hit.tier.number();
    let deadline = if tier == 2 { at + TIME_LIMIT } else { 0 };
    let scope = serde_json::to_string(&hit.paths).map_err(|e| e.to_string())?;
    let changed = tx.execute("INSERT INTO supervisor_design_gates
        (task, project, tier, evidence, existing_review, opened, deadline, scope)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        ON CONFLICT(task) DO UPDATE SET tier = max(tier, excluded.tier),
            evidence = CASE WHEN excluded.tier >= tier AND ?9 THEN excluded.evidence ELSE evidence END,
            existing_review = CASE WHEN excluded.tier >= tier AND ?9 THEN excluded.existing_review ELSE existing_review END,
            opened = CASE WHEN excluded.tier > tier THEN excluded.opened ELSE opened END,
            deadline = CASE WHEN excluded.tier > tier THEN excluded.deadline ELSE deadline END,
            scope = CASE WHEN excluded.tier >= tier AND ?9 THEN excluded.scope ELSE scope END,
            architecture_round = CASE WHEN excluded.tier > tier THEN NULL ELSE architecture_round END,
            status = CASE WHEN excluded.tier > tier THEN 'waiting' ELSE status END
        WHERE project = excluded.project AND (excluded.tier > tier OR (?9 AND excluded.tier = tier AND
            (evidence IS NOT excluded.evidence OR scope <> excluded.scope OR existing_review IS NOT excluded.existing_review)))",
        rusqlite::params![task.key, task.project, tier, hit.evidence, hit.existing_review, at, deadline, scope, from_diff]).map_err(sql)?;
    if changed != 0 && tier == 3 {
        let by = author(tx, &task.project)?;
        if !ops::noticed(tx, &by, &task.key, "design_gate", "design_first")? {
            ops::notice(tx, &by, &task.id, "Design first: this task may touch a T3 surface. Use request_design_review to record the design before coding.",
                json!({"design_gate": "design_first"}), at)?;
        }
    }
    Ok(())
}

pub(super) fn author(tx: &Transaction, project: &str) -> Result<String, String> {
    crate::roles::supervisor::author(tx, project)
}

fn existing_card(tx: &Transaction, gate: &Gate) -> Result<Option<Task>, String> {
    let id: Option<String> = tx.query_row("SELECT slug FROM tasks WHERE id = ?1 AND project = ?2
        AND archived_at IS NULL AND status <> 'DONE'", (&gate.blocker, &gate.project), |r| r.get(0))
        .optional().map_err(sql)?;
    id.map(|id| ops::get(tx, &id)).transpose()
}

fn card(tx: &Transaction, gate: &Gate, task: &Task, at: i64) -> Result<Task, String> {
    if let Some(card) = existing_card(tx, gate)? { return Ok(card); }
    let by = author(tx, &gate.project)?;
    let card = ops::create(tx, &by, NewTask { title: format!("Review the design for {}", task.id),
        text: format!("Review the T{} design for {}. {}", gate.tier, task.id, gate.evidence),
        project: Some(gate.project.clone()), ..NewTask::default() }, at)?;
    super::task_policy::mark_card(tx, &card)?;
    tx.execute("UPDATE supervisor_design_gates SET blocker = ?2, owner_question = NULL WHERE task = ?1",
        (&task.key, &card.key)).map_err(sql)?;
    Ok(card)
}

pub(super) fn block(tx: &Transaction, gate: &Gate, task: &Task, at: i64) -> Result<(), String> {
    let card = card(tx, gate, task, at)?;
    let by = author(tx, &gate.project)?;
    attrs::link(tx, &by, &task.key, &attrs::Link {
        list: "blocked_by", task: &card.id, reason: Some("The design review is in progress."), remove: false,
    }, at)
}

pub(super) fn unblock(tx: &Transaction, task: &Task, project: &str, blocker: Option<&str>, at: i64) -> Result<(), String> {
    let id: Option<String> = tx.query_row("SELECT t.slug FROM task_links l JOIN tasks t ON t.id = l.other
        WHERE l.task = ?1 AND l.other = ?2 AND l.kind = 'blocked_by'", (&task.key, blocker), |r| r.get(0))
        .optional().map_err(sql)?;
    if let Some(id) = id {
        attrs::link(tx, &author(tx, project)?, &task.key, &attrs::Link {
            list: "blocked_by", task: &id, reason: None, remove: true,
        }, at)?;
    }
    Ok(())
}

pub(super) fn escalate(tx: &Transaction, gate: &Gate, task: &Task, why: &str, at: i64) -> Result<(), String> {
    let signoff = gate.tier == 3 && t3_critics_approved(tx, gate)?;
    if gate.status == "escalated" && (!signoff || gate.question.is_some()) { return Ok(()); }
    let card = card(tx, gate, task, at)?;
    let by = author(tx, &gate.project)?;
    let words = if signoff {
        format!("Why you are needed:\nT3 design {} needs your sign-off.\n\nRecommended:\n1. Read the design and both review records on {}. Approve if the migration and rollback are acceptable, or request changes.\n\nDetails:\n{why} {}", task.id, task.id, gate.evidence)
    } else {
        format!("Why you are needed:\nThe T{} design review for {} needs your help.\n\nRecommended:\n1. Arrange the project's review and have the reviewers leave their records on {}.\n\nDetails:\n{why} {}", gate.tier, task.id, task.id, gate.evidence)
    };
    let choices = if signoff {
        json!({"kind":"strategic","recommended":"revise","options":[{"id":"approve","label":"Approve the T3 design"},{"id":"revise","label":"Request changes"}]})
    } else { Value::Null };
    let event = comments::wait_for_owner(tx, &by, &card, &words, &choices, at)?;
    unblock(tx, task, &gate.project, Some(&card.key), at)?;
    tx.execute("UPDATE supervisor_design_gates SET status = 'escalated', owner_question = ?2 WHERE task = ?1",
        (&gate.key, signoff.then_some(&event))).map_err(sql)?;
    Ok(())
}

pub(super) fn close(tx: &Transaction, gate: &Gate, task: &Task, at: i64) -> Result<(), String> {
    if gate.status == "closed" { return Ok(()); }
    unblock(tx, task, &gate.project, gate.blocker.as_deref(), at)?;
    if let Some(card) = existing_card(tx, gate)? {
        ops::move_to(tx, &author(tx, &gate.project)?, &card.id, "DONE", at)?;
    }
    tx.execute("UPDATE supervisor_design_gates SET status = 'closed' WHERE task = ?1", [&gate.key]).map_err(sql)?;
    Ok(())
}

fn t3_critics_approved(tx: &Transaction, gate: &Gate) -> Result<bool, String> {
    if gate.brief.is_none() { return Ok(false); }
    tx.query_row("SELECT count(DISTINCT author) = 2 FROM supervisor_design_records
        WHERE task = ?1 AND tier = 3 AND verdict = 'approved'", [&gate.key], |r| r.get(0)).map_err(sql)
}

pub(super) fn missing(tx: &Transaction, gate: &Gate, tier: u8) -> Result<Option<&'static str>, String> {
    let architecture = approved(tx, &gate.key, "architecture", tier)?;
    if tier == 3 && gate.brief.is_none() { return Ok(Some("a design note, migration and rollback plan")); }
    if !architecture { return Ok(Some("an architecture review with compatibility and rejected advice")); }
    if tier == 2 { return Ok(None); }
    if !t3_critics_approved(tx, gate)? { return Ok(Some("an independent critic's approval")); }
    let owner: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM events WHERE task = ?1
        AND kind = 'comment' AND author = ?2 AND json_extract(data, '$.choice.question') = ?3
        AND json_extract(data, '$.choice.option') = 'approve')", (&gate.blocker, HUMAN, &gate.question),
        |r| r.get(0)).map_err(sql)?;
    Ok((!owner).then_some("the owner's sign-off on the decision card"))
}

pub(super) fn approved(tx: &Transaction, key: &str, role: &str, tier: u8) -> Result<bool, String> {
    tx.query_row("SELECT EXISTS (SELECT 1 FROM supervisor_design_records WHERE task = ?1 AND role = ?2 AND tier >= ?3 AND verdict = 'approved')",
        (key, role, tier), |r| r.get(0)).map_err(sql)
}

#[derive(Deserialize, Serialize)]
pub(super) struct Brief {
    design_note: String,
    migration: String,
    rollback: String,
    #[serde(default)]
    paths: String,
}

fn prose(text: &str, field: &str) -> Result<String, String> {
    let clean = crate::plans::rules::clean(text);
    let clean = clean.trim();
    if clean.is_empty() || clean.len() > 1600 {
        return Err(format!("Give {field} in 1–1600 bytes."));
    }
    Ok(clean.to_owned())
}

pub(super) fn brief(tx: &Transaction, task: &Task, by: &str, draft: Brief, at: i64) -> Result<(), String> {
    let record = Brief { design_note: prose(&draft.design_note, "the design note")?,
        migration: prose(&draft.migration, "the migration plan")?, rollback: prose(&draft.rollback, "the rollback plan")?,
        paths: if draft.paths.trim().is_empty() { String::new() } else { prose(&draft.paths, "the affected paths")? },
    };
    let gate = get(tx, &task.key)?.filter(|gate| gate.project == task.project).ok_or("The Supervisor has not marked this task for review.")?;
    let encoded = serde_json::to_string(&record).map_err(|e| e.to_string())?;
    if gate.brief.as_deref() == Some(&encoded) { return Ok(()); }
    if gate.brief.is_some() {
        return Err("This task already has a design under review. Open a new task for a different design.".into());
    }
    if gate.tier == 3 && record.paths.trim().is_empty() {
        return Err("Give the repository-relative paths covered by this T3 design.".into());
    }
    let paths: Vec<_> = record.paths.lines().map(str::trim).filter(|path| !path.is_empty()).map(|path|
        super::surfaces::Change { path: path.to_owned(), added: false }).collect();
    if paths.iter().any(|change| change.path.split('/').any(|part| part.is_empty() || part == "." || part == "..")
        || change.path.contains(['*', '?'])) {
        return Err("Use exact repository-relative paths, one per line.".into());
    }
    let map = settings::read(tx, &task.project)?.settings.charter.surfaces;
    let existing = gate.existing_review || map.classify(&paths).is_some_and(|hit|
        hit.tier.number() == gate.tier && hit.existing_review);
    tx.execute("UPDATE supervisor_design_gates SET brief = ?2, existing_review = ?3,
        deadline = CASE WHEN deadline = 0 THEN ?4 ELSE deadline END WHERE task = ?1",
        (&task.key, encoded, existing, at + TIME_LIMIT)).map_err(sql)?;
    ops::comment(tx, by, &task.id, &format!("Design note

{}

Migration

{}

Rollback

{}",
        record.design_note, record.migration, record.rollback), at)?;
    if gate.status == "waiting" { block(tx, &gate, task, at)?; }
    Ok(())
}

#[derive(Deserialize, Serialize)]
pub(super) struct Record {
    pub role: String,
    verdict: String,
    compatibility: String,
    rejected_advice: String,
}

pub(super) fn covers(tx: &Transaction, gate: &Gate, tier: u8, paths: &[String]) -> Result<bool, String> {
    let stored = ops::rows(tx, "SELECT scope FROM supervisor_design_records WHERE task = ?1
        AND tier >= ?2 AND verdict = 'approved' AND (role = 'architecture' OR ?2 = 3)",
        (&gate.key, tier), |r| r.get::<_, String>(0))?;
    for raw in stored {
        let allowed: BTreeSet<String> = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        if paths.iter().any(|path| !allowed.contains(path)) { return Ok(false); }
    }
    Ok(true)
}

pub(super) fn record(tx: &Transaction, task: &Task, by: &str, mut draft: Record, at: i64) -> Result<(), String> {
    let gate = get(tx, &task.key)?.filter(|gate| gate.project == task.project).ok_or("The Supervisor has not marked this task for review.")?;
    if !matches!(draft.role.as_str(), "architecture" | "independent") || (draft.role == "independent" && gate.tier != 3) {
        return Err("Choose architecture or, for T3, independent.".into());
    }
    if !matches!(draft.verdict.as_str(), "approved" | "changes_requested") {
        return Err("Choose verdict approved or changes_requested.".into());
    }
    if gate.question.is_some() || gate.status == "closed" {
        return Err("This review is complete. Open a new task for a different design.".into());
    }
    let other: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM supervisor_design_records
        WHERE task = ?1 AND role <> ?2 AND author = ?3)", (&task.key, &draft.role, by), |r| r.get(0)).map_err(sql)?;
    if other || task.owner == by { return Err("Use a different reviewer from the implementer and the other critic.".into()); }
    if draft.verdict == "approved" && gate.tier == 3 && gate.brief.is_none() {
        return Err("Record the T3 design note, migration and rollback plan first.".into());
    }
    draft.compatibility = prose(&draft.compatibility, "the compatibility statement")?;
    draft.rejected_advice = prose(&draft.rejected_advice, "rejected advice and reasons, or None")?;
    if draft.rejected_advice != "None" && !draft.rejected_advice.lines().all(|line|
        line.split_once('|').is_some_and(|(advice, why)| !advice.trim().is_empty() && !why.trim().is_empty())) {
        return Err("Write each rejected suggestion as advice | reason, or enter None.".into());
    }
    let scope = match &gate.brief {
        Some(raw) if gate.tier == 3 => {
            let brief: Brief = serde_json::from_str(raw).map_err(|e| e.to_string())?;
            serde_json::to_string(&brief.paths.lines().map(str::trim).filter(|path| !path.is_empty()).collect::<BTreeSet<_>>())
                .map_err(|e| e.to_string())?
        }
        _ => gate.scope,
    };
    let encoded = serde_json::to_string(&draft).map_err(|e| e.to_string())?;
    let changed = tx.execute("INSERT INTO supervisor_design_records (task, role, author, verdict, record, at, tier, scope)
        VALUES (?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(task, role) DO UPDATE SET author=excluded.author,
        verdict=excluded.verdict, record=excluded.record, at=excluded.at, tier=excluded.tier, scope=excluded.scope
        WHERE supervisor_design_records.author = excluded.author OR supervisor_design_records.tier < excluded.tier",
        (&task.key, &draft.role, by, &draft.verdict, encoded, at, gate.tier, scope)).map_err(sql)?;
    if changed != 1 { return Err("The original critic must update their review record.".into()); }
    ops::comment(tx, by, &task.id, &format!("Design review ({}): {}.\n\nCompatibility: {}\n\nRejected advice and reasons: {}",
        draft.role, draft.verdict, draft.compatibility, draft.rejected_advice), at)?;
    Ok(())
}

pub(crate) fn task_tool(tx: &Transaction, task: &Task, who: &crate::tasks::tools::Who, tool: &str,
    value: &Value, at: i64) -> Result<String, String> {
    if who.project.as_deref() != Some(&task.project) || !super::register::advisor_enabled(tx, &task.project)? {
        return Err("The task needs a Supervisor design gate in this session's project.".into());
    }
    if task.archived_at.is_some() || task.status == "DONE" { return Err("The task is closed.".into()); }
    if tool == BRIEF {
        let draft = serde_json::from_value(value.clone()).map_err(|_| "Supply the design note, migration and rollback fields.")?;
        brief(tx, task, &crate::tasks::tools::author(tx, who)?, draft, at)?;
        return Ok("Design brief recorded.".into());
    }
    if task.session.as_deref() == Some(&who.key) {
        return Err("A reviewer must be independent from the task's session.".into());
    }
    let draft = serde_json::from_value(value.clone()).map_err(|_| "Supply the role, verdict, compatibility and rejected advice fields.")?;
    let by = crate::tasks::tools::author(tx, who)?;
    record(tx, task, &by, draft, at)?;
    Ok("Design review recorded on the task.".into())
}
