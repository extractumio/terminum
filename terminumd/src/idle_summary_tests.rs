// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-153's summary: its lines for each agent and what is known of it, and the command that writes
//! them (a real private tmux: idle_exit_tests.rs).

use super::*;
use crate::tmux::hookstate::Launch;

const ID: &str = "3f2a9c1e-0b7d-4e55-9a1c-2d6f8e4b7a10";

/// Its hooks' word, its options its command line's words after the program, none with a control
/// character, as `Hooks::heard_argv` keeps them.
fn hooks(agent: &'static str, folder: &str, start: bool, argv: &str) -> Started {
    let opts = Some(argv.split(' ').skip(1).filter(|w| !w.is_empty()).map(str::to_string).collect::<Vec<_>>())
        .filter(|o| o.iter().all(|w| !crate::termtext::has_control(w)));
    Started { agent, session: ID.into(), folder: folder.into(), folder_is_start: start, launch: Launch { argv: argv.into(), opts } }
}

fn launched(agent: &'static str, argv: &str, opts: Option<&[&str]>) -> Started {
    let launch = Launch { argv: argv.into(), opts: opts.map(|o| o.iter().map(|w| w.to_string()).collect()) };
    Started { launch, ..hooks(agent, "/Users/alex/api", true, "") }
}

fn line<'a>(lines: &'a [String], label: &str) -> Option<&'a str> {
    lines.iter().find_map(|l| l.strip_prefix(label)).map(str::trim)
}

#[test]
fn ac153_claude_codes_summary_names_it_and_resumes_it_in_the_folder_it_started_in() {
    let s = Summary { kind: "claude".into(), started: hooks("claude", "/Users/alex/my api", true, "claude --dangerously-skip-permissions"),
                      pane_path: "/Users/alex".into(), title: "\u{2733} Fix the login".into(), plan: Some((7, 9)), unopened: 3 * DAY + 5 };
    let l = s.lines("zsh", 1_700_000_000);
    assert_eq!((l.first().map(String::as_str), l.last().map(String::as_str)), (Some(""), Some("")), "a blank line around it");
    assert!(l[1].contains("terminum ended this idle agent: not opened for 3 days"), "{}", l[1]);
    assert_eq!(line(&l, "Agent:"), Some(format!("Claude Code, session {ID}").as_str()));
    assert_eq!(line(&l, "Title:"), Some("\u{2733} Fix the login"));
    assert_eq!(line(&l, "Started as:"), Some("claude --dangerously-skip-permissions"));
    assert_eq!(line(&l, "Folder:"), Some("/Users/alex/my api"));
    assert_eq!(line(&l, "Plan:"), Some("7 of 9 done"));
    assert_eq!(line(&l, "Resume:"), Some(format!("cd '/Users/alex/my api' && claude --dangerously-skip-permissions --resume {ID}").as_str()));
}

#[test]
fn ac153_codex_resumes_by_its_subcommand_and_a_fish_shell_gets_its_own_quoting() {
    let s = Summary { kind: "codex".into(), started: hooks("codex", "/Users/alex/it's", true, "codex --yolo"), ..Summary::default() };
    let l = s.lines("fish", 0);
    assert_eq!(line(&l, "Agent:"), Some(format!("Codex, session {ID}").as_str()));
    assert_eq!(line(&l, "Resume:"), Some(format!(r"cd '/Users/alex/it\'s' && codex --yolo resume {ID}").as_str()));
    assert_eq!(line(&l, "Plan:"), None);
    assert_eq!(line(&l, "Title:"), None, "no title, no line");
    assert!(l[1].contains("for 0 days"), "{}", l[1]);
}

#[test]
fn ac153_without_hooks_the_pane_folder_is_said_as_such_and_the_resume_lists_the_sessions() {
    for kind in ["claude", "codex"] {
        let s = Summary { kind: kind.into(), pane_path: "/Users/alex/api".into(), unopened: DAY, ..Summary::default() };
        let l = s.lines("bash", 0);
        assert!(line(&l, "Agent:").unwrap().ends_with("its session not known (its hooks did not say)"), "{l:?}");
        assert!(line(&l, "Started as:").unwrap().starts_with("not known"), "{l:?}");
        assert_eq!(line(&l, "Folder:"), Some("/Users/alex/api (the pane's; the agent's own is not known)"));
        let words = if kind == "codex" { "codex resume" } else { "claude --resume" };
        assert_eq!(line(&l, "Resume:"), Some(format!("cd /Users/alex/api && {words}   (pick it from the list)").as_str()),
                   "never --continue or --last: another pane's newer session there would be taken");
        assert!(l[1].contains("for 1 day ·"), "{}", l[1]);
    }
    let moved = Summary { kind: "claude".into(), started: hooks("claude", "/srv/x", false, ""), ..Summary::default() };
    assert_eq!(line(&moved.lines("zsh", 0), "Folder:"), Some("/srv/x (where it last worked; where it started is not known)"));
    let bad = Summary { kind: "claude".into(), started: Started { session: "x; rm -rf ~".into(), ..hooks("claude", "/a", true, "") }, ..Summary::default() };
    assert_eq!(line(&bad.lines("zsh", 0), "Resume:"), Some("cd /a && claude --resume   (pick it from the list)"), "an ID of another shape is never on a command line");
}

#[test]
fn ac153_control_and_invisible_characters_are_shown_escaped_and_such_a_folder_is_not_on_the_resume_line() {
    let odd = "/Users/alex/a\nb\u{202e}c\u{1b}[31m";
    let s = Summary { kind: "claude".into(), started: hooks("claude", odd, true, "claude \u{7}x"), title: "t\u{200b}i".into(), ..Summary::default() };
    let l = s.lines("zsh", 0);
    assert!(l.iter().all(|x| !crate::termtext::has_control(x)), "{l:?}");
    assert_eq!(line(&l, "Folder:"), Some(r"/Users/alex/a\nb\u{202e}c\u{1b}[31m"));
    assert_eq!(line(&l, "Started as:"), Some(r"claude \u{7}x"));
    assert_eq!(line(&l, "Title:"), Some(r"t\u{200b}i"));
    assert_eq!(line(&l, "Resume:"), Some(format!("claude --resume {ID}   (add its options from Started as)   (in its folder)").as_str()),
               "an option with a control character is never on it");
}

#[test]
fn ac153_the_command_writes_the_lines_to_the_panes_tty_as_printf_arguments_with_no_control_character() {
    let c = command("%3", &["it's #(id) %s \\n".into(), String::new()]);
    assert!(c.starts_with("run-shell -b -t %3 '"), "{c}");
    assert!(!crate::termtext::has_control(&c), "{c}");
    assert!(c.contains("##(id)"), "run-shell's format runs nothing of a line: {c}");
    assert!(c.contains("#{pane_tty}"), "tmux names the tty when it runs: {c}");
}

#[test]
fn ac153_the_resume_line_takes_the_options_it_started_with() {
    let s = Summary { kind: "claude".into(), started: launched("claude", "claude --dangerously-skip-permissions 'fix it'",
                                                                 Some(&["--dangerously-skip-permissions"])), ..Summary::default() };
    let l = s.lines("zsh", 0);
    assert_eq!(line(&l, "Started as:"), Some("claude --dangerously-skip-permissions 'fix it'"));
    assert_eq!(line(&l, "Resume:"), Some(format!("cd /Users/alex/api && claude --dangerously-skip-permissions --resume {ID}").as_str()),
               "its options, never its prompt");
    let s = Summary { kind: "codex".into(), started: launched("codex", "codex --yolo -c 'model=\"o3\"'",
                                                                Some(&["--yolo", "-c", "model=\"o3\""])), ..Summary::default() };
    assert_eq!(line(&s.lines("bash", 0), "Resume:"), Some(format!(r#"cd /Users/alex/api && codex --yolo -c 'model="o3"' resume {ID}"#).as_str()),
               "Codex's options before its subcommand, as Dispatcher resumes it; each one word of the shell");
    assert_eq!(line(&s.lines("fish", 0), "Resume:"), Some(format!(r#"cd /Users/alex/api && codex --yolo -c 'model="o3"' resume {ID}"#).as_str()));
}

#[test]
fn ac153_options_not_known_or_not_safe_leave_the_plain_resume_and_point_at_the_command_line() {
    let not_known = Summary { kind: "claude".into(), started: launched("claude", "claude --new-thing fix", None), ..Summary::default() };
    assert_eq!(line(&not_known.lines("zsh", 0), "Resume:"),
               Some(format!("cd /Users/alex/api && claude --resume {ID}   (add its options from Started as)").as_str()));
    let mut no_id = launched("claude", "claude --verbose", Some(&["--verbose"]));
    no_id.session.clear();
    let none = Summary { kind: "claude".into(), started: no_id, ..Summary::default() };
    assert_eq!(line(&none.lines("zsh", 0), "Resume:"), Some("cd /Users/alex/api && claude --verbose --resume   (pick it from the list)"));
}
