// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-215, AC-253: judgment quality is measured only in weeks that retained labels;
//! a firing that is merely calculable never counts as measured quality.

use std::{collections::{BTreeMap, BTreeSet}, fmt::Write};

use rusqlite::Transaction;

use crate::tasks::ops::rows;

use super::{Rule, WEEK, rate_intervals};

/// Label kinds, each with its per-week wording and its total's name.
const KINDS: [(&str, &str, &str); 5] = [
    ("decisions", "right", "Decisions judged right"),
    ("replayed firings", "judged right", "Replayed firings judged right"),
    ("report verdicts", "useful", "Report verdicts useful"),
    ("independent reports", "raised first", "Problems raised before an independent report"),
    ("landed fixes", "stayed resolved", "Fixes that stayed resolved"),
];
const FIRINGS: usize = 1;

#[derive(Clone, Copy, Default)]
struct Tally { positive: usize, judged: usize }

/// Weekly labels as of `at`, in weeks ending at `at`, `at - WEEK`, … back to `first`.
pub(super) struct Quality {
    weeks: usize,
    labelled: BTreeMap<i64, [Tally; 5]>,
}

pub(super) fn read(
    tx: &Transaction, project: &str, first: i64, at: i64, firings: &BTreeSet<(Rule, String)>,
) -> Result<Quality, String> {
    // Each label is (kind, time, positive). Latest judgments are taken as of `at`, so an `--at` replay
    // repeats what was known then.
    let mut labels = rows(tx, "
        SELECT 0, o.at, o.verdict = 'right' FROM supervisor_actions a JOIN supervisor_outcomes o
            ON o.action = a.id AND o.days = (SELECT v.days FROM supervisor_outcomes v
                WHERE v.action = a.id AND v.at <= ?3 ORDER BY (v.days = 0) DESC, v.days DESC LIMIT 1)
            WHERE a.project = ?1 AND o.verdict IN ('right','wrong') AND o.at BETWEEN ?2 AND ?3
        UNION ALL SELECT 2, v.at, v.verdict = 'useful' FROM supervisor_verdicts v
            JOIN supervisor_review_reports p USING (task)
            WHERE p.project = ?1 AND v.at BETWEEN ?2 AND ?3 AND v.id = (SELECT max(last.id) FROM supervisor_verdicts last
                WHERE last.task = v.task AND last.fingerprint = v.fingerprint AND last.at <= ?3)
        UNION ALL SELECT 3, observed_at, raised_at IS NOT NULL FROM supervisor_miss_checks
            WHERE project = ?1 AND observed_at BETWEEN ?2 AND ?3
        UNION ALL SELECT 4, f.confirmed_at, NOT EXISTS (SELECT 1 FROM supervisor_problem_events e
                WHERE e.problem = p.id AND e.state = 'recurred' AND e.at > f.confirmed_at AND e.at <= ?3)
            FROM supervisor_problems p JOIN supervisor_fix_observations f ON f.problem = p.id
            WHERE p.project = ?1 AND f.confirmed_at BETWEEN ?2 AND ?3",
        (project, first, at), |r| Ok((r.get::<_, i64>(0)? as usize, r.get::<_, i64>(1)?, r.get::<_, bool>(2)?)))?;
    // A hold verdict labels quality only for a firing this replay calculates again from the same evidence.
    let verdicts = rows(tx, "SELECT detector, firing, fired_at, was_right FROM supervisor_hold_verdicts
        WHERE project = ?1 AND fired_at BETWEEN ?2 AND ?3",
        (project, first, at), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?, r.get::<_, bool>(3)?)))?;
    labels.extend(verdicts.into_iter().filter_map(|(detector, firing, fired_at, right)| {
        let rule = Rule::named(&detector).ok().filter(|rule| rule.id() == detector)?;
        firings.contains(&(rule, firing)).then_some((FIRINGS, fired_at, right))
    }));
    let mut labelled = BTreeMap::<i64, [Tally; 5]>::new();
    for (kind, time, positive) in labels {
        let end = at - (at - time) / WEEK * WEEK;
        let tally = &mut labelled.entry(end).or_default()[kind];
        tally.judged += 1;
        tally.positive += usize::from(positive);
    }
    Ok(Quality { weeks: ((at - first) / WEEK + 1) as usize, labelled })
}

impl Quality {
    pub(super) fn render(&self, output: &mut String) {
        writeln!(output, "\nJudgment quality in labelled weeks (judged decisions, hold verdicts on replayed firings, \
            report verdicts, miss checks and confirmed fixes)").unwrap();
        if self.labelled.is_empty() {
            writeln!(output, "Insufficient labels: none of the {} retained weeks has a label, so quality is not measured; \
                the firings above are only calculable.", self.weeks).unwrap();
            return;
        }
        writeln!(output, "Labelled weeks: {} of {}; the other {} have no labels and their quality is not measured.",
            self.labelled.len(), self.weeks, self.weeks - self.labelled.len()).unwrap();
        let mut totals = [Tally::default(); 5];
        for (end, tallies) in &self.labelled {
            let parts = tallies.iter().zip(KINDS).filter(|(t, _)| t.judged > 0)
                .map(|(t, (name, positive, _))| format!("{name} {} {positive} of {}", t.positive, t.judged))
                .collect::<Vec<_>>();
            writeln!(output, "- Week ending {end}: {}", parts.join("; ")).unwrap();
            for (total, tally) in totals.iter_mut().zip(tallies) {
                total.judged += tally.judged;
                total.positive += tally.positive;
            }
        }
        writeln!(output, "Quality over labelled weeks").unwrap();
        for (total, (_, _, name)) in totals.iter().zip(KINDS) {
            match rate_intervals::share(total.positive, total.judged, None).percent() {
                Some(rate) => writeln!(output, "- {name}: {} of {} ({rate})", total.positive, total.judged),
                None => writeln!(output, "- {name}: insufficient labels (n 0)."),
            }.unwrap();
        }
    }
}
