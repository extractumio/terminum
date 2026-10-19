// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! `terminum hooks` (AC-39): agents found, Codex's hooks.json merged and undone, Claude Code's
//! plugin through a stub `claude` (its arguments logged), never an agent's own settings.

use super::*;
use crate::cli::agentsetup::{self, claude_hooks, codex_add, codex_remove};
use crate::cli::{parse, run, Cmd};

const CMD: &str = "/Users/alex/.terminum/bin/terminum hook codex";

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

    run(&ctx, Cmd::Hooks("status".into(), None)).unwrap();
    assert!(said(&ctx).iter().any(|l| l.starts_with("Codex:") && l.contains("installed")), "{:?}", said(&ctx));

    run(&ctx, Cmd::Hooks("remove".into(), None)).unwrap();
    assert!(std::fs::read_to_string(&log).unwrap().contains("plugin uninstall terminum@terminum-local"));
    let v: Value = serde_json::from_str(&std::fs::read_to_string(codex.join("hooks.json")).unwrap()).unwrap();
    assert_eq!(v["hooks"]["Stop"].as_array().unwrap().len(), 1, "theirs stays");
    assert!(!agentsetup::installed(&ctx, "claude") && !agentsetup::installed(&ctx, "codex"));
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
