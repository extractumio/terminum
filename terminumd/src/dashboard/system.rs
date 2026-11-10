// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The Dashboard's System tab (terminum.md AC-171): the sessions Dispatcher started and how their tasks
//! ended, what failed and why, the schedules' firings, how busy the agents' slots were. A session lasts
//! from its launch row in the input log to its last walk once closed or left, else to now.

use std::collections::{BTreeMap, HashMap};

use rusqlite::Transaction;
use serde_json::{json, Value};

use super::{local, median, Ask, Span};
use crate::tasks::ops::rows;

/// A session of Dispatcher's: its agent, when it began and ended (local ms).
struct Run { agent: String, name: String, color: i64, start: i64, end: i64, open: bool }

/// A task's move by Dispatcher or an agent to an end or to a human; a failure of any kind with its words.
struct Read {
    runs: Vec<Run>,
    /// local ms, where to
    moves: Vec<(i64, String)>,
    /// local ms, kind (task, write, schedule), the reason's class
    failures: Vec<(i64, &'static str, String)>,
    /// local ms, state
    fires: Vec<(i64, String)>,
    /// each agent's slots today
    slots: HashMap<String, i64>,
}

/// A reason without what differs from one time to the next: its words up to a colon, numbers as N.
pub fn class(why: &str) -> String {
    let head = why.split(':').next().unwrap_or("").trim().trim_end_matches('.');
    let mut out = String::new();
    for c in head.chars().take(80) {
        if !c.is_ascii_digit() { out.push(c); } else if !out.ends_with('N') { out.push('N'); }
    }
    if out.is_empty() { "No reason said".into() } else { out }
}

pub fn numbers(tx: &Transaction, ask: &Ask, now: i64) -> Result<Value, String> {
    let read = read(tx, ask, now)?;
    let mut v = summarize(&read, ask.span);
    v["before"] = ask.earlier(|before| Ok(summarize(&read, before)["tiles"].take()))?;
    Ok(v)
}

fn read(tx: &Transaction, ask: &Ask, now: i64) -> Result<Read, String> {
    let args = (ask.since, ask.task_project());
    // a session that began up to a week before the rows asked for may still run in the span
    let runs = rows(tx, &format!("SELECT r.agent_id, r.agent_name, r.agent_color, {}, {}, r.state NOT IN ('closed', 'left') FROM runs r
        JOIN (SELECT run, min(at) AS at FROM inputs WHERE at >= ?1 - {LONGEST} AND component = 'dispatcher.start' GROUP BY run) s ON s.run = r.id
        WHERE (?2 IS NULL OR r.project = ?2)", local("s.at"), local("r.last_walk * 1000"), LONGEST = 7 * super::DAY), args,
        |r| Ok(Run { agent: r.get(0)?, name: r.get(1)?, color: r.get(2)?, start: r.get(3)?, end: r.get(4)?, open: r.get(5)? }))?
        .into_iter().map(|r| Run { end: if r.open { now } else { r.end.max(r.start) }, ..r }).collect();
    let at = local("e.at * 1000");
    let mine = "(?2 IS NULL OR t.project = ?2)";
    // a task's move by Dispatcher or an agent, with Dispatcher's reason said beside it
    let ended: Vec<(i64, String, Option<String>)> = rows(tx, &format!("SELECT {at}, json_extract(e.data, '$.to'),
        (SELECT w.text FROM events w WHERE w.task = e.task AND w.kind = 'why' AND w.at BETWEEN e.at AND e.at + 2 ORDER BY w.at LIMIT 1)
        FROM events e JOIN tasks t ON t.id = e.task WHERE e.kind = 'move' AND e.author != 'human' AND e.at >= ?1 / 1000
        AND json_extract(e.data, '$.to') IN ('DONE', 'FAILED', 'WAIT FOR HUMAN') AND {mine}"), args, |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    let mut failures: Vec<(i64, &'static str, String)> = ended.iter().filter(|m| m.1 == "FAILED").map(|(t, _, why)| (*t, "task", class(why.as_deref().unwrap_or("")))).collect();
    let fires: Vec<(i64, String, Option<String>)> = rows(tx, &format!("SELECT {}, f.state, f.why FROM schedule_fires f JOIN schedules t ON t.id = f.schedule
        WHERE f.at >= ?1 / 1000 AND {mine}", local("f.at * 1000")), args, |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    failures.extend(fires.iter().filter(|f| f.1 == "failed" || f.1 == "skipped").map(|(t, state, why)| (*t, "schedule", class(why.as_deref().unwrap_or(state.as_str())))));
    // a write into a pane that did not get there, the human's or Dispatcher's
    let writes: Vec<(i64, String)> = rows(tx, &format!("SELECT {}, error FROM inputs WHERE error IS NOT NULL AND at >= ?1 AND (?2 IS NULL OR coalesce(project, '') = ?2)", local("at")),
        (ask.since, ask.project), |r| Ok((r.get(0)?, r.get(1)?)))?;
    failures.extend(writes.into_iter().map(|(t, why)| (t, "write", class(&why))));
    let slots = rows(tx, "SELECT id, at_once FROM agents", [], |r| Ok((r.get(0)?, r.get(1)?)))?.into_iter().collect();
    Ok(Read { runs, moves: ended.into_iter().map(|(t, to, _)| (t, to)).collect(), failures, fires: fires.into_iter().map(|(t, state, _)| (t, state)).collect(), slots })
}

/// How much of [`a`, `b`) lies in [`lo`, `hi`).
fn overlap(a: i64, b: i64, lo: i64, hi: i64) -> i64 {
    (b.min(hi) - a.max(lo)).max(0)
}

/// A share as a whole percent of `of`; 0 of nothing.
fn percent(part: i64, of: i64) -> i64 {
    if of > 0 { (part as f64 / of as f64 * 100.0).round() as i64 } else { 0 }
}

fn summarize(read: &Read, span: &Span) -> Value {
    let n = span.len();
    let (lo, hi) = (span.lo(), span.until + 1);
    // the agents that ran in the span, most busy first; one no longer in the pool had one slot
    let mut agents: BTreeMap<&str, (&str, i64, i64, i64)> = BTreeMap::new();
    let mut busy = vec![0i64; n];
    let mut started = vec![0i64; n];
    let mut lengths = Vec::new();
    for r in &read.runs {
        let in_span = overlap(r.start, r.end, lo, hi);
        if in_span == 0 && !span.holds(r.start) { continue; }
        let a = agents.entry(r.agent.as_str()).or_insert((r.name.as_str(), r.color, 0, 0));
        a.2 += in_span;
        for (k, b) in busy.iter_mut().enumerate() {
            *b += overlap(r.start, r.end, span.edges[k], span.end(k));
        }
        if let Some(k) = span.index(r.start) {
            started[k] += 1;
            a.3 += 1;
            if !r.open { lengths.push(r.end - r.start); }
        }
    }
    let slot = |id: &str| read.slots.get(id).copied().unwrap_or(1).max(1);
    let all_slots: i64 = read.slots.values().sum::<i64>() + agents.keys().filter(|id| !read.slots.contains_key(**id)).count() as i64;
    let mut ranked: Vec<Value> = agents.iter().map(|(id, (name, color, ms, sessions))| json!({"id": id, "name": name, "color": color, "busy_ms": ms, "sessions": sessions,
        "slots": slot(id), "load": percent(*ms, (hi - lo) * slot(id))})).collect();
    ranked.sort_by_key(|a| std::cmp::Reverse(a["busy_ms"].as_i64()));
    let count = |list: &[(i64, String)], what: &str| -> Vec<i64> {
        let mut out = vec![0i64; n];
        for (t, s) in list { if s == what && let Some(k) = span.index(*t) { out[k] += 1; } }
        out
    };
    let (done, failed, asked) = (count(&read.moves, "DONE"), count(&read.moves, "FAILED"), count(&read.moves, "WAIT FOR HUMAN"));
    let fire_states = ["started", "skipped", "failed", "called off"];
    let fires: Vec<Vec<i64>> = fire_states.iter().map(|s| count(&read.fires, s)).collect();
    let mut errors = vec![0i64; n];
    let mut why: BTreeMap<(&str, &str), i64> = BTreeMap::new();
    for (t, kind, class) in &read.failures {
        if let Some(k) = span.index(*t) {
            errors[k] += 1;
            *why.entry((kind, class.as_str())).or_default() += 1;
        }
    }
    let mut failures: Vec<Value> = why.iter().map(|((kind, class), n)| json!({"kind": kind, "why": class, "n": n})).collect();
    failures.sort_by_key(|f| std::cmp::Reverse(f["n"].as_i64()));
    let series: Vec<Value> = (0..n).map(|k| json!({"started": started[k], "done": done[k], "failed": failed[k], "asked": asked[k], "errors": errors[k],
        "load": percent(busy[k], (span.end(k).min(hi) - span.edges[k]) * all_slots),
        "fires": fire_states.iter().zip(&fires).map(|(s, f)| (*s, f[k])).collect::<BTreeMap<_, _>>()})).collect();
    let sum = |v: &[i64]| v.iter().sum::<i64>();
    let (d, f) = (sum(&done), sum(&failed));
    let kinds = |kind: &str| why.iter().filter(|((k, _), _)| *k == kind).map(|(_, n)| n).sum::<i64>();
    let tiles = json!({
        "sessions": sum(&started), "done": d, "failed": f, "ended": d + f, "asked": sum(&asked), "failure_rate": (d + f > 0).then(|| f as f64 / (d + f) as f64),
        "errors": sum(&errors), "failed_writes": kinds("write"), "failed_firings": kinds("schedule"),
        "load": percent(busy.iter().sum(), (hi - lo) * all_slots), "busy_ms": sum(&busy), "slots": all_slots, "session_ms": median(lengths),
        "fires": fire_states.iter().zip(&fires).map(|(s, f)| (*s, sum(f))).collect::<BTreeMap<_, _>>(),
    });
    json!({"tiles": tiles, "series": series, "agents": ranked, "failures": failures})
}
