// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial

use std::process::Command;

use serde_json::json;

use super::*;
use crate::supervisor::{Evidence, Rule, evaluate, scoreboard, signals};
use crate::supervisor::engine_tests::{Fixture, NOW, PROJECT};

fn enabled(f: &Fixture, dir: &Path) {
    f.write("UPDATE projects SET dir = ?1 WHERE id = ?2", (dir.to_str().unwrap(), PROJECT));
    f.write("INSERT INTO project_supervisor (project, enabled, generation, enabled_at) VALUES (?1, 1, 1, ?2)",
        (PROJECT, NOW - 2 * DAY));
}

fn git(dir: &Path, at: i64, args: &[&str]) -> String {
    let date = format!("@{at} +0000");
    let output = Command::new("/usr/bin/git").current_dir(dir).env_clear()
        .envs([("PATH", "/usr/bin:/bin"), ("GIT_CONFIG_NOSYSTEM", "1"), ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_AUTHOR_NAME", "Fixture"), ("GIT_AUTHOR_EMAIL", "fixture@example.com"),
            ("GIT_COMMITTER_NAME", "Fixture"), ("GIT_COMMITTER_EMAIL", "fixture@example.com")])
        .env("GIT_AUTHOR_DATE", &date).env("GIT_COMMITTER_DATE", &date)
        .args(args).output().unwrap();
    assert!(output.status.success(), "Git {args:?}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap().trim().into()
}

fn job(f: &Fixture, source: &str, at: i64) -> Job {
    f.store.snapshot(SCHEMA, |tx| Ok(due(tx, at)?.into_iter().find(|j| j.source.name() == source).unwrap())).unwrap()
}

#[tokio::test]
async fn ac260_probe_valid_invalid_timeout() {
    let f = Fixture::new("probe-command");
    let dir = f.path.parent().unwrap();
    enabled(&f, dir);
    f.write("INSERT INTO supervisor_probe_candidates (project, probe, command) VALUES (?1, 'staged', 'exit 9')", [PROJECT]);
    assert!(f.store.snapshot(SCHEMA, |tx| Ok(due(tx, NOW)?.iter().all(|j| j.source.name() != "custom:staged"))).unwrap());
    let line = format!(r#"{{"metric":"product.workspaces","value":11,"unit":"count","at":{},"dims":{{"plan":"standard"}}}}"#, NOW - 1);
    let command = format!("test \"${{LANG}}:${{PATH}}:${{HOME-unset}}\" = C:/usr/bin:/bin:unset && printf '%s\\n' '{line}'");
    f.write("INSERT INTO supervisor_probe_commands VALUES (?1, 'workspaces', ?2)", (PROJECT, &command));
    let approved = job(&f, "custom:workspaces", NOW);
    let sample = || collect(&approved.source, approved.dir.as_deref(), NOW, vec![]);
    let valid = sample().await.unwrap();
    assert_eq!(sample().await.unwrap(), valid);
    assert_eq!(serde_json::from_str::<serde_json::Value>(&valid).unwrap()[0]["value"], 11.0);
    assert!(f.store.with(|tx| publish(tx, &approved, NOW, Ok(valid.clone()))).unwrap());
    assert!(!f.store.with(|tx| publish(tx, &approved, NOW, Ok(valid))).unwrap());

    assert!(matches!(metrics(format!("{line}\ninvalid").as_bytes(), NOW), Err(Failure::Invalid)));
    assert!(matches!(metrics(format!("{line}\n{line}").as_bytes(), NOW), Err(Failure::Invalid)));
    let tiny = Duration::from_millis(150);
    assert_eq!(probe_command::shell(dir, "sleep 30 & exit 0", tiny, 512).await, Err(Failure::Timeout));
    assert_eq!(probe_command::shell(dir, "printf 'too much output'", TIMEOUT, 4).await, Err(Failure::Output));
    assert_eq!(probe_command::shell(dir, "exit 2", TIMEOUT, 512).await, Err(Failure::Exit));

    f.write("UPDATE supervisor_probe_commands SET command='printf junk' WHERE project=?1", [PROJECT]);
    let failed = job(&f, "custom:workspaces", NOW + 1);
    let result = collect(&failed.source, failed.dir.as_deref(), NOW + 1, vec![]).await;
    assert_eq!(result, Err(Failure::Invalid));
    assert!(!f.store.with(|tx| publish(tx, &approved, NOW + DAY, result.clone())).unwrap(),
        "a command replaced while in flight cannot publish");
    assert!(f.store.with(|tx| publish(tx, &failed, NOW + 1, result)).unwrap());
    let detector = Fixture::detector(Rule::ProbeFailed.id());
    evaluate(&f.store, PROJECT, NOW + 2, std::slice::from_ref(&detector)).unwrap().record(&f.store).unwrap();
    let record = signals(&f.store, PROJECT).unwrap().pop().unwrap();
    assert_eq!((record.detector.as_str(), record.subject_id.as_str()), ("self.probe_failed", "custom:workspaces"));
    assert!(matches!(&record.evidence[..], [Evidence::SupervisorProbes(_)]));
    assert!(f.store.snapshot(SCHEMA, |tx| Ok(read::<Vec<serde_json::Value>>(tx, PROJECT, "custom:workspaces", NOW + 2)?.is_none())).unwrap());

    let gate = job(&f, GATE, NOW + 2);
    let unavailable = collect(&gate.source, gate.dir.as_deref(), NOW + 2, vec![]).await;
    assert_eq!(unavailable, Err(Failure::Unavailable));
    assert!(f.store.with(|tx| publish(tx, &gate, NOW + 2, unavailable)).unwrap());
    assert_eq!(f.store.snapshot(SCHEMA, |tx| Ok(statuses(tx, PROJECT, NOW + 2)?.into_iter()
        .find(|s| s.source == GATE).unwrap().state)).unwrap(), "source unavailable");
}

#[tokio::test]
async fn ac261_git_release_metrics_and_owner_split() {
    let f = Fixture::new("release-inputs");
    let dir = f.path.parent().unwrap();
    enabled(&f, dir);
    f.store.with(|tx| crate::tasks::integration::keep(tx, PROJECT,
        &crate::tasks::integration::Settings { on: true, ..Default::default() })).unwrap();
    let init = NOW - 63 * DAY;
    git(dir, init, &["init", "-q", "-b", "main"]);
    git(dir, init, &["commit", "--allow-empty", "-qm", "initial"]);
    git(dir, init, &["branch", "unrelated"]);
    let created = NOW - 20 * DAY;
    let committed = NOW - 12 * DAY;
    git(dir, committed, &["commit", "--allow-empty", "-qm", "feature"]);
    let feature = git(dir, committed, &["rev-parse", "HEAD"]);
    git(dir, NOW - 8 * DAY, &["tag", "-am", "feature release", "v1.1.0"]);
    git(dir, NOW - 6 * DAY, &["tag", "-am", "patch release", "v1.1.1"]);
    git(dir, NOW - 5 * DAY, &["commit", "--allow-empty", "-qm", "unreleased"]);
    let unreleased = git(dir, NOW - 5 * DAY, &["rev-parse", "HEAD"]);
    git(dir, NOW - 14 * DAY, &["checkout", "-q", "unrelated"]);
    git(dir, NOW - 14 * DAY, &["commit", "--allow-empty", "-qm", "unrelated"]);
    git(dir, NOW - 10 * DAY, &["tag", "-am", "other branch", "v1.0.0"]);
    git(dir, NOW - 4 * DAY, &["checkout", "-q", "main"]);

    f.task("feature", Some("Feature"), "P2", "INTEGRATION", created);
    f.task("unreleased", None, "P2", "INTEGRATION", NOW - 10 * DAY);
    for (task, commit, at) in [("feature", &feature, NOW - 11 * DAY), ("unreleased", &unreleased, NOW - 4 * DAY)] {
        f.event(task, at, "dispatcher", "comment", "Landed", Some(json!({"left":"landed", "commit":commit})));
        f.transition(task, at, "INTEGRATION", "DONE");
    }
    let release = job(&f, GIT, NOW);
    let inputs = f.store.snapshot(SCHEMA, |tx| integrated(tx, PROJECT, NOW)).unwrap();
    assert_eq!(inputs.len(), 2);
    let data = collect(&release.source, release.dir.as_deref(), NOW, inputs).await.unwrap();
    let observation: probe_git::Releases = serde_json::from_str(&data).unwrap();
    assert_eq!(observation.tags[observation.tasks["feature"].first_tag.unwrap()].name, "v1.1.0");
    assert_eq!(observation.tasks["unreleased"].first_tag, None);
    assert!(f.store.with(|tx| publish(tx, &release, NOW, Ok(data))).unwrap());

    for (i, kind) in [Some("operational"), Some("strategic"), None].into_iter().enumerate() {
        let at = NOW - 200 + i as i64 * 20;
        let data = kind.map_or_else(|| json!({"questions":true}), |kind| json!({"questions":true,"question":{"kind":kind}}));
        let question = f.event("feature", at, "dispatcher", "comment", "A question", Some(data));
        f.event("feature", at + 1, "human", "comment", "An answer",
            (i == 1).then(|| json!({"choice":{"question":question,"option":"option"}})));
    }
    f.agent("fixture-agent");
    f.run("feature", "feature-run", "fixture-agent", NOW - 30);
    f.run("unreleased", "untyped-run", "fixture-agent", NOW - 20);
    f.cost("feature-run", NOW - 10, 3000);
    f.cost("untyped-run", NOW - 10, 1000);

    let board = || f.store.snapshot(SCHEMA, |tx| Ok(scoreboard::indicators(&scoreboard::read(tx, PROJECT, NOW, init)?))).unwrap();
    let readings = board();
    let metric = |indicator: usize, name| readings[indicator].lines.iter().find(|line| line.name == name).unwrap().current;
    assert_eq!(metric(2, "Median task-days to first release"), Some(12.0));
    assert_eq!(metric(2, "Unreleased tasks with known commits"), Some(1.0));
    assert_eq!(metric(1, "Patches within three days"), Some(1.0));
    assert!(metric(3, "Releases per month").unwrap() > 3.0);
    assert_eq!(metric(6, "Operational answers per week"), Some(0.25));
    assert_eq!(metric(6, "Strategic answers per week"), Some(0.25));
    assert_eq!(metric(6, "Answers without a recorded kind per week"), Some(0.25));
    assert_eq!(metric(6, "Share of answers without a recorded kind"), Some(1.0 / 3.0));

    f.store.snapshot(SCHEMA, |tx| {
        let allocation = super::super::spend::allocation_view(tx, PROJECT, NOW - DAY, NOW, &BTreeMap::new())?;
        let share = |kind| allocation.rows.iter().find(|r| r.task_type == kind).unwrap().share.unwrap();
        assert!((share("Feature") - 0.75).abs() < 1e-9);
        assert!((share("untyped") - 0.25).abs() < 1e-9);
        assert!(allocation.rows.iter().all(|r| r.target.is_none()));
        let targets = [("Feature".to_owned(), 0.6), ("Bug".to_owned(), 0.4)].into_iter().collect();
        let allocation = super::super::spend::allocation_view(tx, PROJECT, NOW - DAY, NOW, &targets)?;
        assert_eq!(allocation.rows.iter().find(|r| r.task_type == "Feature").unwrap().target, Some(0.6));
        assert_eq!(allocation.rows.iter().find(|r| r.task_type == "Bug").unwrap().share, Some(0.0));
        Ok(())
    }).unwrap();

    f.write("UPDATE project_integration SET is_on=0 WHERE project=?1", [PROJECT]);
    let off = board();
    assert_eq!(off[2].lines[1].missing, "Project integration is off.");
    assert_eq!(off[2].lines[1].current, None);
    assert!(off[2].lines[0].current.is_some(), "time to DONE remains measured");
    assert!(off[3].lines[0].current.is_some(), "release cadence is independent of integration");
}

#[tokio::test]
async fn ac260_standard_architecture_and_gate_detectors() {
    let f = Fixture::new("standard-probes");
    let dir = f.path.parent().unwrap();
    enabled(&f, dir);
    f.write("INSERT INTO supervisor_settings (project, revision, pack) VALUES (?1, 1, ?2)",
        (PROJECT, json!({"probes":{"git_file_line_cap":10}}).to_string()));
    let then = NOW - 36 * DAY;
    git(dir, then, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("schema1.sql"), "CREATE TABLE fixture (id int);\n").unwrap();
    std::fs::write(dir.join("app.rs"), "fn main() {}\n").unwrap();
    std::fs::write(dir.join("Cargo.lock"), "[[package]]\nname = \"fixture\"\nversion = \"1.0.0\"\n").unwrap();
    git(dir, then, &["add", "schema1.sql", "app.rs", "Cargo.lock"]);
    git(dir, then, &["commit", "-qm", "initial"]);
    let at = NOW - DAY;
    std::fs::write(dir.join("schema1.sql"), "CREATE TABLE fixture (id text);\n").unwrap();
    std::fs::write(dir.join("schema2.sql"), "CREATE TABLE newer (id int);\n").unwrap();
    std::fs::write(dir.join("app.rs"), "fn main() {\n\n\n\n\n\n\n\n}\n").unwrap();
    std::fs::write(dir.join("Cargo.lock"), "[[package]]\nname = \"fixture\"\nversion = \"1.1.0\"\n\n[[package]]\nname = \"new-package\"\nversion = \"1.0.0\"\n").unwrap();
    std::fs::write(dir.join("package-lock.json"), "{\n  \"packages\": {\n    \"workspace\": {\n      \"version\": \"1.0.0\"\n    }\n  }\n}\n").unwrap();
    git(dir, at, &["add", "schema1.sql", "schema2.sql", "app.rs", "Cargo.lock", "package-lock.json"]);
    git(dir, at, &["commit", "-qm", "fix: typed example"]);
    let path = dir.join(".git/terminum-gate.jsonl");
    let rows = [
        json!({"kind":"gate","at":"","at_unix":then,"by":"rehearse","result":"pass"}),
        json!({"kind":"gate","at":"","at_unix":at,"by":"rehearse","result":"fail",
            "steps":{"check":{"retried":["fixture"],"cause":"ignored prose"}}}),
    ];
    std::fs::write(path, format!("{}\n{}\n{{", rows[0], rows[1])).unwrap();
    for source in [ARCH, GATE] {
        let job = job(&f, source, NOW);
        let result = collect(&job.source, job.dir.as_deref(), NOW, Vec::new()).await.unwrap();
        assert!(!result.contains("ignored prose"));
        assert!(f.store.with(|tx| publish(tx, &job, NOW, Ok(result))).unwrap());
    }
    let findings = |rule| evaluate(&f.store, PROJECT, NOW, &[Fixture::detector(rule)]).unwrap();
    let migration = findings("arch.migration_edited");
    assert_eq!(migration.detectors[0].findings[0].subject_id, "schema1.sql");
    assert_eq!(migration.detectors[0].findings.len(), 1);
    let dependencies = findings("arch.dependency");
    assert_eq!(dependencies.detectors[0].findings[0].measure.value, 1.0);
    let hot = findings("arch.hotspot");
    assert!(hot.detectors[0].findings.iter().any(|r| r.subject_id == "app.rs" && r.metric == "file_line_limit_share"));
    let flaky = findings("flow.flaky");
    assert_eq!(flaky.detectors[0].findings.iter().map(|r| r.metric).collect::<Vec<_>>(), ["gate_retries", "nightly_reds"]);
}
