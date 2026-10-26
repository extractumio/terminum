// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! terminum's agent integration kept current by an install and taken out by an uninstall (AC-16,
//! AC-97), only in an agent where the user ran `terminum hooks install` before: never added elsewhere.
//! A failure is a warning: the install, upgrade or uninstall goes on.

use std::time::Duration;

use super::agentsetup::{self, codex_home, codex_mcp_add, install_claude, installed, marker, mcp_installed, plugin_dir,
                        program, remove_claude, remove_codex, replace_checked, resolved};
use super::Ctx;

/// One `claude` call's limit here: a local marketplace answers in a second or two; a claude that
/// hangs costs at most 20 s (its first call unanswered stops the rest), four slow calls 80 s, not 4 minutes.
pub const QUICK: Duration = Duration::from_secs(20);

fn warn(ctx: &Ctx, agent: &str, what: &str, e: &str) {
    ctx.say(format!("Warning: {}: {what} ({e}); run terminum hooks install", agentsetup::label(agent)));
}

/// After an install or upgrade came up: Claude Code's plugin installed again when it was made by
/// another build or lacks its MCP server; Codex's MCP block made current (hooks.json untouched).
pub fn refresh(ctx: &Ctx) {
    if installed(ctx, "claude") {
        let made = std::fs::read_to_string(marker(ctx)).unwrap_or_default();
        if (made.trim() != crate::config::build_id() || !plugin_dir(ctx).join(".mcp.json").is_file())
            && let Err(e) = install_claude(ctx, QUICK) {
            warn(ctx, "claude", "terminum's plugin was not refreshed", &e);
        }
    }
    if installed(ctx, "codex") && let Err(e) = codex_mcp(ctx) {
        warn(ctx, "codex", "terminum's MCP server was not refreshed", &e);
    }
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
