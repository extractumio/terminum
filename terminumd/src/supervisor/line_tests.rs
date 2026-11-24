// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial

use std::{fs, os::unix::fs::PermissionsExt, path::Path};
use super::*;
use crate::supervisor::{
    engine_tests::{Fixture, NOW, PROJECT}, incidents::Action, incidents_api,
    line_queue::{Kind, Notice}, Checked, Mode, SCHEMA, Scores,
};
use crate::tasks::store::Store;

fn configure(f: &Fixture, quiet: Option<[u16; 2]>) {
    f.write("INSERT INTO project_supervisor (project, enabled, generation, enabled_at) VALUES (?1, 1, 1, ?2)", (PROJECT, NOW));
    f.store.with(|tx| save(tx, 0, Settings {
        channels: vec!["/test/first".into(), "/test/second".into()], ladder: [1, 3, 6],
        link_base: None, quiet,
    }, NOW)).unwrap();
}

fn observed(f: &Fixture, at: i64, subjects: &[(&str, bool, u8, i64)]) {
    let findings = subjects.iter().map(|&(subject, charter, reversibility, amount)| {
        let mut finding = Reading::new("fixture", subject, "failure_count", amount as f64, vec![Evidence::Inputs(amount)], super::super::impact::Source::Present)
            .finding(super::super::Threshold::Absolute(0.0), Window { before: at - 1, start: at - 1, end: at }).unwrap();
        finding.charter_never = charter.then(|| "Lose user data.".into());
        finding.scores = Scores { impact: 3, reversibility, ..Scores::default() };
        finding
    }).collect();
    Evaluation { project: PROJECT.into(), at, detectors: vec![Checked {
        rule: Rule::FailedWithoutReason, mode: Mode::Live, findings, unavailable: Vec::new(),
        thresholds: Default::default(), complete_metrics: Vec::new(), measured: None, observations: Default::default(),
    }] }.record(&f.store).unwrap();
}

fn take(store: &Store, at: i64, minute: u16) -> Option<Notice> {
    match store.with(|tx| claim(tx, at, Some(minute))).unwrap() {
        Some((_, Job::Notice(notice))) => Some(notice),
        Some((_, Job::Weekly(_))) => panic!("unexpected weekly check"),
        Some((_, Job::Discarded)) | None => None,
    }
}

#[test]
fn ac227_templates_exclude_case_prose_and_use_only_a_validated_origin() {
    let f = Fixture::new("line-templates");
    configure(&f, None);
    // Synthetic detector subject, deliberately shaped like private prose and a home path.
    let subject = "fixture task prose /Users/alex/fixture-secret-marker";
    observed(&f, NOW, &[(subject, false, 0, 1)]);
    let initial = take(&f.store, NOW, 600).unwrap();
    let id = initial.incident;
    assert_eq!(initial.render(None), format!("Critical incident {id} · Level 3. Active signals: 1. Acknowledge in Terminum.\n"));
    observed(&f, NOW + 1, &[(subject, false, 0, 2)]);
    let update = take(&f.store, NOW + 61, 600).unwrap();
    assert_eq!(update.kind, Kind::Repeat, "the repeat on the ladder is claimed first");
    let update = take(&f.store, NOW + 121, 600).unwrap();
    assert_eq!(update.render(None), format!("Update on incident {id} · Level 3. Active signals: 1.\n"));
    let base = origin("https://terminum.example:8443/").unwrap();
    let exhausted = Notice { budget: Some(crate::supervisor::governor::JUDGMENT_NOTICE),
        ..initial.clone() };
    assert!(exhausted.render(None).starts_with(crate::supervisor::governor::JUDGMENT_NOTICE), "AC-237");
    let sample = Notice { project: "9feed0123456".into(), ..initial };
    assert!(sample.render(Some(&base)).ends_with(&format!(
        "https://terminum.example:8443/supervisor/?project=9feed0123456&incident={id}#incident-{id}\n")));
    for unsafe_base in ["https://user@terminum.example", "https://terminum.example/?id=x",
        "https://terminum.example/#x", "https://terminum.example/Users/alex", "https://terminum.example/%2f", "http://terminum.example",
        "https://terminum.example:bad", "https://terminum.example\\other"] {
        assert!(origin(unsafe_base).is_none(), "{unsafe_base}");
    }
    observed(&f, NOW + 130, &[]);
    assert_eq!(take(&f.store, NOW + 131, 600).unwrap().render(None), format!("All-clear on incident {id}.\n"));
    f.store.with(|tx| line_queue::enqueue(tx, id, Kind::Answer, false)).unwrap();
    assert_eq!(take(&f.store, NOW + 132, 600).unwrap().render(None), format!("An answer on incident {id}.\n"));
}

#[test]
fn ac228_repeat_escalation_snooze_and_acknowledgment() {
    let f = Fixture::new("line-repeat");
    configure(&f, None);
    let take = |offset| take(&f.store, NOW + offset, 600);
    observed(&f, NOW, &[("target", false, 0, 1)]);
    let open = take(0).unwrap();
    assert_eq!((open.kind, open.channels), (Kind::Opened, 1));
    observed(&f, NOW + 1, &[("target", false, 0, 1)]);
    assert!(take(59).is_none(), "same fingerprint shares the cooldown");
    assert_eq!(take(60).unwrap().channels, 1);
    assert!(take(179).is_none());
    let escalated = take(180).unwrap();
    assert_eq!((escalated.kind, escalated.channels), (Kind::Repeat, 2));
    let act = |action, offset| f.store.with(|tx| incidents::act(tx, open.incident, action, NOW + offset)).unwrap();
    assert!(act(Action::Snooze, 181));
    assert!(take(3780).is_none());
    assert_eq!(take(3781).unwrap().channels, 2);
    assert!(take(3782).is_none(), "missed turns are not replayed");
    assert!(act(Action::Acknowledge, 3783));
    assert!(take(7800).is_none());
}

#[test]
fn ac228_acknowledged_incident_updates_keep_the_cool_down() {
    let f = Fixture::new("line-ack-updates");
    configure(&f, None);
    let take = |offset| take(&f.store, NOW + offset, 600);
    // Reversibility 3 also passes quiet hours, so the cool-down is the only bound on its updates.
    observed(&f, NOW, &[("target", false, 3, 1)]);
    let open = take(0).unwrap();
    assert!(f.store.with(|tx| incidents::act(tx, open.incident, Action::Acknowledge, NOW + 1)).unwrap());
    for (offset, amount) in [(10, 2), (20, 3), (30, 4)] {
        observed(&f, NOW + offset, &[("target", false, 3, amount)]);
        assert!(take(offset).is_none(), "an acknowledged incident's updates wait for the cool-down");
    }
    let update = take(60).unwrap();
    assert_eq!((update.incident, update.kind), (open.incident, Kind::Updated));
    assert!(take(61).is_none(), "pending updates merge into one and repeats stay stopped");
    observed(&f, NOW + 70, &[("target", false, 3, 5)]);
    assert!(take(119).is_none());
    assert_eq!(take(120).unwrap().kind, Kind::Updated);
    assert!(take(3600).is_none(), "acknowledged: no repeat on the ladder");
}

#[test]
fn ac228_quiet_hours_defer_ordinary_notices_and_keep_all_incidents() {
    let f = Fixture::new("line-quiet");
    configure(&f, Some([23 * 60, 8 * 60]));
    observed(&f, NOW, &[("ordinary", false, 0, 1), ("charter", true, 0, 1),
        ("irreversible", false, 3, 1), ("also ordinary", false, 0, 1)]);
    let subjects = f.store.snapshot(SCHEMA, |tx| rows(tx, "SELECT id, subject_id FROM supervisor_incidents",
        [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))).unwrap()
        .into_iter().collect::<std::collections::BTreeMap<_, _>>();
    let overnight = || take(&f.store, NOW, 23 * 60);
    let mut urgent = [overnight().unwrap().incident, overnight().unwrap().incident].map(|id| subjects[&id].as_str());
    urgent.sort_unstable();
    assert_eq!(urgent, ["charter", "irreversible"]);
    assert!(overnight().is_none());
    assert!(take(&f.store, NOW + 10, 8 * 60 - 1).is_none());
    let morning = || take(&f.store, NOW + 10, 8 * 60);
    let mut deferred = [morning().unwrap().incident, morning().unwrap().incident].map(|id| subjects[&id].as_str());
    deferred.sort_unstable();
    assert_eq!(deferred, ["also ordinary", "ordinary"]);
    assert!(morning().is_none());
}

fn incident_of(f: &Fixture, subject: &str) -> i64 {
    f.store.snapshot(SCHEMA, |tx| tx.query_row("SELECT id FROM supervisor_incidents WHERE subject_id = ?1",
        [subject], |r| r.get(0)).map_err(crate::tasks::ops::sql)).unwrap()
}

fn queued(f: &Fixture) -> i64 {
    f.store.snapshot(SCHEMA, |tx| tx.query_row("SELECT count(*) FROM supervisor_line_pending", [],
        |r| r.get(0)).map_err(crate::tasks::ops::sql)).unwrap()
}

#[test]
fn ac228_an_incident_that_cleared_during_quiet_hours_sends_one_all_clear() {
    let f = Fixture::new("line-stale-clear");
    configure(&f, Some([23 * 60, 8 * 60]));
    observed(&f, NOW, &[("ordinary", false, 0, 1)]);
    let id = incident_of(&f, "ordinary");
    assert!(take(&f.store, NOW, 23 * 60).is_none(), "an ordinary incident waits for the morning");
    observed(&f, NOW + 10, &[]);
    assert!(take(&f.store, NOW + 10, 23 * 60).is_none());
    assert_eq!(queued(&f), 1, "the all-clear replaces the incident notice still waiting");
    let morning = take(&f.store, NOW + 20, 8 * 60).unwrap();
    assert_eq!((morning.incident, morning.kind), (id, Kind::AllClear));
    assert!(take(&f.store, NOW + 21, 8 * 60).is_none());
    assert!(take(&f.store, NOW + 4000, 9 * 60).is_none(), "a closed incident has no repeat");
}

#[test]
fn ac228_a_waiting_notice_takes_quiet_hours_eligibility_from_its_own_latest_state() {
    let f = Fixture::new("line-stale-urgent");
    configure(&f, Some([23 * 60, 8 * 60]));
    let night = |offset| take(&f.store, NOW + offset, 23 * 60);
    observed(&f, NOW, &[("target", false, 0, 1)]);
    assert!(night(0).is_none());
    // The waiting notice turns urgent with its incident: one notice goes, with the current numbers.
    observed(&f, NOW + 10, &[("target", true, 0, 2)]);
    assert_eq!(queued(&f), 1, "the change merges into the waiting notice");
    let page = night(10).unwrap();
    assert_eq!((page.kind, page.signals), (Kind::Opened, 1));
    assert_eq!(night(70).unwrap().kind, Kind::Repeat);
    assert!(night(130).is_none(), "no stale update follows the page");

    // An urgent notice that turns ordinary before it goes waits like any ordinary one.
    let f = Fixture::new("line-stale-ordinary");
    configure(&f, Some([23 * 60, 8 * 60]));
    observed(&f, NOW, &[("target", true, 0, 1)]);
    observed(&f, NOW + 1, &[("target", false, 0, 2)]);
    assert!(take(&f.store, NOW + 1, 23 * 60).is_none(), "its older urgent state no longer passes quiet hours");
    let morning = take(&f.store, NOW + 2, 8 * 60).unwrap();
    assert_eq!((morning.kind, morning.signals), (Kind::Opened, 1));
}

#[test]
fn ac228_a_waiting_page_goes_only_while_the_owner_still_needs_it() {
    let f = Fixture::new("line-stale-page");
    configure(&f, Some([23 * 60, 8 * 60]));
    let morning = |offset| take(&f.store, NOW + offset, 8 * 60);
    // Its signal moves on to a new incident before the first page goes: only the new one pages.
    observed(&f, NOW, &[("target", false, 0, 1)]);
    let first = incident_of(&f, "target");
    let later = incidents::GROUP_WINDOW + 1;
    observed(&f, NOW + later, &[("target", false, 0, 2)]);
    let page = morning(later).unwrap();
    assert_ne!(page.incident, first);
    assert_eq!((page.kind, page.level, page.signals), (Kind::Opened, 3, 1));
    assert!(morning(later + 1).is_none(), "the continued incident sends no empty page");
    // The owner acknowledges a waiting incident in Terminum before its page goes.
    observed(&f, NOW + later + 1, &[("target", false, 0, 2), ("acknowledged", false, 0, 1)]);
    let id = incident_of(&f, "acknowledged");
    assert!(f.store.with(|tx| incidents::act(tx, id, Action::Acknowledge, NOW + later + 2)).unwrap());
    assert!(morning(later + 3).is_none(), "an acknowledged incident is not paged");
}

#[test]
fn ac228_a_supervisor_turned_back_on_pages_its_open_incidents_once() {
    let f = Fixture::new("line-stale-enable");
    configure(&f, None);
    let set = |on, offset| super::super::runtime::set_enabled(&f.store, PROJECT, on, NOW + offset).unwrap();
    observed(&f, NOW, &[("open", false, 0, 1), ("acknowledged", false, 0, 1), ("cleared", false, 0, 1)]);
    let acknowledged = incident_of(&f, "acknowledged");
    assert!(f.store.with(|tx| incidents::act(tx, acknowledged, Action::Acknowledge, NOW + 1)).unwrap());
    assert_eq!(queued(&f), 3);
    assert!(set(false, 2).is_some());
    assert_eq!(queued(&f), 0, "turning the Supervisor off drops its queued notices");
    observed(&f, NOW + 3, &[("open", false, 0, 2), ("acknowledged", false, 0, 1)]);
    assert_eq!(queued(&f), 0, "nothing queues while it is off");
    assert!(take(&f.store, NOW + 3, 600).is_none());
    assert!(set(true, 4).is_some());
    let page = take(&f.store, NOW + 4, 600).unwrap();
    assert_eq!((page.incident, page.kind), (incident_of(&f, "open"), Kind::Opened));
    assert!(take(&f.store, NOW + 5, 600).is_none(), "no update, all-clear or acknowledged incident follows");
    assert_eq!(take(&f.store, NOW + 64, 600).unwrap().kind, Kind::Repeat, "the ladder starts again from the page");
}

fn command(root: &Path, name: &str, body: &str) -> String {
    let path = root.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    path.into_os_string().into_string().unwrap()
}

#[tokio::test]
async fn ac227_bounded_channel_stdin_and_exit() {
    let f = Fixture::new("line-command");
    let root = f.path.parent().unwrap();
    let output = root.join("notice");
    let record = crate::termtext::posix_word(output.to_str().unwrap());
    let works = command(root, "literal $(must-not-run)", &format!(
        "test \"$PATH\" = /usr/bin:/bin && test \"$LANG\" = C && test -z \"${{HOME+x}}\" && test \"$(pwd)\" = / || exit 8\ncat > {record}"));
    let timeout = Duration::from_secs(1);
    line_command::send(&works, false, TEST, timeout).await.unwrap();
    assert_eq!(fs::read(&output).unwrap(), TEST.as_bytes());
    let bad = command(root, "bad", "cat >/dev/null\nprintf 'This output is a synthetic attempted reply in the test'\nexit 17");
    assert_eq!(line_command::send(&bad, false, TEST, timeout).await.unwrap_err(), "Exit status 17.");
    let slow = command(root, "slow", "cat >/dev/null\nprintf 'Synthetic reply fixture'\nwait_time=10\nsleep \"$wait_time\" &\nwait");
    assert_eq!(line_command::send(&slow, false, TEST, Duration::from_millis(100)).await.unwrap_err(), "Command timed out.");
    let pid_file = root.join("helper.pid");
    let leaves = command(root, "leaves", &format!(
        "cat >/dev/null\nsleep 30 &\necho $! > {}\nexit 0", crate::termtext::posix_word(pid_file.to_str().unwrap())));
    line_command::send(&leaves, false, TEST, timeout).await.unwrap();
    let helper: libc::pid_t = fs::read_to_string(&pid_file).unwrap().trim().parse().unwrap();
    let gone = || unsafe { libc::kill(helper, 0) } != 0;   // SAFETY: signal 0 only checks the pid
    for _ in 0..100 {
        if gone() { break; }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(gone(), "a helper left in the command's group ends with it");
}

#[tokio::test]
async fn ac229_next_channel_weekly_test_and_local_recovery() {
    let f = Fixture::new("line-health");
    let root = f.path.parent().unwrap();
    let output = root.join("checks");
    let record = crate::termtext::posix_word(output.to_str().unwrap());
    let bad = command(root, "bad", "cat >/dev/null\nexit 17");
    let works = command(root, "works", &format!("printf '%s\\n' \"$1\" >> {record}\ncat >> {record}"));
    let settings = Settings { channels: vec![bad.clone(), works.clone()], quiet: None, ..Settings::default() };
    f.write("INSERT INTO project_supervisor (project, enabled, generation, enabled_at) VALUES (?1, 1, 1, ?2)", (PROJECT, NOW));
    f.store.with(|tx| save(tx, 0, settings, NOW)).unwrap();
    let week = |at| f.store.with(|tx| claim(tx, at, Some(600))).unwrap();
    assert!(week(NOW + WEEK - 1).is_none());
    let (settings, Job::Weekly(project)) = week(NOW + WEEK).unwrap() else { panic!("weekly check due") };
    let wait = Duration::from_secs(1);
    let attempts = deliver(&settings, CHECK, 2, true, wait).await;
    assert_eq!(attempts, [(0, Err("Exit status 17.".into())), (1, Ok(()))]);
    assert_eq!(fs::read_to_string(&output).unwrap(), format!("--check\n{CHECK}"));
    f.store.with(|tx| record_attempts(tx, &project, attempts, NOW + WEEK)).unwrap();
    let local = f.store.snapshot(SCHEMA, incidents_api::pending).unwrap();
    assert_eq!(local.len(), 1);
    assert!(local[0].happened.contains("Channel 1: Exit status 17."));
    let signals = crate::supervisor::signals(&f.store, PROJECT).unwrap();
    assert_eq!(signals[0].detector, "self.line_dead");
    assert!(week(NOW + WEEK + 1).is_none(), "line health does not send itself recursively");

    let attempts = deliver(&settings, TEST, 1, false, wait).await;
    assert_eq!(attempts, [(0, Err("Exit status 17.".into())), (1, Ok(()))], "failure falls through in order");
    assert!(fs::read_to_string(&output).unwrap().ends_with(&format!("\n{TEST}")));
    let repaired = command(root, "bad", "cat >/dev/null");
    assert_eq!(repaired, bad);
    let attempts = deliver(&settings, TEST, 2, false, wait).await;
    assert_eq!(attempts, [(0, Ok(())), (1, Ok(()))], "second rung still uses both channels");
    f.store.with(|tx| record_attempts(tx, &project, attempts, NOW + WEEK + 5)).unwrap();
    let local = f.store.snapshot(SCHEMA, incidents_api::pending).unwrap();
    assert_eq!(local[0].closed_at, Some(NOW + WEEK + 5));
}

#[tokio::test]
async fn ac232_owner_sets_level_and_all_levels_page() {
    for (level, name) in [(super::super::Level::Observer, "observer"), (super::super::Level::Advisor, "advisor"),
        (super::super::Level::Operator, "operator")]
    {
        let f = Fixture::new(name);
        configure(&f, None);
        let output = f.path.with_extension("notices");
        let out = crate::termtext::posix_word(output.to_str().unwrap());
        let channel = command(f.path.parent().unwrap(), "notify", &format!("cat > {out}"));
        let s = Settings { channels: vec![channel], quiet: None, ..Settings::default() };
        f.store.with(|tx| {
            let pack = serde_json::from_value(serde_json::json!({"level": name})).unwrap();
            super::super::settings::write(tx, PROJECT, 0, pack, false)?;
            save(tx, 1, s, NOW)?;
            assert_eq!(super::super::level(tx, PROJECT)?, level);
            Ok(())
        }).unwrap();
        observed(&f, NOW, &[("charter fixture", true, 3, 1)]);
        let (settings, Job::Notice(notice)) = f.store.with(|tx| claim(tx, NOW, Some(600))).unwrap().unwrap()
            else { panic!("the charter incident should page at {name}") };
        assert_eq!(notice.kind, Kind::Opened);
        let message = notice.render(None);
        // A successful send's deadline: a loaded machine can take over a second to start the channel's shell.
        assert_eq!(deliver(&settings, &message, 1, false, Duration::from_secs(10)).await, [(0, Ok(()))]);
        assert_eq!(fs::read_to_string(output).unwrap(), message);
        assert_eq!(f.store.with(|tx| super::super::level(tx, PROJECT)).unwrap(), level);
    }
}
