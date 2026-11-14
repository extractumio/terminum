// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Choosing a batch (terminum-integration.md AC-188, AC-189) on a real repository in a temporary folder.
use super::*;

fn sh(dir: &Path, args: &[&str]) -> String {
    let o = Command::new("git").arg("-C").arg(dir).args(["-c", "user.name=t", "-c", "user.email=t@example.com"]).args(args).output().unwrap();
    assert!(o.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&o.stderr));
    out(&o)
}

/// A repository with a.txt of three lines on main, and a branch a task, each a commit on main's base.
struct Repo {
    dir: std::path::PathBuf,
    base: String,
}

fn repo(name: &str) -> Repo {
    let dir = crate::testutil::fresh_dir(name);
    sh(&dir, &["init", "-q", "-b", "main"]);
    sh(&dir, &["config", "user.name", "t"]);
    sh(&dir, &["config", "user.email", "t@example.com"]);
    std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    sh(&dir, &["add", "a.txt"]);
    sh(&dir, &["commit", "-qm", "base"]);
    let base = sh(&dir, &["rev-parse", "HEAD"]);
    Repo { dir, base }
}

/// A task's commit on `from`: `file` written with `text`.
fn task(r: &Repo, id: &str, from: &str, file: &str, text: &str) -> Queued {
    sh(&r.dir, &["checkout", "-q", "--detach", from]);
    std::fs::write(r.dir.join(file), text).unwrap();
    sh(&r.dir, &["add", file]);
    sh(&r.dir, &["commit", "-qm", id]);
    Queued { task: format!("k-{id}"), id: id.into(), commit: sh(&r.dir, &["rev-parse", "HEAD"]) }
}

#[test]
fn ac188_the_queue_s_order_merges_cleanly_into_one_candidate_up_to_the_size() {
    let r = repo("ib-clean");
    let (a, b, c) = (task(&r, "API-1", &r.base, "b.txt", "b\n"), task(&r, "API-2", &r.base, "c.txt", "c\n"), task(&r, "API-3", &r.base, "d.txt", "d\n"));
    let got = choose(&r.dir, &r.base, &[a.clone(), b.clone(), c.clone()], 2).unwrap();
    assert_eq!(got.chosen, [a.clone(), b.clone()], "at most the size, in the queue's order");
    let cand = got.candidate.unwrap();
    assert_eq!(sh(&r.dir, &["log", "--format=%s", "--first-parent", &format!("{}..{cand}", r.base)]), "Integrate API-2\nIntegrate API-1", "a merge commit a task, named by its ID alone");
    assert_eq!(sh(&r.dir, &["ls-tree", "--name-only", &cand]), "a.txt\nb.txt\nc.txt");
    assert!(got.returned.is_empty() && got.held.is_empty());
}

#[test]
fn ac189_a_conflict_with_the_target_returns_and_one_with_a_batch_mate_is_held() {
    let r = repo("ib-conflicts");
    // main moved past what API-1 was made on, in the same line
    sh(&r.dir, &["checkout", "-q", "main"]);
    std::fs::write(r.dir.join("a.txt"), "one\nTWO\nthree\n").unwrap();
    sh(&r.dir, &["commit", "-qam", "main moved"]);
    let target = sh(&r.dir, &["rev-parse", "HEAD"]);
    let a = task(&r, "API-1", &r.base, "a.txt", "one\nzwei\nthree\n");
    let b = task(&r, "API-2", &target, "b.txt", "b\n");
    let c = task(&r, "API-3", &target, "b.txt", "other b\n");
    let got = choose(&r.dir, &target, &[a.clone(), b.clone(), c.clone()], 4).unwrap();
    assert_eq!(got.returned, [(a, Conflict::Target(vec!["a.txt".into()]))], "back to its agent with the files");
    assert_eq!(got.chosen, [b]);
    assert_eq!(got.held, [(c, vec!["API-2".into()])], "it merges with the target, not with API-2: it waits for API-2");
}

#[test]
fn ac189_a_commit_carrying_another_s_never_goes_without_it() {
    let r = repo("ib-carried");
    let a = task(&r, "API-1", &r.base, "b.txt", "b\n");
    let b = task(&r, "API-2", &a.commit, "c.txt", "c\n");
    // queued before the one it carries: held for it; after it in the queue, it joins with it
    let got = choose(&r.dir, &r.base, &[b.clone(), a.clone()], 4).unwrap();
    assert_eq!(got.held, [(b.clone(), vec!["API-1".into()])]);
    assert_eq!(got.chosen, [a.clone()]);
    let both = choose(&r.dir, &r.base, &[a.clone(), b.clone()], 4).unwrap();
    assert_eq!(both.chosen, [a, b]);
}

#[test]
fn ac188_everything_conflicting_gives_one_task_a_round() {
    let r = repo("ib-all");
    let qs: Vec<Queued> = (1..=3).map(|n| task(&r, &format!("API-{n}"), &r.base, "a.txt", &format!("one\n{n}\nthree\n"))).collect();
    let got = choose(&r.dir, &r.base, &qs, 8).unwrap();
    assert_eq!(got.chosen, [qs[0].clone()]);
    assert_eq!(got.held.iter().map(|(q, w)| (q.id.as_str(), w.clone())).collect::<Vec<_>>(), [("API-2", vec!["API-1".to_string()]), ("API-3", vec!["API-1".to_string()])]);
}
