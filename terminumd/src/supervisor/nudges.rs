// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-241: one ordinary task comment for each qualifying signal and task.

use rusqlite::{OptionalExtension, Transaction};

use crate::dispatcher::runs::STILL;
use crate::tasks::{comments, ops::{rows, sql}};

use super::{Signal, signals::{self, State}};

fn tasks(tx: &Transaction, s: &Signal) -> Result<Vec<(String, String)>, String> {
    let subject = match (s.subject_kind.as_str(), s.detector.as_str()) {
        ("session", "drift.loop" | "drift.subagents") => s.subject_id.split_once('/').map(|(_, id)| id),
        ("session" | "task", _) => Some(s.subject_id.as_str()),
        _ => None,
    };
    let Some(subject) = subject else { return Ok(Vec::new()) };
    rows(tx, &format!("SELECT t.id, t.slug FROM tasks t WHERE t.project = ?1 AND t.archived_at IS NULL
        AND t.status NOT IN ('DONE', 'FAILED') AND (
            (?2 = 'task' AND (t.id = ?3 OR t.slug = ?3)) OR (?2 = 'session' AND EXISTS (
                SELECT 1 FROM runs r JOIN run_tasks rt ON rt.run = r.id WHERE rt.task = t.id AND r.project = ?1
                  AND {STILL} AND (r.pane_key = ?3 OR r.agent_session = ?3) AND r.pane_key = t.session_key)))"),
        (&s.project, &s.subject_kind, subject), |r| Ok((r.get(0)?, r.get(1)?)))
}

fn comment(tx: &Transaction, s: &Signal, at: i64) -> Result<Vec<(String, Option<String>)>, String> {
    if s.level == 0 || !matches!(s.state, State::New | State::Raised | State::Recurred | State::Held) {
        return Ok(Vec::new());
    }
    let problem: Option<(i64, String)> = tx.query_row("SELECT p.id, p.hypothesis
        FROM supervisor_problem_signals y JOIN supervisor_problems p ON p.id = y.problem
        WHERE y.fingerprint = ?1 AND p.project = ?2 AND p.state IN ('confirmed', 'fix_in_work', 'watching', 'recurred')",
        (&s.fingerprint, &s.project), |r| Ok((r.get(0)?, r.get(1)?))).optional().map_err(sql)?;
    let drift = matches!(s.detector.as_str(), "drift.loop" | "drift.diff_outlier" | "drift.stale_plan");
    let hold = s.state == State::Held;
    if !hold && !drift && problem.is_none() { return Ok(Vec::new()); }
    let first = if hold { "Supervisor held this work." }
        else if problem.is_some() { "Supervisor identified a known problem on this task." }
        else { "Supervisor detected drift in this session." };
    let mut words = format!("{first}\n\n{} measured {} against a baseline of {} (threshold {}). Signal `{}`.",
        s.metric, s.measure.value, s.measure.baseline, s.measure.threshold, s.fingerprint);
    if let Some((id, cause)) = problem {
        words.push_str(&format!("\n\nRegister problem {id}. Recorded hypothesis: {cause}"));
    }
    if drift { words.push_str("\n\nReassess the work and ask the owner if a decision is needed."); }
    if hold { words.push_str("\n\nThe hold can be undone from its Supervisor controls."); }

    let mut added = Vec::new();
    for (key, task) in tasks(tx, s)? {
        let current: Option<String> = tx.query_row("SELECT event FROM supervisor_nudges WHERE fingerprint = ?1 AND task = ?2",
            (&s.fingerprint, &key), |r| r.get(0)).optional().map_err(sql)?;
        if let Some(event) = current {
            let changed = tx.execute("UPDATE events SET text = ?2, edited_at = ?3 WHERE id = ?1 AND text <> ?2",
                (&event, &words, at)).map_err(sql)?;
            if changed != 0 { added.push((task, None)); }
        } else {
            let author = crate::roles::supervisor::author(tx, &s.project)?;
            let event = comments::comment(tx, &author, &task, &words, at)?;
            tx.execute("INSERT INTO supervisor_nudges VALUES (?1, ?2, ?3)", (&s.fingerprint, &key, &event)).map_err(sql)?;
            super::outcomes::record(tx, s, super::outcomes::Kind::Nudge, &event, &key, "", at)?;
            added.push((task, Some(event)));
        }
    }
    Ok(added)
}

pub(super) fn collected(tx: &Transaction, project: &str, evaluation: i64, at: i64) -> Result<Vec<(String, Option<String>)>, String> {
    let ids = rows(tx, "SELECT s.fingerprint FROM supervisor_signals s WHERE s.project = ?1 AND s.last_seen = ?2
        AND coalesce(max(json_extract(s.scores, '$.impact'), json_extract(s.scores, '$.blast'),
            json_extract(s.scores, '$.reversibility')), 0) >= 1
        AND (s.detector IN ('drift.loop','drift.diff_outlier','drift.stale_plan') OR s.state = 'held'
            OR EXISTS (SELECT 1 FROM supervisor_problem_signals y WHERE y.fingerprint = s.fingerprint))",
        (project, evaluation), |r| r.get::<_, String>(0))?;
    let mut added = Vec::new();
    if ids.is_empty() { return Ok(added); }
    let settings = super::settings::read(tx, project)?.settings;
    for id in ids {
        let signal = signals::get(tx, project, &id)?;
        if super::Rule::named(&signal.detector).is_ok_and(|rule| settings.detector(rule).mode == super::Mode::Digest) {
            continue;
        }
        added.extend(comment(tx, &signal, at)?);
    }
    Ok(added)
}
