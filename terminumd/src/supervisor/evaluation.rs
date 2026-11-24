// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial

use super::*;
#[cfg(test)]
use crate::tasks::ops::check_project;

/// Validate every rule before opening the store. The catalog owns all SQL and all identifiers used in SQL.
#[cfg(test)]
pub(crate) fn evaluate(store: &Store, project: &str, at: i64, detectors: &[Detector]) -> Result<Evaluation, String> {
    let mut rules = BTreeSet::new();
    for d in detectors {
        if !d.valid() || !rules.insert(d.rule) {
            return Err(format!("Invalid or duplicate Supervisor parameters for {}.", d.rule.id()));
        }
    }
    if !(0..i64::MAX / 1000 - DAY).contains(&at) {
        return Err("Invalid Supervisor evaluation time.".into());
    }
    store.snapshot(SCHEMA, |tx| {
        check_project(tx, project)?;
        evaluate_snapshot(tx, project, at, detectors, EvaluationInputs { current_state: true, ..EvaluationInputs::default() })
    })
}

pub(super) fn evaluate_snapshot(
    tx: &Transaction, project: &str, at: i64, detectors: &[Detector], inputs: EvaluationInputs<'_>,
) -> Result<Evaluation, String> {
    let EvaluationInputs { first, weeks, current_state, observe_thresholds } = inputs;
    let mut checked = Vec::new();
    for detector in detectors {
        let window = Window::of(at, detector.window_days);
        let mut data = measure_detector(tx, project, detector, window, current_state)?;
        let mut thresholds = BTreeMap::new();
        if observe_thresholds {
            for (metric, limit) in data.threshold_hint.into_iter().chain(data.readings.iter()
                .filter_map(|r| r.limits(detector.threshold).map(|(_, limit)| (r.metric, limit))))
            {
                thresholds.entry(metric).and_modify(|band: &mut attach::Range| band.include(limit))
                    .or_insert_with(|| attach::Range::new(limit));
            }
        }
        let measured = matches!(detector.rule, Rule::LimitNear | Rule::StalePlan | Rule::ProbeFailed).then(|| {
            data.readings.iter().map(|r| (r.subject_kind, r.subject_id.clone(), r.metric)).collect()
        });
        let observations = if current_state {
            data.readings.iter().map(|r| ((r.subject_kind.to_owned(), r.subject_id.clone(), r.metric.to_owned()),
                r.limits(detector.threshold).map(|_| (r.value, r.comparison)))).collect()
        } else { BTreeMap::new() };
        let mut findings = collect_findings(&mut data, detector, window, current_state);
        findings.sort_by(|a, b| a.key().cmp(&b.key()));
        checked.push(Checked {
            rule: detector.rule, mode: detector.mode, unavailable: data.unavailable,
            findings, thresholds, complete_metrics: data.complete_metrics, measured, observations,
        });
    }
    let mut result = Evaluation { project: project.into(), at, detectors: checked };
    if !result.detectors.iter().any(|d| !d.findings.is_empty()) {
        return Ok(result);
    }
    let first = match first { Some(first) => first, None => flow::history_bounds(tx, project, at)?.0 };
    let own_weeks;
    let weeks = if let Some(weeks) = weeks { weeks } else {
        own_weeks = scoreboard::read(tx, project, at, first)?;
        &own_weeks
    };
    let calculator = attribution::Attribution::read(tx, project, weeks)?;
    for (detector, checked) in detectors.iter().zip(&mut result.detectors) {
        if checked.findings.is_empty() {
            continue;
        }
        let samples = weeks.iter().map(|week| {
            if !week.available || !detector.rule.needs_weekly_samples() {
                return Ok(Measurements::default());
            }
            let window = Window::week(week.end);
            detector.rule.query(&Query { tx, project, detector, window, historical: true })
        }).collect::<Result<Vec<_>, _>>()?;
        let indexed = samples.iter().map(|week| {
            week.readings.iter().map(|reading| (reading.key(), reading)).collect::<BTreeMap<_, _>>()
        }).collect::<Vec<_>>();
        for found in &mut checked.findings {
            let readings = indexed.iter().map(|week| week.get(&found.key()).copied());
            found.assessment = calculator.assess(detector, found, readings);
            found.scores = Scores {
                impact: found.assessment.score(at - first < 3 * WEEK, found.charter_breach()),
                ..detector.scores
            };
        }
    }
    Ok(result)
}

pub(super) fn measure_detector(
    tx: &Transaction, project: &str, detector: &Detector, window: Window, current_state: bool,
) -> Result<Measurements, String> {
    let query = Query { tx, project, detector, window, historical: !current_state };
    if detector.mode == Mode::Off {
        Ok(Measurements::missing("Detector is off."))
    } else if let Some(why) = detector.rule.present_only().filter(|_| !current_state) {
        Ok(Measurements::missing(why))
    } else {
        detector.rule.query(&query)
    }
}

pub(super) fn collect_findings(data: &mut Measurements, detector: &Detector, window: Window, current_state: bool) -> Vec<Finding> {
    std::mem::take(&mut data.readings).into_iter()
        .filter(|reading| current_state || !matches!(reading.source, impact::Source::Present))
        .filter_map(|reading| reading.finding(detector.threshold, window))
        .map(|mut found| {
            found.charter_never = detector.never.clone().filter(|_| detector.rule.checked_fact().is_some());
            found
        }).collect()
}

