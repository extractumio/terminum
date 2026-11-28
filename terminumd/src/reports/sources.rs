// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Reports terminumd makes itself (terminum-reports.md AC-272): a routine's result when its run made none, the
//! Supervisor's daily digest, and a critical report for each incident it opens.

use rusqlite::Transaction;

use super::record::{self, kind, role, New};
use crate::tasks::ops::{sql, Task};

/// The words a summary takes from a result: its Summary's (terminum-schedules.md AC-162), else its first line that is
/// not its Outcome, else its Outcome's (terminum-dispatcher.md AC-166), cut to fit.
pub fn summary_of(result: &str) -> String {
    let outcome = crate::tasks::closeout::outcome(result);
    let not_outcome = || result.lines().find(|l| !l.trim().is_empty() && !crate::tasks::closeout::matches_heading(l, "Outcome") && Some(*l) != outcome);
    let first = crate::tasks::closeout::section_line(result, "Summary").or_else(not_outcome).or(outcome).unwrap_or_default();
    let one: String = first.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() <= record::SUMMARY_MOST { one } else { format!("{}…", one.chars().take(record::SUMMARY_MOST - 1).collect::<String>()) }
}

/// A result's type: its Outcome's first word when it is ok, attention or failed (a routine's form, AC-162); else a
/// report (AC-265).
pub fn kind_of(result: &str) -> &'static str {
    let word = crate::tasks::closeout::outcome(result).and_then(|o| o.split(|c: char| !c.is_alphanumeric()).find(|w| !w.is_empty())).unwrap_or_default();
    [kind::OK, kind::ATTENTION, kind::FAILED].into_iter().find(|k| word.eq_ignore_ascii_case(k)).unwrap_or(kind::REPORT)
}

/// A report made of a result: its body, summary and type (AC-265, AC-272).
pub fn of_result(result: &str) -> New {
    New { summary: summary_of(result), kind: kind_of(result), body: result.to_string(), ..New::default() }
}

/// A title within its limit.
pub fn titled(title: &str) -> String {
    title.chars().take(record::TITLE_MOST).collect()
}

/// Whether task `t`'s current run `run` made a report for it (by its run, else since its last move), or was started
/// by a human's reply to one, whose answer goes below it or into its follow-up (AC-270). A reply is that run's when
/// it came after the task was last DONE before the run: the reply is kept once RUN took it, and the Dispatcher can
/// move the task to IN PROGRESS later, past the gate.
fn run_reported(tx: &Transaction, t: &Task, run: Option<&str>) -> Result<bool, String> {
    tx.query_row("WITH moves AS (SELECT e.rowid AS n, e.at, json_extract(e.data, '$.to') AS goes FROM events e WHERE e.task = ?1 AND e.kind = 'move'),
        moved AS (SELECT coalesce(max(at), 0) AS at, coalesce(max(n), 0) AS n FROM moves WHERE goes = 'IN PROGRESS'),
        done AS (SELECT coalesce(max(m.at), 0) AS at FROM moves m, moved WHERE m.goes = 'DONE' AND m.n < moved.n)
        SELECT EXISTS (SELECT 1 FROM reports r, moved, done WHERE r.task = ?1 AND (r.run = ?2 OR (?2 IS NULL AND r.created_at >= moved.at) OR r.replied_at >= done.at))",
        (&t.key, run), |r| r.get(0)).map_err(sql)
}

/// A routine's task finished with `result` by `author`: its report, made from the result, unless its run made one
/// itself (AC-272). The report's ID.
pub fn routine_result(tx: &Transaction, t: &Task, result: &str, author: &str, at: i64) -> Result<Option<String>, String> {
    routine_report(tx, t, result, summary_of(result), author, at)
}

/// The Supervisor's weekly review finished: its report is the review as kept, whose decisions stay questions on
/// their tasks, each linked from it (AC-279). The report's ID.
pub fn weekly_review(tx: &Transaction, t: &Task, document: &str, decisions: usize, author: &str, at: i64) -> Result<Option<String>, String> {
    let summary = match decisions {
        0 => "No decision is waiting for you.".to_string(),
        1 => "1 decision is waiting for you.".to_string(),
        n => format!("{n} decisions are waiting for you."),
    };
    routine_report(tx, t, document, summary, author, at)
}

fn routine_report(tx: &Transaction, t: &Task, body: &str, summary: String, author: &str, at: i64) -> Result<Option<String>, String> {
    let Some(s) = crate::tasks::schedules::of_task(tx, &t.key)? else { return Ok(None) };
    let run = crate::dispatcher::runs::last_of(tx, &t.key)?.map(|r| r.id);
    if run_reported(tx, t, run.as_deref())? {
        return Ok(None);
    }
    let new = New { project: t.project.clone(), title: titled(&t.title), summary, author: author.to_string(), role: role::TERMINUM,
        session: t.session.clone(), task: Some(t.key.clone()), schedule: Some(s.id), run, ..of_result(body) };
    Ok(Some(record::create(tx, new, at)?.1))
}

/// The Supervisor's daily digest of `project`, its lines as Markdown (terminum-supervisor.md AC-243).
pub fn digest(tx: &Transaction, project: &str, lines: &[String], at: i64) -> Result<String, String> {
    let date = crate::tasks::cron::stamp(at).chars().take(10).collect::<String>();
    let author = crate::roles::supervisor::author(tx, project)?;
    let new = New { project: project.to_string(), title: format!("Supervisor digest, {date}"), summary: format!("{} new signal{}", lines.len(), if lines.len() == 1 { "" } else { "s" }),
        body: lines.iter().map(|l| format!("- {l}")).collect::<Vec<_>>().join("\n"), author, role: role::TERMINUM, kind: kind::DIGEST, ..New::default() };
    record::create(tx, new, at).map(|(_, id, _)| id)
}

/// The Supervisor opened incident `incident` of `project`: its critical report, which alerts through the incident
/// alone (AC-271). `subject`: what it is about; `why`: its first evidence.
pub fn incident(tx: &Transaction, project: &str, incident: i64, subject: &str, why: &str, at: i64) -> Result<String, String> {
    let author = crate::roles::supervisor::author(tx, project)?;
    let new = New { project: project.to_string(), title: titled(&format!("Incident: {subject}")), summary: summary_of(why),
        body: format!("{why}\n\nIts card: /supervisor/?incident={incident}"), critical: true, author, role: role::TERMINUM, incident: Some(incident), kind: kind::INCIDENT, ..New::default() };
    record::create(tx, new, at).map(|(_, id, _)| id)
}
