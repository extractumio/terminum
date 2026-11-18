// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-205: the words a resumed agent hears, and the hour against a server that keeps ending.
use super::*;

#[test]
fn ac205_the_words_are_fixed_with_the_open_tasks_ids_alone() {
    let w = words(&["API-12".into()]);
    assert!(w.starts_with("[terminum Dispatcher] API-12: your session ended because terminum's tmux server stopped (a crash or a restart of the machine), and Dispatcher resumed it."), "{w}");
    assert!(w.contains("do not redo finished steps") && w.contains("Programs and servers you ran in the background are gone")
        && w.ends_with("check your working tree (git status, files half written, a stale .git/index.lock) and start again what you still need."), "{w}");
    assert!(!w.chars().any(char::is_control), "typed through the gate: one line, no control characters");
    assert!(words(&["API-12".into(), "API-13".into()]).starts_with("[terminum Dispatcher] API-12, API-13: "), "a main task's run names each open task (AC-185)");
}

#[test]
fn ac205_one_end_resumes_all_it_cut_off_and_another_within_the_hour_none() {
    let t0 = 1_000_000;
    let first = breaker(None, "a1", t0).expect("a server's first end resumes");
    assert_eq!(first, ("a1".to_string(), t0));
    assert_eq!(breaker(Some(first.clone()), "a1", t0 + 5), Some(first.clone()), "the same end's other runs resume, the hour unmoved");
    assert_eq!(breaker(Some(first.clone()), "b2", t0 + AGAIN_WITHIN - 1), None, "another end within the hour of the resume: none");
    assert_eq!(breaker(Some(first), "c3", t0 + AGAIN_WITHIN), Some(("c3".to_string(), t0 + AGAIN_WITHIN)), "an hour on, it resumes again");
}
