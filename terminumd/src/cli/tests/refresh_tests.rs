// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! An install's refresh of terminum's agent integration and an uninstall's removal (AC-16,
//! AC-97): only where `terminum hooks install` ran, a stub `claude` (its arguments logged).

use super::*;
use crate::cli::{agentrefresh, agentsetup};

struct Setup {
    home: Home,
    log: PathBuf,
    codex: PathBuf,
}

/// A home with a stub claude on PATH (exit `status`) and an empty Codex home.
fn setup(status: u8) -> (Setup, Ctx) {
    let home = Home::new();
    let log = home.path("claude.log");
    // `plugin list` shows the plugin unless the home has a file "unlisted"
    let unlisted = home.path("unlisted");
    write_exec(&home.path("bin/claude"), &format!("#!/bin/sh\necho \"$@\" >> '{}'\n[ \"$2\" = list ] && [ ! -e '{}' ] && echo 'terminum@terminum-local  user  enabled'\nexit {status}\n",
                                               log.display(), unlisted.display()));
    let codex = home.path("codexhome");
    std::fs::create_dir_all(&codex).unwrap();
    let path = format!("{}:/usr/bin:/bin", home.path("bin").display());
    let ctx = home.ctx(&[("PATH", &path), ("CODEX_HOME", &codex.display().to_string())]);
    (Setup { home, log, codex }, ctx)
}

fn calls(s: &Setup) -> String {
    std::fs::read_to_string(&s.log).unwrap_or_default()
}

fn hooks_in_codex(s: &Setup, ctx: &Ctx) -> String {
    let v = agentsetup::codex_add(None, &agentsetup::command(ctx, "codex")).unwrap();
    let text = serde_json::to_string_pretty(&v).unwrap() + "\n";
    std::fs::write(s.codex.join("hooks.json"), &text).unwrap();
    text
}

fn mark_claude(ctx: &Ctx, build: &str, mcp: bool) {
    let plugin = ctx.root.join("hooks/claude/plugins/terminum");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(ctx.root.join("hooks/claude.installed"), build).unwrap();
    if mcp {
        std::fs::write(plugin.join(".mcp.json"), "{}").unwrap();
    }
}

#[test]
fn nothing_is_added_where_hooks_were_never_installed() {
    let (s, ctx) = setup(0);
    std::fs::write(s.codex.join("config.toml"), "model = \"x\"\n").unwrap();
    agentrefresh::refresh(&ctx);
    assert_eq!(calls(&s), "", "claude never called");
    assert_eq!(std::fs::read_to_string(s.codex.join("config.toml")).unwrap(), "model = \"x\"\n");
    assert!(!s.codex.join("hooks.json").exists() && !ctx.root.join("hooks/claude.installed").exists());
    assert!(said(&ctx).is_empty(), "{:?}", said(&ctx));
}

#[test]
fn claudes_plugin_is_installed_again_for_another_build_or_without_its_mcp() {
    let (s, ctx) = setup(0);
    mark_claude(&ctx, crate::config::build_id(), true);
    agentrefresh::refresh(&ctx);
    assert_eq!(calls(&s), "", "this build's plugin with its MCP server: left alone");

    mark_claude(&ctx, "0.0.1-old", true);
    agentrefresh::refresh(&ctx);
    assert_eq!(calls(&s), "plugin marketplace update terminum-local\nplugin update terminum@terminum-local\nplugin list\n", "updated over, never uninstalled first");
    assert_eq!(std::fs::read_to_string(ctx.root.join("hooks/claude.installed")).unwrap(), crate::config::build_id());
    assert!(said(&ctx).iter().any(|l| l.starts_with("Claude Code: terminum's plugin")), "{:?}", said(&ctx));

    std::fs::remove_file(&s.log).unwrap();
    std::fs::remove_file(ctx.root.join("hooks/claude/plugins/terminum/.mcp.json")).unwrap();
    agentrefresh::refresh(&ctx);
    assert!(calls(&s).contains("plugin update"), "same build without .mcp.json: updated again");
    assert!(ctx.root.join("hooks/claude/plugins/terminum/.mcp.json").is_file());
    let _ = &s.home;
}

#[test]
fn codexs_mcp_block_is_added_with_its_hooks_kept_byte_for_byte() {
    let (s, ctx) = setup(0);
    let hooks = hooks_in_codex(&s, &ctx);
    std::fs::write(s.codex.join("config.toml"), "model = \"x\"\n").unwrap();
    agentrefresh::refresh(&ctx);
    let conf = std::fs::read_to_string(s.codex.join("config.toml")).unwrap();
    assert_eq!(conf, format!("model = \"x\"\n{}", agentsetup::codex_mcp_block(&agentsetup::program(&ctx))));
    assert_eq!(std::fs::read_to_string(s.codex.join("hooks.json")).unwrap(), hooks, "hooks.json byte-identical");
    assert!(said(&ctx).iter().any(|l| l.starts_with("Codex: terminum's MCP server refreshed")), "{:?}", said(&ctx));
    let n = said(&ctx).len();
    agentrefresh::refresh(&ctx);
    assert_eq!(said(&ctx).len(), n, "current already: nothing said");
    assert_eq!(calls(&s), "");
}

#[test]
fn failures_are_warnings_and_the_hint_names_hooks_install() {
    let (s, ctx) = setup(1);
    mark_claude(&ctx, "0.0.1-old", false);
    hooks_in_codex(&s, &ctx);
    let mine = "[mcp_servers.terminum]\ncommand = \"mine\"\n";
    std::fs::write(s.codex.join("config.toml"), mine).unwrap();
    agentrefresh::refresh(&ctx); // returns: never fails the install
    let warned: Vec<String> = said(&ctx).into_iter().filter(|l| l.starts_with("Warning: ")).collect();
    assert_eq!(warned.len(), 2, "{warned:?}");
    assert!(warned.iter().all(|l| l.ends_with("run terminum hooks install")), "{warned:?}");
    assert_eq!(std::fs::read_to_string(s.codex.join("config.toml")).unwrap(), mine, "a refused config.toml is untouched");
    let hint = agentsetup::hint(&ctx).unwrap();
    assert!(hint.contains("Codex has terminum's hooks without its MCP server") && hint.contains("terminum hooks install"), "{hint}");
}

#[test]
fn claude_not_on_path_is_a_warning() {
    let (s, ctx) = setup(0);
    std::fs::remove_file(s.home.path("bin/claude")).unwrap();
    mark_claude(&ctx, "0.0.1-old", true);
    agentrefresh::refresh(&ctx);
    assert!(said(&ctx).iter().any(|l| l.contains("Warning: Claude Code") && l.contains("claude is not on PATH")), "{:?}", said(&ctx));
}

#[test]
fn uninstall_takes_hooks_and_mcp_out_of_both_agents() {
    let (s, ctx) = setup(0);
    mark_claude(&ctx, crate::config::build_id(), true);
    hooks_in_codex(&s, &ctx);
    std::fs::write(s.codex.join("config.toml"), format!("model = \"x\"\n{}", agentsetup::codex_mcp_block(&agentsetup::program(&ctx)))).unwrap();
    agentrefresh::remove(&ctx);
    assert!(calls(&s).contains("plugin uninstall terminum@terminum-local"), "{}", calls(&s));
    assert!(!agentsetup::installed(&ctx, "claude") && !agentsetup::installed(&ctx, "codex"));
    assert_eq!(std::fs::read_to_string(s.codex.join("config.toml")).unwrap(), "model = \"x\"\n");

    let (s2, ctx2) = setup(1);
    mark_claude(&ctx2, crate::config::build_id(), true);
    agentrefresh::remove(&ctx2);
    assert!(said(&ctx2).iter().any(|l| l.starts_with("Warning: Claude Code: terminum's plugin was not removed")), "{:?}", said(&ctx2));
    let _ = &s2.home;
}

#[test]
fn a_failed_claude_refresh_warns_and_keeps_the_old_install() {
    for (status, unlisted) in [(1, false), (0, true)] {
        let (s, ctx) = setup(status);
        if unlisted {
            std::fs::write(s.home.path("unlisted"), "").unwrap();
        }
        mark_claude(&ctx, "0.0.1-old", true);
        agentrefresh::refresh(&ctx);
        assert!(!calls(&s).contains("uninstall") && !calls(&s).contains("marketplace remove"), "{}", calls(&s));
        assert_eq!(std::fs::read_to_string(ctx.root.join("hooks/claude.installed")).unwrap(), "0.0.1-old", "tried again next time");
        let w = said(&ctx).into_iter().find(|l| l.starts_with("Warning: Claude Code")).unwrap_or_default();
        assert!(w.contains("the one installed stays"), "{w}");
        if unlisted {
            assert!(w.contains("plugin list does not show"), "{w}");
        }
    }
}

#[test]
fn codexs_old_async_session_end_entry_is_migrated_alone_once() {
    let (s, ctx) = setup(0);
    let cmd = agentsetup::command(&ctx, "codex");
    let mut old = agentsetup::codex_add(None, &cmd).unwrap();
    old["hooks"]["Stop"].as_array_mut().unwrap().insert(0, serde_json::json!({"hooks": [{"type": "command", "command": "say done", "async": true}]}));
    old["hooks"]["SessionEnd"][0]["hooks"][0]["async"] = serde_json::json!(true);   // as an older build wrote it
    let text = serde_json::to_string_pretty(&old).unwrap() + "\n";
    std::fs::write(s.codex.join("hooks.json"), &text).unwrap();
    agentrefresh::refresh(&ctx);
    let new = std::fs::read_to_string(s.codex.join("hooks.json")).unwrap();
    assert_eq!(text.lines().count(), new.lines().count() + 1, "one line less");
    let diff: Vec<(usize, &str)> = text.lines().enumerate().filter(|(i, l)| new.lines().nth(*i) != Some(*l)).take(1).collect();
    assert!(text.lines().nth(diff[0].0).unwrap().contains("\"async\": true"), "the first change is the dropped async line: {diff:?}");
    let v: serde_json::Value = serde_json::from_str(&new).unwrap();
    assert!(v["hooks"]["SessionEnd"][0]["hooks"][0].get("async").is_none());
    assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["async"], true, "another's async kept");
    assert_eq!(v["hooks"]["Stop"][1]["hooks"][0]["async"], true, "terminum's other events stay async");
    let mut back: Vec<&str> = new.lines().collect();
    back.insert(diff[0].0, text.lines().nth(diff[0].0).unwrap());
    assert_eq!(back.join("\n") + "\n", text, "every other byte kept");
    assert!(said(&ctx).iter().any(|l| l == agentrefresh::CODEX_RETRUST), "{:?}", said(&ctx));
    agentrefresh::refresh(&ctx);
    assert_eq!(std::fs::read_to_string(s.codex.join("hooks.json")).unwrap(), new, "a second run changes nothing");
    assert_eq!(said(&ctx).iter().filter(|l| *l == agentrefresh::CODEX_RETRUST).count(), 1);
}

#[test]
fn a_hand_laid_hooks_json_is_not_migrated_but_warned() {
    let (s, ctx) = setup(0);
    let mut old = agentsetup::codex_add(None, &agentsetup::command(&ctx, "codex")).unwrap();
    old["hooks"]["SessionEnd"][0]["hooks"][0]["async"] = serde_json::json!(true);
    let text = serde_json::to_string(&old).unwrap();
    std::fs::write(s.codex.join("hooks.json"), &text).unwrap();
    agentrefresh::refresh(&ctx);
    assert_eq!(std::fs::read_to_string(s.codex.join("hooks.json")).unwrap(), text);
    assert!(said(&ctx).iter().any(|l| l.contains("laid out by hand")), "{:?}", said(&ctx));
}

#[test]
fn hooks_install_writes_codexs_session_end_without_async() {
    let (_s, ctx) = setup(0);
    let cmd = agentsetup::command(&ctx, "codex");
    let v = agentsetup::codex_add(None, &cmd).unwrap();
    assert!(v["hooks"]["SessionEnd"][0]["hooks"][0].get("async").is_none());
    assert!(v["hooks"]["SessionEnd"][0]["hooks"][0]["timeout"].as_u64().unwrap() <= 3);
    assert_eq!(v["hooks"]["Stop"][0]["hooks"][0]["async"], true);
}
