// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-221: a project-scoped attach report and the gate for live-at-attach holds.

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::tasks::{integration::local_dir, ops::{project_place, rows, sql}};

use super::{Detector, Evaluation, Mode, Rule, Threshold, WEEK, catalog, replay, settings::Settings};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub(super) struct Range {
    min: f64,
    max: f64,
}

impl Range {
    pub fn new(limit: f64) -> Self {
        Self { min: limit, max: limit }
    }

    pub fn include(&mut self, limit: f64) {
        self.min = self.min.min(limit);
        self.max = self.max.max(limit);
    }
}

#[derive(Serialize, Deserialize)]
pub(super) struct Report {
    attached_at: i64,
    scanned_at: i64,
    first_observation: Option<i64>,
    limited_history: bool,
    code_only: bool,
    sources: Sources,
    detectors: BTreeMap<String, Scan>,
}

#[derive(Serialize, Deserialize)]
struct Sources {
    integration: bool,
    code_host: bool,
    pool_agents: i64,
    routines: i64,
}

#[derive(Serialize, Deserialize)]
struct Scan {
    mode: Mode,
    can_run: bool,
    unavailable: Vec<String>,
    threshold: Option<Threshold>,
    observed_thresholds: BTreeMap<String, Range>,
    replay_firings: usize,
    holds: Gate,
}

#[derive(Clone, Copy, Default, Serialize, Deserialize)]
struct Gate {
    live_at_attach: bool,
    replay_judged: u64,
    replay_right: u64,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HoldState {
    Inactive,
    Shadow,
    Eligible,
}

impl Gate {
    fn state(self, elapsed: i64, charter_breach: bool) -> HoldState {
        if !self.live_at_attach {
            return HoldState::Inactive;
        }
        if charter_breach || elapsed >= WEEK
            || (self.replay_judged >= 3 && self.replay_right >= self.replay_judged - self.replay_judged / 3)
        {
            HoldState::Eligible
        } else {
            HoldState::Shadow
        }
    }
}

impl Report {
    pub(super) fn mode(&self, detector: &Detector) -> Mode {
        if detector.mode == Mode::Live && self.detectors.get(detector.rule.id()).is_none_or(|scan| !scan.holds.live_at_attach) {
            Mode::Shadow
        } else {
            detector.mode
        }
    }

    /// A later hold caller passes a verified charter breach, never a claim from imported settings.
    pub(crate) fn hold_state(&self, rule: Rule, at: i64, charter_breach: bool) -> HoldState {
        self.detectors.get(rule.id()).filter(|scan| scan.mode == Mode::Live && scan.can_run)
            .map_or(HoldState::Inactive, |scan| scan.holds.state(at.saturating_sub(self.attached_at), charter_breach))
    }

    pub(super) fn status(&self, at: i64, active: bool) -> Result<serde_json::Value, String> {
        let mut view = serde_json::to_value(self).map_err(|_| "Could not encode Supervisor attachment status.")?;
        for &rule in Rule::ALL {
            if let Some(detector) = view["detectors"].get_mut(rule.id()) {
                let state = if active { self.hold_state(rule, at, false) } else { HoldState::Inactive };
                detector["hold_state"] = serde_json::to_value(state)
                    .map_err(|_| "Could not encode Supervisor hold state.")?;
            }
        }
        Ok(view)
    }
}

pub(super) fn read(tx: &Transaction, project: &str) -> Result<Option<Report>, String> {
    let json: Option<String> = tx.query_row("SELECT attachment FROM project_supervisor WHERE project = ?1",
        [project], |r| r.get::<_, Option<String>>(0)).optional().map_err(sql)?.flatten();
    json.map(|json| serde_json::from_str(&json).map_err(|_| "Invalid saved Supervisor attachment.")).transpose().map_err(Into::into)
}

/// Turning a detector off retires live-at-attach authority even between collector beats.
pub(super) fn retire_live(tx: &Transaction, project: &str, rules: &[Rule]) -> Result<(), String> {
    if rules.is_empty() {
        return Ok(());
    }
    for rule in rules {
        let path = format!(r#"$.detectors."{}".holds.live_at_attach"#, rule.id());
        tx.execute("UPDATE project_supervisor SET attachment = json_replace(attachment, ?2, json('false'))
            WHERE project = ?1 AND attachment IS NOT NULL", (project, path)).map_err(sql)?;
    }
    Ok(())
}

pub(super) fn scan(
    tx: &Transaction, evaluation: &mut Evaluation, settings: &Settings, detectors: &[Detector], previous: Option<&Report>, bounds: (i64, i64),
) -> Result<Report, String> {
    let project = &evaluation.project;
    let at = evaluation.at;
    let attached_at = previous.map_or(at, |report| report.attached_at);
    let (first, latest) = bounds;
    let mut firings = BTreeMap::<Rule, std::collections::BTreeSet<String>>::new();
    replay::visit_history(tx, project, first, attached_at, detectors, |rule, finding| {
        firings.entry(rule).or_default().insert(replay::firing_key(project, rule, finding)?);
        Ok(())
    })?;
    let verdicts = rows(tx, "SELECT detector, firing, was_right FROM supervisor_hold_verdicts
        WHERE project = ?1 AND fired_at < ?2", (project, at),
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, bool>(2)?)))?;
    let mut judged = BTreeMap::<Rule, (u64, u64)>::new();
    for (detector, firing, right) in verdicts {
        if let Ok(rule) = Rule::named(&detector)
            && rule.id() == detector
            && firings.get(&rule).is_some_and(|keys| keys.contains(&firing))
        {
            let stats = judged.entry(rule).or_default();
            stats.0 += 1;
            stats.1 += u64::from(right);
        }
    }

    let mut scans = BTreeMap::new();
    for (detector, checked) in detectors.iter().zip(&mut evaluation.detectors) {
        let rule = detector.rule;
        let can_run = detector.mode != Mode::Off && (checked.unavailable.is_empty()
            || !checked.findings.is_empty() || !checked.complete_metrics.is_empty());
        let old = previous.and_then(|report| report.detectors.get(rule.id()));
        let live_at_attach = old.is_some_and(|scan| scan.holds.live_at_attach)
            || (previous.is_none() && can_run && detector.mode == Mode::Live);
        if checked.mode == Mode::Live && !live_at_attach {
            checked.mode = Mode::Shadow;
        }
        let (replay_judged, replay_right) = judged.get(&rule).copied().unwrap_or_default();
        scans.insert(rule.id().into(), Scan {
            mode: checked.mode,
            can_run,
            unavailable: checked.unavailable.iter().map(|reason| (*reason).into()).collect(),
            threshold: Some(detector.threshold),
            observed_thresholds: checked.thresholds.iter().map(|(metric, band)| ((*metric).into(), *band)).collect(),
            replay_firings: firings.get(&rule).map_or(0, |found| found.len()),
            holds: Gate { live_at_attach, replay_judged, replay_right },
        });
    }
    for &(name, reason) in catalog::UNAVAILABLE {
        scans.entry(name.into()).or_insert_with(|| Scan {
            mode: Mode::Off, can_run: false, unavailable: vec![reason.into()], threshold: None,
            observed_thresholds: BTreeMap::new(), replay_firings: 0, holds: Gate::default(),
        });
    }
    scans.insert(Rule::CollectorDead.id().into(), Scan {
        mode: Mode::Live, can_run: true, unavailable: vec![], threshold: Some(Rule::CollectorDead.defaults()),
        observed_thresholds: BTreeMap::new(), replay_firings: 0, holds: Gate::default(),
    });

    Ok(Report { attached_at, scanned_at: at, first_observation: (latest > 0).then_some(first),
        limited_history: at - first < 3 * WEEK, code_only: settings.tiers.is_empty(),
        sources: sources(tx, project)?, detectors: scans })
}

fn sources(tx: &Transaction, project: &str) -> Result<Sources, String> {
    let (integration, pool_agents, routines) = tx.query_row("SELECT
        EXISTS(SELECT 1 FROM project_integration WHERE project = ?1 AND is_on = 1),
        (SELECT count(*) FROM agents WHERE project = ?1),
        (SELECT count(*) FROM schedules WHERE project = ?1 AND deleted_at IS NULL)",
        [project], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(sql)?;
    let (_, host, dir) = project_place(tx, project)?;
    // Local inventory only. Git receives no remote URL, and no command or network probe runs.
    let code_host = host.is_none() && !dir.trim().is_empty() && crate::integrate::git::run(&local_dir(&dir), &["remote"])
        .is_ok_and(|remotes| !remotes.trim().is_empty());
    Ok(Sources { integration, code_host, pool_agents, routines })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ac221_attach_short_history_and_hold_shadow() {
        let gate = |replay_judged, replay_right| Gate { live_at_attach: true, replay_judged, replay_right };
        assert_eq!(gate(2, 2).state(WEEK - 1, false), HoldState::Shadow);
        assert_eq!(gate(3, 1).state(0, false), HoldState::Shadow);
        assert_eq!(gate(3, 2).state(0, false), HoldState::Eligible);
        assert_eq!(gate(4, 2).state(0, false), HoldState::Shadow);
        assert_eq!(gate(4, 3).state(0, false), HoldState::Eligible);
        assert_eq!(gate(0, 0).state(WEEK, false), HoldState::Eligible);
        assert_eq!(gate(0, 0).state(0, true), HoldState::Eligible);
        assert_eq!(Gate::default().state(WEEK, true), HoldState::Inactive);
    }
}
