// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-236, AC-238: a register problem in words a newcomer acts on: an automatic problem's title, hypothesis
//! and disproof from its detector's measure, and its fix task's text with the measures, weeks, impact share
//! and a link to each evidence row.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use rusqlite::{OptionalExtension, Transaction};

use crate::tasks::ops::sql;

use serde_json::Value;

use super::{Evidence, Signal, register::{Fix, Problem}, round_evidence, signals};

/// Evidence rows written into a task; the signal's page lists them all.
const EVIDENCE_MOST: usize = 20;
/// A task ID or project ID kept readable in a link (`TERM-12`).
const ENCODED: &AsciiSet = &NON_ALPHANUMERIC.remove(b'-').remove(b'_').remove(b'.');

fn number(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').into()
}

fn words(s: &str) -> String {
    s.replace('_', " ")
}

fn capitalized(s: &str) -> String {
    let mut chars = s.chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// The detector's name without its lens: `flow.backlog_growth` → "Backlog growth".
fn what(detector: &str) -> String {
    capitalized(&words(detector.split_once('.').map_or(detector, |(_, name)| name)))
}

fn task_name(tx: &Transaction, project: &str, id: &str) -> Result<String, String> {
    Ok(tx.query_row("SELECT coalesce(slug, id) FROM tasks WHERE project = ?1 AND (id = ?2 OR slug = ?2) LIMIT 1", (project, id), |r| r.get(0))
        .optional().map_err(sql)?.unwrap_or_else(|| id.to_string()))
}

pub(super) fn task_link(name: &str) -> String {
    format!("[{name}](/kanban/{})", utf8_percent_encode(name, ENCODED))
}

fn subject(tx: &Transaction, s: &Signal) -> Result<String, String> {
    Ok(match s.subject_kind.as_str() {
        "project" => String::new(),
        "task" => format!(" on {}", task_name(tx, &s.project, &s.subject_id)?),
        kind => format!(" for {} {}", words(kind), s.subject_id),
    })
}

fn day(at: i64) -> String {
    crate::dashboard::civil(at.div_euclid(86_400))
}

/// "Backlog growth: 4 growing weeks": what fired and its number.
fn headline(tx: &Transaction, s: &Signal) -> Result<String, String> {
    Ok(format!("{}: {} {}{}", what(&s.detector), number(s.measure.value), words(&s.metric), subject(tx, s)?))
}

/// The headline and the threshold: "Backlog growth: 4 growing weeks, threshold 3".
pub(super) fn title(tx: &Transaction, s: &Signal) -> Result<String, String> {
    Ok(format!("{}, threshold {}", headline(tx, s)?, number(s.measure.threshold)))
}

/// A problem code opened from a measure alone: no cause is claimed, and the text says what would find one.
pub(super) fn unexplained(s: &Signal, weeks: u8) -> (String, String, Fix) {
    let (metric, limit) = (words(&s.metric), number(s.measure.threshold));
    (format!("No investigation has run yet: code opened this problem from the {} measure alone, so its cause is unknown. \
        An investigation would read the evidence rows, check whether they share one cause, \
        and find the change that brings {metric} back within the threshold of {limit}.", s.detector),
     format!("{} stays within the threshold of {limit} for {weeks} weeks with no change made.", capitalized(&metric)),
     Fix { layer: "check".into(), action: format!("Find the cause in the evidence rows, change what produces it, \
        and keep {metric} within {limit} through the observation window.") })
}

/// One evidence row as `round_evidence` resolves it in this project: its task linked, what it was, its day.
fn row(tx: &Transaction, project: &str, e: &Evidence) -> Result<String, String> {
    let key = serde_json::to_value(e).map_err(|e| e.to_string())?;
    let id = key.get("id").map_or_else(String::new, |id| id.as_str().map_or_else(|| id.to_string(), str::to_string));
    let name = format!("{} row {id}", words(key["table"].as_str().unwrap_or_default()));
    let Some(found) = round_evidence::resolve(tx, project, e)? else { return Ok(format!("{name}, not found in this project")) };
    let mut text = match found.get("task").and_then(Value::as_str) {
        Some(task) if matches!(e, Evidence::Tasks(_)) => task_link(task),
        Some(task) => format!("{}: {name}", task_link(task)),
        None => name,
    };
    let facts = found.as_object().into_iter().flatten().filter(|(k, _)| *k != "task" && *k != "at")
        .filter_map(|(k, v)| match v {
            Value::Null => None,
            Value::String(v) => Some(format!("{} {v}", words(k))),
            v => Some(format!("{} {v}", words(k))),
        })
        .collect::<Vec<_>>();
    if !facts.is_empty() { text += &format!(", {}", facts.join(", ")); }
    if let Some(at) = found.get("at").and_then(Value::as_i64) { text += &format!(", on {}", day(at)); }
    Ok(text)
}

fn signal_link(project: &str, fp: &str) -> String {
    format!("/supervisor/?project={}&signal={fp}#signal-{fp}", utf8_percent_encode(project, ENCODED))
}

/// The measure against its baseline, the weeks it covers, and a link to every evidence row.
fn measured(tx: &Transaction, project: &str, s: &Signal) -> Result<String, String> {
    let link = signal_link(project, &s.fingerprint);
    let m = &s.measure;
    let against = if m.baseline == m.threshold { format!("its limit of {}", number(m.threshold)) }
        else { format!("a baseline of {} and a threshold of {}", number(m.baseline), number(m.threshold)) };
    let mut text = format!("{} in the weeks from {} to {}, against {against}. Detector `{}`, [signal]({link}).\n\nEvidence rows ({}):",
        headline(tx, s)?, day(m.window_start), day(m.window_end), s.detector, s.evidence.len());
    for e in s.evidence.iter().take(EVIDENCE_MOST) {
        text += &format!("\n- {}", row(tx, project, e)?);
    }
    if s.evidence.len() > EVIDENCE_MOST {
        text += &format!("\n- {} more on the [signal]({link})", s.evidence.len() - EVIDENCE_MOST);
    }
    if s.evidence.is_empty() { text += "\n- none recorded"; }
    Ok(text)
}

/// A fix task's text in the default template's sections (`tasks::template`), standing on its own.
pub(super) fn task(tx: &Transaction, project: &str, p: &Problem) -> Result<String, String> {
    let quote = |text: &str| text.lines().map(|line| format!("> {line}")).collect::<Vec<_>>().join("\n");
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
    let mut exception = String::new();
    if p.exception {
        exception += "A verified charter rule was breached. This fix is exempt from the spend limit.";
        if !p.breaches.is_empty() { exception += " The charter's never rule and the checked fact that broke it:"; }
        for b in &p.breaches {
            exception += &format!("\n- \"{}\": detector `{}`, [signal]({})", b.never, b.detector, signal_link(project, &b.fingerprint));
        }
        exception += "\n\n";
    }
    let options = p.fix_options.iter().map(|f| format!("- {}: {}", f.layer, f.action)).collect::<Vec<_>>().join("\n");
    Ok(format!("{exception}Supervisor register problem {} (impact {}).\n\n## Problem or goal\n{}. {share}\n\n{}\n\n\
        ## Context\nHypothesis, to test rather than trust:\n{}\n\nWhat would disprove it:\n{}\n\nFix options:\n{options}\n\n\
        ## Expected result\nThe cause is named with the evidence rows and fixed, and the Supervisor's yardstick holds:\n{}\n\n\
        ## Prerequisites\nNone recorded.",
        p.id, p.impact, p.title, measures.join("\n\n"), quote(&p.hypothesis), quote(&p.disproof), watch.join("\n")))
}
