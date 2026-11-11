// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The Dashboard's Tasks tab (terminum.md AC-176): who does the work on tasks, the human or automation. An
//! operation is a decision about a task, counted once, for who decided: a task made, edited, moved, commented
//! on, put under another, archived (the board's events, by their author) or started (a session's launch row
//! in the input log, for who its run says asked for it: terminum-dispatcher.md AC-183). What Dispatcher does to carry a decision out (the move that follows a start or an
//! end, a budget set, its notes `told` and `why`, a session's link) is no operation: of its own it only
//! makes tasks and starts them.

use std::collections::{BTreeMap, HashMap};

use rusqlite::Transaction;
use serde_json::{json, Value};

use super::{local, Ask, Span};
use crate::tasks::ops::rows;

/// Who did it, as the page's keys: the human, an agent, Dispatcher.
const WHO: [&str; 3] = ["you", "agents", "dispatcher"];
const YOU: usize = 0;
const AGENTS: usize = 1;
const DISPATCHER: usize = 2;
/// The operations, in the page's order.
const KINDS: [&str; 7] = ["made", "edit", "move", "comment", "parent", "start", "archive"];
const MADE: usize = 0;
const START: usize = 5;
/// The tasks listed at most: those with most operations.
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
        ops.push(Op { t, kind, who: who_of(&by), task: row_of(slug, title) });
    }
    // a start is whose its run says (terminum-dispatcher.md AC-183); a run older than that record says nothing:
    // Dispatcher's own when the task was a round's choice just before, or a schedule's or a round's own task
    let starts: Vec<(i64, i64, String, Option<String>)> = rows(tx, &format!("SELECT {}, i.at / 1000, i.tasks, u.kind FROM inputs i
        LEFT JOIN runs r ON r.id = i.run LEFT JOIN users u ON u.id = r.started_by
        WHERE i.at >= ?1 AND i.component = 'dispatcher.start' AND i.kind = 'launch' AND i.tasks IS NOT NULL", local("i.at")), [ask.since], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
    let mut task = tx.prepare_cached(&format!("SELECT t.title, EXISTS (SELECT 1 FROM schedule_fires f WHERE f.task = t.id) OR EXISTS (SELECT 1 FROM auto_rounds a WHERE a.task = t.id)
        OR EXISTS (SELECT 1 FROM auto_starts s WHERE s.task = t.id AND s.at BETWEEN ?3 - {CHOSEN_WITHIN} AND ?3)
        FROM tasks t WHERE t.slug = ?1 AND {mine}")).map_err(crate::tasks::ops::sql)?;
    for (t, at, slugs, by) in starts {
        for slug in slugs.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let found: Option<(String, bool)> = task.query_row((slug, ask.task_project(), at), |r| Ok((r.get(0)?, r.get(1)?))).ok();
            if let Some((title, own)) = found {
                let who = by.as_deref().map_or(if own { DISPATCHER } else { YOU }, who_of);
                ops.push(Op { t, kind: START, who, task: row_of(slug.to_string(), title) });
            }
        }
    }
    Ok(Read { ops, tasks })
}

/// Which of WHO a user's kind is.
fn who_of(kind: &str) -> usize {
    match kind { "human" => YOU, "agent" => AGENTS, _ => DISPATCHER }
}

/// The human's share of `by` (WHO's counts); none of nothing.
fn share(by: &[i64; 3]) -> Option<f64> {
    let all: i64 = by.iter().sum();
    (all > 0).then(|| by[YOU] as f64 / all as f64)
}

fn counts(by: &[i64; 3]) -> BTreeMap<&'static str, i64> {
    WHO.iter().copied().zip(by.iter().copied()).collect()
}

fn summarize(read: &Read, span: &Span) -> Value {
    let n = span.len();
    let mut series = vec![[0i64; 3]; n];
    let mut kinds = [[0i64; 3]; KINDS.len()];
    let mut per_task = vec![[0i64; 3]; read.tasks.len()];
    let mut made_by_hand = vec![false; read.tasks.len()];
    for op in &read.ops {
        let Some(k) = span.index(op.t) else { continue };
        series[k][op.who] += 1;
        kinds[op.kind][op.who] += 1;
        per_task[op.task][op.who] += 1;
        made_by_hand[op.task] |= op.kind == MADE && op.who == YOU;
    }
    let mut total = [0i64; 3];
    for s in &series { for (t, v) in total.iter_mut().zip(s) { *t += v; } }
    // the tasks worked on: operations, the task's row; most first
    let mut ranked: Vec<(i64, usize)> = per_task.iter().enumerate().map(|(i, by)| (by.iter().sum(), i)).filter(|t| t.0 > 0).collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(read.tasks[a.1].0.cmp(&read.tasks[b.1].0)));
    let mut spread: BTreeMap<&str, i64> = SPREAD.iter().map(|s| (*s, 0)).collect();
    for (all, i) in &ranked {
        let mine = per_task[*i][YOU];
        *spread.entry(if mine == 0 { "none" } else if mine == *all { "all" } else if mine * 2 < *all { "less" } else { "more" }).or_default() += 1;
    }
    let tasks: Vec<Value> = ranked.iter().take(LISTED).map(|(all, i)| json!({"id": read.tasks[*i].0, "title": read.tasks[*i].1, "by": counts(&per_task[*i]), "n": all, "share": share(&per_task[*i])})).collect();
    let tiles = json!({
        "ops": total.iter().sum::<i64>(), "by": counts(&total), "share": share(&total),
        "tasks": ranked.len(), "made_by_hand": made_by_hand.iter().filter(|m| **m).count(), "spread": spread,
    });
    json!({
        "tiles": tiles,
        "series": series.iter().map(|s| json!({"by": counts(s)})).collect::<Vec<_>>(),
        "kinds": KINDS.iter().zip(&kinds).map(|(k, by)| json!({"kind": k, "by": counts(by), "share": share(by)})).collect::<Vec<_>>(),
        "tasks": tasks, "more": ranked.len().saturating_sub(LISTED),
    })
}
