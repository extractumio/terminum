// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-236, AC-238: a register problem in words a newcomer acts on: an automatic problem's title, hypothesis
//! and disproof from its detector's measure, and its fix task's text with the measures, weeks, impact share
//! and a link to each evidence row.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use rusqlite::{OptionalExtension, Transaction};

use crate::tasks::ops::sql;

use super::{Evidence, Signal, register::{Fix, Problem}, signals};

/// Evidence rows written into a task; the signal's page lists them all.
const EVIDENCE_MOST: usize = 20;
/// A task ID or project ID kept readable in a link (`TERM-12`).
const ENCODED: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.');

pub(super) fn number(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').into()
}

fn words(s: &str) -> String {
    s.replace('_', " ")
}

/// The detector's name without its lens: `flow.backlog_growth` → "Backlog growth".
fn what(detector: &str) -> String {
    let name = words(detector.split_once('.').map_or(detector, |(_, name)| name));
    let mut chars = name.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

fn task_name(tx: &Transaction, id: &str) -> Result<String, String> {
    Ok(tx.query_row("SELECT coalesce(slug, id) FROM tasks WHERE id = ?1 OR slug = ?1 LIMIT 1", [id], |r| r.get(0))
        .optional().map_err(sql)?.unwrap_or_else(|| id.to_string()))
}

fn task_link(tx: &Transaction, id: &str) -> Result<String, String> {
    let name = task_name(tx, id)?;
    Ok(format!("[{name}](/kanban/{})", utf8_percent_encode(&name, ENCODED)))
}

fn subject(tx: &Transaction, s: &Signal) -> Result<String, String> {
    Ok(match s.subject_kind.as_str() {
        "project" => String::new(),
        "task" => format!(" on {}", task_name(tx, &s.subject_id)?),
        kind => format!(" for {} {}", words(kind), s.subject_id),
    })
}

fn day(tx: &Transaction, at: i64) -> Result<String, String> {
    tx.query_row("SELECT date(?1, 'unixepoch')", [at], |r| r.get(0)).map_err(sql)
}

/// "Backlog growth: 4 growing weeks, threshold 3": what fired, its number and its limit.
pub(super) fn title(tx: &Transaction, s: &Signal) -> Result<String, String> {
    Ok(format!("{}: {} {}{}, threshold {}", what(&s.detector), number(s.measure.value), words(&s.metric),
        subject(tx, s)?, number(s.measure.threshold)))
}

/// A problem code opened from a measure alone: no cause is claimed, and the text says what would find one.
pub(super) fn unexplained(s: &Signal, weeks: u8) -> (String, String, Fix) {
    let (metric, limit) = (words(&s.metric), number(s.measure.threshold));
    (format!("No investigation has run yet: code opened this problem from the {} measure alone, so its cause is unknown. \
        An investigation would read the evidence rows, check whether they share one cause, \
        and find the change that brings {metric} back within the threshold of {limit}.", s.detector),
     format!("{} stays within the threshold of {limit} for {weeks} weeks with no change made.", what(&s.metric)),
     Fix { layer: "check".into(), action: format!("Find the cause in the evidence rows, change what produces it, \
        and keep {metric} within {limit} through the observation window.") })
}

fn row(tx: &Transaction, e: &Evidence) -> Result<String, String> {
    Ok(match e {
        Evidence::Tasks(id) => task_link(tx, id)?,
        Evidence::Events(id) => match tx.query_row("SELECT task, kind, at FROM events WHERE id = ?1", [id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?))).optional().map_err(sql)?
        {
            Some((task, kind, at)) => format!("{}: {} on {} (event {id})", task_link(tx, &task)?, words(&kind), day(tx, at)?),
            None => format!("event {id}"),
        },
        Evidence::Runs(id) => match tx.query_row("SELECT task FROM run_tasks WHERE run = ?1 LIMIT 1", [id],
            |r| r.get::<_, String>(0)).optional().map_err(sql)?
        {
            Some(task) => format!("run {id} of {}", task_link(tx, &task)?),
            None => format!("run {id}"),
        },
        other => {
            let v = serde_json::to_value(other).map_err(|e| e.to_string())?;
            let id = v.get("id").map_or_else(String::new, |id| id.as_str().map_or_else(|| id.to_string(), str::to_string));
            format!("{} {id}", words(v["table"].as_str().unwrap_or_default()))
        }
    })
}

/// The measure against its baseline, the weeks it covers, and a link to every evidence row.
fn measured(tx: &Transaction, project: &str, s: &Signal) -> Result<String, String> {
    let link = format!("/supervisor/?project={}&signal={fp}#signal-{fp}",
        utf8_percent_encode(project, ENCODED), fp = s.fingerprint);
    let m = &s.measure;
    let against = if m.baseline == m.threshold { format!("its limit of {}", number(m.threshold)) }
        else { format!("a baseline of {} and a threshold of {}", number(m.baseline), number(m.threshold)) };
    let mut text = format!("{}: {} {}{} in the weeks from {} to {}, against {against}. Detector `{}`, [signal]({link}).\n\nEvidence rows ({}):",
        what(&s.detector), number(m.value), words(&s.metric), subject(tx, s)?, day(tx, m.window_start)?,
        day(tx, m.window_end)?, s.detector, s.evidence.len());
    for e in s.evidence.iter().take(EVIDENCE_MOST) {
        text += &format!("\n- {}", row(tx, e)?);
    }
    if s.evidence.len() > EVIDENCE_MOST {
        text += &format!("\n- {} more on the [signal]({link})", s.evidence.len() - EVIDENCE_MOST);
    }
    if s.evidence.is_empty() { text += "\n- none recorded"; }
    Ok(text)
}

/// A fix task's text in the default template's sections (`tasks::template`), standing on its own.
pub(super) fn task(tx: &Transaction, project: &str, p: &Problem) -> Result<String, String> {
    let quote = |words: &str| words.lines().map(|line| format!("> {line}")).collect::<Vec<_>>().join("\n");
    let mut measures = Vec::new();
    let mut watch = Vec::new();
    for y in &p.yardstick {
        let s = signals::get(tx, project, &y.fingerprint)?;
        measures.push(measured(tx, project, &s)?);
        watch.push(format!("- {} within {} for {} weeks", words(&s.metric), number(y.threshold), y.weeks));
    }
    let share = p.indicator.map_or_else(|| "No impact share is measured.".into(), |unit| {
        let unit = serde_json::to_value(unit).ok().and_then(|v| v.as_str().map(words)).unwrap_or_default();
        format!("At its peak it was {:.0}% of the project's {unit} in a week.", p.share * 100.0)
    });
    let options = p.fix_options.iter().map(|f| format!("- {}: {}", f.layer, f.action)).collect::<Vec<_>>().join("\n");
    Ok(format!("{}Supervisor register problem {} (impact {}).\n\n## Problem or goal\n{}. {share}\n\n{}\n\n\
        ## Context\nHypothesis, to test rather than trust:\n{}\n\nWhat would disprove it:\n{}\n\nFix options:\n{options}\n\n\
        ## Expected result\nThe cause is named with the evidence rows and fixed, and the Supervisor's yardstick holds:\n{}\n\n\
        ## Prerequisites\nNone recorded.",
        if p.exception { "A verified charter rule was breached. This fix is exempt from the spend limit.\n\n" } else { "" },
        p.id, p.impact, p.title, measures.join("\n\n"), quote(&p.hypothesis), quote(&p.disproof), watch.join("\n")))
}
