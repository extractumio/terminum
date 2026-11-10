// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The Dashboard's Costs tab (terminum.md AC-171): what the sessions Dispatcher started spent, as their
//! hooks reported it (`dispatcher::usage`): tokens that count, cache reads, dollars at list prices, by
//! agent, project and model; beside them each agent's last 7 and 30 days against its limits.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::Transaction;
use serde_json::{json, Value};

use super::{local, stack_key, stacked, Ask, Span, DAY};
use crate::dispatcher::usage::{self, Totals};
use crate::tasks::ops::rows;

/// What one report added: local ms, its run, agent, project (`""` none), model, the tokens.
struct Row { t: i64, run: String, agent: String, project: String, model: String, tokens: Totals }

/// Tokens that count, cache reads, dollars where the model's price is known, and the tokens that count of
/// the models with none.
#[derive(Clone, Copy, Default)]
struct Sum { counted: i64, cache_read: i64, usd: f64, unpriced: i64 }

impl Sum {
    fn add(&mut self, r: &Row) {
        self.counted += r.tokens.counted();
        self.cache_read += r.tokens.cache_read;
        match usage::dollars(&r.tokens, &r.model) {
            Some(usd) => self.usd += usd,
            None => self.unpriced += r.tokens.counted(),
        }
    }

    fn json(&self) -> Value {
        json!({"counted": self.counted, "cache_read": self.cache_read, "usd": self.usd, "unpriced": self.unpriced})
    }
}

pub fn numbers(tx: &Transaction, ask: &Ask, now_s: i64) -> Result<Value, String> {
    let list: Vec<Row> = rows(tx, &format!("SELECT {}, u.run, u.agent_id, CASE WHEN r.project = 'default' THEN '' ELSE r.project END, coalesce(u.model, ''),
        u.input, u.output, u.cache_write, u.cache_write_1h, u.cache_read FROM usage u JOIN runs r ON r.id = u.run
        WHERE u.at >= ?1 / 1000 AND (?2 IS NULL OR r.project = ?2)", local("u.at * 1000")), (ask.since, ask.task_project()),
        |r| Ok(Row { t: r.get(0)?, run: r.get(1)?, agent: r.get(2)?, project: r.get(3)?, model: r.get(4)?,
                     tokens: Totals { input: r.get(5)?, output: r.get(6)?, cache_write: r.get(7)?, cache_write_1h: r.get(8)?, cache_read: r.get(9)? } }))?;
    let kept_from: Option<i64> = tx.query_row(&format!("SELECT {} FROM (SELECT min(at) AS at FROM usage) WHERE at IS NOT NULL", local("at * 1000")), [], |r| r.get(0)).ok();
    // an agent's name and color: the pool's, else what its last run kept
    let mut agents: HashMap<String, (String, i64)> = rows(tx, "SELECT agent_id, agent_name, agent_color FROM runs ORDER BY rowid", [], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?.into_iter().collect();
    let pool: Vec<(String, String, i64, Option<f64>, Option<f64>)> = rows(tx, "SELECT id, name, color, week_limit, month_limit FROM agents ORDER BY name", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?;
    for (id, name, color, ..) in &pool {
        agents.insert(id.clone(), (name.clone(), *color));
    }
    let names = super::project_names(tx)?;
    let mut v = summarize(tx, &list, ask.span, ask, &agents, &names)?;
    v["before"] = match ask.earlier(kept_from, || Value::Bool(true)) {
        Value::Null => Value::Null,
        _ => summarize(tx, &list, ask.before, ask, &agents, &names)?["tiles"].take(),
    };
    v["kept_from"] = json!(kept_from.filter(|k| *k > ask.span.lo()).map(|k| super::civil(k.div_euclid(DAY))));
    // each agent's last 7 and 30 days, of every project, against its limits (millions of tokens): as the board says them
    let spent = usage::of_agents(tx, now_s)?;
    v["limits"] = pool.iter().map(|(id, name, color, week_limit, month_limit)| {
        let (week, month) = spent.get(id).copied().unwrap_or((0, 0));
        json!({"id": id, "name": name, "color": color, "week": week, "month": month, "week_limit": week_limit, "month_limit": month_limit})
    }).collect();
    Ok(v)
}

/// `sums` as rows for the page, the most tokens first, each with what `more` says of its id.
fn ranked(sums: &HashMap<&str, Sum>, more: impl Fn(&str) -> Value) -> Vec<Value> {
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
    let mut per: Vec<HashMap<&str, Sum>> = vec![HashMap::new(); n];
    let (mut by_agent, mut by_project, mut by_model): (HashMap<&str, Sum>, HashMap<&str, Sum>, HashMap<&str, Sum>) = Default::default();
    let mut runs: HashSet<&str> = HashSet::new();
    for r in list {
        let Some(k) = span.index(r.t) else { continue };
        total.add(r);
        per[k].entry(r.agent.as_str()).or_default().add(r);
        by_agent.entry(r.agent.as_str()).or_default().add(r);
        by_project.entry(r.project.as_str()).or_default().add(r);
        by_model.entry(r.model.as_str()).or_default().add(r);
        runs.insert(r.run.as_str());
    }
    let tasks: i64 = tx.query_row("SELECT count(DISTINCT task) FROM run_tasks WHERE run IN (SELECT value FROM json_each(?1))", [json!(runs).to_string()], |r| r.get(0)).map_err(crate::tasks::ops::sql)?;
    let agent = |id: &str| { let (name, color) = agents.get(id).cloned().unwrap_or_default(); json!({"id": id, "name": name, "color": color}) };
    let agents_ranked = ranked(&by_agent, agent);
    let stack = stacked(&agents_ranked.iter().map(|a| (a["id"].as_str().unwrap_or("").to_string(), ())).collect::<Vec<_>>());
    let series: Vec<Value> = per.iter().map(|bucket| {
        let (mut sum, mut usd, mut tokens) = (Sum::default(), BTreeMap::<String, f64>::new(), BTreeMap::<String, i64>::new());
        for (id, s) in bucket {
            sum.counted += s.counted;
            sum.cache_read += s.cache_read;
            sum.usd += s.usd;
            sum.unpriced += s.unpriced;
            *usd.entry(stack_key(&stack, id)).or_default() += s.usd;
            *tokens.entry(stack_key(&stack, id)).or_default() += s.counted;
        }
        let mut v = sum.json();
        v["by_usd"] = json!(usd);
        v["by_tokens"] = json!(tokens);
        v
    }).collect();
    let mut tiles = total.json();
    tiles["sessions"] = json!(runs.len());
    tiles["tasks"] = json!(tasks);
    tiles["usd_per_task"] = json!((tasks > 0).then(|| total.usd / tasks as f64));
    Ok(json!({"tiles": tiles, "series": series, "stacked": stack, "agents": agents_ranked,
        "projects": ranked(&by_project, |id| ask.project_json(id, names.get(id).map_or("", String::as_str))),
        "models": ranked(&by_model, |id| json!({"model": id}))}))
}
