// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-241: one ordinary task comment for each qualifying signal and task.

use rusqlite::{OptionalExtension, Transaction};

use crate::dispatcher::runs::STILL;
use crate::tasks::{comments, ops::{rows, sql}};

use super::{Signal, holds::Kind, signals::{self, State}};

/// A signal's tasks: its subject's open tasks, and any task one of its holds in effect names, whatever its status.
fn tasks(tx: &Transaction, s: &Signal) -> Result<Vec<(String, String)>, String> {
    let subject = match (s.subject_kind.as_str(), s.detector.as_str()) {
        ("session", "drift.loop" | "drift.subagents") => s.subject_id.split_once('/').map(|(_, id)| id),
        ("session" | "task", _) => Some(s.subject_id.as_str()),
        _ => None,
    };
    rows(tx, &format!("SELECT t.id, t.slug FROM tasks t WHERE t.project = ?1 AND t.archived_at IS NULL AND ((
            t.status NOT IN ('DONE', 'FAILED') AND (
            (?2 = 'task' AND (t.id = ?3 OR t.slug = ?3)) OR (?2 = 'session' AND EXISTS (
                SELECT 1 FROM runs r JOIN run_tasks rt ON rt.run = r.id WHERE rt.task = t.id AND r.project = ?1
                  AND {STILL} AND (r.pane_key = ?3 OR r.agent_session = ?3) AND r.pane_key = t.session_key))))
            OR EXISTS (SELECT 1 FROM supervisor_holds h WHERE h.fingerprint = ?4 AND h.target = t.id
                AND h.shadow = 0 AND h.released_at IS NULL))"),
        (&s.project, &s.subject_kind, subject, &s.fingerprint), |r| Ok((r.get(0)?, r.get(1)?)))
}

/// The signal's holds in effect on this task: its own and the project-wide ones.
fn holds(tx: &Transaction, s: &Signal, key: &str) -> Result<Vec<(i64, Kind)>, String> {
    rows(tx, "SELECT id, kind FROM supervisor_holds WHERE project = ?1 AND fingerprint = ?2 AND shadow = 0
        AND released_at IS NULL AND (target = ?3 OR target = '') ORDER BY id", (&s.project, &s.fingerprint, key),
        |r| Ok((r.get(0)?, Kind::read(&r.get::<_, String>(1)?))))
}

/// The comment's words for one task, or none when nothing on it is nudge-worthy.
fn words(tx: &Transaction, s: &Signal, problem: Option<&(i64, String)>, key: &str) -> Result<Option<String>, String> {
    let drift = matches!(s.detector.as_str(), "drift.loop" | "drift.diff_outlier" | "drift.stale_plan");
    let held = holds(tx, s, key)?;
    if held.is_empty() && !drift && problem.is_none() { return Ok(None); }
    let first = if !held.is_empty() {
        let labels: Vec<_> = held.iter().map(|(_, kind)| kind.label()).collect();
        format!("Supervisor held this work: {}.", labels.join("; "))
    } else if problem.is_some() { "Supervisor identified a known problem on this task.".into() }
    else { "Supervisor detected drift in this session.".into() };
    let mut words = format!("{first}\n\n{} measured {} against a baseline of {} (threshold {}). Signal `{}`.",
        s.metric, s.measure.value, s.measure.baseline, s.measure.threshold, s.fingerprint);
    if let Some((id, cause)) = problem {
        words.push_str(&format!("\n\nRegister problem {id}. Recorded hypothesis: {cause}"));
    }
    if drift { words.push_str("\n\nReassess the work and ask the owner if a decision is needed."); }
    for (id, kind) in held {
        words.push_str(&format!("\n\nUndo hold {id}: {}.", kind.undo()));
    }
    Ok(Some(words))
}

/// Writes or updates each task's one comment: the task and, for a comment it added or whose words changed, the event.
fn comment(tx: &Transaction, s: &Signal, at: i64) -> Result<Vec<(String, String, bool)>, String> {
    if s.level == 0 || !matches!(s.state, State::New | State::Raised | State::Recurred | State::Held) {
        return Ok(Vec::new());
    }
    let problem: Option<(i64, String)> = tx.query_row("SELECT p.id, p.hypothesis
        FROM supervisor_problem_signals y JOIN supervisor_problems p ON p.id = y.problem
        WHERE y.fingerprint = ?1 AND p.project = ?2 AND p.state IN ('confirmed', 'fix_in_work', 'watching', 'recurred')",
        (&s.fingerprint, &s.project), |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(sql)?;
    let mut written = Vec::new();
    for (key, task) in tasks(tx, s)? {
        let Some(words) = words(tx, s, problem.as_ref(), &key)? else { continue };
        let current: Option<String> = tx.query_row("SELECT event FROM supervisor_nudges WHERE fingerprint = ?1 AND task = ?2",
            (&s.fingerprint, &key), |r| r.get(0)).optional().map_err(sql)?;
        if let Some(event) = current {
            let changed = tx.execute("UPDATE events SET text = ?2, edited_at = ?3 WHERE id = ?1 AND text <> ?2",
                (&event, &words, at)).map_err(sql)?;
            if changed != 0 { written.push((task, event, false)); }
        } else {
            let author = crate::roles::supervisor::author(tx, &s.project)?;
            let event = comments::comment(tx, &author, &task, &words, at)?;
            tx.execute("INSERT INTO supervisor_nudges VALUES (?1, ?2, ?3)", (&s.fingerprint, &key, &event)).map_err(sql)?;
            super::outcomes::record(tx, s, super::outcomes::Kind::Nudge, &event, &key, "", at)?;
            written.push((task, event, true));
        }
    }
    Ok(written)
}

/// An applied hold's comments (AC-241): each with its event, whose notice its linked agent hears at its next hook,
/// a comment updated by the hold included. Its signal's comment on a task stays one.
pub(super) fn held(tx: &Transaction, s: &Signal, at: i64) -> Result<Vec<(String, String)>, String> {
    Ok(comment(tx, s, at)?.into_iter().map(|(task, event, _)| (task, event)).collect())
}

pub(super) fn collected(tx: &Transaction, project: &str, evaluation: i64, at: i64) -> Result<Vec<(String, Option<String>)>, String> {
    let ids = rows(tx, "SELECT s.fingerprint FROM supervisor_signals s WHERE s.project = ?1 AND s.last_seen = ?2
        AND coalesce(max(json_extract(s.scores, '$.impact'), json_extract(s.scores, '$.blast'),
            json_extract(s.scores, '$.reversibility')), 0) >= 1
        AND (s.detector IN ('drift.loop','drift.diff_outlier','drift.stale_plan')
            OR EXISTS (SELECT 1 FROM supervisor_holds h WHERE h.fingerprint = s.fingerprint AND h.shadow = 0
                AND h.released_at IS NULL)
            OR EXISTS (SELECT 1 FROM supervisor_problem_signals y WHERE y.fingerprint = s.fingerprint))",
        (project, evaluation), |r| r.get::<_, String>(0))?;
    let mut added = Vec::new();
    if ids.is_empty() { return Ok(added); }
    let saved = super::settings::read(tx, project)?.settings;
    let attach = super::attach::read(tx, project)?;
    let weekly = super::config_lifecycle::on_weekly(tx, project)?;
    for id in ids {
        let signal = signals::get(tx, project, &id)?;
        if let Ok(rule) = super::Rule::named(&signal.detector)
            && super::config_lifecycle::mode(&saved, attach.as_ref(), &saved.detector(rule), &weekly, at) != super::Mode::Live
        {
            continue;
        }
        added.extend(comment(tx, &signal, at)?.into_iter().map(|(task, event, new)| (task, new.then_some(event))));
    }
    Ok(added)
}
