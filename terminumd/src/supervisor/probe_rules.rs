// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-260 and AC-261: detector queries over retained probe samples.

use std::collections::{BTreeMap, BTreeSet};

use super::{Comparison, DAY, Evidence, Measurements, Query, Reading, Threshold, catalog, percentile, percentile_of_others,
    impact::Source, probe_arch::Architecture, probe_gate::Gate, probe_git::Releases, probes};

pub(super) const GIT_MISSING: &str = "Git release tags are unavailable.";
pub(super) const ARCH_MISSING: &str = "Git architecture observations are unavailable.";
pub(super) const GATE_MISSING: &str = "Gate record history is unavailable for this window.";
pub(super) const INTEGRATION_OFF: &str = "Project integration is off.";
pub(super) const MONTH: f64 = 365.2425 / 12.0;
const CODE: &str = "A code-structure risk is in no weekly project total; the failures and returns it later causes are sized by flow.failed_repeat and flow.returned.";
const GATE: &str = "The gate's retries and nightly results are not task, owner or spend quantities.";
const RELEASES: &str = "Release timing and cadence measure the whole project, so its share of itself would always be all of it; delay to DONE is sized by flow.lead_time_shift.";

pub(super) fn release_counts(releases: &Releases, start: i64, end: i64) -> (f64, f64) {
    let releases_in_period = releases.tags.iter().filter(|tag| (start..=end).contains(&tag.at)).count();
    let patches = releases.tags.windows(2).filter(|pair| {
        let [previous, tag] = pair else { return false; };
        tag.patch && (start..=end).contains(&tag.at) && tag.at - previous.at <= 3 * DAY
    }).count();
    (releases_in_period as f64, patches as f64)
}

pub(super) fn release_days(releases: &Releases, start: i64, end: i64) -> impl Iterator<Item = f64> + '_ {
    releases.tasks.values().filter_map(move |task| {
        let tag = releases.tags.get(task.first_tag?)?;
        (start..=end).contains(&tag.at).then(|| (tag.at - task.created) as f64 / DAY as f64)
    })
}

pub(super) fn nightly_reds(gate: &Gate, start: i64, end: i64) -> Option<f64> {
    let since = gate.nightly_since?;
    (start >= since).then(|| gate.events.iter().filter(|e| e.nightly_red && (start..=end).contains(&e.at)).count() as f64)
}

fn evidence(id: i64) -> Vec<Evidence> { vec![Evidence::SupervisorProbes(id)] }

fn comparison(q: &Query<'_, '_>, id: i64, metric: &'static str, value: f64, old: f64, direction: Comparison) -> Reading {
    Reading::new("project", q.project, metric, value, evidence(id), Source::Unsized(RELEASES)).history(vec![old]).comparison(direction)
}

pub(super) fn release_time(q: &Query<'_, '_>) -> Result<Measurements, String> {
    if !catalog::integration_on(q.tx, q.project)? { return Ok(Measurements::missing(INTEGRATION_OFF)); }
    let Some((id, releases)) = probes::read::<Releases>(q.tx, q.project, probes::GIT, q.window.end)? else {
        return Ok(Measurements::missing(GIT_MISSING));
    };
    let median = |start, end| percentile(&mut release_days(&releases, start, end).collect::<Vec<_>>(), 50);
    let Some((old, now)) = median(q.window.before, q.window.start - 1).zip(median(q.window.start, q.window.end)) else {
        return Ok(Measurements::missing("Both windows need released tasks with known commits."));
    };
    Ok(super::flow::one(comparison(q, id, "median_task_days_to_release", now, old, Comparison::Above)))
}

pub(super) fn releases(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let Some((id, tags)) = probes::read::<Releases>(q.tx, q.project, probes::GIT, q.window.end)? else {
        return Ok(Measurements::missing(GIT_MISSING));
    };
    let (old, past_patches) = release_counts(&tags, q.window.before, q.window.start - 1);
    let (now, patches) = release_counts(&tags, q.window.start, q.window.end);
    let days = (q.window.end - q.window.start + 1) as f64 / DAY as f64;
    Ok(Measurements { readings: vec![
        comparison(q, id, "releases_per_month", now * MONTH / days, old * MONTH / days, Comparison::Below),
        comparison(q, id, "patches_within_three_days", patches, past_patches, Comparison::Above),
    ], ..Measurements::default() })
}

pub(super) fn flaky(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let Some((id, gate)) = probes::read::<Gate>(q.tx, q.project, probes::GATE, q.window.end)? else {
        return Ok(Measurements::missing(GATE_MISSING));
    };
    let mut out = Measurements::default();
    let retries: usize = gate.events.iter().filter(|e| (q.window.start..=q.window.end).contains(&e.at)).map(|e| e.retried).sum();
    if gate.since <= q.window.start || retries > 0 {
        out.readings.push(Reading::new("project", q.project, "gate_retries", retries as f64, evidence(id), Source::Unsized(GATE)));
    } else {
        out.unavailable.push(GATE_MISSING);
    }
    let reds = gate.events.iter().filter(|e| e.nightly_red && q.window.current(e.at)).count();
    if reds > 0 || gate.nightly_since.is_some_and(|since| since <= q.window.start) {
        out.readings.push(Reading::new("project", q.project, "nightly_reds", reds as f64, evidence(id), Source::Unsized(GATE)));
    } else {
        out.unavailable.push("Nightly rehearsal history is unavailable for this window.");
    }
    Ok(out)
}

pub(super) fn architecture(q: &Query<'_, '_>) -> Result<Option<(i64, Architecture)>, String> {
    probes::read(q.tx, q.project, probes::ARCH, q.window.end)
}

pub(super) fn migrations(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let Some((id, arch)) = architecture(q)? else { return Ok(Measurements::missing(ARCH_MISSING)); };
    let mut paths = BTreeMap::<String, f64>::new();
    for (at, path) in arch.migrations {
        if q.window.current(at) { *paths.entry(path).or_default() += 1.0; }
    }
    Ok(Measurements {
        readings: paths.into_iter().map(|(path, count)| Reading::new("file", path, "existing_migration_edits", count, evidence(id), Source::Unsized(CODE))).collect(),
        complete_metrics: (arch.since <= q.window.start).then_some("existing_migration_edits").into_iter().collect(),
        ..Measurements::default()
    })
}

pub(super) fn dependencies(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let Some((id, arch)) = architecture(q)? else { return Ok(Measurements::missing(ARCH_MISSING)); };
    let Some(changes) = arch.dependencies else { return Ok(Measurements::missing("No supported lockfile is available.")); };
    let added: u64 = changes.iter().filter(|(at, _)| q.window.current(*at)).map(|(_, count)| count).sum();
    if arch.since > q.window.start && added == 0 { return Ok(Measurements::missing("Git dependency history is incomplete.")); }
    Ok(super::flow::one(Reading::new("project", q.project, "lockfile_dependencies_added", added as f64, evidence(id), Source::Unsized(CODE))))
}

pub(super) fn hotspots(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let Some((id, arch)) = architecture(q)? else { return Ok(Measurements::missing(ARCH_MISSING)); };
    let mut periods: [BTreeMap<&str, (f64, f64)>; 2] = Default::default();
    for row in arch.changes.iter().filter(|c| c.at >= q.window.before) {
        let totals = periods[usize::from(q.window.current(row.at))].entry(row.path.as_str()).or_default();
        totals.0 += row.churn as f64;
        totals.1 += f64::from(row.fix);
    }
    let score = |path: &str, (churn, fixes): (f64, f64)| {
        arch.sizes.get(path).map(|bytes| churn * *bytes as f64 * fixes).filter(|value| *value > 0.0)
    };
    // AC-212, AC-221: each file is ranked against every other file a fix touched in either period,
    // so a project whose fixes are known only inside the current window still fires on its top few percent.
    let mut scores = periods.iter().flatten().filter_map(|(path, stats)| score(path, *stats)).collect::<Vec<_>>();
    scores.sort_unstable_by(f64::total_cmp);
    let baseline = |value| match q.detector.threshold {
        Threshold::Baseline { percentile: p, .. } => percentile_of_others(&scores, value, p).into_iter().collect(),
        _ => Vec::new(),
    };
    let [_, recent] = periods;
    let mut readings = recent.into_iter().filter_map(|(path, stats)| score(path, stats).map(|value| (path, value)))
        .map(|(path, value)| Reading::new("file", path, "churn_size_fix_count", value, evidence(id), Source::Unsized(CODE)).history(baseline(value)))
        .collect::<Vec<_>>();
    let mut out = Measurements { complete_metrics: (arch.since <= q.window.start).then_some("churn_size_fix_count")
        .into_iter().collect(), ..Measurements::default() };
    if let Some(files) = arch.near_cap {
        readings.extend(files.into_iter().map(|(path, share)| Reading::new("file", path, "file_line_limit_share", share, evidence(id), Source::Unsized(CODE))
            .comparison(Comparison::AtLeast).fixed(Threshold::Absolute(0.9))));
        out.complete_metrics.push("file_line_limit_share");
    } else {
        out.unavailable.push("No Git source file line limit is configured.");
    }
    out.readings = readings;
    Ok(out)
}

/// `arch.bug_cluster`: per folder, the distinct commits of Bug tasks (or `fix:` commits) that changed it in the window.
pub(super) fn bug_clusters(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let Some((id, arch)) = architecture(q)? else { return Ok(Measurements::missing(ARCH_MISSING)); };
    let mut folders = BTreeMap::<&str, BTreeSet<(&str, i64)>>::new();
    for row in arch.changes.iter().filter(|c| c.fix && q.window.current(c.at)) {
        folders.entry(row.path.rsplit_once('/').map_or(".", |(folder, _)| folder)).or_default().insert((&row.commit, row.at));
    }
    Ok(Measurements {
        readings: folders.into_iter().map(|(folder, commits)| Reading::new("folder", folder, "fix_commits",
            commits.len() as f64, evidence(id), Source::Unsized(CODE)).comparison(Comparison::AtLeast)).collect(),
        complete_metrics: (arch.since <= q.window.start).then_some("fix_commits").into_iter().collect(),
        ..Measurements::default()
    })
}

pub(super) fn failures(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let mut out = Measurements::default();
    for status in probes::statuses(q.tx, q.project, q.window.end)? {
        let Some(id) = status.id else { continue; };
        if status.state == super::probe_command::Failure::Unavailable.code() || status.state == probes::OVERDUE { continue; }
        let failed = status.state != probes::MEASURED;
        out.readings.push(Reading::new("probe", status.source, "probe_failed", f64::from(failed), evidence(id), super::impact::OWN_HEALTH)
            .fixed(Threshold::Absolute(0.0)));
    }
    Ok(out)
}
