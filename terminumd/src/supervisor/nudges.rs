// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-241: one ordinary task comment for each qualifying signal and task.

use rusqlite::{OptionalExtension, Transaction};

use crate::dispatcher::runs::STILL;
use crate::tasks::{comments, ops::{self, rows, sql}};

use super::{Signal, holds::{self, Hold}, signals::{self, State}};

/// Tasks to tell after commit, each with the comment whose notice goes to its linked agent, if any (AC-241).
pub(crate) type Nudges = Vec<(String, Option<String>)>;

/// A signal's tasks: its subject's open tasks, then any task one of its holds in effect names, whatever its status.
fn tasks(tx: &Transaction, s: &Signal, held: &[Hold]) -> Result<Vec<(String, String)>, String> {
    let subject = match (s.subject_kind.as_str(), s.detector.as_str()) {
        ("session", "drift.loop" | "drift.subagents") => s.subject_id.split_once('/').map(|(_, id)| id),
        ("session" | "task", _) => Some(s.subject_id.as_str()),
        _ => None,
    };
    let mut found = match subject {
        None => Vec::new(),
        Some(subject) => rows(tx, &format!("SELECT t.id, t.slug FROM tasks t WHERE t.project = ?1 AND t.archived_at IS NULL
            AND t.status NOT IN ('DONE', 'FAILED') AND (
                (?2 = 'task' AND (t.id = ?3 OR t.slug = ?3)) OR (?2 = 'session' AND EXISTS (
                    SELECT 1 FROM runs r JOIN run_tasks rt ON rt.run = r.id WHERE rt.task = t.id AND r.project = ?1
                      AND {STILL} AND (r.pane_key = ?3 OR r.agent_session = ?3) AND r.pane_key = t.session_key)))"),
            (&s.project, &s.subject_kind, subject), |r| Ok((r.get(0)?, r.get(1)?)))?,
    };
    for slug in held.iter().filter_map(|h| h.task.as_ref()) {
        if !found.iter().any(|(_, known)| known == slug) { found.push((ops::key_of(tx, slug)?, slug.clone())); }
    }
    Ok(found)
}

/// The comment's words for one task, or none when nothing on it is nudge-worthy.
fn words(s: &Signal, problem: Option<&(i64, String)>, held: &[&Hold]) -> Option<String> {
    let drift = matches!(s.detector.as_str(), "drift.loop" | "drift.diff_outlier" | "drift.stale_plan");
    if held.is_empty() && !drift && problem.is_none() { return None; }
    let first = if !held.is_empty() {
        let labels: Vec<_> = held.iter().map(|h| h.label).collect();
        format!("Supervisor held this work: {}.", labels.join("; "))
    } else if problem.is_some() { "Supervisor identified a known problem on this task.".into() }
    else { "Supervisor detected drift in this session.".into() };
    let mut words = format!("{first}\n\n{} measured {} against a baseline of {} (threshold {}). Signal `{}`.",
        s.metric, s.measure.value, s.measure.baseline, s.measure.threshold, s.fingerprint);
    if let Some((id, cause)) = problem {
        words.push_str(&format!("\n\nRegister problem {id}. Recorded hypothesis: {cause}"));
    }
    if drift { words.push_str("\n\nReassess the work and ask the owner if a decision is needed."); }
    for h in held {
        words.push_str(&format!("\n\nUndo hold {}: {}.", h.id, h.undo));
    }
    Some(words)
}

/// Writes or updates each task's one comment. An added comment's notice always goes; an updated one's only when
/// `updates` says so, as for a hold, which is news; a repeat's update is not (AC-241).
fn comment(tx: &Transaction, s: &Signal, updates: bool, at: i64) -> Result<Nudges, String> {
    if s.level == 0 || !matches!(s.state, State::New | State::Raised | State::Recurred) {
        return Ok(Vec::new());
    }
    let problem: Option<(i64, String)> = tx.query_row("SELECT p.id, p.hypothesis
        FROM supervisor_problem_signals y JOIN supervisor_problems p ON p.id = y.problem
        WHERE y.fingerprint = ?1 AND p.project = ?2 AND p.state IN ('confirmed', 'fix_in_work', 'watching', 'recurred')",
        (&s.fingerprint, &s.project), |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(sql)?;
    let held: Vec<Hold> = holds::list(tx, &s.project)?.into_iter().filter(|h| h.signal == s.fingerprint).collect();
    let mut written = Vec::new();
    for (key, task) in tasks(tx, s, &held)? {
        let on: Vec<&Hold> = held.iter().filter(|h| h.task.is_none() || h.task.as_deref() == Some(&task)).collect();
        let Some(words) = words(s, problem.as_ref(), &on) else { continue };
        let current: Option<String> = tx.query_row("SELECT event FROM supervisor_nudges WHERE fingerprint = ?1 AND task = ?2",
            (&s.fingerprint, &key), |r| r.get(0)).optional().map_err(sql)?;
        if let Some(event) = current {
            let changed = tx.execute("UPDATE events SET text = ?2, edited_at = ?3 WHERE id = ?1 AND text <> ?2",
                (&event, &words, at)).map_err(sql)?;
            if changed != 0 { written.push((task, updates.then_some(event))); }
        } else {
            let author = crate::roles::supervisor::author(tx, &s.project)?;
            let event = comments::comment(tx, &author, &task, &words, at)?;
            tx.execute("INSERT INTO supervisor_nudges VALUES (?1, ?2, ?3)", (&s.fingerprint, &key, &event)).map_err(sql)?;
            super::outcomes::record(tx, s, super::outcomes::Kind::Nudge, &event, &key, "", at)?;
            written.push((task, Some(event)));
        }
    }
    Ok(written)
}

/// An applied hold's comments (AC-241): its signal's one comment on each task, added or updated, with its notice.
pub(super) fn held(tx: &Transaction, s: &Signal, at: i64) -> Result<Nudges, String> {
    comment(tx, s, true, at)
}

pub(super) fn collected(tx: &Transaction, project: &str, evaluation: i64, at: i64) -> Result<Nudges, String> {
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
        added.extend(comment(tx, &signal, false, at)?);
    }
    Ok(added)
}
