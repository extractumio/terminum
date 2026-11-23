// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-214: project history, additive inputs, and independently guarded verdicts.

use std::collections::BTreeMap;

use rusqlite::Transaction;

use crate::tasks::ops::{rows, sql};

use super::flow::{FIRST_DONE, project_events};
use super::impact::absolute_noise;
use super::people::{Load, OwnerAction, owner_actions, owner_load};
use super::{WEEK, Window, catalog, percentile, probes,
    probe_gate::Gate, probe_git::Releases, probe_rules::{self, GATE_MISSING, GIT_MISSING, INTEGRATION_OFF}};

pub(super) const WEEKS: usize = 8;

#[derive(Default)]
pub(super) struct Week {
    pub end: i64,
    pub covered: bool,
    pub available: bool,
    pub done: f64,
    pub features: f64,
    untyped: f64,
    bugs: [f64; 3],
    pub failures: f64,
    integration_reds: Option<f64>,
    releases: Option<f64>,
    patches: Option<f64>,
    nightly_reds: Option<f64>,
    release_days: Vec<f64>,
    unreleased: Option<f64>,
    integration_on: bool,
    lead_days: Vec<f64>,
    pub todo: f64,
    todo_ages: Vec<f64>,
    net: BTreeMap<String, f64>,
    archived: f64,
    pub load: Load,
    pub operations: [f64; 4],
    strategic: [f64; 4],
    unknown_answers: f64,
    pub schedule_events: f64,
    pub launches: f64,
}

impl Week {
    pub fn net_growth(&self) -> f64 {
        self.net.values().sum::<f64>().max(0.0)
    }

    pub fn excess_lead_days(&self, baseline: f64) -> f64 {
        self.lead_days.iter().map(|days| (days - baseline).max(0.0)).sum()
    }

    pub fn lead_days(&self) -> f64 {
        self.lead_days.iter().sum()
    }
}

pub(super) fn read(tx: &Transaction, project: &str, at: i64, first: i64) -> Result<Vec<Week>, String> {
    let integration = catalog::integration_on(tx, project)?;
    let git = probes::read::<Releases>(tx, project, probes::GIT, at)?.map(|(_, data)| data);
    let gate = probes::read::<Gate>(tx, project, probes::GATE, at)?.map(|(_, data)| data);
    (0..WEEKS).rev().map(|weeks_ago| {
        let end = at - weeks_ago as i64 * WEEK;
        let start = Window::week(end).start;
        let window = Window { before: start.max(first), start: start.max(first), end };
        let mut week = collect(tx, project, window, start >= first, integration)?;
        if let Some(git) = &git {
            let (releases, patches) = probe_rules::release_counts(git, start, end);
            week.releases = Some(releases);
            week.patches = Some(patches);
            if integration {
                week.release_days.extend(probe_rules::release_days(git, start, end));
                if !git.shallow {
                    week.unreleased = Some(git.tasks.values().filter(|t| t.first_tag.is_none()
                        && (start..=end).contains(&t.landed)).count() as f64);
                }
            }
        }
        if let Some(gate) = &gate { week.nightly_reds = probe_rules::nightly_reds(gate, start, end); }
        Ok(week)
    }).collect()
}

fn collect(tx: &Transaction, project: &str, window: Window, covered: bool, integration_on: bool) -> Result<Week, String> {
    let (start, end) = (window.start, window.end);
    let mut week = Week { end, covered, integration_on, available: start <= end, ..Week::default() };
    if !week.available {
        return Ok(week);
    }
    let events = project_events();
    let params = (project, end, start);
    let count = |statement: &str| tx.query_row(statement, params, |r| r.get(0)).map_err(sql);
    let changes = rows(tx, &format!("{events}, {FIRST_DONE}
        SELECT 1, t.type, t.severity, NULL FROM project_events e JOIN tasks t ON t.id = e.task
            WHERE e.at >= ?3 AND e.kind = 'made'
        UNION ALL SELECT 2, t.type, t.severity, CASE WHEN t.created_at > 0 AND t.created_at <= e.at
            AND NOT EXISTS (SELECT 1 FROM events p WHERE p.task = t.id AND p.kind = 'project')
            THEN (e.at - t.created_at) / 86400.0 END
            FROM first_done e JOIN tasks t ON t.id = e.task WHERE e.at >= ?3"),
        params, |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, String>(2)?, r.get::<_, Option<f64>>(3)?)))?;
    for (kind, task_type, priority, days) in changes {
        let task_type = task_type.as_deref().unwrap_or("untyped");
        let made = kind == 1;
        *week.net.entry(task_type.into()).or_default() += if made { 1.0 } else { -1.0 };
        if made && task_type == "Bug" {
            week.bugs[0] += 1.0;
            match priority.as_str() {
                "P0" => week.bugs[1] += 1.0,
                "P1" => week.bugs[2] += 1.0,
                _ => (),
            }
        }
        if !made {
            week.done += 1.0;
            week.features += f64::from(task_type == "Feature");
            week.untyped += f64::from(task_type == "untyped");
            week.lead_days.extend(days);
        }
    }
    week.failures = count(&format!("{events} SELECT count(*) FROM project_events
        WHERE at >= ?3 AND kind = 'move' AND json_extract(data, '$.to') = 'FAILED'"))?;
    if integration_on {
        week.integration_reds = Some(count("SELECT count(*) FROM integration_rounds r JOIN integration_batches b ON b.id = r.batch
            WHERE b.project = ?1 AND r.ended BETWEEN ?3 AND ?2 AND r.result = 'red'")?);
    }
    week.schedule_events = count("SELECT coalesce(sum(1 + missed), 0) FROM schedule_fires f JOIN schedules s ON s.id = f.schedule
        WHERE s.project = ?1 AND f.at BETWEEN ?3 AND ?2")?;
    week.launches = count(&format!("SELECT count(DISTINCT r.id) FROM runs r JOIN inputs i ON i.run = r.id
        WHERE r.project = ?1 AND {} AND i.at BETWEEN ?3 * 1000 AND ?2 * 1000 + 999", super::flow::LAUNCH))?;
    backlog(tx, project, start, &mut week, &events)?;
    owner(tx, project, start, &mut week)?;
    Ok(week)
}

fn backlog(tx: &Transaction, project: &str, start: i64, week: &mut Week, events: &str) -> Result<(), String> {
    let params = (project, week.end, start);
    // The generated legacy archive record has no structured direction. Match the entire
    // archived sentence; the paired restore sentence cannot match it.
    let archived = "e.kind = 'archive' AND e.text = 'Archived by ' || coalesce((SELECT name FROM users WHERE id = e.author), '')";
    let todo = rows(tx, &format!("{events}
        SELECT ( ?2 - t.created_at ) / 86400.0 FROM tasks t
        WHERE t.project = ?1 AND t.created_at BETWEEN 1 AND ?2
        AND NOT EXISTS (SELECT 1 FROM events p WHERE p.task = t.id AND p.kind = 'project')
        AND coalesce(
            (SELECT ({archived}) FROM project_events e WHERE e.task = t.id AND e.kind = 'archive' ORDER BY e.seq DESC LIMIT 1),
            t.archived_at <= ?2, 0) = 0
        AND coalesce(
            (SELECT json_extract(e.data, '$.to') FROM project_events e WHERE e.task = t.id AND e.kind = 'move' ORDER BY e.seq DESC LIMIT 1),
            (SELECT json_extract(e.data, '$.from') FROM events e WHERE e.task = t.id AND e.kind = 'move' AND e.at > ?2 ORDER BY e.rowid LIMIT 1),
            t.status) = 'TODO'"), (project, week.end), |r| r.get(0))?;
    week.todo = todo.len() as f64;
    week.todo_ages = todo;
    week.archived = tx.query_row(&format!("{events}, {FIRST_DONE}
        SELECT count(DISTINCT e.task) FROM project_events e WHERE e.at >= ?3 AND ({archived})
        AND NOT EXISTS (SELECT 1 FROM first_done d WHERE d.task = e.task AND d.at <= e.at)"), params, |r| r.get(0)).map_err(sql)?;
    Ok(())
}

fn owner(tx: &Transaction, project: &str, start: i64, week: &mut Week) -> Result<(), String> {
    week.load = owner_load(tx, project, start, week.end)?;
    for (kind, _, _) in owner_actions(tx, project, start, week.end)? {
        match kind {
            OwnerAction::OperationalAnswer => week.operations[0] += 1.0,
            OwnerAction::UnknownAnswer => week.unknown_answers += 1.0,
            OwnerAction::StrategicAnswer => week.strategic[2] += 1.0,
            OwnerAction::DesignSignoff => week.strategic[3] += 1.0,
            OwnerAction::Run => week.operations[1] += 1.0,
            OwnerAction::Move => week.operations[2] += 1.0,
            OwnerAction::Edit => week.operations[3] += 1.0,
            OwnerAction::Priority => week.strategic[0] += 1.0,
            OwnerAction::PlanWork => week.strategic[1] += 1.0,
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Verdict { Better, Flat, Worse, Unmeasured }

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Self::Better => "better",
            Self::Flat => "flat",
            Self::Worse => "worse",
            Self::Unmeasured => "not measured yet",
        }
    }
}

type Measure = fn(&[Week]) -> Option<f64>;

struct Metric {
    name: &'static str,
    compare: Option<bool>,
    read: Measure,
    missing: &'static str,
    probe: bool,
}

impl Metric {
    fn compare(name: &'static str, higher_is_better: bool, read: Measure) -> Self {
        Self { name, compare: Some(higher_is_better), read, missing: "insufficient history or denominator", probe: false }
    }

    fn context(name: &'static str, read: Measure) -> Self {
        Self { compare: None, ..Self::compare(name, false, read) }
    }

    fn because(mut self, reason: &'static str) -> Self {
        self.missing = reason;
        self
    }

    fn probe(mut self) -> Self {
        self.probe = true;
        self
    }

    fn measure(self, weeks: &[Week]) -> Line {
        Self::line(self.name, self.compare, self.missing, weeks, self.probe, self.read)
    }

    fn line(name: &'static str, comparison: Option<bool>, missing: &'static str, weeks: &[Week], probe: bool, read: impl Fn(&[Week]) -> Option<f64>) -> Line {
        let measure = |weeks: &[Week]| (probe || weeks.iter().all(|w| w.covered)).then(|| read(weeks)).flatten();
        let (before, current) = (measure(&weeks[..4]), measure(&weeks[4..]));
        let series: Vec<_> = weeks.iter().map(|week| measure(std::slice::from_ref(week))).collect();
        let noise = absolute_noise(&series);
        let verdict = compare(before, current, noise, comparison);
        Line { name, before, current, noise, verdict, missing, context: comparison.is_none() }
    }
}

pub(super) struct Line {
    pub name: &'static str,
    pub before: Option<f64>,
    pub current: Option<f64>,
    pub noise: f64,
    pub verdict: Verdict,
    pub missing: &'static str,
    pub context: bool,
}

pub(super) struct Indicator {
    pub name: &'static str,
    pub guard: &'static str,
    pub verdict: Verdict,
    pub lines: Vec<Line>,
}

fn indicator(name: &'static str, guard: &'static str, weeks: &[Week], metrics: Vec<Metric>) -> Indicator {
    let lines: Vec<_> = metrics.into_iter().map(|m| m.measure(weeks)).collect();
    let any = |v| lines.iter().any(|l| !l.context && l.verdict == v);
    let verdict = if any(Verdict::Worse) { Verdict::Worse }
        else if any(Verdict::Better) { Verdict::Better }
        else if any(Verdict::Flat) { Verdict::Flat }
        else { Verdict::Unmeasured };
    Indicator { name, guard, verdict, lines }
}

pub(super) fn compare(before: Option<f64>, current: Option<f64>, noise: f64, comparison: Option<bool>) -> Verdict {
    match (before.zip(current), comparison) {
        (Some((old, now)), Some(up)) if (now - old).abs() > noise + 1e-9 =>
            if (now > old) == up { Verdict::Better } else { Verdict::Worse },
        (Some(_), Some(_)) => Verdict::Flat,
        _ => Verdict::Unmeasured,
    }
}

pub(super) fn guarded(verdict: Verdict, guard: Verdict) -> Verdict {
    match (verdict, guard) {
        (Verdict::Better, Verdict::Worse) => Verdict::Worse,
        (Verdict::Better, Verdict::Unmeasured) => Verdict::Unmeasured,
        _ => verdict,
    }
}

fn mean(weeks: &[Week], extract: impl Fn(&Week) -> f64) -> Option<f64> {
    Some(weeks.iter().map(extract).sum::<f64>() / weeks.len() as f64)
}

fn total(weeks: &[Week], extract: impl Fn(&Week) -> Option<f64>) -> Option<f64> {
    weeks.iter().map(extract).sum()
}

fn ratio(weeks: &[Week], numerator: impl Fn(&Week) -> f64, denominator: impl Fn(&Week) -> f64) -> Option<f64> {
    let total: f64 = weeks.iter().map(denominator).sum();
    (total > 0.0).then(|| weeks.iter().map(numerator).sum::<f64>() / total)
}

pub(super) fn indicators(weeks: &[Week]) -> [Indicator; 7] {
    use Metric as M;
    let mut features = indicator("Features per owner hour", "Stability", weeks, vec![
        M::compare("Feature tasks DONE per active hour", true, |w| ratio(w, |x| x.features, |x| x.load.hours)),
        M::context("Untyped share of DONE tasks", |w| ratio(w, |x| x.untyped, |x| x.done)),
    ]);
    let stability = indicator("Stability", "Reference guard", weeks, vec![
        M::compare("Bugs opened per week", false, |w| mean(w, |x| x.bugs[0])),
        M::compare("P0 Bugs opened per week", false, |w| mean(w, |x| x.bugs[1])),
        M::compare("P1 Bugs opened per week", false, |w| mean(w, |x| x.bugs[2])),
        M::compare("FAILED share of DONE and FAILED transitions", false, |w| ratio(w, |x| x.failures, |x| x.done + x.failures)),
        M::compare("Red integration rounds per week", false, |w| total(w, |x| x.integration_reds).map(|n| n / w.len() as f64))
            .because(INTEGRATION_OFF),
        M::compare("Nightly reds", false, |w| total(w, |x| x.nightly_reds)).because(GATE_MISSING).probe(),
        M::compare("Patches within three days", false, |w| total(w, |x| x.patches)).because(GIT_MISSING).probe(),
    ]);
    features.verdict = guarded(features.verdict, stability.verdict);
    let lead_missing = if !weeks.last().is_some_and(|w| w.integration_on) { INTEGRATION_OFF }
        else if weeks.last().is_none_or(|w| w.releases.is_none()) { GIT_MISSING }
        else { "No measured tasks with a containing release in this window." };
    let mut lead = indicator("Lead time", "Stability", weeks, vec![
        M::compare("Median task-days to first DONE", false, |w| percentile(&mut w.iter().flat_map(|x| x.lead_days.iter().copied()).collect::<Vec<_>>(), 50)),
        M::compare("Median task-days to first release", false, |w|
            percentile(&mut w.iter().flat_map(|x| x.release_days.iter().copied()).collect::<Vec<_>>(), 50)).because(lead_missing).probe(),
        M::context("Unreleased tasks with known commits", |w| total(w, |x| x.unreleased))
            .because(if lead_missing == INTEGRATION_OFF || lead_missing == GIT_MISSING { lead_missing }
                else { "A complete Git history is required to confirm unreleased commits." }).probe(),
    ]);
    lead.verdict = guarded(lead.verdict, stability.verdict);
    let mut release = indicator("Release cadence", "Stability", weeks, vec![
        M::compare("Releases per month", true, |w| total(w, |x| x.releases)
            .map(|n| n * probe_rules::MONTH / (7 * w.len()) as f64)).because(GIT_MISSING).probe(),
    ]);
    release.verdict = guarded(release.verdict, stability.verdict);
    let mut backlog = indicator("Backlog", "Archived without DONE is counted separately", weeks, vec![
        M::compare("TODO tasks (weekly snapshots)", false, |w| mean(w, |x| x.todo)),
        M::compare("Median TODO age in days", false, |w| percentile(&mut w.iter().flat_map(|x| x.todo_ages.iter().copied()).collect::<Vec<_>>(), 50)),
        M::compare("Tasks made minus first DONE per week", false, |w| mean(w, |x| x.net.values().sum())),
        M::compare("Archived without DONE per week", false, |w| mean(w, |x| x.archived)),
    ]);
    for kind in crate::tasks::attrs::TYPES.map(|(kind, _)| kind).into_iter().chain(["untyped"]) {
        backlog.lines.push(M::line(kind, None, "insufficient history", weeks, false,
            |range| mean(range, |week| week.net.get(kind).copied().unwrap_or_default())));
    }
    let archived = backlog.lines[3].verdict;
    backlog.lines[3].context = true;
    if backlog.verdict == Verdict::Better && backlog.lines[..3].iter().all(|line| line.verdict != Verdict::Better) {
        backlog.verdict = Verdict::Flat;
    }
    backlog.verdict = guarded(backlog.verdict, archived);
    let mut load = indicator("Cognitive load", "Features per owner hour", weeks, vec![
        M::compare("Switches per active hour", false, |w| ratio(w, |x| x.load.switch_count, |x| x.load.hours)),
        M::compare("Active hours per week", false, |w| mean(w, |x| x.load.hours)),
        M::compare("Peak concurrent sessions", false, |w| w.iter().map(|x| x.load.peak).reduce(f64::max)),
        M::compare("Off-hours share", false, |w| ratio(w, |x| x.load.off_hours, |x| x.load.hours)),
    ]);
    load.verdict = guarded(load.verdict, features.verdict);
    let mut operations = indicator("Operational and strategic work", "Owner retains strategic acts", weeks, vec![
        M::compare("Owner operations per DONE task", false, |w| ratio(w, |x| x.operations.iter().sum(), |x| x.done)),
        M::context("Operational answers per week", |w| mean(w, |x| x.operations[0])),
        M::context("Runs per week", |w| mean(w, |x| x.operations[1])),
        M::context("Moves per week", |w| mean(w, |x| x.operations[2])),
        M::context("Edits per week", |w| mean(w, |x| x.operations[3])),
        M::compare("Strategic acts per week", true, |w| mean(w, |x| x.strategic.iter().sum())),
        M::context("Owner priority settings per week", |w| mean(w, |x| x.strategic[0])),
        M::context("Owner Feature and Story creation per week", |w| mean(w, |x| x.strategic[1])),
        M::context("Strategic answers per week", |w| mean(w, |x| x.strategic[2] + x.strategic[3])),
        M::context("Design sign-offs per week", |w| mean(w, |x| x.strategic[3])),
        M::context("Answers without a recorded kind per week", |w| mean(w, |x| x.unknown_answers)),
        M::context("Share of answers without a recorded kind", |w|
            ratio(w, |x| x.unknown_answers, |x| x.operations[0] + x.strategic[2] + x.strategic[3] + x.unknown_answers)),
    ]);
    let strategic = operations.lines[5].verdict;
    operations.verdict = if strategic == Verdict::Worse { Verdict::Worse }
        else { guarded(operations.lines[0].verdict, strategic) };
    [features, stability, lead, release, backlog, load, operations]
}

pub(super) fn period_improved(indicators: &[Indicator]) -> bool {
    indicators.iter().any(|i| i.verdict == Verdict::Better) && indicators.iter().all(|i| i.verdict != Verdict::Worse)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_load() -> Load {
        let mut load = Load::default();
        load.hours = 10.0;
        load
    }

    #[test]
    fn ac214_scoreboard_guard_verdicts() {
        let mut weeks: Vec<_> = (0..8).map(|i| Week {
            covered: true, integration_on: true, integration_reds: Some(0.0),
            features: if i < 4 { 2.0 } else { 5.0 },
            done: 10.0, load: test_load(), ..Week::default()
        }).collect();
        let board = indicators(&weeks);
        assert_eq!(board.len(), 7);
        assert_eq!((board[0].lines[0].before, board[0].lines[0].current), (Some(0.2), Some(0.5)));
        assert_eq!(board[0].verdict, Verdict::Better);
        assert_eq!(board[3].verdict, Verdict::Unmeasured);
        assert!(board[3].lines[0].missing.to_lowercase().contains("release tags"));
        assert!(period_improved(&board));

        for week in &mut weeks[4..] {
            week.integration_reds = Some(3.0);
        }
        let board = indicators(&weeks);
        assert_eq!(board[0].guard, "Stability");
        assert_eq!(board[0].lines[0].verdict, Verdict::Better);
        assert_eq!(board[0].verdict, Verdict::Worse);
        assert_eq!(board[1].verdict, Verdict::Worse);
        assert_eq!(guarded(Verdict::Better, Verdict::Worse), Verdict::Worse);
        assert!(!period_improved(&board));

        weeks[0].covered = false;
        let short = indicators(&weeks);
        assert_eq!(short[0].verdict, Verdict::Unmeasured);
        assert_eq!(short[0].lines[0].before, None);
        assert_eq!(short[0].lines[0].current, Some(0.5));
    }
}
