// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-215: judgment quality is measured only in weeks that retained labels;
//! a firing that is merely calculable never counts as measured quality.

use std::{collections::BTreeMap, fmt::Write};

use rusqlite::Transaction;

use crate::tasks::ops::rows;

use super::{WEEK, measures, outcomes, rate_intervals, replay};

struct Kind { name: &'static str, positive: &'static str, total: &'static str }

// Positions in KINDS; the label query tags each row with one.
const DECISIONS: usize = 0;
const FIRINGS: usize = 1;
const REPORTS: usize = 2;
const MISSES: usize = 3;
const FIXES: usize = 4;
const KINDS: [Kind; 5] = [
    Kind { name: "decisions", positive: "right", total: "Decisions judged right" },
    Kind { name: "replayed firings", positive: "judged right", total: "Replayed firings judged right" },
    Kind { name: "report verdicts", positive: "useful", total: "Report verdicts useful" },
    Kind { name: "independent reports", positive: "raised first", total: "Problems raised before an independent report" },
    Kind { name: "landed fixes", positive: "stayed resolved", total: "Fixes that stayed resolved" },
];

#[derive(Clone, Copy, Default)]
struct Tally { positive: usize, judged: usize }

/// Appends the quality section for weeks ending at `at`, `at - WEEK`, … back to `first`.
/// Each label counts by its latest judgment as of `at`, so an `--at` replay repeats what was known then.
pub(super) fn append(
    tx: &Transaction, project: &str, (first, at): (i64, i64), firings: &replay::Firings, output: &mut String,
) -> Result<(), String> {
    let mut labels = rows(tx, &format!("
        SELECT {DECISIONS}, o.at, o.verdict = 'right' FROM supervisor_actions a {}
            WHERE a.project = ?1 AND o.verdict IN ('right','wrong') AND o.at BETWEEN ?2 AND ?3
        UNION ALL SELECT {REPORTS}, v.at, v.verdict = 'useful' FROM supervisor_verdicts v
            JOIN supervisor_review_reports p USING (task)
            WHERE p.project = ?1 AND v.at BETWEEN ?2 AND ?3 AND {}
        UNION ALL SELECT {MISSES}, observed_at, raised_at IS NOT NULL FROM supervisor_miss_checks
            WHERE project = ?1 AND observed_at BETWEEN ?2 AND ?3
        UNION ALL SELECT {FIXES}, f.confirmed_at, {} FROM supervisor_problems p
            JOIN supervisor_fix_observations f ON f.problem = p.id
            WHERE p.project = ?1 AND f.confirmed_at BETWEEN ?2 AND ?3",
        outcomes::latest_as_of("?3"), measures::latest_report_verdict("?3"), measures::stayed_resolved("?3")),
        (project, first, at), |r| Ok((r.get::<_, i64>(0)? as usize, r.get::<_, i64>(1)?, r.get::<_, bool>(2)?)))?;
    labels.extend(replay::replayed_verdicts(tx, project, at + 1, firings)?.into_iter()
        .filter(|&(_, fired_at, _)| fired_at >= first).map(|(_, fired_at, right)| (FIRINGS, fired_at, right)));
    let mut weeks = BTreeMap::<i64, [Tally; KINDS.len()]>::new();
    for (kind, time, positive) in labels {
        let tally = &mut weeks.entry(at - (at - time) / WEEK * WEEK).or_default()[kind];
        tally.judged += 1;
        tally.positive += usize::from(positive);
    }

    let retained = (at - first) / WEEK + 1;
    writeln!(output, "\nJudgment quality in labelled weeks (judged decisions, hold verdicts on replayed firings, \
        report verdicts, miss checks and confirmed fixes)").unwrap();
    if weeks.is_empty() {
        writeln!(output, "Insufficient labels: none of the {retained} retained weeks has a label, so quality is not measured; \
            the firings above are only calculable.").unwrap();
        return Ok(());
    }
    writeln!(output, "Labelled weeks: {} of {retained}; the other {} have no labels and their quality is not measured.",
        weeks.len(), retained - weeks.len() as i64).unwrap();
    let mut totals = [Tally::default(); KINDS.len()];
    for (end, tallies) in &weeks {
        let parts = tallies.iter().zip(&KINDS).filter(|(t, _)| t.judged > 0)
            .map(|(t, kind)| format!("{} {} {} of {}", kind.name, t.positive, kind.positive, t.judged))
            .collect::<Vec<_>>();
        writeln!(output, "- Week ending {end}: {}", parts.join("; ")).unwrap();
        for (total, tally) in totals.iter_mut().zip(tallies) {
            total.judged += tally.judged;
            total.positive += tally.positive;
        }
    }
    writeln!(output, "Quality over labelled weeks").unwrap();
    for (total, kind) in totals.iter().zip(&KINDS) {
        match rate_intervals::share_percent(total.positive, total.judged) {
            Some(rate) => writeln!(output, "- {}: {} of {} ({rate})", kind.total, total.positive, total.judged),
            None => writeln!(output, "- {}: insufficient labels (n 0).", kind.total),
        }.unwrap();
    }
    Ok(())
}
