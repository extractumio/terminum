// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-236: problem identities, evidence, code-owned yardsticks and owner rulings.

use std::collections::BTreeSet;

use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::tasks::ops::{rows, sql};

use super::{Level, Rule, Signal, WEEK, settings, signals};

const MAX_SIGNALS: usize = 16;
const MAX_TEXT: usize = 1_000;
const YARDSTICK_WEEKS: u8 = 4;
const STATES: [&str; 4] = ["suspected", "confirmed", "fix_in_work", "watching"];

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Fix {
    pub layer: String,
    pub action: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Draft {
    pub title: String,
    pub lens: String,
    pub hypothesis: String,
    pub disproof: String,
    pub fix_options: Vec<Fix>,
    pub confidence: f64,
    pub effort: u8,
    pub signals: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Update {
    pub id: i64,
    pub hypothesis: Option<String>,
    pub disproof: Option<String>,
    pub fix_options: Option<Vec<Fix>>,
    pub confidence: Option<f64>,
    pub effort: Option<u8>,
    pub state: Option<String>,
    pub observation_weeks: Option<u8>,
}

#[derive(Serialize)]
pub(crate) struct Yardstick {
    pub fingerprint: String,
    detector: String,
    pub threshold: f64,
    pub weeks: u8,
    clear_at: Option<i64>,
}

#[derive(Serialize)]
struct Event {
    at: i64,
    state: String,
    detail: String,
}

#[derive(Serialize)]
pub(crate) struct Problem {
    pub id: i64,
    pub title: String,
    lens: String,
    pub hypothesis: String,
    pub disproof: String,
    pub(super) fix_options: Vec<Fix>,
    confidence: f64,
    effort: u8,
    pub state: String,
    pub impact: u8,
    pub exception: bool,
    pub action_ready: bool,
    share: f64,
    indicator: Option<super::impact::Unit>,
    pub rank: f64,
    pub yardstick: Vec<Yardstick>,
    pub tasks: Vec<String>,
    opened_at: i64,
    ruling: Option<String>,
    new_evidence: Option<String>,
    history: Vec<Event>,
}

impl Problem {
    pub(crate) fn active(&self) -> bool {
        self.state != "resolved" && self.state != "accepted"
    }
}

fn check_words(s: &str) -> Result<String, String> {
    let clean = crate::sent::redact(&crate::plans::rules::clean(s));
    let clean = clean.trim();
    if clean.is_empty() || clean.chars().count() > MAX_TEXT {
        return Err("Give problem text between 1 and 1000 characters.".into());
    }
    Ok(clean.into())
}

fn checked_options(mut options: Vec<Fix>) -> Result<String, String> {
    const LAYERS: [&str; 6] = ["check", "test", "hook", "setting", "documentation", "memory"];
    if options.len() > LAYERS.len() { return Err("Give at most six fix options.".into()); }
    for fix in &mut options {
        if !LAYERS.contains(&fix.layer.as_str()) { return Err("Unknown fix layer.".into()); }
        fix.action = check_words(&fix.action)?;
    }
    options.sort_by_key(|fix| LAYERS.iter().position(|layer| *layer == fix.layer));
    serde_json::to_string(&options).map_err(|e| e.to_string())
}

fn check_rank(confidence: f64, effort: u8) -> Result<(), String> {
    if (0.0..=1.0).contains(&confidence) && (1..=4).contains(&effort) { Ok(()) }
    else { Err("Confidence is 0 to 1, and effort class is 1 to 4.".into()) }
}

fn link_signal(tx: &Transaction, problem: i64, signal: &Signal) -> Result<(), String> {
    tx.execute("INSERT INTO supervisor_problem_signals (problem, fingerprint, threshold, weeks)
        VALUES (?1, ?2, ?3, ?4)", (problem, &signal.fingerprint, signal.measure.threshold, YARDSTICK_WEEKS)).map_err(sql)?;
    Ok(())
}

fn event(tx: &Transaction, problem: i64, at: i64, state: &str, detail: &str) -> Result<(), String> {
    tx.execute("INSERT INTO supervisor_problem_events (problem, at, state, detail) VALUES (?1, ?2, ?3, ?4)",
        (problem, at, state, detail)).map_err(sql)?;
    Ok(())
}

pub(super) fn open(tx: &Transaction, project: &str, draft: Draft, at: i64) -> Result<i64, String> {
    let Draft { title, lens, hypothesis, disproof, fix_options, confidence, effort, signals: fingerprints } = draft;
    if fingerprints.is_empty() || fingerprints.len() > MAX_SIGNALS || fingerprints.iter().collect::<BTreeSet<_>>().len() != fingerprints.len() {
        return Err("Give one to sixteen distinct signal fingerprints.".into());
    }
    if !["flow", "architecture", "product", "owner"].contains(&lens.as_str()) { return Err("Unknown problem lens.".into()); }
    check_rank(confidence, effort)?;
    let inferred = fingerprints.iter().map(|key| signals::get(tx, project, key)).collect::<Result<Vec<_>, _>>()?;
    for signal in &inferred {
        if Rule::named(&signal.detector).is_err() {
            return Err("This signal has no detector yardstick.".into());
        }
        if matches!(signal.state, signals::State::Shadow | signals::State::Muted | signals::State::Rejected) {
            return Err("A shadow, muted or rejected signal cannot open a problem.".into());
        }
        if let Some(problem) = tx.query_row("SELECT problem FROM supervisor_problem_signals WHERE fingerprint = ?1",
            [&signal.fingerprint], |r| r.get::<_, i64>(0)).optional().map_err(sql)?
        {
            return Err(format!("Signal {} already belongs to problem {problem}.", signal.fingerprint));
        }
    }
    let id = tx.query_row("INSERT INTO supervisor_problems
        (project, title, lens, hypothesis, disproof, options, confidence, effort, state, opened_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'suspected', ?9) RETURNING id",
        (project, check_words(&title)?, lens, check_words(&hypothesis)?, check_words(&disproof)?,
            checked_options(fix_options)?, confidence, effort, at), |r| r.get(0)).map_err(sql)?;
    for signal in &inferred { link_signal(tx, id, signal)?; }
    event(tx, id, at, "suspected", "Opened from recorded signals.")?;
    Ok(id)
}

pub(super) fn update(tx: &Transaction, project: &str, change: Update, at: i64) -> Result<(), String> {
    let p = get(tx, project, change.id, at)?;
    if !p.active() { return Err("A closed problem needs new evidence before it can change.".into()); }
    let state = change.state.as_deref().unwrap_or(&p.state);
    if state != p.state && !STATES.contains(&state) {
        return Err("Only the yardstick or an owner ruling can close a problem.".into());
    }
    let confidence = change.confidence.unwrap_or(p.confidence);
    let effort = change.effort.unwrap_or(p.effort);
    check_rank(confidence, effort)?;
    if let Some(weeks) = change.observation_weeks {
        if weeks > 52 || p.yardstick.iter().any(|y| weeks < y.weeks) {
            return Err("Observation can be lengthened up to 52 weeks, but never shortened.".into());
        }
        tx.execute("UPDATE supervisor_problem_signals SET weeks = ?2 WHERE problem = ?1", (p.id, weeks)).map_err(sql)?;
    }
    tx.execute("UPDATE supervisor_problems SET hypothesis = ?2, disproof = ?3,
        options = ?4, confidence = ?5, effort = ?6, state = ?7 WHERE id = ?1",
        (p.id, check_words(change.hypothesis.as_deref().unwrap_or(&p.hypothesis))?,
         check_words(change.disproof.as_deref().unwrap_or(&p.disproof))?,
         checked_options(change.fix_options.unwrap_or(p.fix_options))?, confidence, effort, state)).map_err(sql)?;
    event(tx, p.id, at, state, "Updated through a checked Supervisor tool.")
}

pub(super) fn rule(tx: &Transaction, project: &str, id: i64, reason: &str, at: i64) -> Result<(), String> {
    let p = get(tx, project, id, at)?;
    if !p.active() { return Err("This problem is already closed.".into()); }
    let reason = check_words(reason)?;
    tx.execute("UPDATE supervisor_problems SET state = 'accepted', ruling = ?2, new_evidence = NULL WHERE id = ?1",
        (id, &reason)).map_err(sql)?;
    tx.execute("UPDATE supervisor_problem_signals SET ruling_evidence =
        (SELECT evidence FROM supervisor_signals s WHERE s.fingerprint = supervisor_problem_signals.fingerprint)
        WHERE problem = ?1", [id]).map_err(sql)?;
    event(tx, id, at, "accepted", &reason)
}

pub(super) fn require_finding(tx: &Transaction, id: i64, run: Option<&str>) -> Result<(), String> {
    tx.execute("UPDATE supervisor_problems SET automatic = 0, source_round = ?2 WHERE id = ?1", (id, run)).map_err(sql)?;
    Ok(())
}

pub(crate) fn get(tx: &Transaction, project: &str, id: i64, at: i64) -> Result<Problem, String> {
    let mut p = tx.query_row("SELECT title, lens, hypothesis, disproof, options, confidence, effort, state, opened_at, ruling, new_evidence,
            automatic OR EXISTS (SELECT 1 FROM supervisor_rounds q WHERE q.run = source_round AND q.finding IS NOT NULL AND q.failure IS NULL)
        FROM supervisor_problems WHERE project = ?1 AND id = ?2", (project, id), |r| Ok((
            r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get::<_, String>(4)?, r.get(5)?,
            r.get(6)?, r.get(7)?, r.get(8)?, r.get(9)?, r.get(10)?, r.get(11)?))).optional().map_err(sql)?
        .map(|(title, lens, hypothesis, disproof, options, confidence, effort, state, opened_at, ruling, new_evidence, action_ready)| {
            Ok::<_, String>(Problem { id, title, lens, hypothesis, disproof,
                fix_options: serde_json::from_str(&options).map_err(|e| e.to_string())?,
                confidence, effort, state, impact: 0, exception: false, action_ready, share: 0., indicator: None, rank: 0., yardstick: vec![],
                tasks: vec![], opened_at, ruling, new_evidence, history: vec![] })
        }).transpose()?.ok_or("No such problem in this project.")?;
    let signals: Vec<(_, _, signals::RecordedScores)> = rows(tx, "SELECT y.fingerprint, s.detector, y.threshold, y.weeks,
        y.clear_at, s.first_seen, s.scores FROM supervisor_problem_signals y JOIN supervisor_signals s USING (fingerprint)
        WHERE y.problem = ?1 ORDER BY y.fingerprint", [id], |r| Ok((Yardstick { fingerprint: r.get(0)?,
            detector: r.get(1)?, threshold: r.get(2)?, weeks: r.get(3)?, clear_at: r.get(4)? },
            r.get::<_, i64>(5)?, signals::decode(r, 6)?)))?;
    let mut first = p.opened_at;
    for (yardstick, first_seen, recorded) in signals {
        p.yardstick.push(yardstick);
        p.impact = p.impact.max(recorded.scores.impact);
        p.exception |= recorded.charter_breach;
        first = first.min(first_seen);
        // Signal evidence can overlap. Take the strongest measured indicator share without adding it twice.
        for share in recorded.impact_basis.shares {
            if share.fraction() > p.share { p.share = share.fraction(); p.indicator = Some(share.unit); }
        }
    }
    let standing = ((at - first).max(0) as f64 / WEEK as f64).max(1.0);
    p.rank = p.share * standing * p.confidence / f64::from(p.effort);
    p.tasks = rows(tx, "SELECT t.slug FROM supervisor_problem_tasks pt JOIN tasks t ON t.id = pt.task
        WHERE pt.problem = ?1 AND t.project = ?2 ORDER BY t.number", (id, project), |r| r.get(0))?;
    Ok(p)
}

fn sorted(tx: &Transaction, project: &str, at: i64, ids: Vec<i64>) -> Result<Vec<Problem>, String> {
    let mut problems = ids.into_iter().map(|id| get(tx, project, id, at)).collect::<Result<Vec<_>, _>>()?;
    problems.sort_by(|a, b| b.active().cmp(&a.active()).then(b.impact.cmp(&a.impact))
        .then(b.rank.total_cmp(&a.rank)).then(a.id.cmp(&b.id)));
    Ok(problems)
}

pub(crate) fn ranked(tx: &Transaction, project: &str, at: i64, closed: bool) -> Result<Vec<Problem>, String> {
    let ids = rows(tx, "SELECT id FROM supervisor_problems WHERE project = ?1
        AND (?2 OR state NOT IN ('resolved', 'accepted'))", (project, closed), |r| r.get(0))?;
    let mut problems = sorted(tx, project, at, ids)?;
    if closed {
        for p in &mut problems {
            p.history = rows(tx, "SELECT at, state, detail FROM supervisor_problem_events WHERE problem = ?1 ORDER BY id DESC LIMIT 20",
                [p.id], |r| Ok(Event { at: r.get(0)?, state: r.get(1)?, detail: r.get(2)? }))?;
        }
    }
    Ok(problems)
}

pub(super) fn needing_fixes(tx: &Transaction, project: &str, at: i64) -> Result<Vec<Problem>, String> {
    let ids = rows(tx, "SELECT p.id FROM supervisor_problems p WHERE project = ?1
        AND p.state NOT IN ('resolved','accepted','watching') AND p.options <> '[]'
        AND (p.automatic OR EXISTS (SELECT 1 FROM supervisor_rounds q
            WHERE q.run = p.source_round AND q.finding IS NOT NULL AND q.failure IS NULL))
        AND NOT EXISTS (SELECT 1 FROM supervisor_problem_tasks t WHERE t.problem = p.id)
        AND EXISTS (SELECT 1 FROM supervisor_problem_signals y JOIN supervisor_signals s USING (fingerprint)
            WHERE y.problem = p.id AND json_extract(s.scores, '$.impact') >= 2
              AND s.state IN ('new','raised','recurred'))", [project], |r| r.get(0))?;
    sorted(tx, project, at, ids)
}


fn linkable(tx: &Transaction, p: &Problem) -> Result<Vec<String>, String> {
    rows(tx, "SELECT DISTINCT t.id FROM supervisor_problem_signals y
        JOIN supervisor_signals s USING (fingerprint)
        JOIN tasks t ON t.project = s.project AND (t.id = s.subject_id OR t.slug = s.subject_id)
        WHERE y.problem = ?1 AND s.subject_kind = 'task' AND t.archived_at IS NULL
        ORDER BY t.number", [p.id], |r| r.get(0))
}

pub(super) fn relates(tx: &Transaction, p: &Problem, key: &str) -> Result<bool, String> {
    let linked = tx.query_row("SELECT EXISTS(SELECT 1 FROM supervisor_problem_tasks WHERE problem = ?1 AND task = ?2)",
        (p.id, key), |r| r.get::<_, bool>(0)).map_err(sql)?;
    Ok(linked || linkable(tx, p)?.iter().any(|task| task == key))
}

pub(super) fn follow_up(tx: &Transaction, p: &Problem, task: &crate::tasks::ops::Task, author: &str, at: i64) -> Result<(), String> {
    tx.execute("INSERT OR IGNORE INTO supervisor_problem_tasks VALUES (?1, ?2)", (p.id, &task.key)).map_err(sql)?;
    for source in linkable(tx, p)? {
        if source != task.key && super::task_policy::override_of(tx, &source, at)?.is_none() {
            crate::tasks::attrs::link(tx, author, &source, &crate::tasks::attrs::Link {
                list: "follow_ups", task: &task.id, reason: Some("Fix for a Supervisor register problem."), remove: false,
            }, at)?;
        }
    }
    tx.execute("UPDATE supervisor_problems SET state = 'fix_in_work' WHERE id = ?1", [p.id]).map_err(sql)?;
    event(tx, p.id, at, "fix_in_work", &format!("Fix task {} is linked.", task.id))
}

fn seed(tx: &Transaction, project: &str, at: i64) -> Result<(), String> {
    let keys = rows(tx, "SELECT s.fingerprint FROM supervisor_signals s LEFT JOIN supervisor_attach_seeds a USING (fingerprint)
        WHERE s.project = ?1 AND s.state IN ('new','raised','recurred')
          AND (a.fingerprint IS NOT NULL OR json_extract(s.scores, '$.charter_breach') = 1)
          AND NOT EXISTS (SELECT 1 FROM supervisor_problem_signals y WHERE y.fingerprint = s.fingerprint)
          ORDER BY s.first_seen, s.fingerprint", [project], |r| r.get::<_, String>(0))?;
    for key in keys {
        let s = signals::get(tx, project, &key)?;
        let Ok(detector) = Rule::named(&s.detector) else { continue };
        let title = format!("{}: {}", s.metric.replace('_', " "), s.subject_kind.replace('_', " "));
        let lens = s.detector.split('.').next().unwrap_or_default();
        open(tx, project, Draft {
            title, lens: match lens { "product" | "owner" => lens, "drift" => "architecture", _ => "flow" }.into(),
            hypothesis: format!("The condition measured by {} is contributing to this problem.", detector.id()),
            disproof: "The detector remains clear during the observation window without the proposed change.".into(),
            fix_options: vec![Fix { layer: "check".into(), action: "Confirm the cause in the retained evidence and fix the measured condition.".into() }],
            confidence: s.scores.confidence, effort: 1, signals: vec![key],
        }, at)?;
    }
    Ok(())
}

pub(super) fn advance(tx: &Transaction, project: &str, at: i64) -> Result<(), String> {
    seed(tx, project, at)?;
    let changes = rows(tx, "SELECT p.id, p.state,
        (SELECT s.fingerprint FROM supervisor_problem_signals y JOIN supervisor_signals s USING (fingerprint)
        WHERE y.problem = p.id AND s.state IN ('new','raised','recurred')
          AND (p.state = 'resolved' OR (p.state = 'accepted' AND EXISTS (
            SELECT 1 FROM json_each(s.evidence) e WHERE NOT EXISTS (
              SELECT 1 FROM json_each(y.ruling_evidence) old WHERE old.value = e.value))))
        ORDER BY s.fingerprint LIMIT 1),
        EXISTS (SELECT 1 FROM supervisor_problem_signals y WHERE y.problem = p.id)
        AND NOT EXISTS (SELECT 1 FROM supervisor_problem_signals y WHERE y.problem = p.id
            AND (y.clear_at IS NULL OR y.clear_at + y.weeks * ?3 > ?2 OR y.clear_seen <= ?2 - 1800)),
        EXISTS (SELECT 1 FROM supervisor_problem_signals y WHERE y.problem = p.id AND y.clear_at IS NOT NULL)
        AND NOT EXISTS (SELECT 1 FROM supervisor_problem_signals y WHERE y.problem = p.id AND y.clear_at IS NULL)
        FROM supervisor_problems p WHERE p.project = ?1", (project, at, WEEK), |r| Ok((
            r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, bool>(3)?, r.get::<_, bool>(4)?)))?;
    for (id, state, new, cleared, watching) in changes {
        let next = if let Some(ref key) = new {
            Some(("recurred", format!("New evidence from signal {key}.")))
        } else if state == "accepted" || state == "resolved" {
            None
        } else if cleared {
            Some(("resolved", "Every detector met its yardstick for the observation window.".into()))
        } else if watching && state != "watching" {
            Some(("watching", "Every detector is below its firing threshold. The observation window is open.".into()))
        } else if !watching && state == "watching" {
            Some(("recurred", "The observation window no longer has continuous clear evidence.".into()))
        } else { None };
        if let Some((next, detail)) = next {
            tx.execute("UPDATE supervisor_problems SET state = ?2, new_evidence = coalesce(?3, new_evidence) WHERE id = ?1",
                (id, next, new.as_ref().map(|_| &detail))).map_err(sql)?;
            event(tx, id, at, next, &detail)?;
            let fixed = next == "recurred" && tx.query_row("SELECT EXISTS (
                SELECT 1 FROM supervisor_problem_tasks f JOIN tasks t ON t.id = f.task
                WHERE f.problem = ?1 AND t.project = ?2 AND t.status = 'DONE')",
                (id, project), |r| r.get::<_, bool>(0)).map_err(sql)?;
            if fixed {
                let event = tx.last_insert_rowid();
                super::reflection::enqueue(tx, project, ("problem", &format!("{id}@{event}"), new.as_deref()),
                    "fix_recurred", &serde_json::json!({"problem":id,"event":event}), at)?;
            }
        }
    }
    Ok(())
}

pub(crate) fn advisor_enabled(tx: &Transaction, project: &str) -> Result<bool, String> {
    Ok(super::rounds::enabled(tx, project)?.is_some() && settings::level(tx, project)? != Level::Observer)
}

pub(super) fn case_context(tx: &Transaction, project: &str, fingerprint: &str, at: i64) -> Result<Option<Problem>, String> {
    tx.query_row("SELECT problem FROM supervisor_problem_signals WHERE fingerprint = ?1", [fingerprint], |r| r.get(0))
        .optional().map_err(sql)?.map(|id| get(tx, project, id, at)).transpose()
}

/// Live problems already linked to a task, with an explicit rank proposal first.
pub(super) fn active_for_task(tx: &Transaction, project: &str, task: &str) -> Result<Vec<i64>, String> {
    rows(tx, "SELECT p.id FROM (
        SELECT problem, 0 AS place FROM supervisor_rank_proposals WHERE task = ?2
        UNION ALL SELECT problem, 1 FROM supervisor_problem_tasks WHERE task = ?2
        ) l JOIN supervisor_problems p ON p.id = l.problem
        WHERE p.project = ?1 AND p.state NOT IN ('resolved','accepted') GROUP BY p.id ORDER BY min(l.place), p.id",
        (project, task), |r| r.get(0))
}
