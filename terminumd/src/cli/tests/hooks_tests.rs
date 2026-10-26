// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! `terminum hooks` (AC-39): agents found, Codex's hooks.json merged and undone, Claude Code's
//! plugin through a stub `claude` (its arguments logged), never an agent's own settings.

use super::*;
use crate::cli::agentsetup::{self, claude_hooks, codex_add, codex_mcp_add, codex_mcp_block, codex_mcp_remove, codex_remove};
use crate::cli::{parse, run, Cmd};

const CMD: &str = "/Users/alex/.terminum/bin/terminum hook codex";
const PROG: &str = "/Users/alex/.terminum/bin/terminum";

#[test]
fn codex_entries_are_added_once_and_removed_leaving_others() {
    let theirs = r#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]}, "other": 1}"#;
    let v = codex_add(Some(theirs), CMD).unwrap();
    assert_eq!(v["other"], 1);
    assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 2, "theirs kept, ours added");
    assert_eq!(v["hooks"]["PreToolUse"][0]["matcher"], ".*");
    assert_eq!(v["hooks"]["Stop"][1]["hooks"][0]["async"], true);
    assert_eq!(codex_add(Some(&v.to_string()), CMD).unwrap(), v, "a second install changes nothing");
    let back = codex_remove(&v, CMD);
    assert_eq!(back, serde_json::from_str::<Value>(theirs).unwrap(), "only ours go; an event left empty goes");
    assert!(codex_add(None, CMD).unwrap()["hooks"]["SessionStart"].is_array(), "no file: a new one");
    assert!(codex_add(Some("{nope"), CMD).unwrap_err().contains("does not parse"));
    assert!(codex_add(Some(r#"{"hooks": {"Stop": 3}}"#), CMD).is_err());
    assert!(codex_add(Some("[1]"), CMD).is_err());
}

#[test]
fn claudes_plugin_hooks_every_lifecycle_event_in_the_background() {
    let v = claude_hooks("/x/terminum hook claude");
    for e in ["SessionStart", "UserPromptSubmit", "PreToolUse", "PermissionRequest", "Notification", "Stop", "StopFailure", "SubagentStart", "SubagentStop", "SessionEnd"] {
        let h = &v["hooks"][e][0]["hooks"][0];
        assert_eq!((h["command"].as_str(), h["async"].as_bool()), (Some("/x/terminum hook claude"), Some(true)), "{e}");
    }
    assert_eq!(v["hooks"].as_object().unwrap().len(), 13);
    // the reminder's carrier: Claude Code delivers no async hook's output (AC-97)
    assert_eq!(v["hooks"]["PostToolBatch"], json!([{"hooks": [{"type": "command", "command": "/x/terminum hook claude", "timeout": 2}]}]));
}

#[test]
fn the_command_parses() {
    let p = |a: &[&str]| parse(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    assert_eq!(p(&["hooks"]), Ok(Cmd::Hooks("status".into(), None)));
    assert_eq!(p(&["hooks", "install", "codex"]), Ok(Cmd::Hooks("install".into(), Some("codex".into()))));
    assert!(p(&["hooks", "install", "vim"]).is_err());
    assert!(p(&["hooks", "wipe"]).is_err());
}

#[test]
fn install_status_and_remove_with_codex_and_a_stub_claude() {
    let home = Home::new();
    let bin = home.path("bin");
    let log = home.path("claude.log");
    write_exec(&bin.join("claude"), &format!("#!/bin/sh\necho \"$@\" >> '{}'\n", log.display()));
    let codex = home.path("codexhome");
    std::fs::create_dir_all(&codex).unwrap();
    std::fs::write(codex.join("hooks.json"), r#"{"hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]}}"#).unwrap();
    std::fs::set_permissions(codex.join("hooks.json"), std::os::unix::fs::PermissionsExt::from_mode(0o640)).unwrap();
    std::fs::write(codex.join("config.toml"), "model = \"x\"\n").unwrap();
    std::fs::set_permissions(codex.join("config.toml"), std::os::unix::fs::PermissionsExt::from_mode(0o644)).unwrap();
    let path = format!("{}:/usr/bin:/bin", bin.display());
    let ctx = home.ctx(&[("PATH", &path), ("CODEX_HOME", &codex.display().to_string())]);
    assert!(agentsetup::found(&ctx, "claude") && agentsetup::found(&ctx, "codex"));
    assert!(agentsetup::hint(&ctx).unwrap().contains("Claude Code and Codex"));

    run(&ctx, Cmd::Hooks("install".into(), None)).unwrap();
    let calls = std::fs::read_to_string(&log).unwrap();
    assert!(calls.contains(&format!("plugin marketplace add {}", ctx.root.join("hooks/claude").display())), "{calls}");
    assert!(calls.contains("plugin install terminum@terminum-local --scope user"), "{calls}");
    let plugin: Value = serde_json::from_str(&std::fs::read_to_string(ctx.root.join("hooks/claude/plugins/terminum/hooks/hooks.json")).unwrap()).unwrap();
    assert_eq!(plugin["hooks"]["Stop"][0]["hooks"][0]["command"], agentsetup::command(&ctx, "claude"));
    let v: Value = serde_json::from_str(&std::fs::read_to_string(codex.join("hooks.json")).unwrap()).unwrap();
    assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 2);
    assert_eq!(mode(&codex.join("hooks.json")) & 0o777, 0o640, "Codex's file keeps its permissions");
    assert!(std::fs::read_dir(&codex).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with("hooks.json.terminum-")), "backed up");
    assert!(said(&ctx).iter().any(|l| l.contains("/hooks")), "says to trust it in Codex");
    assert!(agentsetup::installed(&ctx, "claude") && agentsetup::installed(&ctx, "codex"));
    assert!(agentsetup::hint(&ctx).is_none());
    assert!(!home.path(".claude/settings.json").exists(), "Claude Code's settings never written");
    let mcp: Value = serde_json::from_str(&std::fs::read_to_string(ctx.root.join("hooks/claude/plugins/terminum/.mcp.json")).unwrap()).unwrap();
    assert_eq!(mcp["mcpServers"]["terminum"], serde_json::json!({"command": agentsetup::program(&ctx), "args": ["mcp"]}));
    let conf = std::fs::read_to_string(codex.join("config.toml")).unwrap();
    assert_eq!(conf, "model = \"x\"\n".to_string() + &codex_mcp_block(&agentsetup::program(&ctx)));
    assert_eq!(mode(&codex.join("config.toml")) & 0o777, 0o644, "config.toml keeps its permissions");
    assert!(std::fs::read_dir(&codex).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with("config.toml.terminum-")), "backed up");
    assert!(agentsetup::mcp_installed(&ctx, "claude") && agentsetup::mcp_installed(&ctx, "codex"));
    let hooks_before = std::fs::read_to_string(codex.join("hooks.json")).unwrap();
    run(&ctx, Cmd::Hooks("install".into(), Some("codex".into()))).unwrap();
    assert_eq!(std::fs::read_to_string(codex.join("config.toml")).unwrap(), conf, "a second install changes nothing");
    assert_eq!(std::fs::read_to_string(codex.join("hooks.json")).unwrap(), hooks_before);

    run(&ctx, Cmd::Hooks("status".into(), None)).unwrap();
    assert!(said(&ctx).iter().any(|l| l.starts_with("Codex:") && l.contains("installed")), "{:?}", said(&ctx));
    assert!(said(&ctx).iter().any(|l| l.starts_with("Claude Code:") && l.ends_with("MCP server in")), "{:?}", said(&ctx));
    assert!(said(&ctx).iter().any(|l| l.starts_with("Codex:") && l.ends_with("MCP server in")), "{:?}", said(&ctx));

    run(&ctx, Cmd::Hooks("remove".into(), None)).unwrap();
    assert!(std::fs::read_to_string(&log).unwrap().contains("plugin uninstall terminum@terminum-local"));
    let v: Value = serde_json::from_str(&std::fs::read_to_string(codex.join("hooks.json")).unwrap()).unwrap();
    assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 1, "theirs stays");
    assert!(!agentsetup::installed(&ctx, "claude") && !agentsetup::installed(&ctx, "codex"));
    assert_eq!(std::fs::read_to_string(codex.join("config.toml")).unwrap(), "model = \"x\"\n", "only the block goes");
    assert!(!agentsetup::mcp_installed(&ctx, "claude") && !agentsetup::mcp_installed(&ctx, "codex"));
}

#[test]
fn a_broken_hooks_file_is_refused_and_untouched() {
    let home = Home::new();
    let codex = home.path(".codex");
    std::fs::create_dir_all(&codex).unwrap();
    std::fs::write(codex.join("hooks.json"), "{broken").unwrap();
    let ctx = home.ctx(&[]);
    let e = run(&ctx, Cmd::Hooks("install".into(), Some("codex".into()))).unwrap_err();
    assert!(e.contains("does not parse"), "{e}");
    assert_eq!(std::fs::read_to_string(codex.join("hooks.json")).unwrap(), "{broken");
    assert!(run(&ctx, Cmd::Hooks("install".into(), Some("claude".into()))).unwrap_err().contains("not on PATH"));
}

#[test]
fn codex_mcp_block_is_added_once_replaced_when_outdated_and_removed_alone() {
    let ours = codex_mcp_block(PROG);
    assert!(ours.starts_with("# >>> terminum: MCP server") && ours.ends_with("# <<< terminum\n"), "{ours}");
    assert!(ours.contains("[mcp_servers.terminum]\ncommand = \"/Users/alex/.terminum/bin/terminum\"\nargs = [\"mcp\"]\n"), "{ours}");
    assert!(ours.contains("env_vars = [\"TMUX\", \"TMUX_PANE\"]\ndefault_tools_approval_mode = \"approve\"\n"), "{ours}");
    assert_eq!(codex_mcp_add(None, PROG).unwrap(), ours, "no file: the block alone");
    assert_eq!(codex_mcp_add(Some(""), PROG).unwrap(), ours);

    let theirs = "model = \"o4\"\n\n[mcp_servers.other]\ncommand = \"x\"\n\n[profiles.fast]\nmodel = \"y\"\n";
    let with = codex_mcp_add(Some(theirs), PROG).unwrap();
    assert_eq!(with, format!("{theirs}{ours}"), "appended, theirs untouched");
    assert_eq!(codex_mcp_add(Some(&with), PROG).unwrap(), with, "a second install changes nothing");
    assert_eq!(codex_mcp_remove(&with).unwrap(), theirs, "remove keeps the rest byte for byte");
    assert_eq!(codex_mcp_remove(theirs).unwrap(), theirs, "nothing of ours: nothing changes");

    assert_eq!(codex_mcp_add(Some("model = \"o4\""), PROG).unwrap(), format!("model = \"o4\"\n{ours}"), "a last line without its newline");

    let old = with.replace("/Users/alex/.terminum/bin/terminum", "/opt/old/terminum") + "[tail]\nk = 1\n";
    let new = codex_mcp_add(Some(&old), PROG).unwrap();
    assert_eq!(new, format!("{theirs}{ours}[tail]\nk = 1\n"), "an outdated block replaced in place");
    let twice = format!("{old}{}", codex_mcp_block("/opt/older"));
    assert_eq!(codex_mcp_add(Some(&twice), PROG).unwrap(), new, "a second block of ours taken out");
}

#[test]
fn a_codex_server_named_terminum_of_the_users_is_refused() {
    for own in ["[mcp_servers.terminum]\ncommand = \"mine\"\n", "[ mcp_servers . \"terminum\" ]\ncommand = \"mine\"\n",
                "[mcp_servers.'terminum'.env]\nA = \"1\"\n", "mcp_servers.terminum.command = \"mine\"\n",
                "[mcp_servers]\nterminum = { command = \"mine\" }\n", "[mcp_servers]\nterminum.command = \"mine\"\n",
                "mcp_servers = { other = { command = \"x\" } }\n"] {
        let text = format!("model = \"o4\"\n{own}");
        let e = codex_mcp_add(Some(&text), PROG).unwrap_err();
        assert!(e.contains("of its own") && e.contains("nothing changed"), "{own}: {e}");
    }
    for fine in ["[mcp_servers]\nother = { command = \"x\" }\n", "[mcp_servers.terminum2]\ncommand = \"x\"\n",
                 "# [mcp_servers.terminum]\n", "[profiles.p]\nterminum = 1\n"] {
        assert!(codex_mcp_add(Some(fine), PROG).is_ok(), "{fine}");
    }
    let open = format!("a = 1\n{}", codex_mcp_block(PROG).replace("# <<< terminum\n", ""));
    assert!(codex_mcp_add(Some(&open), PROG).unwrap_err().contains("no \"# <<< terminum\" line"));
    assert!(codex_mcp_remove(&open).is_err(), "a broken block is not guessed at");
}

#[test]
fn a_command_path_with_a_quote_or_backslash_is_escaped() {
    let ours = codex_mcp_block("/Users/alex/my \"tools\"\\bin/terminum");
    assert!(ours.contains("command = \"/Users/alex/my \\\"tools\\\"\\\\bin/terminum\"\n"), "{ours}");
    assert!(codex_mcp_block("/a\tb").contains("command = \"/a\\u0009b\""));
}

#[test]
fn a_codex_conflict_changes_neither_file() {
    let home = Home::new();
    let codex = home.path(".codex");
    std::fs::create_dir_all(&codex).unwrap();
    let mine = "[mcp_servers.terminum]\ncommand = \"mine\"\n";
    std::fs::write(codex.join("config.toml"), mine).unwrap();
    let ctx = home.ctx(&[]);
    let e = run(&ctx, Cmd::Hooks("install".into(), Some("codex".into()))).unwrap_err();
    assert!(e.contains("config.toml") && e.contains("of its own"), "{e}");
    assert_eq!(std::fs::read_to_string(codex.join("config.toml")).unwrap(), mine);
    assert!(!codex.join("hooks.json").exists(), "hooks.json not written either");
    assert_eq!(std::fs::read_dir(&codex).unwrap().count(), 1, "no backup");
}

#[test]
fn a_new_codex_config_is_private() {
    let home = Home::new();
    let codex = home.path(".codex");
    std::fs::create_dir_all(&codex).unwrap();
    let ctx = home.ctx(&[]);
    run(&ctx, Cmd::Hooks("install".into(), Some("codex".into()))).unwrap();
    assert_eq!(mode(&codex.join("config.toml")) & 0o777, 0o600);
    assert!(said(&ctx).iter().any(|l| l.contains("MCP server is in")), "{:?}", said(&ctx));
}

#[test]
fn others_lines_between_the_markers_survive_remove_and_reinstall() {
    let ours = codex_mcp_block(PROG);
    let table = "\n[projects.\"/Users/alex/api\"]\ntrust_level = \"trusted\"\n";
    // Codex's own writes put a new table before the file's last comment: inside terminum's markers
    let text = format!("model = \"o4\"\n{}", ours.replace("# <<< terminum\n", &format!("{table}# <<< terminum\n")));
    assert_eq!(codex_mcp_add(Some(&text), PROG).unwrap(), text, "a second install changes nothing");
    let old = text.replace(PROG, "/opt/old/terminum");
    assert_eq!(codex_mcp_add(Some(&old), PROG).unwrap(), text, "an outdated block: only terminum's lines replaced");
    assert_eq!(codex_mcp_remove(&text).unwrap(), format!("model = \"o4\"\n{table}"), "remove keeps their table byte for byte");
    let comment = ours.replace("[mcp_servers.terminum]\n", "# mine\n[mcp_servers.terminum]\n");
    assert_eq!(codex_mcp_remove(&comment).unwrap(), "# mine\n", "a comment of theirs stays");
}

#[test]
fn a_line_of_terminums_changed_by_hand_is_refused_with_why() {
    let edited = codex_mcp_block(PROG).replace("default_tools_approval_mode = \"approve\"", "default_tools_approval_mode = \"ask\"");
    for e in [codex_mcp_add(Some(&edited), PROG).unwrap_err(), codex_mcp_remove(&edited).unwrap_err()] {
        assert!(e.contains("changed by hand") && e.contains("default_tools_approval_mode = \"ask\"") && e.contains("nothing changed"), "{e}");
    }
    let added = codex_mcp_block(PROG).replace("args = [\"mcp\"]\n", "args = [\"mcp\"]\nstartup_timeout_sec = 30\n");
    assert!(codex_mcp_add(Some(&added), PROG).unwrap_err().contains("startup_timeout_sec"));
}

#[test]
fn a_linked_codex_config_is_written_at_its_target_and_stays_linked() {
    let home = Home::new();
    let codex = home.path(".codex");
    let dots = home.path("dotfiles");
    std::fs::create_dir_all(&codex).unwrap();
    std::fs::create_dir_all(&dots).unwrap();
    std::fs::write(dots.join("config.toml"), "model = \"x\"\n").unwrap();
    std::fs::write(dots.join("hooks.json"), "{}").unwrap();
    for f in ["config.toml", "hooks.json"] {
        std::os::unix::fs::symlink(dots.join(f), codex.join(f)).unwrap();
    }
    let ctx = home.ctx(&[]);
    run(&ctx, Cmd::Hooks("install".into(), Some("codex".into()))).unwrap();
    for f in ["config.toml", "hooks.json"] {
        assert!(codex.join(f).is_symlink(), "{f} stays a link");
    }
    assert!(std::fs::read_to_string(dots.join("config.toml")).unwrap().contains("[mcp_servers.terminum]"));
    assert!(agentsetup::installed(&ctx, "codex"));
    let backups = |d: &std::path::Path| std::fs::read_dir(d).unwrap().filter(|e| e.as_ref().unwrap().file_name().to_string_lossy().contains(".terminum-")).count();
    assert_eq!((backups(&dots), backups(&codex)), (2, 0), "the targets are backed up beside them");
    assert!(said(&ctx).iter().any(|l| l.contains("restart them")), "{:?}", said(&ctx));
    run(&ctx, Cmd::Hooks("remove".into(), Some("codex".into()))).unwrap();
    assert!(codex.join("config.toml").is_symlink());
    assert_eq!(std::fs::read_to_string(dots.join("config.toml")).unwrap(), "model = \"x\"\n");
    std::fs::remove_file(dots.join("hooks.json")).unwrap();
    let e = run(&ctx, Cmd::Hooks("install".into(), Some("codex".into()))).unwrap_err();
    assert!(e.contains("link") && e.contains("nothing changed"), "a dangling link is refused: {e}");
}

#[test]
fn a_file_changed_since_it_was_read_is_not_replaced() {
    let home = Home::new();
    let conf = home.path("config.toml");
    std::fs::write(&conf, "model = \"x\"\n").unwrap();
    std::fs::write(&conf, "model = \"y\"\n").unwrap();                 // Codex wrote it after terminum read "x"
    let e = agentsetup::replace_checked(&conf, Some("model = \"x\"\n"), "new").unwrap_err();
    assert!(e.contains("changed while terminum was editing it") && e.contains("nothing changed"), "{e}");
    assert_eq!(std::fs::read_to_string(&conf).unwrap(), "model = \"y\"\n");
    assert!(agentsetup::replace_checked(&home.path("none.toml"), Some("x"), "new").is_err(), "gone since it was read");
    agentsetup::replace_checked(&conf, Some("model = \"y\"\n"), "new").unwrap();
    assert_eq!(std::fs::read_to_string(&conf).unwrap(), "new");
}
