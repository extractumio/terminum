// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A project's integration settings (terminum-integration.md AC-186).
use std::process::Command;

use super::*;
use crate::tasks::store::Store;

fn store(name: &str) -> Store {
    let s = Store::open(crate::testutil::fresh_dir(name).join("kanban.sqlite"));
    s.with(|tx| {
        tx.execute("INSERT INTO projects (id, name, color, dir, place) VALUES ('p1', 'Billing', 0, '/srv/billing', 0)", []).map_err(sql)?;
        tx.execute("INSERT INTO projects (id, name, color, dir, place) VALUES ('p2', 'Web', 1, '/srv/web', 1)", []).map_err(sql)?;
        tx.execute("INSERT INTO agents (id, project, name, kind, command, color) VALUES ('a1', 'p1', 'Opus reviewer', 'claude', 'claude', 0)", []).map_err(sql)?;
        tx.execute("INSERT INTO agents (id, project, name, kind, command, color) VALUES ('a2', 'p2', 'Web coder', 'claude', 'claude', 1)", []).map_err(sql)?;
        Ok(())
    }).unwrap();
    s
}

fn git(dir: &FsPath, args: &[&str]) {
    let done = Command::new("git").arg("-C").arg(dir).args(["-c", "user.name=t", "-c", "user.email=t@example.com"]).args(args).output().unwrap();
    assert!(done.status.success(), "{}", String::from_utf8_lossy(&done.stderr));
}

#[test]
fn ac186_a_project_with_none_has_the_defaults_off() {
    let s = store("integration-defaults");
    let got = s.with(|tx| read(tx, "p1")).unwrap();
    assert_eq!(got, Settings::default());
    assert!(!got.on);
    assert_eq!((got.target.as_str(), got.batch_most, got.check_minutes, got.agent_budget_minutes, got.red_rounds_most), ("origin/main", 8, 90, 60, 3));
    assert_eq!(got.agent, None, "no agent: a red batch waits for a human");
}

#[test]
fn ac186_kept_as_given_and_read_back() {
    let s = store("integration-kept");
    let wanted = Settings { on: true, check: "make smoke".into(), after: "make install".into(), agent: Some("a1".into()), batch_most: 4, ..Settings::default() };
    s.with(|tx| keep(tx, "p1", &wanted)).unwrap();
    assert_eq!(s.with(|tx| read(tx, "p1")).unwrap(), wanted);
    let off = Settings { on: false, ..wanted.clone() };
    s.with(|tx| keep(tx, "p1", &off)).unwrap();
    assert_eq!(s.with(|tx| read(tx, "p1")).unwrap(), off, "turned off, the rest stays");
    assert_eq!(s.with(|tx| read(tx, "p2")).unwrap(), Settings::default(), "each project its own");
}

#[test]
fn ac186_the_default_prompt_stays_the_default() {
    let s = store("integration-prompt");
    s.with(|tx| keep(tx, "p1", &Settings { prompt: DEFAULT_PROMPT.into(), ..Settings::default() })).unwrap();
    assert_eq!(s.with(|tx| read(tx, "p1")).unwrap().prompt, "", "a later default reaches it");
    s.with(|tx| keep(tx, "p1", &Settings { prompt: "Read the log first.".into(), ..Settings::default() })).unwrap();
    assert_eq!(s.with(|tx| read(tx, "p1")).unwrap().prompt, "Read the log first.");
}

#[test]
fn ac186_the_agent_is_one_of_its_own_pool() {
    let s = store("integration-agent");
    for (other, words) in [("a2", "Web coder is Web's agent, not Billing's: choose one of its own."), ("gone", "That agent is gone.")] {
        assert_eq!(s.with(|tx| keep(tx, "p1", &Settings { agent: Some(other.into()), ..Settings::default() })).unwrap_err(), words);
    }
    s.with(|tx| keep(tx, "p1", &Settings { agent: Some("a1".into()), ..Settings::default() })).unwrap();
    s.with(|tx| tx.execute("DELETE FROM agents WHERE id = 'a1'", []).map_err(sql)).unwrap();
    assert_eq!(s.with(|tx| read(tx, "p1")).unwrap().agent, None, "an agent deleted leaves none");
    assert_eq!(s.with(|tx| keep(tx, "gone", &Settings::default())).unwrap_err(), "That project is gone.");
}

#[test]
fn ac186_each_field_refused_in_its_words() {
    let d = Settings::default;
    let cases: Vec<(Settings, &str)> = vec![
        (Settings { check: "x".repeat(2001), ..d() }, "Check: one line, at most 2,000 characters."),
        (Settings { publish: "a\nb".into(), ..d() }, "Publish: one line, at most 2,000 characters."),
        (Settings { after: "make\u{7}".into(), ..d() }, "After: one line, at most 2,000 characters."),
        (Settings { target: "main".into(), ..d() }, "main: name a remote's branch, as origin/main."),
        (Settings { target: "origin/-x".into(), ..d() }, "origin/-x: name a remote's branch, as origin/main."),
        (Settings { target: String::new(), ..d() }, "Target: name a remote's branch, as origin/main."),
        (Settings { batch_most: 0, ..d() }, "Tasks in a batch: 1 to 8."),
        (Settings { batch_most: 9, ..d() }, "Tasks in a batch: 1 to 8."),
        (Settings { check_minutes: 1441, ..d() }, "Check ends after: 1 to 1440 minutes."),
        (Settings { agent_budget_minutes: 4, ..d() }, "Time budget: 5 to 1440 minutes."),
        (Settings { red_rounds_most: 11, ..d() }, "Red rounds: 1 to 10."),
        (Settings { prompt: "x".repeat(8 * 1024 + 1), ..d() }, "Prompt: at most 8 KiB."),
    ];
    for (s, words) in cases {
        assert_eq!(check_fields(&s).unwrap_err(), words);
    }
    assert!(check_fields(&Settings { check: "x".repeat(2000), batch_most: 1, check_minutes: 1440, red_rounds_most: 10, ..d() }).is_ok(), "the bounds themselves are fine");
}

#[test]
fn ac186_a_page_s_body_keeps_what_it_left_out() {
    let s = Settings::from_body(&json!({"on": true, "check": "  make smoke ", "agent": ""})).unwrap();
    assert_eq!(s, Settings { on: true, check: "make smoke".into(), ..Settings::default() });
    assert_eq!(Settings::from_body(&json!({"batch_most": "eight"})).unwrap_err(), "Tasks in a batch: 1 to 8.");
    assert_eq!(Settings::from_body(&json!({"on": "yes"})).unwrap_err(), "on: true or false", "never off by a wrong type");
}

#[test]
fn ac186_where_it_runs_a_checkout_its_target_and_git() {
    let root = crate::testutil::fresh_dir("integration-place");
    assert_eq!(check_place(&root.join("none"), "origin/main", "git").unwrap_err(), format!("{} is not a git checkout.", root.join("none").display()));
    std::fs::create_dir_all(root.join("plain")).unwrap();
    assert_eq!(check_place(&root.join("plain"), "origin/main", "git").unwrap_err(), format!("{} is not a git checkout.", root.join("plain").display()));
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["commit", "-q", "--allow-empty", "-m", "one"]);
    assert_eq!(check_place(&repo, "origin/main", "git").unwrap_err(), "origin/main: no such branch at origin.");
    git(&repo, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
    check_place(&repo, "origin/main", "git").unwrap();
    let old = root.join("old-git");
    std::fs::write(&old, "#!/bin/sh\necho 'git version 2.30.1'\n").unwrap();
    std::fs::set_permissions(&old, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    assert_eq!(check_place(&repo, "origin/main", old.to_str().unwrap()).unwrap_err(), "git 2.38 or later is needed: this is 2.30.1.");
}

#[test]
fn ac186_what_runs_says_each_line_and_where() {
    let empty = runs("p1", "/srv/billing", &Settings::default());
    assert!(empty[0].contains("integration/p1/tree: origin/main and a merge of each task's commit"), "{}", empty[0]);
    assert_eq!(&empty[1..4], ["No check: a candidate that merges is published.", "terminum pushes the candidate to origin/main, never forced.", "Nothing runs after."]);
    assert!(empty[4].contains("-lc`, with your PATH and logins, and TM_INTEGRATION_BATCH") && empty[4].ends_with("nothing of terminum's token."), "{}", empty[4]);
    let set = runs("p1", "/srv/billing", &Settings { check: "make smoke".into(), publish: "scripts/publish.sh".into(), after: "make install".into(), ..Settings::default() });
    assert!(set[1].starts_with("Check runs `make smoke` in ") && set[2].starts_with("Publish runs `scripts/publish.sh` in "), "{set:?}");
    assert_eq!(set[3], "After runs `make install` in /srv/billing, once a batch.");
}
