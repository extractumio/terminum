// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-237 and AC-238: reserve room at admission, then charge reported usage once per run.

use std::collections::HashMap;

use rusqlite::Transaction;
use serde::Serialize;

use crate::tasks::ops::{rows, sql};

use super::{DAY, Evidence, WEEK, settings, signals, spend};

pub(crate) const JUDGMENT_NOTICE: &str = "Supervisor judgment budget limit reached. Code-only work continues.";
pub(crate) const FIX_NOTICE: &str = "Supervisor fix spend limit reached.";
const DEFAULT_ROUND_TOKENS: u64 = 100_000;

#[derive(Default)]
struct Usage {
    tokens: f64,
    daily: f64,
    dollars: f64,
    unpriced: bool,
}

#[derive(Default, Serialize)]
pub(crate) struct Judgment {
    pub tokens: u64,
    pub held_tokens: u64,
    pub limit_tokens: u64,
    pub day_tokens: u64,
    pub day_limit_tokens: u64,
    pub routine_left: u64,
    pub investigation_left: u64,
    pub round_limit_tokens: u64,
}

impl Judgment {
    fn room(&self, investigation: bool) -> u64 {
        let week = if investigation { self.investigation_left } else { self.routine_left };
        week.min(self.round_limit_tokens)
    }

    pub(super) fn stopped(&self) -> bool {
        self.room(true) == 0
    }
}

#[derive(Default, Serialize)]
pub(crate) struct Fixes {
    pub dollars: f64,
    pub held_dollars: f64,
    pub limit_dollars: f64,
    pub unpriced_runs: usize,
    pub exception_dollars: f64,
    pub estimated_dollars_per_run: f64,
}

impl Fixes {
    pub(crate) fn room(&self, released_reservation: f64) -> bool {
        self.limit_dollars > 0.0
            && self.dollars + (self.held_dollars - released_reservation).max(0.0) + self.estimated_dollars_per_run <= self.limit_dollars + 1e-9
    }
}

#[derive(Default, Serialize)]
pub(crate) struct Spending {
    pub judgment: Judgment,
    pub fixes: Fixes,
}

fn usage(tx: &Transaction, project: &str, at: i64) -> Result<HashMap<String, Usage>, String> {
    let mut grouped = HashMap::<String, Usage>::new();
    for report in spend::usage_reports(tx, project, at - WEEK + 1, at)? {
        let use_of_run = grouped.entry(report.run).or_default();
        use_of_run.tokens += report.tokens;
        if report.at > at - DAY { use_of_run.daily += report.tokens; }
        if let Some(price) = report.dollars { use_of_run.dollars += price; } else { use_of_run.unpriced = true; }
    }
    Ok(grouped)
}

fn token_count(n: f64) -> u64 {
    n.clamp(0.0, i64::MAX as f64) as u64
}

fn judgment_for(tx: &Transaction, project: &str, budgets: &settings::Budgets, uses: &HashMap<String, Usage>, at: i64) -> Result<Judgment, String> {
    let rounds = rows(tx, "SELECT run, reserved_class, coalesce(token_cap, 0), ended_at
        FROM supervisor_rounds WHERE project = ?1 AND (ended_at IS NULL OR ended_at > ?2 OR run IN
          (SELECT u.run FROM usage u JOIN runs r ON r.id = u.run WHERE r.project = ?1 AND u.at > ?2))
        UNION ALL
        SELECT q.run, 0, q.cap, CASE WHEN r.state IN ('ending','gone','closed','left') OR NOT EXISTS (SELECT 1 FROM run_tasks m WHERE m.run = q.run AND m.result = 0)
            THEN r.last_walk END
        FROM supervisor_review_runs q JOIN runs r ON r.id = q.run WHERE q.project = ?1
        AND (r.state NOT IN ('gone','closed','left') OR r.last_walk > ?2 OR EXISTS (
            SELECT 1 FROM usage u WHERE u.run = q.run AND u.at > ?2))",
        (project, at - WEEK), |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?,
            r.get::<_, i64>(2)?, r.get::<_, Option<i64>>(3)?)))?;
    let floor = budgets.judgment_floor_tokens;
    let weekly_project = uses.values().map(|u| u.tokens).sum::<f64>();
    let limit = token_count(weekly_project * budgets.judgment_percent / 100.).max(floor)
        .min(token_count(budgets.judgment_ceiling.unwrap_or(i64::MAX as f64)));
    let routine_limit = token_count(limit as f64 * (100. - budgets.investigation_reserve_percent) / 100.);
    let day_limit = budgets.daily_token_ceiling.unwrap_or(limit);
    let (mut week, mut day, mut routine, mut held, mut held_today) = (0_u64, 0_u64, 0_u64, 0_u64, 0_u64);
    for (run, reserved, cap, ended) in rounds {
        let spent = uses.get(&run).map_or(0, |u| token_count(u.tokens));
        let daily = uses.get(&run).map_or(0, |u| token_count(u.daily));
        let awaiting_usage = !uses.contains_key(&run) && ended.is_some_and(|t| t > at - WEEK);
        let remaining = if ended.is_none() || awaiting_usage { (cap as u64).saturating_sub(spent) } else { 0 };
        if ended.is_none_or(|t| t > at - DAY) {
            held_today = held_today.saturating_add(remaining);
        }
        week = week.saturating_add(spent);
        day = day.saturating_add(daily);
        held = held.saturating_add(remaining);
        if !reserved { routine = routine.saturating_add(spent).saturating_add(remaining); }
    }
    let investigation_left = limit.saturating_sub(week.saturating_add(held))
        .min(day_limit.saturating_sub(day.saturating_add(held_today)));
    Ok(Judgment {
        tokens: week, held_tokens: held, limit_tokens: limit, day_tokens: day, day_limit_tokens: day_limit,
        routine_left: investigation_left.min(routine_limit.saturating_sub(routine)),
        investigation_left, round_limit_tokens: budgets.round_token_ceiling.unwrap_or(DEFAULT_ROUND_TOKENS).min(i64::MAX as u64),
    })
}

pub(super) fn admit_round(tx: &Transaction, project: &str, at: i64, important: bool) -> Result<u64, String> {
    let settings = settings::read(tx, project)?.settings;
    let room = judgment_for(tx, project, &settings.budgets, &usage(tx, project, at)?, at)?.room(important);
    if room == 0 { Err(JUDGMENT_NOTICE.into()) } else { Ok(room) }
}

pub(crate) fn judge_usage(tx: &Transaction, run: &str, at: i64) -> Result<(), String> {
    let rounds = rows(tx, "SELECT q.id, q.project, q.token_cap FROM supervisor_rounds q JOIN project_supervisor p
        ON p.project = q.project AND p.enabled = 1 AND p.generation = q.generation
        WHERE q.run = ?1 AND q.token_cap IS NOT NULL AND q.failure IS NULL",
        [run], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?;
    for (id, project, cap) in rounds {
        let tokens: f64 = tx.query_row("SELECT coalesce(sum(0.0 + input + output + cache_write + cache_read), 0)
            FROM usage WHERE run = ?1", [run], |r| r.get(0)).map_err(sql)?;
        if tokens > cap as f64 {
            super::rounds::failure(tx, run, "The investigation exceeded its token limit.", false)?;
            signals::diagnostic(tx, &project, "self.budget", ("round", &id, "judgment_round_tokens"),
                &[Evidence::SupervisorRounds(id.clone())], (tokens, cap as f64), at)?;
        }
    }
    let changed = tx.execute("UPDATE supervisor_review_runs SET overrun = 1 WHERE run = ?1 AND NOT overrun
        AND cap < (SELECT coalesce(sum(0.0 + input + output + cache_write + cache_read), 0) FROM usage WHERE run = ?1)",
        [run]).map_err(sql)?;
    if changed != 0 {
        let (project, cap, used): (String, i64, f64) = tx.query_row("SELECT project, cap,
            (SELECT sum(0.0 + input + output + cache_write + cache_read) FROM usage WHERE run = ?1)
            FROM supervisor_review_runs WHERE run = ?1", [run], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(sql)?;
        signals::diagnostic(tx, &project, "self.budget", ("run", run, "weekly_review_tokens"), &[], (used, cap as f64), at)?;
    }
    Ok(())
}

fn fixes_for(tx: &Transaction, project: &str, budgets: &settings::Budgets, uses: &HashMap<String, Usage>, at: i64) -> Result<Fixes, String> {
    let attributed = rows(tx, "SELECT r.id, r.state NOT IN ('closed', 'left', 'gone'), coalesce(f.estimate,
            (SELECT max(m.estimate) FROM run_tasks t JOIN supervisor_task_marks m ON m.task = t.task WHERE t.run = r.id), 0),
            EXISTS (SELECT 1 FROM run_tasks t JOIN supervisor_task_marks m ON m.task = t.task
                WHERE t.run = r.id AND m.exception = 1),
            coalesce(r.started_by, '') = 'supervisor:' || ?1 OR f.run IS NOT NULL OR EXISTS (SELECT 1 FROM run_tasks t JOIN supervisor_task_marks m ON m.task = t.task
                WHERE t.run = r.id AND m.project = ?1),
            EXISTS (SELECT 1 FROM supervisor_rounds q WHERE q.run = r.id)
             OR EXISTS (SELECT 1 FROM supervisor_review_runs v WHERE v.run = r.id)
        FROM runs r LEFT JOIN supervisor_fix_runs f ON f.run = r.id WHERE r.project = ?1
          AND (r.state NOT IN ('closed', 'left', 'gone') OR (f.run IS NOT NULL AND r.last_walk >= ?2)
            OR EXISTS (SELECT 1 FROM usage u WHERE u.run = r.id AND u.at >= ?2))",
        (project, at - WEEK + 1), |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?,
            r.get::<_, f64>(2)?, r.get::<_, bool>(3)?, r.get::<_, bool>(4)?, r.get::<_, bool>(5)?)))?;
    let mut out = Fixes::default();
    let (mut baseline, mut comparable, mut runs) = (0., 0., 0);
    for (run, active, estimate, exception, charged, judgment) in attributed {
        let observed = uses.get(&run);
        let dollars = observed.map_or(0., |u| u.dollars);
        if !judgment { baseline += dollars; }
        if charged {
            out.dollars += dollars;
            if exception { out.exception_dollars += dollars; }
            let unpriced = observed.map_or(active || estimate > 0.0, |u| u.unpriced);
            if unpriced { out.unpriced_runs += 1; }
            if active || unpriced { out.held_dollars += (estimate - dollars).max(0.); }
        } else if !judgment && !active && observed.is_some_and(|u| !u.unpriced) && dollars > 0. {
            comparable += dollars;
            runs += 1;
        }
    }
    out.limit_dollars = budgets.fixes_ceiling.unwrap_or(baseline * budgets.fixes_percent / 100.);
    out.estimated_dollars_per_run = if runs > 0 { comparable / f64::from(runs) } else { out.limit_dollars };
    let held: f64 = tx.query_row("SELECT coalesce(sum(m.estimate), 0) FROM supervisor_task_marks m JOIN tasks t ON t.id = m.task
        WHERE m.project = ?1 AND t.project = ?1 AND t.status IN ('TODO','IN PROGRESS','INTEGRATION','WAIT FOR HUMAN')
          AND t.archived_at IS NULL AND NOT EXISTS (SELECT 1 FROM run_tasks a WHERE a.task = t.id)", [project],
        |r| r.get(0)).map_err(sql)?;
    out.held_dollars += held;
    Ok(out)
}

pub(crate) fn spending(tx: &Transaction, project: &str, at: i64) -> Result<Spending, String> {
    let settings = settings::read(tx, project)?.settings;
    let uses = usage(tx, project, at)?;
    Ok(Spending { judgment: judgment_for(tx, project, &settings.budgets, &uses, at)?,
        fixes: fixes_for(tx, project, &settings.budgets, &uses, at)? })
}
