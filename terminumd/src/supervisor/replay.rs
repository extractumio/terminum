// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-215: read-only replay over a fixed project history snapshot.

use std::{collections::BTreeMap, fmt::Write, path::Path};

use rusqlite::{OptionalExtension, Transaction};

use crate::tasks::{ops::{check_project, sql}, store::Store};

use super::{Detector, Evaluation, EvaluationInputs, Finding, Rule, SCHEMA, WEEK, Window, collect_findings, evaluate_snapshot, measure_detector, flow::history_bounds, scoreboard, settings};

pub(crate) fn replay(path: &Path, project: &str, as_of: Option<i64>) -> Result<String, String> {
    if as_of.is_some_and(|at| !(1..i64::MAX / 1000 - WEEK).contains(&at)) {
        return Err("Invalid Supervisor replay timestamp.".into());
    }
    Store::snapshot_path(path, SCHEMA, |tx| {
        check_project(tx, project)?;
        let (first, latest) = history_bounds(tx, project, as_of.unwrap_or(i64::MAX / 1000 - WEEK))?;
        let at = as_of.unwrap_or(latest);
        let first = first.min(at);
        let current_state = as_of.is_none();
        let weeks = scoreboard::read(tx, project, at, first)?;
        let settings = settings::read(tx, project)?.settings;
        let detectors = Rule::ALL.iter().map(|&rule| settings.detector(rule)).collect::<Vec<_>>();
        let inputs = EvaluationInputs { first: Some(first), weeks: Some(&weeks), current_state, observe_thresholds: false };
        let evaluation = evaluate_snapshot(tx, project, at, &detectors, inputs)?;
        let mut historical = BTreeMap::new();
        visit_history(tx, project, first, at, &detectors, |rule, _| {
            *historical.entry(rule).or_default() += 1;
            Ok(())
        })?;
        render(tx, &evaluation, &weeks, first, &historical)
    })
}

pub(super) fn visit_history(
    tx: &Transaction, project: &str, first: i64, at: i64, detectors: &[Detector],
    mut visit: impl FnMut(Rule, &Finding) -> Result<(), String>,
) -> Result<(), String> {
    let mut end = at - 2 * WEEK;
    while end >= first && end > 0 {
        for detector in detectors.iter().filter(|detector| detector.rule.has_historical_query()) {
            let window = Window::of(end, detector.window_days);
            let mut data = measure_detector(tx, project, detector, window, false)?;
            for finding in collect_findings(&mut data, detector, window, false) {
                visit(detector.rule, &finding)?;
            }
        }
        end -= WEEK;
    }
    Ok(())
}

pub(super) fn firing_key(project: &str, rule: Rule, finding: &Finding) -> Result<String, String> {
    // A standing finding backed by the same evidence at several cutoffs counts once for holds.
    let key = serde_json::to_vec(&(project, rule.id(), finding.key(), &finding.evidence))
        .map_err(|_| "Could not encode Supervisor firing evidence.")?;
    Ok(crate::auth::crypto::sha256_hex(&key))
}

fn subject(tx: &Transaction, f: &Finding) -> Result<String, String> {
    if f.subject_kind == "task" {
        let slug: Option<String> = tx.query_row("SELECT coalesce(slug, id) FROM tasks WHERE id = ?1",
            [&f.subject_id], |r| r.get(0)).optional().map_err(sql)?;
        if let Some(slug) = slug {
            return Ok(slug.escape_default().to_string());
        }
    }
    Ok(f.subject_id.escape_default().to_string())
}

fn render(tx: &Transaction, result: &Evaluation, weeks: &[scoreboard::Week], first: i64, historical: &BTreeMap<Rule, usize>) -> Result<String, String> {
    let mut output = String::new();
    writeln!(output, "Supervisor replay for {} (UTC Unix seconds, retained from {first}, as of {})",
        result.project.escape_default(), result.at).unwrap();
    if result.at - first < 3 * WEEK {
        writeln!(output, "Limited history: ordinary impact capped at 1.").unwrap();
    }

    let (ranked, inactive) = result.ranked_findings();
    writeln!(output, "\nRanked findings ({}, {inactive} past-window findings outside rank)", ranked.len()).unwrap();
    for (i, (rule, f)) in ranked.iter().enumerate() {
        let m = &f.measure;
        writeln!(output, "{}. {rule} {} {}: {:.3} (baseline {:.3}, threshold {:.3}, window {}–{}); impact {}, level {}, response {}",
            i + 1, subject(tx, f)?, f.metric, m.value, m.baseline, m.threshold, m.window_start, m.window_end,
            f.scores.impact, f.scores.level(), f.scores.response(false)).unwrap();
        if f.assessment.shares.is_empty() {
            writeln!(output, "   No attributable weekly project share.").unwrap();
        } else {
            for share in &f.assessment.shares {
                writeln!(output, "   Week ending {}: {:.3}/{:.3} {:?} ({:.2}%, weekly noise {:.2}%{})",
                    share.week, share.amount, share.total, share.unit, share.fraction() * 100.0, share.noise * 100.0,
                    if share.complete { "" } else { ", partial week" }).unwrap();
            }
        }
        let preview = serde_json::to_string(&f.evidence[..f.evidence.len().min(3)]).map_err(|e| e.to_string())?;
        writeln!(output, "   Evidence: {} retained references; first: {preview}", f.evidence.len()).unwrap();
    }

    writeln!(output, "\nHistorical firings (detector detections at weekly cutoffs before the last two weeks)").unwrap();
    for (rule, count) in historical {
        writeln!(output, "- {}: {count}", rule.id()).unwrap();
    }

    let mut gaps: Vec<_> = result.detectors.iter().flat_map(|d| d.unavailable.iter().map(|why| (d.rule.id(), *why)))
        .chain(super::catalog::UNAVAILABLE.iter().copied().filter(|(id, _)| Rule::named(id).is_err())).collect();
    gaps.sort_unstable();
    writeln!(output, "\nUnavailable detector inputs").unwrap();
    for (rule, why) in gaps {
        writeln!(output, "- {rule}: {why}").unwrap();
    }

    let indicators = scoreboard::indicators(weeks);
    writeln!(output, "\nImprovement scoreboard (preceding four weeks → trailing four weeks; current retained task labels)").unwrap();
    for indicator in &indicators {
        writeln!(output, "- {}: {}. Guard: {}.", indicator.name, indicator.verdict.label(), indicator.guard).unwrap();
        for line in &indicator.lines {
            let print = |v: Option<f64>| v.map_or_else(|| "not measured yet".into(), |n| format!("{n:.3}"));
            write!(output, "  {}: {} → {}", line.name, print(line.before), print(line.current)).unwrap();
            if line.before.is_none() || line.current.is_none() {
                write!(output, " ({})", line.missing).unwrap();
            } else if !line.context {
                write!(output, " ({}, weekly noise {:.3})", line.verdict.label(), line.noise).unwrap();
            }
            writeln!(output).unwrap();
        }
    }
    let outcome = if indicators.iter().all(|indicator| indicator.verdict == scoreboard::Verdict::Unmeasured) {
        "not measured yet"
    } else if scoreboard::period_improved(&indicators) {
        "yes"
    } else {
        "no"
    };
    writeln!(output, "Period improved: {outcome}.").unwrap();
    Ok(output)
}
