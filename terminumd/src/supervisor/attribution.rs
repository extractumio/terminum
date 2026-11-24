// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-213: only typed, additive evidence can contribute to project impact.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::Transaction;

use crate::tasks::ops::rows;

use super::flow::project_events;
use super::impact::{Assessment, Share, Source, Unit, relative_noise};
use super::scoreboard::Week;
use super::spend::usage_reports;
use super::{DAY, Detector, Finding, Reading, WEEK, Window};

pub(super) struct Attribution<'a> {
    weeks: &'a [Week],
    usage: Vec<BTreeMap<String, (f64, Option<f64>)>>,
    waits: BTreeMap<String, Vec<f64>>,
    totals: BTreeMap<Unit, (f64, Vec<Option<f64>>)>,
}

impl<'a> Attribution<'a> {
    pub fn read(tx: &Transaction, project: &str, weeks: &'a [Week]) -> Result<Self, String> {
        let end = weeks.last().expect("scoreboard weeks").end;
        let start = Window::week(weeks[0].end).start;
        let mut usage: Vec<BTreeMap<_, (f64, Option<f64>)>> = weeks.iter().map(|_| BTreeMap::new()).collect();
        for report in usage_reports(tx, project, start, end)? {
            let (tokens, dollars) = usage[((report.at - start) / WEEK) as usize]
                .entry(report.run).or_insert((0.0, Some(0.0)));
            *tokens += report.tokens;
            *dollars = dollars.zip(report.dollars).map(|(sum, cost)| sum + cost);
        }

        // Each retained WAIT transition contributes only the part of its interval in each week.
        // The next move ends the wait even if the task returned to WAIT again later.
        let events = project_events();
        let waits = rows(tx, &format!("{events}
            SELECT e.task, e.at, coalesce(
                (SELECT x.at FROM project_events x WHERE x.task = e.task AND x.kind IN ('move', 'archive')
                    AND x.seq > e.seq ORDER BY x.seq LIMIT 1), ?2 + 1)
            FROM project_events e WHERE e.kind = 'move' AND json_extract(e.data, '$.to') = 'WAIT FOR HUMAN'
                AND coalesce(json_extract(e.data, '$.from'), '') <> 'WAIT FOR HUMAN'
            ORDER BY e.at, e.seq"), (project, end),
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?)))?;
        let mut by_task = BTreeMap::<String, Vec<f64>>::new();
        let mut wait_totals = vec![0.0; weeks.len()];
        for (task, from, until) in waits {
            if until <= start {
                continue;
            }
            let task_days = by_task.entry(task).or_insert_with(|| vec![0.0; weeks.len()]);
            for (i, week) in weeks.iter().enumerate() {
                let days = overlap_days(from, until, week);
                task_days[i] += days;
                wait_totals[i] += days;
            }
        }

        // A P0 or P1 task is open from creation until its latest DONE or FAILED move, while it is
        // still in that status, or its archiving. Priority is the task's current one.
        let priority = rows(tx, &format!("{events}
            SELECT t.created_at, CASE WHEN t.status IN ('DONE', 'FAILED') THEN coalesce(
                (SELECT max(e.at) FROM project_events e WHERE e.task = t.id AND e.kind = 'move'
                    AND json_extract(e.data, '$.to') = t.status), t.updated_at) END, t.archived_at
            FROM tasks t WHERE t.project = ?1 AND t.severity IN ('P0', 'P1') AND t.created_at BETWEEN 1 AND ?2
                AND coalesce(t.archived_at, ?2) >= ?3 AND (t.status NOT IN ('DONE', 'FAILED') OR t.updated_at >= ?3)"),
            (project, end, start), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<i64>>(1)?, r.get::<_, Option<i64>>(2)?)))?;
        let mut priority_totals = vec![0.0; weeks.len()];
        for (created, closed, archived) in priority {
            let until = closed.into_iter().chain(archived).min().unwrap_or(end + 1);
            for (i, week) in weeks.iter().enumerate() {
                priority_totals[i] += overlap_days(created, until, week);
            }
        }
        let totals = Unit::ALL.into_iter().map(|unit| {
            let series: Vec<_> = weeks.iter().enumerate().map(|(i, week)| {
                if !week.available { return None; }
                Some(match unit {
                    Unit::FailureEvents => week.failures,
                    Unit::ScheduleEvents => week.schedule_events,
                    Unit::OwnerHours => week.load.hours,
                    Unit::TaskDaysOfWait => wait_totals[i],
                    Unit::TaskDaysToDone => week.lead_days(),
                    Unit::PriorityTaskDays => priority_totals[i],
                    Unit::Tasks => week.todo,
                    Unit::Switches => week.load.switch_count,
                    Unit::Operations => week.operations.iter().sum(),
                    Unit::StrategicActs => week.strategic_acts(),
                    Unit::Tokens => usage[i].values().map(|r| r.0).sum(),
                    Unit::Spend => usage[i].values().map(|r| r.1).sum::<Option<f64>>()?,
                    Unit::RunEvents => week.launches,
                })
            }).collect();
            (unit, (relative_noise(&series), series))
        }).collect();
        Ok(Self { weeks, usage, waits: by_task, totals })
    }

    pub fn assess<'r>(&self, detector: &Detector, found: &Finding, readings: impl Iterator<Item = Option<&'r Reading>>) -> Assessment {
        let latest = self.weeks.len() - 1;
        let mut assessment = Assessment::default();
        let mut dropped = false;
        for (i, (week, reading)) in self.weeks.iter().zip(readings).enumerate() {
            let value = reading.map_or(0.0, |r| r.value);
            let some = |unit, amount| vec![(unit, amount)];
            let amounts = match found.source {
                Source::WaitDays => some(Unit::TaskDaysOfWait, self.waits.get(&found.subject_id).map_or(0.0, |days| days[i])),
                Source::AgeSince(since) => {
                    let overdue = since + (found.measure.threshold * 60.0) as i64;
                    some(Unit::PriorityTaskDays, overlap_days(overdue, found.measure.window_end + 1, week))
                }
                Source::BlockingLinks if i == latest => some(Unit::Tasks, found.measure.value),
                Source::Count(unit) => {
                    let mut amounts = some(unit, value);
                    if let Some(reading) = reading {
                        amounts.extend(self.report_amounts(i, &reading.attributed_runs));
                    }
                    amounts
                }
                Source::RateExcess(unit, _) => some(unit, reading.and_then(|r| rate_excess(r, detector)).unwrap_or(0.0)),
                Source::BacklogGrowth => some(Unit::Tasks, week.net_growth()),
                Source::SpendAbove(target) => some(Unit::Spend, (value - target).max(0.0) * self.totals[&Unit::Spend].1[i].unwrap_or_default()),
                Source::ExcessOwnerHours | Source::ExcessOperations | Source::ExcessSwitches | Source::ExcessOffHours | Source::LeadDays
                    => self.excess(i, found.source, reading),
                Source::RunUsage => self.report_amounts(i, &found.attributed_runs),
                Source::BlockingLinks | Source::Present | Source::Unsized(_) => Vec::new(),
            };
            let measured = amounts.iter().any(|(_, amount)| *amount > 0.0);
            let firing = reading.is_some_and(|r| r.exceeds(detector.threshold).is_some() && !r.evidence.is_empty());
            let present = detector.rule.present_only().is_some() || matches!(found.source, Source::Present | Source::BlockingLinks);
            if i + 2 >= self.weeks.len() && (measured || firing || present) {
                assessment.ranked = true;
            }
            for (unit, amount) in amounts.into_iter().filter(|(_, amount)| *amount > 0.0) {
                let (noise, totals) = &self.totals[&unit];
                match totals[i] {
                    Some(total) if total > 0.0 && amount <= total + 1e-8 => assessment.shares.push(Share {
                        week: week.end, complete: week.covered, unit, amount: amount.min(total), total, noise: *noise,
                    }),
                    Some(_) => dropped = true,
                    None => (),
                }
            }
        }
        if assessment.shares.is_empty() {
            assessment.no_share = Some(match found.source {
                Source::Unsized(reason) => reason,
                Source::Present => "A present state has no weekly quantity; it ranks, and only a round's evidence raises its level.",
                _ if dropped => "A measured amount exceeded its week's project total, so no share was counted.",
                _ => "No week of the last eight measured an amount above zero.",
            }.into());
        }
        assessment
    }

    fn report_amounts(&self, i: usize, runs: &BTreeSet<String>) -> Vec<(Unit, f64)> {
        let attributed = || runs.iter().filter_map(|id| self.usage[i].get(id));
        let tokens = attributed().map(|r| r.0).sum();
        let dollars = attributed().map(|r| r.1).sum::<Option<f64>>();
        [(Unit::Tokens, Some(tokens)), (Unit::Spend, dollars)].into_iter()
            .filter_map(|(unit, amount)| Some((unit, amount?))).collect()
    }

    fn excess(&self, i: usize, source: Source, reading: Option<&Reading>) -> Vec<(Unit, f64)> {
        let Some(old) = i.checked_sub(1).map(|i| &self.weeks[i]).filter(|w| w.covered) else { return Vec::new() };
        let week = &self.weeks[i];
        let rate = |n: f64, d: f64| (d > 0.0).then_some(n / d);
        let pair = match source {
            Source::ExcessOwnerHours => rate(old.load.hours, old.features)
                .map(|expected| (Unit::OwnerHours, week.load.hours - week.features * expected)),
            Source::ExcessOperations => rate(old.operations.iter().sum(), old.done)
                .map(|expected| (Unit::Operations, week.operations.iter().sum::<f64>() - week.done * expected)),
            Source::ExcessSwitches => rate(old.load.switch_count, old.load.hours)
                .map(|expected| (Unit::Switches, week.load.switch_count - week.load.hours * expected)),
            Source::ExcessOffHours => rate(old.load.off_hours, old.load.hours)
                .map(|expected| (Unit::OwnerHours, week.load.off_hours - week.load.hours * expected)),
            Source::LeadDays => reading.and_then(|r| r.history.first())
                .map(|baseline| (Unit::TaskDaysToDone, week.excess_lead_days(baseline / 1440.0))),
            _ => None,
        };
        pair.into_iter().collect()
    }
}

/// A week's own sample carries its observations; the excess over the band is a count.
fn rate_excess(reading: &Reading, detector: &Detector) -> Option<f64> {
    let Source::RateExcess(_, observations) = reading.source else { return None };
    let (band, _) = reading.limits(detector.threshold)?;
    Some((reading.value - band).max(0.0) * observations)
}

fn overlap_days(from: i64, until: i64, week: &Week) -> f64 {
    (until.min(week.end + 1) - from.max(Window::week(week.end).start)).max(0) as f64 / DAY as f64
}
