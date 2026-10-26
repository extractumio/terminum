// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! terminum's agent integration kept current by an install and taken out by an uninstall (AC-16,
//! AC-97), only in an agent where the user ran `terminum hooks install` before: never added elsewhere.
//! A failure is a warning: the install, upgrade or uninstall goes on.

use std::time::Duration;

use super::agentsetup::{self, claude, codex_home, codex_mcp_add, installed, marker, mcp_installed, plugin_dir, program,
                        remove_claude, remove_codex, replace_checked, resolved, write_plugin, MARKET, PLUGIN};
use serde_json::Value;

use super::Ctx;

/// One `claude` call's limit here: a local marketplace answers in a second or two; a claude that
/// hangs costs at most 20 s (its first call unanswered stops the rest), four slow calls 80 s, not 4 minutes.
pub const QUICK: Duration = Duration::from_secs(20);

fn warn(ctx: &Ctx, agent: &str, what: &str, e: &str) {
    ctx.say(format!("Warning: {}: {what} ({e}); run terminum hooks install", agentsetup::label(agent)));
}

/// After an install or upgrade came up: Claude Code's plugin installed again when it was made by
/// another build or lacks its MCP server; Codex's MCP block made current (hooks.json untouched but for codex_end).
pub fn refresh(ctx: &Ctx) {
    if installed(ctx, "claude") {
        let made = std::fs::read_to_string(marker(ctx)).unwrap_or_default();
        if (made.trim() != crate::config::build_id() || !plugin_dir(ctx).join(".mcp.json").is_file())
            && let Err(e) = update_claude(ctx) {
            warn(ctx, "claude", "terminum's plugin was not refreshed; the one installed stays", &e);
        }
    }
    if installed(ctx, "codex") && let Err(e) = codex_mcp(ctx) {
        warn(ctx, "codex", "terminum's MCP server was not refreshed", &e);
    }
    if installed(ctx, "codex") && let Err(e) = codex_end(ctx) {
        warn(ctx, "codex", "terminum's SessionEnd hook was not made synchronous", &e);
    }
}

/// hooks.json's one change a refresh makes: terminum's SessionEnd entry without `async`, only
/// in a file as terminum writes it (every other byte kept); otherwise untouched.
fn codex_end(ctx: &Ctx) -> Result<(), String> {
    let path = resolved(&codex_home(ctx).join("hooks.json"))?;
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut v: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let pretty = |v: &serde_json::Value| serde_json::to_string_pretty(v).map(|t| t + "\n").map_err(|e| e.to_string());
    let was = pretty(&v)?;
    if !codex_sync_end(&mut v, &agentsetup::command(ctx, "codex")) {
        return Ok(());
    }
    if was != text {
        return Err(format!("{} is laid out by hand, so it was left as it is", ctx.shown(&path)));
    }
    replace_checked(&path, Some(&text), &pretty(&v)?)?;
    ctx.say(CODEX_RETRUST);
    Ok(())
}

/// This build's plugin over the installed one, never taken out first (a failing claude would
/// leave no hooks): the local marketplace updated, the plugin updated, then seen in `plugin list`.
pub(super) fn update_claude(ctx: &Ctx) -> Result<(), String> {
    write_plugin(ctx)?;
    claude(ctx, QUICK, &["plugin", "marketplace", "update", MARKET])?;
    claude(ctx, QUICK, &["plugin", "update", PLUGIN])?;
    let list = claude(ctx, QUICK, &["plugin", "list"])?;
    if !list.contains(PLUGIN) {
        return Err(format!("claude plugin list does not show {PLUGIN}"));
    }
    std::fs::write(marker(ctx), crate::config::build_id()).map_err(|e| e.to_string())?;
    ctx.say("Claude Code: terminum's plugin (hooks and MCP server) is updated; sessions started from now on use it.");
    Ok(())
}

/// Codex's config.toml with this build's MCP block, said only when it changed.
fn codex_mcp(ctx: &Ctx) -> Result<(), String> {
    let conf = resolved(&codex_home(ctx).join("config.toml"))?;
    let toml = std::fs::read_to_string(&conf).ok();
    let new = codex_mcp_add(toml.as_deref(), &program(ctx)).map_err(|e| format!("{}: {e}", conf.display()))?;
    if toml.as_deref() != Some(new.as_str()) {
        replace_checked(&conf, toml.as_deref(), &new)?;
        ctx.say(format!("Codex: terminum's MCP server refreshed in {}.", ctx.shown(&conf)));
    }
    Ok(())
}

/// Before an uninstall removes the command: terminum's hooks and MCP server out of every agent
/// that has them, so none is left calling a missing command.
pub fn remove(ctx: &Ctx) {
    if installed(ctx, "claude") && let Err(e) = remove_claude(ctx, QUICK) {
        ctx.say(format!("Warning: Claude Code: terminum's plugin was not removed ({e}); remove it with claude plugin uninstall terminum@terminum-local"));
    }
    if (installed(ctx, "codex") || mcp_installed(ctx, "codex")) && let Err(e) = remove_codex(ctx) {
        ctx.say(format!("Warning: Codex: terminum's hooks or MCP server were not removed ({e}); take terminum's lines out of hooks.json and config.toml"));
    }
}

/// Codex runs an async SessionEnd hook synchronously and warns at each start: terminum's
/// SessionEnd entry loses its `async` (only that entry). True when it had one.
pub fn codex_sync_end(v: &mut Value, cmd: &str) -> bool {
    let mut changed = false;
    let ends = v.get_mut("hooks").and_then(|h| h.get_mut("SessionEnd")).and_then(Value::as_array_mut);
    for e in ends.into_iter().flatten() {
        for h in e.get_mut("hooks").and_then(Value::as_array_mut).into_iter().flatten().filter(|h| h["command"] == cmd) {
            changed |= h.as_object_mut().and_then(|o| o.remove("async")).is_some();
        }
    }
    changed
}

/// What install and refresh say when terminum's SessionEnd entry changed (its trust hash with it).
pub const CODEX_RETRUST: &str = "Codex: trust terminum's changed SessionEnd hook in /hooks once.";
