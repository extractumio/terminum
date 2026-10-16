// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Folds of a pane's history (AC-32): an edit's diff, a long command and a long output of a
//! coding agent's tools, rows of encoded data and nearly equal lines; and what never folds.
//! Ported from iterm-enhancer's test_web_fold.py, case for case.

use super::*;
use serde_json::json;

const B64: &str = "iVBORw0KGgoAAAANSUhEUgAAALgAAABQCAQAAAAALXejAAAAAmJLR0QA/4ePzL8AAAAHdElNRQfqCR0J";

fn rows<S: AsRef<str>>(lines: &[S]) -> Vec<(String, bool)> {
    lines.iter().map(|t| (t.as_ref().to_string(), true)).collect()
}

fn spans(found: &[RowFold]) -> Vec<(usize, usize, String)> {
    found.iter().map(|f| (f.from, f.to, f.label.clone())).collect()
}

fn one(a: usize, b: usize, label: &str) -> Vec<(usize, usize, String)> {
    vec![(a, b, label.to_string())]
}

fn cat(parts: &[&[String]]) -> Vec<String> {
    parts.iter().flat_map(|p| p.iter().cloned()).collect()
}

fn s(texts: &[&str]) -> Vec<String> {
    texts.iter().map(|t| t.to_string()).collect()
}

#[test]
fn an_edit_diff_folds_under_its_line() {
    let diff: Vec<String> = (1..13).map(|k| format!("      {k} {}  line {k}", if k % 2 == 1 { '+' } else { '-' })).collect();
    let r = rows(&cat(&[&s(&["⏺ Update(src/app.ts)", "  ⎿  Updated src/app.ts with 6 additions and 6 removals"]), &diff, &s(&["", "⏺ Done."])]));
    assert_eq!(spans(&folds(&r, true)), one(2, 13, "⋯ 12 lines of diff"));
    let short = rows(&["⏺ Update(a.ts)", "  ⎿  Updated a.ts with 1 addition", "      1 +  x", "      2    y", "", "⏺ ok"]);
    assert_eq!(folds(&short, true), vec![], "a short diff stays");
}

#[test]
fn a_tools_output_with_blank_lines_its_long_command_and_blank_runs_fold() {
    let mut script = s(&["#!/usr/bin/env bash", "# Pure bundle-identity selector.", "set -euo pipefail", "",
        "CANONICAL_BUNDLE_ID=\"app.iqtz.iquantize\"", "", "is_iquantize_bundle_id() {",
        "  local bundle_id=\"$1\"", "}", "", "select_iquantize_bundle_id() {"]);
    script.extend((0..20).map(|k| format!("  step {k}")));
    script.push("}".into());
    let body: Vec<String> = script[1..].iter().map(|t| if t.is_empty() { String::new() } else { format!("     {t}") }).collect();
    let r = rows(&cat(&[
        &s(&["⏺ iPhone 14 Pro is connected.", "",
            "⏺ Bash(cat apps/iquantize/scripts/select-ipad7-bundle-id.sh | head -60; xcrun devicectl device info",
            "      apps --device 3EB72D0B | grep -i iquantize | head)"]),
        &[format!("  ⎿  {}", script[0])], &body,
        &s(&["", "⏺ Bash(S=/private/tmp/claude-501/scratchpad; DEV=3EB72D0B; ok=0; for i in $(seq 1 120); do",
            "      if xcrun devicectl device info apps --device $DEV > $S/iphone-apps.txt 2>&1; then ok=1;",
            "      break; fi; sleep 10; done; if [ $ok = 0 ]; then echo TIMEOUT; exit 3; fi; echo unlocked;",
            "      grep -o -E app.iqtz $S/iphone-apps.txt | sort -u; ./deploy-ipad7.sh > $S/deploy.log 2>&1;",
            "      echo deploy exit=$?)", "", "", "", "", "",
            "  ⎿  exit=1", "     ERROR: The developer disk image could not be mounted.",
            "", "", "", "", "", "", "", "", "", "", "⏺ The iPhone is locked."]),
    ]));
    let found = spans(&folds(&r, true));
    let out = found.iter().find(|f| f.2.contains("of output")).expect("an output fold");
    assert_eq!(r[out.0 - 9].0, "  ⎿  #!/usr/bin/env bash", "its line and the output's first 8 lines stay");
    assert_eq!(out.1, 3 + 1 + script.len() - 1, "to the output's last line, over its blank lines");
    let at = |t: &str| r.iter().position(|(x, _)| x == t).unwrap();
    let cmd = (at("      break; fi; sleep 10; done; if [ $ok = 0 ]; then echo TIMEOUT; exit 3; fi; echo unlocked;"),
        at("      echo deploy exit=$?)"), "⋯ 3 more lines of the command".to_string());
    assert!(found.contains(&cmd), "{found:?}");
    let blanks: Vec<&str> = found.iter().filter(|f| f.2.ends_with("blank lines")).map(|f| f.2.as_str()).collect();
    assert_eq!(blanks, ["⋯ 5 blank lines", "⋯ 10 blank lines"], "{found:?}");
}

fn enc(texts: &[String]) -> Vec<Value> {
    texts.iter().map(|t| if t.is_empty() { json!({"r": [], "e": true}) } else { json!({"r": [[t, null, null, 0]], "e": true}) }).collect()
}

#[test]
fn an_output_scrolling_in_at_a_blank_line_folds_whole() {
    let words: Vec<&str> = "build test lint pack sign push tag note mail sync copy move link wipe load save scan sort plot read".split(' ').collect();
    let mut script = vec!["#!/bin/sh".to_string()];
    script.extend(words.iter().map(|w| format!("{w} --all")));
    script.push(String::new());
    script.extend(words.iter().map(|w| format!("echo {w} done")));
    let body: Vec<String> = script[1..].iter().map(|t| if t.is_empty() { String::new() } else { format!("     {t}") }).collect();
    let lines = enc(&cat(&[&s(&["⏺ Bash(cat run.sh)"]), &[format!("  ⎿  {}", script[0])], &body, &s(&["", "⏺ Ran."])]));
    let mut h = History::new(100000);
    let cut = 2 + 21; // the history ends on the script's blank line
    h.take(0, lines[..cut].to_vec(), false);
    let first = h.scan(SCAN_EVERY);
    h.take(cut as i64, lines[cut..].to_vec(), false);
    let later: Vec<(i64, i64)> = h.scan(2.0 * SCAN_EVERY).iter().map(|f| (f.n, f.to)).collect();
    assert_eq!((first, later), (vec![], vec![(10, script.len() as i64)]));
}

#[test]
fn blank_rows_fold_only_before_an_agents_next_line() {
    let boxes = rows(&["╭ ask ╮", "", "", "", "╭ ask ╮", "", "", "", "╭ ask ╮", "", "", "", "╭ ask ╮", "", "", "", "╭ ask ╮", "", "", "", "╭ ask ╮", "⏺ Done."]);
    let labels: Vec<String> = folds(&boxes, true).into_iter().map(|f| f.label).collect();
    assert_eq!(labels, ["⋯ 5 more copies of this line"], "the copies, not the blank rows");
    let file = rows(&["$ cat a.py", "import os", "", "", "", "def main():", "    pass", "$ "]);
    assert_eq!(folds(&file, true), vec![], "a file's own blank lines stay");
}

#[test]
fn a_written_file_and_a_long_command_and_output() {
    let body: Vec<String> = (1..10).map(|k| format!("       {k} text")).collect();
    let r = rows(&cat(&[&s(&["⏺ Write(notes.md)", "  ⎿  Wrote 9 lines to notes.md"]), &body, &s(&["", "⏺ Done."])]));
    assert_eq!(spans(&folds(&r, true)), one(2, 10, "⋯ 9 lines of the file"));
    let lines: Vec<String> = (0..10).map(|k| format!("      line {k}")).collect();
    let cmd = rows(&cat(&[&s(&["⏺ Bash(python3 - <<'EOF'"]), &lines, &s(&["      EOF)", "  ⎿  done", ""])]));
    assert_eq!(spans(&folds(&cmd, true)), one(2, 11, "⋯ 10 more lines of the command"));
    let cc: Vec<String> = (0..30).map(|k| format!("     cc file{k}.c -o file{k}.o")).collect();
    let out = rows(&cat(&[&s(&["⏺ Bash(make)", "  ⎿  cc a.c"]), &cc, &s(&["", "⏺ Built."])]));
    assert_eq!(spans(&folds(&out, true)), one(10, 31, "⋯ 22 more lines of output"));
}

#[test]
fn rows_tmux_wrapped_are_one_line() {
    let mut r = vec![("⏺ Update(a.ts)".to_string(), true), ("  ⎿  Updated a.ts with 7 additions".to_string(), true)];
    for k in 0..7 {
        r.push((format!("      {k} +  a long line that iTerm wrapped at the"), false));
        r.push(("width of the window".to_string(), true));
    }
    r.extend([(String::new(), true), ("⏺ Done.".to_string(), true)]);
    assert_eq!(spans(&folds(&r, true)), one(2, 15, "⋯ 7 lines of diff"));
}

#[test]
fn blobs_and_nearly_equal_lines() {
    let r = rows(&cat(&[&s(&["  --logo: url('data:image/png;base64,iVBOR"]), &vec![B64.to_string(); 5], &s(&["  ggg==');"])]));
    assert_eq!(spans(&folds(&r, true)), one(1, 5, "⋯ 400 B of base64 · 5 lines"));
    let files: Vec<String> = (0..40).map(|k| format!("vpatch/src/file{k}.py")).collect();
    let listing = rows(&cat(&[&s(&["$ ls"]), &files, &s(&["$ "])]));
    assert_eq!(spans(&folds(&listing, true)), one(4, 38, "⋯ 35 similar lines"));
}

#[test]
fn a_long_base64_line_tmux_wrapped_folds_its_rows_of_data() {
    let mut r = vec![("  --logo: url('data:image/png;base64,iVBOR".to_string(), false)];
    r.extend(vec![(B64.to_string(), false); 6]);
    r.extend([("ggg==');".to_string(), true), ("}".to_string(), true)]);
    assert_eq!(spans(&folds(&r, true)), one(1, 6, "⋯ 480 B of base64 · 6 lines"));
}

#[test]
fn a_line_that_says_something_failed_is_never_one_of_many() {
    let mut log: Vec<String> = (0..12).map(|k| format!("GET /api/item/{k} 200 {k}ms")).collect();
    log.push("GET /api/item/12 500 error".into());
    log.extend((13..40).map(|k| format!("GET /api/item/{k} 200 3ms")));
    log.push("$ ".into());
    let found = folds(&rows(&log), true);
    assert!(!found.is_empty() && found.iter().all(|f| !(f.from <= 12 && 12 <= f.to)), "{found:?}");
}

#[test]
fn an_input_box_redrawn_into_the_history_folds_to_one_copy() {
    let rule = format!("{} @critic-rebound ─", "─".repeat(30));
    let r = rows(&[
        "Done.", &rule, &rule, "-", &rule, "", &rule, &rule, &rule, "-", &rule, "What is sound", "",
    ]);
    assert_eq!(spans(&folds(&r, true)), one(2, 10, "⋯ 6 more copies of this line"));
    let few = rows(&["build", "ok", "build", "ok", "build", "done"]);
    assert_eq!(folds(&few, true), vec![], "three copies stay");
}

#[test]
fn what_does_not_fold() {
    let nine: Vec<String> = (0..9).map(|k| format!("      {k} +  x")).collect();
    let unfinished = rows(&cat(&[&s(&["⏺ Update(a.ts)", "  ⎿  Updated a.ts with 9 additions"]), &nine]));
    assert_eq!(folds(&unfinished, true), vec![], "it may go on after the last row");
    let items: Vec<String> = (0..30).map(|k| format!("  - item {k}")).collect();
    let prose = rows(&cat(&[&s(&["⏺ Here is what I changed:"]), &items, &s(&[""])]));
    assert_eq!(folds(&prose, true).len(), 1, "a long list of nearly equal lines folds");
    let talk = rows(&["⏺ First paragraph of the answer.", "  Second line of it.", "", "⏺ Another message.", ""]);
    assert_eq!(folds(&talk, true), vec![]);
    let rule = rows(&cat(&[&vec!["=".repeat(60); 5], &s(&["end"])]));
    assert!(folds(&rule, true).iter().all(|f| !f.label.contains("base64")), "a rule is not data");
}

#[test]
fn text_of_an_encoded_line() {
    assert_eq!(text_of(&json!({"r": [["ab", null, null, 0], [["漢", "", "x"], 3, null, 1]], "e": true})), "ab漢x");
}

#[test]
fn shapes_follow_pythons_patterns() {
    assert_eq!(shape("vpatch/src/file12.py"), "vpatch/p");
    assert_eq!(shape("a/b/c~"), "a/p~");
    assert_eq!(shape("~ /x"), "~ /x");
    assert_eq!(shape("~/x"), "/p");
    assert_eq!(shape("  x\t\t 12  y "), "x 0 y");
    assert_eq!(shape(".../errors.js"), ".../errors.js!.../errors.js");
    assert_eq!(shape("FAILED 3"), "FAILED 0!FAILED 0");
    assert_eq!(shape("x /a//b /c/d/~e/"), "x /p /p/~e/");
    assert_eq!(shape("//"), "//");
    assert_eq!(shape("a~~/b-c/d@e+f.g"), "a/p");
    assert_eq!(shape("ı ſ İ x\u{1c} y"), "ı ſ İ x y");
    let sizes: Vec<String> = [1280, 1075, 3277, 1587, 2816].into_iter().map(size_of).collect();
    assert_eq!(sizes, ["1.2 KB", "1.0 KB", "3.2 KB", "1.5 KB", "2.8 KB"], "rounded as Python rounds");
}

fn diff() -> Vec<String> {
    let mut d = s(&["⏺ Update(src/app.ts)", "  ⎿  Updated src/app.ts with 6 additions and 6 removals"]);
    d.extend((0..12).map(|k| format!("      {k} +  line {k}")));
    d.extend(s(&["", "⏺ Done."]));
    d
}

fn fold(n: i64, to: i64, label: &str) -> Fold {
    Fold { n, to, label: label.to_string() }
}

#[test]
fn a_bulk_load_sends_no_folded_line_and_tells_the_fold() {
    let mut h = History::new(10000);
    let (lines, found) = h.take(100, enc(&diff()), true);
    assert_eq!(found, vec![fold(102, 113, "⋯ 12 lines of diff")]);
    let nulls: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| l.is_null()).map(|(i, _)| i).collect();
    assert_eq!(nulls, (2..14).collect::<Vec<_>>());
    assert!(h.told(102, 113));
    assert!(h.told(104, 110), "a part of it (its top may have been dropped)");
    assert!(!h.told(100, 113), "nothing outside it");
    let (_, found) = h.take(100, enc(&diff()), true);
    assert_eq!(found, vec![], "a fold is told once");
}

#[test]
fn lines_that_scrolled_off_are_sent_and_folded_by_a_later_look() {
    let mut h = History::new(10000);
    for (k, line) in enc(&diff()).into_iter().enumerate() {
        // one at a time, as the screen scrolls
        let (sent, found) = h.take(500 + k as i64, vec![line.clone()], false);
        assert_eq!((sent, found), (vec![line], vec![]));
    }
    assert!(h.due(SCAN_EVERY));
    assert_eq!(h.scan(SCAN_EVERY), vec![fold(502, 513, "⋯ 12 lines of diff")]);
    assert!(!h.due(SCAN_EVERY + 0.1), "nothing new since");
}

#[test]
fn a_long_listing_loaded_in_pages_folds_whole() {
    let mut texts = s(&["$ ls -R"]);
    texts.extend((0..4500).map(|k| format!("vpatch/ws/pkg{k}/lib/file{k}.js")));
    texts.extend(s(&["vpatch/ws/x/errors.js", "$ "]));
    let listing = enc(&texts);
    let (mut h, n) = (History::new(10000), listing.len());
    let mut first = n - 1000;
    let mut out = h.take(first as i64, listing[first..].to_vec(), true).0;
    while first > 0 {
        // the page asks for older pages, newest first
        let a = first.saturating_sub(1000);
        let mut older = h.take(a as i64, listing[a..first].to_vec(), true).0;
        older.extend(out);
        (out, first) = (older, a);
    }
    assert!(out.iter().filter(|l| l.is_null()).count() > 4400, "a name with 'error' in it is one of many");
    assert!(h.told(10, 4400), "back-to-back folds may be asked for together");
    assert!(!h.told(0, 4400), "but not lines outside them");
}

#[test]
fn it_keeps_only_what_the_page_keeps() {
    let mut h = History::new(100);
    h.take(0, enc(&vec!["x".to_string(); 3000]), false);
    assert!(h.known() <= 100 + 1000);
}

/// The stream's unfold check (mirror.py's MirrorTest): only lines of a fold told, a part of one.
#[test]
fn only_a_told_fold_or_a_part_of_it_may_be_unfolded() {
    let mut h = History::new(10000);
    let (lines, found) = h.take(0, enc(&cat(&[&s(&["$ make"]), &diff(), &s(&["$ "])])), true);
    assert_eq!(found, vec![fold(3, 14, "⋯ 12 lines of diff")]);
    assert_eq!(lines.iter().filter(|l| l.is_null()).count(), 12);
    assert!(!h.told(1, 5), "not a fold it was told of");
    assert!(h.told(3, 14) && h.told(6, 14), "the fold, or its part whose top was dropped");
    assert!(!h.told(0, 14), "more than the fold");
}
