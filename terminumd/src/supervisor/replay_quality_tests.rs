// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-215: the replay measures judgment quality only in labelled weeks of its own project.

use std::fs;

use super::engine_tests::{Fixture, NOW, OTHER, PROJECT};
use super::replay_tests::{cli, replay_args};
use super::{Rule, WEEK, flow, replay, settings};

fn replay_at(dir: &std::path::Path, at: Option<i64>) -> String {
    let mut args = replay_args();
    if let Some(at) = at { args.extend(["--at".into(), at.to_string()]); }
    cli(dir, &args).unwrap()
}

fn quality_section(report: &str) -> &str {
    &report[report.find("\nJudgment quality in labelled weeks").expect(report)..]
}

/// The first firing the replay calculates for `rule`, with the key a hold verdict names.
fn replayed_firing(f: &Fixture, rule: Rule) -> (String, i64) {
    f.store.with(|tx| {
        let (first, at) = flow::history_bounds(tx, PROJECT, i64::MAX / 1000 - WEEK)?;
        let mut firing = None;
        replay::visit_history(tx, PROJECT, first, at, &[settings::read(tx, PROJECT)?.settings.detector(rule)], |rule, finding| {
            firing.get_or_insert((replay::firing_key(PROJECT, rule, finding)?, finding.measure.window_end));
            Ok(())
        })?;
        Ok(firing.expect("the fixture's history fires"))
    }).unwrap()
}

fn action(f: &Fixture, id: i64, project: &str, outcomes: &[(i64, i64, &str)]) {
    f.write("INSERT INTO supervisor_actions (id,project,detector,response,kind,source,fingerprint,target,at)
        VALUES (?1,?2,'flow.wait_human_age','R2','alert',?1,'fixture','waiting',?3)", (id, project, NOW - 3 * WEEK));
    for &(days, at, verdict) in outcomes {
        f.write("INSERT INTO supervisor_outcomes VALUES (?1,?2,?3,?4,'fixture')", (id, days, at, verdict));
    }
}

#[test]
fn ac215_replay_reports_quality_only_in_labelled_weeks() {
    let f = Fixture::new("replay-quality");
    let dir = f.path.parent().unwrap();
    f.task("history", Some("Story"), "P2", "DONE", NOW - 9 * WEEK);
    f.task("waiting", Some("Feature"), "P2", "IN PROGRESS", NOW - 6 * WEEK);
    f.transition("waiting", NOW - 5 * WEEK, "IN PROGRESS", "WAIT FOR HUMAN");
    f.task("latest", None, "P2", "IN PROGRESS", NOW);
    f.task_in(OTHER, "other", None, "P2", "DONE", NOW - WEEK);

    let unlabelled = replay_at(dir, None);
    assert!(unlabelled.contains(", as of 1800000000)"), "{unlabelled}");
    assert!(quality_section(&unlabelled).contains("Insufficient labels: none of the 10 retained weeks has a label"), "{unlabelled}");
    assert!(!unlabelled.contains("Quality over labelled weeks"));

    // Week ending NOW - WEEK: a right decision, a matched hold verdict and a fix that later recurs.
    // Week ending NOW: an explicit wrong verdict after a right check, a useful-then-wrong report verdict, a miss.
    action(&f, 1, PROJECT, &[(1, NOW - WEEK - 100, "right")]);
    action(&f, 2, PROJECT, &[(1, NOW - 100, "right"), (0, NOW - 50, "wrong")]);
    action(&f, 3, PROJECT, &[(1, NOW - 100, "undecided")]);
    action(&f, 4, OTHER, &[(1, NOW - 100, "right")]);
    let (firing, fired_at) = replayed_firing(&f, Rule::WaitHuman);
    f.write("INSERT INTO supervisor_hold_verdicts VALUES (?1,'flow.wait_human_age',?2,?3,1),
        (?1,'flow.wait_human_age','not-replayed',?3,0), (?4,'flow.wait_human_age',?2,?3,0)",
        (PROJECT, &firing, fired_at, OTHER));
    for (project, key) in [(PROJECT, "fixed"), (OTHER, "other-fixed")] {
        f.write("INSERT INTO supervisor_signals (fingerprint,project,detector,subject_kind,subject_id,metric,
            evidence,measure,scores,first_seen,last_seen,count,state)
            VALUES (?1,?2,'flow.wait_human_age','task','waiting','fixture','[]','{}','{}',?3,?3,1,'resolved')",
            (key, project, NOW - 2 * WEEK));
    }
    f.write("INSERT INTO supervisor_problems (id,project,title,lens,hypothesis,disproof,options,confidence,effort,state,opened_at)
        VALUES (1,?1,'Fixture','flow','','','[]',1,1,'recurred',?2), (2,?3,'Other','flow','','','[]',1,1,'resolved',?2)",
        (PROJECT, NOW - 2 * WEEK, OTHER));
    f.write("INSERT INTO supervisor_fix_observations VALUES (1,?1,?2), (2,?1,?2)", (NOW - 2 * WEEK, NOW - WEEK - 200));
    f.write("INSERT INTO supervisor_problem_events (problem,at,state,detail) VALUES (1,?1,'recurred','Fixture.')", [NOW - 10]);
    let review = f.event("history", NOW - 300, "dispatcher", "comment", "Review.", None);
    f.write("INSERT INTO supervisor_review_reports VALUES ('history',?1,?2,?3)", (PROJECT, NOW - 300, &review));
    f.write("INSERT INTO supervisor_review_items VALUES ('history','fixed',0)", []);
    f.write("INSERT INTO supervisor_verdicts (task,fingerprint,at,verdict) VALUES ('history','fixed',?1,'useful'),
        ('history','fixed',?2,'wrong')", (NOW - 200, NOW - 20));
    f.write("INSERT INTO supervisor_miss_checks VALUES (?1,'owner_bug','task','latest','latest',?2,?2,NULL,NULL,NULL,NULL),
        (?3,'owner_bug','task','other','other',?2,?2,NULL,NULL,NULL,NULL)", (PROJECT, NOW - 200, OTHER));

    let wal = f.path.with_extension("sqlite-wal");
    let before = (fs::read(&f.path).unwrap(), fs::read(&wal).unwrap());
    let report = replay_at(dir, None);
    assert_eq!(report, replay_at(dir, None));
    assert_eq!(before, (fs::read(&f.path).unwrap(), fs::read(&wal).unwrap()));
    // Replayed firings stop two weeks before the latest cutoff, so the verdict's week has no other label.
    let week = NOW - (NOW - fired_at) / WEEK * WEEK;
    assert!(week <= NOW - 2 * WEEK);
    let quality = quality_section(&report);
    assert!(quality.contains("Labelled weeks: 3 of 10; the other 7 have no labels"), "{quality}");
    for (end, text) in [
        (week, "replayed firings 1 judged right of 1"),
        (NOW - WEEK, "decisions 1 right of 1; landed fixes 0 stayed resolved of 1"),
        (NOW, "decisions 0 right of 1; report verdicts 0 useful of 1; independent reports 0 raised first of 1"),
    ] {
        assert!(quality.contains(&format!("- Week ending {end}: {text}\n")), "{end}: {text}\n{quality}");
    }
    assert!(quality.contains("- Decisions judged right: 1 of 2 (50.0%, 95% interval"), "{quality}");
    assert!(quality.contains("- Replayed firings judged right: 1 of 1 (100.0%"), "{quality}");
    assert!(quality.contains("- Report verdicts useful: 0 of 1 (0.0%"), "{quality}");
    assert!(quality.contains("- Problems raised before an independent report: 0 of 1 (0.0%"), "{quality}");
    assert!(quality.contains("- Fixes that stayed resolved: 0 of 1 (0.0%"), "{quality}");

    // As of the end of the earlier week, the fix had not yet recurred and later labels did not exist.
    let earlier = replay_at(dir, Some(NOW - WEEK));
    let earlier = quality_section(&earlier);
    assert!(earlier.contains("landed fixes 1 stayed resolved of 1"), "{earlier}");
    assert!(earlier.contains("- Report verdicts useful: insufficient labels (n 0)."), "{earlier}");
    assert!(!earlier.contains(&format!("Week ending {NOW}")), "{earlier}");
}

