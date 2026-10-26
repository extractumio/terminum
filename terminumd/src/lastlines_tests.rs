// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::*;
use crate::testutil::fresh_dir;

fn row(pane: &str, activity: u64) -> Row {
    Row { identity: "abc".into(), pane: pane.into(), activity, alt: false, history: 0, session: "main".into(), window: "api".into(), dir: "/Users/alex".into() }
}

fn lines(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_read_keeps_shown_panes_but_quick_shells_with_the_identity_of_the_same_answer() {
    let read = lines(&["abc\t%1\t100\t0\t40\t\tmain\tapi\t/Users/alex/api", "abc\t%2\t90\t1\t0\t\tmain\tvim\t/Users/alex/a\tb",
                       "abc\t%3\t95\t0\t0\t%1\tmain\tquick\t/tmp", "abc\t%4\t95\t0\t0\t\tother\tx\t/tmp", "garbage"]);
    let r = rows(&read, |s| s == "main");
    assert_eq!(r.len(), 2, "a quick shell (AC-88) and a session not shown are left out: {r:?}");
    assert_eq!((r[0].identity.as_str(), r[0].pane.as_str(), r[0].activity, r[0].alt, r[0].history), ("abc", "%1", 100, false, 40));
    assert_eq!((r[1].alt, r[1].dir.as_str()), (true, "/Users/alex/a\tb"), "a folder keeps its tab");
}

#[test]
fn the_tail_drops_blank_ends_and_keeps_the_last_lines_in_order() {
    let history = lines(&["one  ", "two"]);
    let screen = lines(&["three", "", "four   ", "", ""]);
    assert_eq!(tail(&[history.clone(), screen.clone()], 300), "one\ntwo\nthree\n\nfour");
    assert_eq!(tail(&[history, screen], 2), "\nfour");
    assert_eq!(tail(&[lines(&["a"]), lines(&["$ vim notes", "", ""])], 300), "a\n$ vim notes", "the history, then the normal screen");
    assert_eq!(tail(&[lines(&["", ""])], 300), "");
}

#[test]
fn under_a_full_screen_program_the_normal_screen_is_read_not_the_programs() {
    let mut r = row("%3", 1);
    assert_eq!(captures(&r, 300), vec!["capture-pane -p -J -t %3 -S -600"], "twice the rows: joined, wrapped rows give fewer lines");
    r.alt = true;
    r.history = 12;
    let alt = captures(&r, 300);
    assert_eq!(alt.len(), 2);
    assert!(alt[0].ends_with("-S -600 -E -1") && alt[1].contains(" -a "), "{alt:?}");
    r.history = 0;
    assert_eq!(captures(&r, 300).len(), 1, "no history: its capture would be the screen's first line");
}

#[test]
fn files_are_named_by_server_identity_and_pane() {
    assert_eq!(folder("local:terminum"), "local@terminum");
    assert_eq!(folder("dev box:default/x"), "dev_box@default_x");
    let d = std::path::Path::new("/s");
    assert_eq!(file(d, "abc", "%7", false), std::path::PathBuf::from("/s/abc_7.json"));
    assert_eq!(file(d, "abc", "%7", true), std::path::PathBuf::from("/s/abc_7.before.json"));
    assert!(usable_identity("0f3a9c") && !usable_identity("") && !usable_identity("../x") && !usable_identity(&"a".repeat(65)));
}

#[test]
fn a_pane_is_read_again_only_after_its_window_printed() {
    let mut seen = Seen::default();
    let r = [row("%1", 100), row("%2", 50)];
    assert_eq!(seen.due(&r).len(), 2, "never read: both");
    assert!(seen.read("%1", 100, "hi") && seen.read("%2", 100, "x"));
    assert!(!seen.read("%2", 100, "x"), "the same text is not written again");
    let later = [row("%1", 100), row("%2", 99)];
    assert_eq!(seen.due(&later).iter().map(|r| r.pane.as_str()).collect::<Vec<_>>(), ["%1"], "printed in the second it was read: read again");
    assert_eq!(seen.due(&[row("%1", 99), row("%2", 101)]).iter().map(|r| r.pane.as_str()).collect::<Vec<_>>(), ["%2"]);
}

#[test]
fn a_pane_closes_only_when_absent_from_reads_30_s_apart() {
    let mut seen = Seen::default();
    seen.read("%1", 1, "a");
    seen.read("%2", 1, "b");
    let t0 = Instant::now();
    assert!(seen.closed(&[row("%1", 1)], t0).is_empty(), "absent once: kept (a reboot ends programs before tmux)");
    assert!(seen.closed(&[row("%1", 1)], t0 + Duration::from_secs(10)).is_empty());
    assert!(seen.closed(&[row("%1", 1), row("%2", 1)], t0 + Duration::from_secs(20)).is_empty(), "back: no longer absent");
    assert!(seen.closed(&[row("%1", 1)], t0 + Duration::from_secs(40)).is_empty(), "absent anew: its 30 s start again");
    assert_eq!(seen.closed(&[row("%1", 1)], t0 + Duration::from_secs(70)), ["%2"]);
    assert!(seen.closed(&[row("%1", 1)], t0 + Duration::from_secs(200)).is_empty(), "closed once, forgotten");
}

#[test]
fn panes_with_files_from_an_earlier_terminumd_are_known_so_one_closed_meanwhile_goes() {
    let dir = fresh_dir("lastlines-seed");
    for f in ["abc_3.json", "abc_4.before.json", "old_5.json", ".abc_6.json.1.2.new", "abc_x.json"] {
        std::fs::write(dir.join(f), "{}").unwrap();
    }
    let mut seen = Seen::new("abc", &dir);
    let mut known: Vec<&String> = seen.panes.keys().collect();
    known.sort();
    assert_eq!(known, ["%3", "%4"], "only this identity's panes");
    let t0 = Instant::now();
    seen.closed(&[row("%3", 1)], t0);
    assert_eq!(seen.closed(&[row("%3", 1)], t0 + GONE_FOR), ["%4"]);
}

#[test]
fn the_sweep_keeps_the_current_identity_and_young_files_and_removes_old_ones() {
    let dir = fresh_dir("lastlines-sweep");
    let srv = dir.join("local@terminum");
    std::fs::create_dir_all(&srv).unwrap();
    let old = std::time::SystemTime::now() - Duration::from_secs(8 * 86400);
    for f in ["now_1.json", "now_2.before.json", "gone_1.json", "young_1.json", ".now_1.json.9.9.new"] {
        let h = std::fs::File::create(srv.join(f)).unwrap();
        if f != "young_1.json" {
            h.set_modified(old).unwrap();
        }
    }
    let other = dir.join("devbox@default");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::File::create(other.join("x_1.json")).unwrap().set_modified(old).unwrap();
    sweep(&dir, &HashMap::from([("local@terminum".to_string(), "now".to_string())]), KEEP_OLD);
    let mut left: Vec<String> = std::fs::read_dir(&srv).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
    left.sort();
    assert_eq!(left, ["now_1.json", "now_2.before.json", "young_1.json"]);
    let age = std::fs::metadata(srv.join("now_1.json")).unwrap().modified().unwrap().elapsed().unwrap();
    assert!(age < Duration::from_secs(60), "the current identity's files are touched: {age:?}");
    assert!(!other.exists(), "a server folder left empty goes");
}

#[test]
fn a_restart_hands_on_the_old_panes_lines_or_those_from_an_earlier_restart_under_a_mere_prompt() {
    let dir = fresh_dir("lastlines-hand");
    let write = |name: &str, text: &str| std::fs::write(dir.join(name), json!({"text": text}).to_string()).unwrap();
    let text = |name: &str| serde_json::from_slice::<Value>(&std::fs::read(dir.join(name)).unwrap()).unwrap()["text"].as_str().unwrap().to_string();
    write("a_1.json", "$ make\nbuilt\n$");
    write("a_1.before.json", "older");
    assert!(hand_on(&dir.join("a_1.json"), &dir.join("a_1.before.json"), &dir.join("b_4.before.json")));
    assert_eq!(text("b_4.before.json"), "$ make\nbuilt\n$", "it printed since: its own lines");
    let age = std::fs::metadata(dir.join("b_4.before.json")).unwrap().modified().unwrap().elapsed().unwrap();
    assert!(age < Duration::from_secs(60), "touched: the sweep ages it from now");
    write("c_2.json", "\n$ ");
    write("c_2.before.json", "the agent's last turn");
    assert!(hand_on(&dir.join("c_2.json"), &dir.join("c_2.before.json"), &dir.join("d_2.before.json")));
    assert_eq!(text("d_2.before.json"), "the agent's last turn", "a second restart before it printed: the lines from before the first");
    write("e_3.before.json", "kept");
    assert!(hand_on(&dir.join("e_3.json"), &dir.join("e_3.before.json"), &dir.join("f_3.before.json")), "no live file: the earlier ones");
    assert!(!hand_on(&dir.join("x_9.json"), &dir.join("x_9.before.json"), &dir.join("y_9.before.json")), "nothing kept: nothing handed on");
}
