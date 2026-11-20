// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
use super::*;

fn w(s: &str) -> Vec<String> {
    s.split(' ').map(str::to_string).collect()
}

#[test]
fn ac208_only_claude_code_and_codex_are_agents_here() {
    assert_eq!(agent("claude", &w("claude --dangerously-skip-permissions")), Some("claude"));
    assert_eq!(agent("codex", &w("/opt/homebrew/bin/codex --no-alt-screen")), Some("codex"));
    // a Codex started through npm runs as node: its words name it
    assert_eq!(agent("node", &w("node /usr/local/lib/node_modules/@openai/codex/bin/codex resume")), Some("codex"));
    assert_eq!(agent("node", &w("/usr/local/bin/node /usr/local/lib/node_modules/@openai/codex/bin/codex.js")), Some("codex"), "npm's script");
    assert_eq!(agent("codex", &w("codex.js")), None, "a script's name counts only after its interpreter");
    // full-screen programs, shells, other agents and a node that is no Codex: no window
    for (command, words) in [("vim", "vim notes.md"), ("mc", "mc"), ("htop", "htop"), ("less", "less log"), ("tmux", "tmux attach"),
                             ("zsh", "-zsh"), ("opencode", "opencode"), ("node", "node server.js"), ("python3", "python3 tui.py")] {
        assert_eq!(agent(command, &w(words)), None, "{command}");
    }
    // the pane's program and its foreground process must agree: vim under Claude Code's name is no Claude Code
    assert_eq!(agent("claude", &w("vim /tmp/claude-prompt.md")), None);
    assert_eq!(agent("vim", &w("claude")), None);
}

#[test]
fn ac208_a_codex_attaches_unless_its_words_keep_it_in_its_own_process() {
    for words in ["codex", "codex --no-alt-screen", "codex resume", "codex fork", "codex --model gpt-5 fix", "node /x/codex/bin/codex"] {
        assert!(may_attach(&w(words)), "{words}");
    }
    for words in ["codex -c tui.alternate_screen=never", "codex -ctui.alternate_screen=never", "codex --config x=1", "codex --config=x=1",
                  "codex --no-daemon", "codex exec fix", "codex e fix", "codex -m gpt-5 exec fix", "codex --model gpt-5 e", "claude", "vim"] {
        assert!(!may_attach(&w(words)), "{words}");
    }
}

#[test]
fn ac208_a_pane_is_looked_at_when_an_agent_starts_or_its_screen_changes() {
    let f = |command: &str, alt: bool, cursor_on: bool| Fields { command: command.into(), alt, cursor_on, ..Default::default() };
    // opened on an agent, an agent started in a shell's pane, its screen changed
    assert!(worth_a_look(None, &f("claude", true, false)));
    assert!(worth_a_look(Some(&f("zsh", false, true)), &f("codex", false, true)));
    assert!(worth_a_look(Some(&f("codex", false, true)), &f("codex", true, true)));
    assert!(worth_a_look(Some(&f("node", false, true)), &f("node", true, true)), "a node Codex: its words decide");
    // Claude Code's Ctrl+G editor showed the cursor, then it is back in its own TUI
    assert!(worth_a_look(Some(&f("claude", true, true)), &f("claude", true, false)));
    // every other frame: nothing; Codex's cursor blinking in its box is no change; vim and shells never
    assert!(!worth_a_look(Some(&f("claude", true, false)), &f("claude", true, false)));
    assert!(!worth_a_look(Some(&f("codex", true, true)), &f("codex", true, false)));
    assert!(!worth_a_look(Some(&f("claude", false, false)), &f("claude", false, true)), "the normal screen's cursor says nothing");
    for p in ["vim", "mc", "htop", "zsh", "opencode", "tmux"] {
        assert!(!worth_a_look(None, &f(p, true, false)), "{p}");
    }
}

#[test]
fn ac208_full_screen_is_the_alternate_screen_with_the_agents_own_tui() {
    // Claude Code's fullscreen TUI hides the terminal's cursor; its Ctrl+G editor shows it (AC-05)
    assert_eq!(due("claude", true, false, false), vec![FULL_SCREEN]);
    assert!(due("claude", true, true, false).is_empty());
    assert!(due("claude", false, false, false).is_empty());
    // Codex shows the cursor in its own box on the alternate screen (measured on 0.170)
    assert_eq!(due("codex", true, true, false), vec![FULL_SCREEN]);
    assert!(due("codex", false, true, false).is_empty());
    // a plain codex under a shared server and full screen: both, the server's first
    assert_eq!(due("codex", true, true, true), vec![CODEX_SHARED, FULL_SCREEN]);
    assert_eq!(due("codex", false, true, true), vec![CODEX_SHARED]);
    assert!(due("claude", false, true, true).is_empty(), "a shared server is Codex's");
}

#[test]
fn ac208_a_run_is_its_process_and_its_mark_says_what_it_was_shown() {
    assert_eq!(noticed("4242 codexShared,fullScreen", 4242), vec![CODEX_SHARED, FULL_SCREEN]);
    assert_eq!(noticed("4242 fullScreen\n", 4242), vec![FULL_SCREEN]);
    // another process in the same pane is a new run; nothing, or a mark a user wrote, says nothing
    assert!(noticed("4242 fullScreen", 4243).is_empty());
    for v in ["", "junk", "4242", "4242 other", "-1 fullScreen"] {
        assert!(noticed(v, 4242).is_empty(), "{v}");
    }
}

#[test]
fn ac208_the_foreground_run_is_the_ttys_foreground_groups_leader() {
    // pid ppid tpgid etime: the shell, Codex in front, its MCP server under it
    let rows = "  100     1   200    01:02:03\n  200   100   200       00:07\n  201   200   200       00:06\n";
    assert_eq!(foreground_of(rows), Some((200, 7, true)));
    // an attached Codex has nothing of its own under it; days count
    assert_eq!(foreground_of("100 1 300 1-00:00:01\n300 100 300 2-01:00:00\n"), Some((300, 2 * 86_400 + 3600, false)));
    assert_eq!(foreground_of("100 1 0 00:01\n"), None, "no foreground group");
    assert_eq!(foreground_of(""), None);
}

/// A window's commands, as the page shows them to copy.
fn cmds(v: &Value) -> Vec<String> {
    v["parts"].as_array().unwrap().iter().filter_map(|p| p["cmd"].as_str().map(str::to_string)).collect()
}

#[test]
fn ac208_each_window_names_what_is_lost_and_the_commands_to_copy() {
    let plain = started("codex", &w("codex"));
    let shared = window("tmux:local:terminum:%4", CODEX_SHARED, "codex", Some(77), &plain);
    assert_eq!((shared["t"].as_str(), shared["id"].as_str(), shared["kind"].as_str()), (Some("notice"), Some("tmux:local:terminum:%4"), Some(CODEX_SHARED)));
    assert_eq!(cmds(&shared), ["codex -c tui.alternate_screen=never", "codex --no-daemon", "codex features disable daemon_auto_start", "codex app-server daemon stop"]);
    let words = shared["parts"].to_string();
    assert!(words.contains("(process 77)") && words.contains("task board") && words.contains("start it again"), "{words}");
    // one source: the words terminum mcp and terminum hooks say hold the same commands
    for c in cmds(&shared).iter().filter(|c| !c.contains("--no-daemon")) {
        assert!(codexdaemon::SAID.contains(c.as_str()), "{c}");
    }
    let claude = window("k", FULL_SCREEN, "claude", None, &started("claude", &w("claude")));
    assert_eq!((claude["title"].as_str(), cmds(&claude)), (Some("Claude Code runs full screen"), vec!["/tui classic".to_string(), r#"claude --settings '{"tui":"default"}'"#.to_string()]));
    assert!(claude["parts"].to_string().contains("\\\"tui\\\": \\\"default\\\""));
    let codex = window("k", FULL_SCREEN, "codex", None, &plain);
    assert_eq!((codex["title"].as_str(), cmds(&codex)), (Some("Codex runs full screen"), vec!["codex -c tui.alternate_screen=never".to_string()]));
    assert!(codex["parts"].to_string().contains("scrollback"));
}

#[test]
fn ac208_the_commands_start_the_agent_again_as_this_run_was_started() {
    // the user's alias or function (codsp) runs the real program with its options: those, not a plain codex; its prompt left out
    let run = w("/opt/homebrew/lib/node_modules/@openai/codex/vendor/bin/codex --no-alt-screen --dangerously-bypass-approvals-and-sandbox -m gpt-5 fix the tests");
    assert_eq!(cmds(&window("k", CODEX_SHARED, "codex", None, &started("codex", &run)))[..2],
               ["codex --no-alt-screen --dangerously-bypass-approvals-and-sandbox -m gpt-5 -c tui.alternate_screen=never",
                "codex --no-alt-screen --dangerously-bypass-approvals-and-sandbox -m gpt-5 --no-daemon"]);
    // a node Codex, a resumed one (the session it picked is no option to repeat), a value that needs quoting
    assert_eq!(started("codex", &w("node /x/codex/bin/codex.js resume 3f2a --yolo")), ["codex", "--yolo"]);
    let quoted = vec!["claude".to_string(), "--append-system-prompt".into(), "be brief".into(), "--model".into(), "opus".into()];
    assert_eq!(cmds(&window("k", FULL_SCREEN, "claude", None, &started("claude", &quoted)))[1],
               r#"claude --append-system-prompt 'be brief' --model opus --settings '{"tui":"default"}'"#);
    // a run with --settings of its own: no line that would drop it, only /tui classic
    let own = started("claude", &w("claude --settings /Users/alex/s.json --verbose"));
    assert_eq!(cmds(&window("k", FULL_SCREEN, "claude", None, &own)), ["/tui classic"]);
    // a secret among the options, or words that cannot be read: the program's name alone
    assert_eq!(started("codex", &w("codex --yolo --api-key=opensesame-value")), ["codex"]);
    assert_eq!(started("codex", &w("vim")), ["codex"]);
}

#[test]
fn ac208_settings_keep_the_kinds_turned_off_and_say_a_wrong_one() {
    use crate::settings::parse;
    assert!(parse("").0.agent_notices_off.is_empty(), "every kind on by default");
    let (s, m) = parse(r#"{"agentNoticesOff": ["fullScreen"]}"#);
    assert!(m.is_empty(), "{m:?}");
    assert_eq!(s.agent_notices_off, ["fullScreen"]);
    assert_eq!(s.page_json()["agentNoticesOff"], json!(["fullScreen"]), "the page's switch reads it");
    for bad in [r#"["popups"]"#, r#""fullScreen""#, "[1]"] {
        let (s, m) = parse(&format!(r#"{{"agentNoticesOff": {bad}}}"#));
        assert!(s.agent_notices_off.is_empty() && m.iter().any(|t| t.contains("agentNoticesOff must be")), "{bad}: {m:?}");
    }
}

fn page() -> (Arc<Page>, tokio::sync::mpsc::Receiver<String>) {
    let recorder = crate::page::trace::Recorder::new(crate::testutil::temp_root().join("trace-not-written"));
    crate::page::Pages::default().add(Default::default(), String::new(), recorder).unwrap()
}

#[tokio::test]
async fn ac208_do_not_show_again_turns_one_kind_off_and_settings_turn_both_back_on() {
    let (root, app) = crate::testutil::app("notices");
    let (page, mut rx) = page();
    let file = root.join("home/settings.json");
    std::fs::write(&file, "{\n  \"agentExitDays\": 3\n}\n").unwrap();
    let off = |app: &Shared| crate::settings::load(&file).0.agent_notices_off == app.looks.settings.read().agent_notices_off;
    crate::looks::set_agent_notices(&app, &page, &json!({"t": "notices", "kind": FULL_SCREEN, "on": false}));
    assert_eq!(crate::settings::load(&file).0.agent_notices_off, [FULL_SCREEN]);
    assert!(off(&app), "in force at once, before the file's watcher reads it");
    crate::looks::set_agent_notices(&app, &page, &json!({"t": "notices", "kind": CODEX_SHARED, "on": false}));
    assert_eq!(crate::settings::load(&file).0.agent_notices_off, [CODEX_SHARED, FULL_SCREEN]);
    // Settings' switch: both on again, then both off; the user's other keys stay
    crate::looks::set_agent_notices(&app, &page, &json!({"t": "notices", "on": true}));
    assert!(crate::settings::load(&file).0.agent_notices_off.is_empty() && off(&app));
    crate::looks::set_agent_notices(&app, &page, &json!({"t": "notices", "on": false}));
    let s = crate::settings::load(&file).0;
    assert_eq!((s.agent_notices_off.len(), s.agent_exit_days), (2, 3));
    // a wrong kind and a file that cannot be written are said where the user acted; the kind stays on
    crate::looks::set_agent_notices(&app, &page, &json!({"t": "notices", "on": true}));
    crate::looks::set_agent_notices(&app, &page, &json!({"t": "notices", "kind": "popups", "on": false}));
    std::fs::write(&file, "{ not json").unwrap();
    crate::looks::set_agent_notices(&app, &page, &json!({"t": "notices", "kind": FULL_SCREEN, "on": false}));
    let said: Vec<Value> = std::iter::from_fn(|| rx.try_recv().ok()).filter_map(|m| serde_json::from_str(&m).ok()).filter(|m: &Value| m["t"] == "error").collect();
    assert_eq!(said.len(), 2, "{said:?}");
    assert_eq!(said[0]["msg"], "Not a kind of notice");
    assert!(said[1]["msg"].as_str().unwrap().contains("settings.json"), "{said:?}");
    assert!(app.looks.settings.read().agent_notices_off.is_empty(), "not kept: still on");
    std::fs::remove_dir_all(root).ok();
}
