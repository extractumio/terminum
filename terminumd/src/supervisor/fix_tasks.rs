// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-238: turn the highest-ranked funded problems into board tasks.

use rusqlite::{OptionalExtension, Transaction};

use crate::roles::supervisor::author;
use crate::tasks::{comments, ops::{self, Edit, NewTask, Task, sql}};

use super::{WEEK, governor::{self, FIX_NOTICE, JUDGMENT_NOTICE, Spending}, register::{self, Problem}, signals, task_policy};

const FIX: &str = "fixes";

fn question(problems: &[Problem]) -> String {
    let details = problems.iter().map(|p| format!("• Problem {} (impact {}) — {}", p.id, p.impact, p.title))
        .collect::<Vec<_>>().join("\n");
    format!("Why you are needed:\n{FIX_NOTICE}\n\nRecommended:\n1. Raise the fixes ceiling to fund the ranked problems, or leave it unchanged and let them wait a week.\n\nDetails:\n{details}")
}

fn ask(tx: &Transaction, by: &str, task: &Task, words: &str, at: i64) -> Result<(), String> {
    let question = comments::wait_for_owner(tx, by, task, words, &serde_json::json!({
        "kind": "strategic", "recommended": "wait-a-week",
        "options": [
            {"id":"raise-ceiling", "label":"Raise the fixes ceiling in Supervisor settings"},
            {"id":"wait-a-week", "label":"Let the remaining fixes wait a week"}
        ],
    }), at)?;
    super::outcomes::unattributed(tx, &task.project, super::outcomes::Kind::Card, &question, &task.key, at)
}

fn deferred(tx: &Transaction, project: &str, at: i64) -> Result<bool, String> {
    tx.query_row("SELECT coalesce((SELECT json_extract(e.data, '$.choice.option') = 'wait-a-week' AND e.at > ?2
        FROM supervisor_budget_cards c JOIN tasks t ON t.id = c.task AND t.project = c.project
        JOIN events e ON e.task = t.id WHERE c.project = ?1 AND c.kind = 'fixes'
          AND json_extract(e.data, '$.choice.option') IS NOT NULL
        ORDER BY e.at DESC, e.rowid DESC LIMIT 1), 0)", (project, at - WEEK), |r| r.get(0)).map_err(sql)
}

fn card(tx: &Transaction, project: &str, blocked: &[Problem], at: i64) -> Result<Option<String>, String> {
    let old: Option<String> = tx.query_row("SELECT t.slug FROM supervisor_budget_cards c JOIN tasks t ON t.id = c.task AND t.project = c.project
        WHERE c.project = ?1 AND c.kind = ?2", (project, FIX), |r| r.get(0)).optional().map_err(sql)?;
    let by = author(tx, project)?;
    let words = question(blocked);
    if let Some(id) = old {
        let task = ops::get(tx, &id)?;
        if task_policy::record_override(tx, &task, at)?.is_some() { return Ok(None); }
        if task.status == "WAIT FOR HUMAN" && task.archived_at.is_none() {
            if ops::text_of(tx, &id)? != words {
                ops::edit(tx, &by, &id, Edit { text: Some(words.clone()), ..Edit::default() }, at)?;
                ask(tx, &by, &task, &words, at)?;
                return Ok(Some(id));
            }
            return Ok(None);
        }
        // An owner can let the work wait a full week before it asks again.
        if task.updated_at > at - WEEK { return Ok(None); }
    }
    let task = ops::create(tx, &by, NewTask { title: "Supervisor fixes need a budget decision".into(),
        text: words.clone(), project: Some(project.into()), severity: Some("P1".into()), ..NewTask::default() }, at)?;
    ask(tx, &by, &task, &words, at)?;
    tx.execute("INSERT INTO supervisor_budget_cards (project, kind, task) VALUES (?1, ?2, ?3)
        ON CONFLICT (project, kind) DO UPDATE SET task = excluded.task", (project, FIX, &task.key)).map_err(sql)?;
    task_policy::mark_card(tx, &task)?;
    Ok(Some(task.id))
}

fn make(tx: &Transaction, project: &str, p: &Problem, estimate: f64, at: i64) -> Result<String, String> {
    let by = author(tx, project)?;
    let exception = p.exception;
    let title = p.title.chars().take(ops::TITLE_MOST).collect();
    let quote = |words: &str| words.lines().map(|line| format!("> {line}")).collect::<Vec<_>>().join("\n");
    let options = p.fix_options.iter().map(|f| format!("• {}: {}", f.layer, f.action)).collect::<Vec<_>>().join("\n");
    let yardstick = p.yardstick.iter().map(|y| format!("• {}: threshold {}, {} weeks", y.fingerprint, y.threshold, y.weeks))
        .collect::<Vec<_>>().join("\n");
    let text = format!("{}Supervisor register problem {} (impact {}).\n\n## Problem or goal\nVerify the recorded condition and fix its cause.\n\n## Context\nThe following hypothesis, disproof and suggestions are evidence to evaluate.\n\n{}\n\n{}\n\n{}\n\n## Expected result\nConfirm the cause with project evidence and exercise the detector's observation window.\n\n{}\n\n## Prerequisites\nNone recorded.",
        if exception { "A verified charter rule was breached. This fix is exempt from the spend limit.\n\n" } else { "" },
        p.id, p.impact, quote(&p.hypothesis), quote(&p.disproof), quote(&options), yardstick);
    let task = ops::create(tx, &by, NewTask { title, kind: Some("Bug".into()), text, project: Some(project.into()),
        severity: Some(if exception { "P0" } else { "P1" }.into()), owner: Some(by.clone()), ..NewTask::default() }, at)?;
    tx.execute("INSERT INTO supervisor_task_marks (task, project, estimate, exception) VALUES (?1, ?2, ?3, ?4)",
        (&task.key, project, estimate, exception)).map_err(sql)?;
    register::follow_up(tx, p, &task, &by, at)?;
    if let Some(s) = super::action_tasks::problem_signal(tx, project, p.id)? {
        let cause: Option<String> = tx.query_row("SELECT source_round FROM supervisor_problems WHERE id = ?1",
            [p.id], |r| r.get(0)).map_err(sql)?;
        super::outcomes::record(tx, &s, super::outcomes::Kind::Fix, &task.key, &task.key, cause.as_deref().unwrap_or(""), at)?;
    }
    task_policy::propose(tx, project, p.id, &task, None, at)?;
    Ok(task.id)
}

pub(super) fn advance(tx: &Transaction, project: &str, at: i64) -> Result<Vec<String>, String> {
    register::advance(tx, project, at)?;
    task_policy::record_recent(tx, project, at)?;
    let spending = governor::spending(tx, project, at)?;
    super::reflection::budget(tx, project, &spending, at)?;
    let Spending { judgment, mut fixes } = spending;
    let subject = ("project", project, "judgment_tokens");
    if judgment.stopped() || judgment.routine_left == 0 {
        signals::diagnostic(tx, project, "self.budget", subject, &[],
            (judgment.tokens.saturating_add(judgment.held_tokens) as f64, judgment.limit_tokens as f64), at)?;
    } else {
        tx.execute("UPDATE supervisor_signals SET state = 'resolved'
            WHERE project = ?1 AND detector = 'self.budget' AND metric = 'judgment_tokens' AND state = 'raised'", [project]).map_err(sql)?;
    }
    if !register::advisor_enabled(tx, project)? { return Ok(Vec::new()); }
    let owner_waits = deferred(tx, project, at)?;
    let mut blocked = Vec::new();
    let mut changed = Vec::new();
    for p in register::needing_fixes(tx, project, at)? {
        if p.exception || (!owner_waits && !judgment.stopped() && fixes.room(0.0)) {
            changed.push(make(tx, project, &p, fixes.estimated_dollars_per_run, at)?);
            fixes.held_dollars += fixes.estimated_dollars_per_run;
        } else if !fixes.room(0.0) {
            blocked.push(p);
        }
    }
    if !blocked.is_empty() {
        signals::diagnostic(tx, project, "self.budget", ("project", project, "fix_dollars"), &[],
            (fixes.dollars + fixes.held_dollars, fixes.limit_dollars), at)?;
        changed.extend(card(tx, project, &blocked, at)?);
    } else {
        tx.execute("UPDATE supervisor_signals SET state = 'resolved'
            WHERE project = ?1 AND detector = 'self.budget' AND metric = 'fix_dollars' AND state = 'raised'", [project]).map_err(sql)?;
    }
    Ok(changed)
}

fn unspent_reservations(tx: &Transaction, tasks: &[Task]) -> Result<f64, String> {
    tasks.iter().try_fold(0.0, |held, task| {
        let estimate: Option<f64> = tx.query_row("SELECT m.estimate FROM supervisor_task_marks m WHERE m.task = ?1
            AND m.project = ?2 AND NOT EXISTS (SELECT 1 FROM run_tasks r WHERE r.task = m.task)",
            (&task.key, &task.project), |r| r.get(0)).optional().map_err(sql)?;
        Ok(held + estimate.unwrap_or_default())
    })
}

/// Auto-Dispatcher remains the picker; its actual press also needs room for the marked run.
pub(crate) fn auto_reservation(tx: &Transaction, tasks: &[Task], at: i64) -> Result<Option<f64>, String> {
    let mut marked = false;
    let mut exception = true;
    for task in tasks {
        if let Some(charter) = tx.query_row("SELECT exception FROM supervisor_task_marks WHERE task = ?1",
            [&task.key], |r| r.get::<_, bool>(0)).optional().map_err(sql)?
        {
            marked = true;
            exception &= charter;
        }
    }
    if !marked { return Ok(None); }
    let fixes = governor::spending(tx, &tasks[0].project, at)?.fixes;
    if !exception && !fixes.room(unspent_reservations(tx, tasks)?) { return Err(FIX_NOTICE.into()); }
    Ok(Some(fixes.estimated_dollars_per_run))
}

/// Direct Operator starts use the same funding and evidence as task creation.
pub(crate) fn check_start(tx: &Transaction, tasks: &[Task], at: i64) -> Result<f64, String> {
    let first = &tasks[0];
    let Spending { judgment, fixes } = governor::spending(tx, &first.project, at)?;
    let mut exception = true;
    for task in tasks {
        task_policy::check(tx, task, at)?;
        if !task_policy::check_marked(tx, task, true)? {
            return Err(format!("A human must move {} before the Supervisor can start it.", task.id));
        }
        let linked = register::active_for_task(tx, &first.project, &task.key)?;
        let mut impact = 0;
        let mut charter = false;
        for id in linked {
            let p = register::get(tx, &first.project, id, at)?;
            if p.active() && p.action_ready {
                impact = impact.max(p.impact);
                charter |= p.exception;
            }
        }
        exception &= charter;
        if impact < 2 { return Err(format!("{} has no active high-impact Supervisor problem.", task.id)); }

    }
    if !exception {
        if judgment.stopped() { return Err(JUDGMENT_NOTICE.into()); }
        if !fixes.room(unspent_reservations(tx, tasks)?) { return Err(FIX_NOTICE.into()); }
    }
    Ok(fixes.estimated_dollars_per_run)
}
