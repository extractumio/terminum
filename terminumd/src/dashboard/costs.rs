// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The Dashboard's Costs tab (terminum.md AC-172): what the sessions Dispatcher started spent, as their
//! hooks reported it (`dispatcher::usage`): tokens that count, cache reads, dollars at list prices, by
//! agent, project and model; beside them each agent's last 7 and 30 days against its limits.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::Transaction;
use serde_json::{json, Value};

use super::{local, stack_key, stacked, Ask, Span};
use crate::dispatcher::usage::{self, Totals};
use crate::tasks::ops::rows;

/// What one report added: local ms, its run, agent, project (`""` none), model, the tokens.
struct Row { t: i64, run: String, agent: String, project: String, model: String, tokens: Totals }

/// Tokens that count, cache reads, dollars where the model's price is known, and the tokens that count of
/// the models with none.
#[derive(Clone, Copy, Default)]
struct Sum { counted: i64, cache_read: i64, usd: f64, unpriced: i64 }

/// Sums by an id: an agent's, a project's, a model's.
type Sums<'a> = HashMap<&'a str, Sum>;

impl Sum {
    /// What one report added.
    fn of(r: &Row) -> Sum {
        let (counted, usd) = (r.tokens.counted(), usage::dollars(&r.tokens, &r.model));
        Sum { counted, cache_read: r.tokens.cache_read, usd: usd.unwrap_or(0.0), unpriced: if usd.is_some() { 0 } else { counted } }
    }

    fn add(&mut self, o: &Sum) {
        self.counted += o.counted;
        self.cache_read += o.cache_read;
        self.usd += o.usd;
        self.unpriced += o.unpriced;
    }

    fn json(&self) -> Value {
        json!({"counted": self.counted, "cache_read": self.cache_read, "usd": self.usd, "unpriced": self.unpriced})
    }
}

pub fn numbers(tx: &Transaction, ask: &Ask, now_s: i64) -> Result<Value, String> {
    let list: Vec<Row> = rows(tx, &format!("SELECT {}, u.run, u.agent_id, CASE WHEN r.project = 'default' THEN '' ELSE r.project END, coalesce(u.model, ''), {}
        FROM usage u JOIN runs r ON r.id = u.run WHERE u.at >= ?1 / 1000 AND (?2 IS NULL OR r.project = ?2)", local("u.at * 1000"), usage::TOTALS),
        (ask.since, ask.task_project()), |r| Ok(Row { t: r.get(0)?, run: r.get(1)?, agent: r.get(2)?, project: r.get(3)?, model: r.get(4)?, tokens: usage::totals_at(r, 5)? }))?;
    // an agent's name and color: the pool's, else what its last run kept
    let mut agents: HashMap<String, (String, i64)> = rows(tx, "SELECT agent_id, agent_name, agent_color FROM runs ORDER BY rowid", [], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?.into_iter().collect();
    let mut pool = crate::tasks::agents::list(tx)?;
    pool.retain(|a| ask.task_project().is_none_or(|p| a.project == p));
    pool.sort_by(|a, b| a.name.cmp(&b.name));
    for a in &pool {
        agents.insert(a.id.clone(), (a.name.clone(), a.color));
    }
    let names = super::project_names(tx)?;
    let mut v = summarize(tx, &list, ask.span, ask, &agents, &names)?;
    v["before"] = ask.earlier(|before| Ok(summarize(tx, &list, before, ask, &agents, &names)?["tiles"].take()))?;
    // each agent's last 7 and 30 days, of every project, against its limits (millions of tokens): as the board says them
    let spent = usage::of_agents(tx, now_s)?;
    v["limits"] = pool.iter().map(|a| {
        let (week, month) = spent.get(&a.id).copied().unwrap_or((0, 0));
        // a limit is in millions of tokens; the share of it used, past 1 when over
        let share = |used: i64, limit: Option<f64>| limit.map(|l| used as f64 / (l * 1e6));
        json!({"id": a.id, "name": a.name, "color": a.color, "week": week, "month": month, "week_limit": a.week_limit, "month_limit": a.month_limit,
               "week_share": share(week, a.week_limit), "month_share": share(month, a.month_limit)})
    }).collect();
    Ok(v)
}

/// `sums` as rows for the page, the most tokens first, each with what `more` says of its id.
fn ranked(sums: &Sums, more: impl Fn(&str) -> Value) -> Vec<Value> {
    let mut list: Vec<(&str, &Sum)> = sums.iter().map(|(id, s)| (*id, s)).collect();
    list.sort_by(|a, b| b.1.counted.cmp(&a.1.counted).then(a.0.cmp(b.0)));
    list.into_iter().map(|(id, s)| {
        let mut row = more(id);
        for (k, n) in s.json().as_object().into_iter().flatten() { row[k] = n.clone(); }
        row
    }).collect()
}

fn summarize(tx: &Transaction, list: &[Row], span: &Span, ask: &Ask, agents: &HashMap<String, (String, i64)>, names: &HashMap<String, String>) -> Result<Value, String> {
    let n = span.len();
    let mut total = Sum::default();
    let mut per: Vec<Sums> = vec![Sums::new(); n];
    let (mut by_agent, mut by_project, mut by_model) = (Sums::new(), Sums::new(), Sums::new());
    let mut runs: HashSet<&str> = HashSet::new();
    for r in list {
        let Some(k) = span.index(r.t) else { continue };
        let sum = Sum::of(r);
        total.add(&sum);
        for (by, id) in [(&mut per[k], &r.agent), (&mut by_agent, &r.agent), (&mut by_project, &r.project), (&mut by_model, &r.model)] {
            by.entry(id.as_str()).or_default().add(&sum);
        }
        runs.insert(r.run.as_str());
    }
    let tasks: i64 = tx.query_row("SELECT count(DISTINCT task) FROM run_tasks WHERE run IN (SELECT value FROM json_each(?1))", [json!(runs).to_string()], |r| r.get(0)).map_err(crate::tasks::ops::sql)?;
    let agent = |id: &str| { let (name, color) = agents.get(id).cloned().unwrap_or_default(); json!({"id": id, "name": name, "color": color}) };
    let agents_ranked = ranked(&by_agent, agent);
    let stack = stacked(agents_ranked.iter().filter_map(|a| a["id"].as_str()));
    let series: Vec<Value> = per.iter().map(|bucket| {
        let (mut sum, mut usd, mut tokens) = (Sum::default(), BTreeMap::<String, f64>::new(), BTreeMap::<String, i64>::new());
        for (id, s) in bucket {
            sum.add(s);
            *usd.entry(stack_key(&stack, id)).or_default() += s.usd;
            *tokens.entry(stack_key(&stack, id)).or_default() += s.counted;
        }
        json!({"counted": sum.counted, "usd": sum.usd, "unpriced": sum.unpriced, "by_usd": usd, "by_tokens": tokens})
    }).collect();
    let mut tiles = total.json();
    tiles["sessions"] = json!(runs.len());
    tiles["tasks"] = json!(tasks);
    tiles["usd_per_task"] = json!((tasks > 0).then(|| total.usd / tasks as f64));
    Ok(json!({"tiles": tiles, "series": series, "stacked": stack, "agents": agents_ranked,
        "projects": ranked(&by_project, |id| ask.project_json(id, names.get(id).map_or("", String::as_str))),
        "models": ranked(&by_model, |id| json!({"model": id}))}))
}
