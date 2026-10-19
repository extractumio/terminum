// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! `terminum hooks [status|install|remove] [claude|codex]` (AC-39): the coding agents found on
//! this Mac and terminum's hooks in them, which tell terminumd exactly when an agent works,
//! waits for an approval or an answer, stops or fails, its subagents and the prompts sent
//! (`terminum hook`). Only this command writes there, and only when asked:
//! - Claude Code: a plugin of terminum's (`~/.terminum/hooks/claude`, a local marketplace),
//!   added with Claude's own `claude plugin marketplace add` and `claude plugin install`; its
//!   settings are never edited by terminum;
//! - Codex: terminum's entries in `$CODEX_HOME/hooks.json` (default ~/.codex), the file backed
//!   up first, others' entries kept; a file that does not parse is refused. Codex runs a hook
//!   only once trusted in its /hooks.
//!
//! `terminum status`, `open` and `install` only say when an agent is found without them.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::Ctx;

pub use crate::tmux::hookstate::AGENTS;
const NAME: &str = "terminum";
/// The local marketplace terminum's plugin comes from (named apart from any other "terminum").
const MARKET: &str = "terminum-local";
/// terminum's plugin as Claude Code names it.
const PLUGIN: &str = "terminum@terminum-local";
const CLAUDE_EVENTS: [&str; 12] = ["SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse", "PostToolUseFailure", "PermissionRequest",
                                    "Notification", "Stop", "StopFailure", "SubagentStart", "SubagentStop", "SessionEnd"];
const CODEX_EVENTS: [&str; 10] = ["SessionStart", "UserPromptSubmit", "PreToolUse", "PermissionRequest", "PostToolUse", "Stop",
                                   "SubagentStart", "SubagentStop", "SessionEnd", "Interrupt"];
/// Codex's events whose matcher is a tool name (a regular expression: every tool).
const CODEX_TOOL_EVENTS: [&str; 3] = ["PreToolUse", "PermissionRequest", "PostToolUse"];

pub fn label(agent: &str) -> &'static str {
    if agent == "claude" { "Claude Code" } else { "Codex" }
}

/// The program on PATH, else None.
fn on_path(ctx: &Ctx, prog: &str) -> Option<PathBuf> {
    ctx.var("PATH")?.split(':').map(|d| Path::new(d).join(prog)).find(|p| p.is_file())
}

fn codex_home(ctx: &Ctx) -> PathBuf {
    ctx.var("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| ctx.home.join(".codex"))
}

/// Whether `agent` is on this Mac: its program on PATH or its folder in the home.
pub fn found(ctx: &Ctx, agent: &str) -> bool {
    on_path(ctx, agent).is_some() || if agent == "claude" { ctx.home.join(".claude").is_dir() } else { codex_home(ctx).is_dir() }
}

/// The command a hook runs: the stable `terminum` command (an upgrade keeps it), as a shell word.
pub fn command(ctx: &Ctx, agent: &str) -> String {
    format!("{} hook {agent}", super::commands::quote(&ctx.bin().join("terminum").display().to_string()))
}

/// One hook that runs `cmd` in the background, past terminumd's budget at most 3 s (Codex's most for its end events).
fn entry(cmd: &str) -> Value {
    json!({"hooks": [{"type": "command", "command": cmd, "async": true, "timeout": 3}]})
}

fn marker(ctx: &Ctx) -> PathBuf {
    ctx.root.join("hooks/claude.installed")
}

/// Whether terminum's hooks are in `agent`.
pub fn installed(ctx: &Ctx, agent: &str) -> bool {
    if agent == "claude" {
        return marker(ctx).is_file();
    }
    std::fs::read_to_string(codex_home(ctx).join("hooks.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .is_some_and(|v| has_ours(&v, &command(ctx, "codex")))
}

fn has_ours(v: &Value, cmd: &str) -> bool {
    v["hooks"].as_object().is_some_and(|m| m.values().flat_map(|l| l.as_array().into_iter().flatten()).any(|e| is_ours(e, cmd)))
}

fn is_ours(entry: &Value, cmd: &str) -> bool {
    entry["hooks"].as_array().is_some_and(|hs| hs.iter().any(|h| h["command"] == cmd))
}

/// The hooks file of Claude Code's plugin: every lifecycle event runs `cmd` in the background.
pub fn claude_hooks(cmd: &str) -> Value {
    let hooks: serde_json::Map<String, Value> = CLAUDE_EVENTS.iter()
        .map(|e| (e.to_string(), json!([entry(cmd)]))).collect();
    json!({"description": "terminum: tells terminumd what this session does (terminum hooks)", "hooks": hooks})
}

/// `hooks.json` of Codex with terminum's entries added (Err: it does not parse, said why).
pub fn codex_add(text: Option<&str>, cmd: &str) -> Result<Value, String> {
    let mut v: Value = match text.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => serde_json::from_str(t).map_err(|e| format!("it does not parse ({e}): nothing changed"))?,
        None => json!({}),
    };
    if !v.is_object() || (!v["hooks"].is_null() && !v["hooks"].is_object()) {
        return Err("it is not an object with \"hooks\": nothing changed".into());
    }
    if v["hooks"].is_null() {
        v["hooks"] = json!({});
    }
    let hooks = v["hooks"].as_object_mut().expect("an object");
    for e in CODEX_EVENTS {
        let list = hooks.entry(e).or_insert_with(|| json!([]));
        let Some(l) = list.as_array_mut() else { return Err(format!("its {e} is not a list: nothing changed")) };
        if !l.iter().any(|x| is_ours(x, cmd)) {
            let mut ours = entry(cmd);
            if CODEX_TOOL_EVENTS.contains(&e) {
                ours["matcher"] = json!(".*");
            }
            l.push(ours);
        }
    }
    Ok(v)
}

/// `hooks.json` of Codex without terminum's entries (others' kept, an event left empty gone).
pub fn codex_remove(v: &Value, cmd: &str) -> Value {
    let mut v = v.clone();
    if let Some(hooks) = v["hooks"].as_object_mut() {
        for l in hooks.values_mut() {
            if let Some(a) = l.as_array_mut() {
                a.retain(|x| !is_ours(x, cmd));
            }
        }
        hooks.retain(|_, l| l.as_array().is_none_or(|a| !a.is_empty()));
    }
    v
}

/// `v` into `path` whole or not at all, keeping the file's permissions (Codex's file is the user's).
fn write(path: &Path, v: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())? + "\n";
    crate::settings::write_atomic(path, text.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))
}

fn backup(path: &Path) -> Result<(), String> {
    if path.is_file() {
        let to = path.with_extension(format!("json.terminum-{}", crate::auth::store::unix_now()));
        std::fs::copy(path, &to).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    Ok(())
}

fn claude(ctx: &Ctx, args: &[&str]) -> Result<String, String> {
    let prog = on_path(ctx, "claude").ok_or("claude is not on PATH")?;
    let mut cmd = std::process::Command::new(prog);
    cmd.args(args).env("PATH", ctx.var("PATH").unwrap_or(""));
    let out = super::launch::run_timeout(cmd, std::time::Duration::from_secs(60)).map_err(|e| format!("claude {}: {e}", args.join(" ")))?;
    let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)).trim().to_string();
    if out.status.success() { Ok(said) } else { Err(format!("claude {}: {said}", args.join(" "))) }
}

fn install_claude(ctx: &Ctx) -> Result<(), String> {
    let market = ctx.root.join("hooks/claude");
    let plugin = market.join("plugins").join(NAME);
    write(&market.join(".claude-plugin/marketplace.json"), &json!({"name": MARKET, "owner": {"name": "terminum"}, "description": "terminum's hooks for Claude Code",
        "plugins": [{"name": NAME, "source": "./plugins/terminum", "description": "terminum: the session's states, prompts and subagents"}]}))?;
    write(&plugin.join(".claude-plugin/plugin.json"), &json!({"name": NAME, "version": crate::config::build_id(), "author": {"name": "terminum"},
        "description": "terminum: tells terminumd what this session does (terminum hooks)"}))?;
    write(&plugin.join("hooks/hooks.json"), &claude_hooks(&command(ctx, "claude")))?;
    // added anew each time, so Claude Code copies this build's plugin, not one it cached before
    let _ = claude(ctx, &["plugin", "uninstall", PLUGIN]);
    let _ = claude(ctx, &["plugin", "marketplace", "remove", MARKET]);
    claude(ctx, &["plugin", "marketplace", "add", &market.display().to_string()])?;
    claude(ctx, &["plugin", "install", PLUGIN, "--scope", "user"])?;
    std::fs::write(marker(ctx), crate::config::build_id()).map_err(|e| e.to_string())?;
    ctx.say("Claude Code: terminum's plugin is installed; sessions started from now on use it.");
    Ok(())
}

fn remove_claude(ctx: &Ctx) -> Result<(), String> {
    claude(ctx, &["plugin", "uninstall", PLUGIN])?;
    let _ = claude(ctx, &["plugin", "marketplace", "remove", MARKET]);
    let _ = std::fs::remove_file(marker(ctx));
    ctx.say("Claude Code: terminum's plugin is removed.");
    Ok(())
}

fn install_codex(ctx: &Ctx) -> Result<(), String> {
    let path = codex_home(ctx).join("hooks.json");
    let text = std::fs::read_to_string(&path).ok();
    let v = codex_add(text.as_deref(), &command(ctx, "codex")).map_err(|e| format!("{}: {e}", path.display()))?;
    if text.as_deref().and_then(|t| serde_json::from_str::<Value>(t).ok()).as_ref() == Some(&v) {
        ctx.say("Codex: terminum's hooks are in already.");
        return Ok(());
    }
    backup(&path)?;
    write(&path, &v)?;
    ctx.say(format!("Codex: terminum's hooks are in {}. Codex runs a hook only once you trust it: open /hooks in Codex and trust terminum's.", ctx.shown(&path)));
    if std::fs::read_to_string(codex_home(ctx).join("config.toml")).is_ok_and(|t| t.lines().any(|l| l.trim_start().starts_with("[hooks"))) {
        ctx.say("Codex: config.toml has hooks too; Codex runs both.");
    }
    Ok(())
}

fn remove_codex(ctx: &Ctx) -> Result<(), String> {
    let path = codex_home(ctx).join("hooks.json");
    let Ok(text) = std::fs::read_to_string(&path) else { ctx.say("Codex: no hooks.json, nothing to remove."); return Ok(()) };
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("{}: it does not parse ({e}): nothing changed", path.display()))?;
    let out = codex_remove(&v, &command(ctx, "codex"));
    if out != v {
        backup(&path)?;
        write(&path, &out)?;
    }
    ctx.say("Codex: terminum's hooks are removed.");
    Ok(())
}

/// The agents asked for: one by name, else every one found.
fn which(ctx: &Ctx, agent: Option<&str>) -> Result<Vec<&'static str>, String> {
    match agent {
        Some(a) => Ok(AGENTS.iter().copied().filter(|x| *x == a).collect()),     // parse() knows it
        None => Ok(AGENTS.iter().copied().filter(|a| found(ctx, a)).collect()),
    }
}

/// `terminum hooks …`.
pub fn run(ctx: &Ctx, action: &str, agent: Option<&str>) -> Result<(), String> {
    let agents = which(ctx, agent)?;
    match action {
        "install" | "remove" => {
            if agents.is_empty() {
                ctx.say("No coding agent found (claude or codex on PATH, ~/.claude, ~/.codex).");
            }
            for a in agents {
                match (action, a) {
                    ("install", "claude") => install_claude(ctx)?,
                    ("install", _) => install_codex(ctx)?,
                    (_, "claude") => remove_claude(ctx)?,
                    _ => remove_codex(ctx)?,
                }
            }
        }
        _ => {
            let last = super::launch::health(ctx).map(|h| h["hooks"].clone()).unwrap_or(Value::Null);
            for a in AGENTS {
                let state = if !found(ctx, a) { "not found".to_string() } else if installed(ctx, a) {
                    match last[a].as_u64() {
                        Some(t) => format!("found; terminum's hooks installed, last heard {} s ago", crate::auth::store::unix_now().saturating_sub(t)),
                        None => "found; terminum's hooks installed, not heard since terminumd started".to_string(),
                    }
                } else { "found; terminum's hooks not installed (terminum hooks install)".to_string() };
                ctx.say(format!("{:<13}{state}", format!("{}:", label(a))));
            }
        }
    }
    Ok(())
}

/// One line for `terminum status`, `open` and `install` when an agent is found without terminum's hooks.
pub fn hint(ctx: &Ctx) -> Option<String> {
    let missing: Vec<&str> = AGENTS.iter().copied().filter(|a| found(ctx, a) && !installed(ctx, a)).map(label).collect();
    (!missing.is_empty()).then(|| format!("{} found without terminum's hooks: run terminum hooks install for exact states, prompts and reminders", missing.join(" and ")))
}
