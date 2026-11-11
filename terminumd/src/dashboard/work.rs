// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The Dashboard's Tasks tab (terminum.md AC-176): who does the work on tasks, the human or automation. An
//! operation is a decision about a task, counted once, for who decided: a task made, edited, moved, commented
//! on, put under another, archived (the board's events, by their author) or started (a session's launch row
//! in the input log: Dispatcher's own when Auto-Dispatcher chose the task or a schedule or a round made it,
//! else the human's Run). What Dispatcher does to carry a decision out (the move that follows a start or an
//! end, a budget set, its notes `told` and `why`, a session's link) is no operation: of its own it only
//! makes tasks and starts them.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::Transaction;
use serde_json::{json, Value};

use super::{local, Ask, Span};
use crate::tasks::ops::rows;

/// Who did it, as the page's keys: the human, an agent, Dispatcher.
pub const WHO: [&str; 3] = ["you", "agents", "dispatcher"];
/// The operations, in the page's order.
pub const KINDS: [&str; 7] = ["made", "edit", "move", "comment", "parent", "start", "archive"];
const START: usize = 5;
/// The tasks listed: those with most operations.
const LISTED: usize = 200;
/// An Auto-Dispatcher's choice starts its task within this of being written, seconds.
const CHOSEN_WITHIN: i64 = 600;
/// The tasks' spread by the human's share: none, under a half, a half and more, all.
const SPREAD: [&str; 4] = ["none", "less", "more", "all"];

/// An operation: when (local ms), which of KINDS and of WHO, the task's row in `Read::tasks`.
struct Op { t: i64, kind: usize, who: usize, task: usize }

struct Read {
    ops: Vec<Op>,
    /// slug, title
    tasks: Vec<(String, String)>,
}

pub fn numbers(tx: &Transaction, ask: &Ask) -> Result<Value, String> {
    let read = read(tx, ask)?;
    let mut v = summarize(&read, ask.span);
    v["before"] = ask.earlier(|before| Ok(summarize(&read, before)["tiles"].take()))?;
    Ok(v)
}

fn read(tx: &Transaction, ask: &Ask) -> Result<Read, String> {
    let args = (ask.since, ask.task_project());
    let mine = "(?2 IS NULL OR t.project = ?2)";
    let mut tasks: Vec<(String, String)> = Vec::new();
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut row_of = |slug: String, title: String| *seen.entry(slug.clone()).or_insert_with(|| { tasks.push((slug, title)); tasks.len() - 1 });
    let mut ops = Vec::new();
    let done: Vec<(i64, String, String, String, String)> = rows(tx, &format!("SELECT {}, e.kind, u.kind, coalesce(t.slug, t.id), t.title
        FROM events e JOIN tasks t ON t.id = e.task JOIN users u ON u.id = e.author
        WHERE e.at >= ?1 / 1000 AND e.kind IN ('made', 'edit', 'move', 'comment', 'parent', 'archive') AND (u.kind != 'system' OR e.kind = 'made') AND {mine}", local("e.at * 1000")), args,
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
    for (t, kind, by, slug, title) in done {
        let Some(kind) = KINDS.iter().position(|k| *k == kind) else { continue };
        let who = match by.as_str() { "human" => 0, "agent" => 1, _ => 2 };
        ops.push(Op { t, kind, who, task: row_of(slug, title) });
    }
    // a start: Dispatcher's own when the task was a round's choice just before, or a schedule's or a round's own task
    let starts: Vec<(i64, i64, String)> = rows(tx, &format!("SELECT {}, at / 1000, tasks FROM inputs
        WHERE at >= ?1 AND component = 'dispatcher.start' AND kind = 'launch' AND tasks IS NOT NULL", local("at")), [ask.since], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    let mut task = tx.prepare_cached(&format!("SELECT t.title, EXISTS (SELECT 1 FROM schedule_fires f WHERE f.task = t.id) OR EXISTS (SELECT 1 FROM auto_rounds a WHERE a.task = t.id)
        OR EXISTS (SELECT 1 FROM auto_starts s WHERE s.task = t.id AND s.at BETWEEN ?3 - {CHOSEN_WITHIN} AND ?3)
        FROM tasks t WHERE t.slug = ?1 AND {mine}")).map_err(crate::tasks::ops::sql)?;
    for (t, at, slugs) in starts {
        for slug in slugs.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let found: Option<(String, bool)> = task.query_row((slug, ask.task_project(), at), |r| Ok((r.get(0)?, r.get(1)?))).ok();
            if let Some((title, own)) = found {
                ops.push(Op { t, kind: START, who: if own { 2 } else { 0 }, task: row_of(slug.to_string(), title) });
            }
        }
    }
    Ok(Read { ops, tasks })
}

/// The human's share of `by` (WHO's counts); none of nothing.
fn share(by: &[i64; 3]) -> Option<f64> {
    let all: i64 = by.iter().sum();
    (all > 0).then(|| by[0] as f64 / all as f64)
}

fn counts(by: &[i64; 3]) -> BTreeMap<&'static str, i64> {
    WHO.iter().copied().zip(by.iter().copied()).collect()
}

fn summarize(read: &Read, span: &Span) -> Value {
    let n = span.len();
    let mut series = vec![[0i64; 3]; n];
    let mut kinds = [[0i64; 3]; KINDS.len()];
    let mut per_task: HashMap<usize, [i64; 3]> = HashMap::new();
    let mut made_by_hand: HashSet<usize> = HashSet::new();
    for op in &read.ops {
        let Some(k) = span.index(op.t) else { continue };
        series[k][op.who] += 1;
        kinds[op.kind][op.who] += 1;
        per_task.entry(op.task).or_default()[op.who] += 1;
        if op.kind == 0 && op.who == 0 { made_by_hand.insert(op.task); }
    }
    let mut total = [0i64; 3];
    for s in &series { for (t, v) in total.iter_mut().zip(s) { *t += v; } }
    let mut spread = [0i64; 4];
    for by in per_task.values() {
        let all: i64 = by.iter().sum();
        spread[if by[0] == 0 { 0 } else if by[0] == all { 3 } else if by[0] * 2 < all { 1 } else { 2 }] += 1;
    }
    let mut ranked: Vec<(&usize, &[i64; 3])> = per_task.iter().collect();
    ranked.sort_by(|a, b| b.1.iter().sum::<i64>().cmp(&a.1.iter().sum()).then(read.tasks[*a.0].0.cmp(&read.tasks[*b.0].0)));
    let tasks: Vec<Value> = ranked.iter().take(LISTED).map(|(i, by)| json!({"id": read.tasks[**i].0, "title": read.tasks[**i].1, "by": counts(by), "n": by.iter().sum::<i64>(), "share": share(by)})).collect();
    let tiles = json!({
        "ops": total.iter().sum::<i64>(), "by": counts(&total), "share": share(&total),
        "tasks": per_task.len(), "made_by_hand": made_by_hand.len(),
        "spread": SPREAD.iter().copied().zip(spread).collect::<BTreeMap<_, _>>(),
    });
    json!({
        "tiles": tiles,
        "series": series.iter().map(|s| json!({"by": counts(s), "share": share(s)})).collect::<Vec<_>>(),
        "kinds": KINDS.iter().zip(&kinds).map(|(k, by)| json!({"kind": k, "by": counts(by), "share": share(by)})).collect::<Vec<_>>(),
        "tasks": tasks, "more": per_task.len().saturating_sub(LISTED),
    })
}
