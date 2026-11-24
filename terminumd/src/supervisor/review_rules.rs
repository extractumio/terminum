// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-212, AC-251: named queries over the design-review gates: a task touching a T2 or T3 surface,
//! and a T2 or T3 change reaching Verify without its review record.

use std::collections::BTreeMap;

use crate::tasks::ops::rows;

use super::{Evidence, Measurements, Query, Reading, catalog::integration_on, flow::project_events, impact::Source,
    probe_rules::INTEGRATION_OFF, settings};

const REVIEW: &str = "A design-review gate is a check of process, in no weekly project total; the rework a missed review causes is sized by flow.returned.";
const NO_MAP: &str = "No T2 or T3 surface is configured.";

fn surfaces_on(q: &Query<'_, '_>) -> Result<bool, String> {
    Ok(settings::read(q.tx, q.project)?.settings.charter.surfaces.enabled())
}

/// `arch.surface_touched`: each task whose words or first diff matched a T2 or T3 surface in the window, by its tier.
pub(super) fn surface_touched(q: &Query<'_, '_>) -> Result<Measurements, String> {
    if !surfaces_on(q)? { return Ok(Measurements::missing(NO_MAP)); }
    let gates = rows(q.tx, "SELECT task, tier FROM supervisor_design_gates
        WHERE project = ?1 AND opened BETWEEN ?2 AND ?3 ORDER BY opened, task",
        (q.project, q.window.start, q.window.end), |r| Ok((r.get::<_, String>(0)?, r.get::<_, u8>(1)?)))?;
    Ok(Measurements {
        readings: gates.into_iter().map(|(task, tier)| Reading::new("task", task.clone(), "surface_tier", f64::from(tier),
            vec![Evidence::Tasks(task)], Source::Unsized(REVIEW))).collect(),
        complete_metrics: vec!["surface_tier"],
        ..Measurements::default()
    })
}

/// `arch.review_missing`: each task whose Verify found a T2 or T3 change without its review record, by the notice it left.
pub(super) fn review_missing(q: &Query<'_, '_>) -> Result<Measurements, String> {
    if !surfaces_on(q)? { return Ok(Measurements::missing(NO_MAP)); }
    if !integration_on(q.tx, q.project)? { return Ok(Measurements::missing(INTEGRATION_OFF)); }
    let notices = rows(q.tx, &format!("{} SELECT task, id FROM project_events e
        WHERE e.at >= ?3 AND e.kind = 'comment' AND json_extract(e.data, '$.design_gate') = ?4
        ORDER BY e.at, e.seq", project_events()),
        (q.project, q.window.end, q.window.start, super::verify_review::REVIEW_MISSING), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut tasks = BTreeMap::<String, Vec<Evidence>>::new();
    for (task, event) in notices { tasks.entry(task).or_default().push(super::flow::events(event)); }
    Ok(Measurements {
        readings: tasks.into_iter().map(|(task, evidence)| Reading::new("task", task, "verify_without_review",
            evidence.len() as f64, evidence, Source::Unsized(REVIEW))).collect(),
        complete_metrics: vec!["verify_without_review"],
        ..Measurements::default()
    })
}
