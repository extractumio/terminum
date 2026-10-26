// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! `terminum-host hooks install|remove|status --host-mode --marker <m> [--claude <path>] [--refresh]`:
//! terminum's hooks and MCP server put into the coding agents of an ssh host, run there by the
//! helper (`~/.terminum/bin/terminumd-agent`) the Mac copied. The same code as on a Mac
//! (agentsetup) with one stable command, `~/.terminum/bin/terminum-host`, a link to the helper.
//! One line per agent after the Mac's random marker, `<m> <agent> <result>`, so a login file
//! printing to stdout never confuses the Mac; every other line is only said.
//!
//! Results: installed, removed, off (a refresh of an agent without terminum's), not-found,
//! needs-trust (Codex runs a changed hook once trusted in /hooks), skipped-own (the host runs its
//! own terminum with hooks: "uses <host>'s own terminum"), refused: why (an entry named terminum
//! with another command; nothing changed), failed: why.

use std::path::PathBuf;

use serde_json::{json, Value};

use super::agentsetup::{self, codex_home, codex_mcp_block, codex_remove, command, found, installed, plugin_dir, program, AGENTS};
use super::Ctx;

/// The stable command hooks and MCP run on a host, a link to the helper.
pub const HOST_PROGRAM: &str = "terminum-host";
/// The helper the Mac copies (tunnel.rs), the link's target.
pub const HELPER: &str = "terminumd-agent";

/// What `hooks … --host-mode` was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostArgs {
    pub action: String,
    pub marker: String,
    pub claude: Option<String>,
    pub refresh: bool,
}

/// `[install|remove|status] --host-mode --marker <m> [--claude <abs path>] [--refresh]`.
pub fn parse(rest: &[String]) -> Result<HostArgs, String> {
    let usage = || "usage: terminum-host hooks install|remove|status --host-mode --marker <m> [--claude <abs path>] [--refresh]".to_string();
    let (mut action, mut marker, mut claude, mut refresh, mut host) = (None, None, None, false, false);
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "install" | "remove" | "status" if action.is_none() => action = Some(a.clone()),
            "--host-mode" => host = true,
            "--refresh" => refresh = true,
            "--marker" => marker = Some(it.next().ok_or_else(usage)?.clone()),
            "--claude" => claude = Some(it.next().ok_or_else(usage)?.clone()),
            _ => return Err(usage()),
        }
    }
    let marker = marker.filter(|m| m.len() >= 8 && m.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')).ok_or_else(usage)?;
    if !host || claude.as_deref().is_some_and(|c| !c.starts_with('/')) || (refresh && action.as_deref() != Some("install")) {
        return Err(usage());
    }
    Ok(HostArgs { action: action.ok_or_else(usage)?, marker, claude, refresh })
}

/// The context made a host's: the stable command terminum-host, claude's folder first on PATH
/// (a claude under nvm needs its node beside it).
pub fn prepare(ctx: &mut Ctx, args: &HostArgs) {
    ctx.host_mode = true;
    if let Some(dir) = args.claude.as_deref().map(PathBuf::from).and_then(|c| c.parent().map(|p| p.display().to_string())) {
        let path = ctx.var("PATH").map(|p| format!("{dir}:{p}")).unwrap_or(dir);
        ctx.env.insert("PATH".into(), path);
    }
}

/// The host's own terminum's command for `agent` (a terminum installed there, not the helper).
fn own_command(ctx: &Ctx, agent: &str) -> String {
    format!("{} hook {agent}", super::commands::quote(&ctx.bin().join("terminum").display().to_string()))
}

/// Whether the host has its own terminum with hooks in `agent`.
fn own(ctx: &Ctx, agent: &str) -> bool {
    if !ctx.bin().join("terminum").is_file() {
        return false;
    }
    let cmd = own_command(ctx, agent);
    if agent == "claude" {
        let needle = serde_json::to_string(&cmd).unwrap_or_default();
        return std::fs::read_to_string(plugin_dir(ctx).join("hooks/hooks.json")).is_ok_and(|t| t.contains(&needle));
    }
    read_json(&codex_home(ctx).join("hooks.json")).is_some_and(|v| codex_remove(&v, &cmd) != v)
}

fn read_json(path: &std::path::Path) -> Option<Value> {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok())
}

/// The command of the terminum entry `agent` has, when it is not this host's terminum-host:
/// Claude's plugin's MCP server, Codex's MCP block.
fn foreign(ctx: &Ctx, agent: &str) -> Option<String> {
    let ours = program(ctx);
    if agent == "claude" {
        let v = read_json(&plugin_dir(ctx).join(".mcp.json"))?;
        let cmd = v["mcpServers"]["terminum"]["command"].as_str().unwrap_or("").to_string();
        return (cmd != ours).then_some(cmd);
    }
    let text = std::fs::read_to_string(codex_home(ctx).join("config.toml")).ok()?;
    let want = codex_mcp_block(&ours).lines().find(|l| l.starts_with("command = "))?.to_string();
    let mut inside = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with("# >>> terminum:") {
            inside = true;
        } else if line == "# <<< terminum" {
            inside = false;
        } else if inside && line.starts_with("command = ") && line != want {
            return Some(line["command = ".len()..].trim_matches('"').to_string());
        }
    }
    None
}

/// `~/.terminum/bin/terminum-host` pointing at the helper (a temporary link renamed over it) and
/// `~/.terminum/run` 0700 (the folder sshd binds the Mac's link socket in).
pub fn link(ctx: &Ctx) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let bin = ctx.bin();
    let to = bin.join(HOST_PROGRAM);
    if std::fs::read_link(&to).is_ok_and(|t| t == std::path::Path::new(HELPER)) {
        // already
    } else {
        let tmp = bin.join(format!(".{HOST_PROGRAM}.{}", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        std::os::unix::fs::symlink(HELPER, &tmp).and_then(|_| std::fs::rename(&tmp, &to)).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    let run = ctx.root.join("run");
    let meta = std::fs::symlink_metadata(&run);
    if meta.as_ref().is_ok_and(|m| !m.is_dir()) {
        return Err(format!("{} is not a folder", run.display()));
    }
    std::fs::create_dir_all(&run).and_then(|_| std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o700)))
        .map_err(|e| format!("{}: {e}", run.display()))
}

/// The plugin's hooks and MCP files as they would be written now (the build's version left out).
fn plugin_current(ctx: &Ctx) -> bool {
    read_json(&plugin_dir(ctx).join("hooks/hooks.json")) == Some(agentsetup::claude_hooks(&command(ctx, "claude")))
        && read_json(&plugin_dir(ctx).join(".mcp.json")) == Some(json!({"mcpServers": {"terminum": {"command": program(ctx), "args": ["mcp"]}}}))
}

/// The result for one agent.
fn one(ctx: &Ctx, args: &HostArgs, agent: &str) -> String {
    if own(ctx, agent) {
        return "skipped-own".into();
    }
    let present = if agent == "claude" { args.claude.is_some() } else { found(ctx, agent) };
    if !present {
        return "not-found".into();
    }
    if let Some(cmd) = foreign(ctx, agent) {
        return format!("refused: an entry named terminum runs {cmd}, not {}", program(ctx));
    }
    let res = match (args.action.as_str(), agent) {
        ("status", a) => return if installed(ctx, a) { "installed".into() } else { "off".into() },
        ("install", _) if args.refresh && !installed(ctx, agent) => return "off".into(),
        ("install", "claude") if args.refresh && plugin_current(ctx) => Ok("installed"),
        ("install", "claude") if args.refresh => super::agentrefresh::update_claude(ctx).map(|_| "installed"),
        ("install", "claude") if installed(ctx, "claude") && plugin_current(ctx) => Ok("installed"),
        ("install", "claude") => agentsetup::install_claude(ctx, agentsetup::ASKED).map(|_| "installed"),
        ("install", _) => {
            let path = codex_home(ctx).join("hooks.json");
            let before = std::fs::read_to_string(&path).ok();
            agentsetup::install_codex(ctx).map(|_| if std::fs::read_to_string(&path).ok() != before { "needs-trust" } else { "installed" })
        }
        (_, "claude") if !installed(ctx, "claude") => Ok("off"),
        (_, "claude") => agentsetup::remove_claude(ctx, agentsetup::ASKED).map(|_| "removed"),
        _ => agentsetup::remove_codex(ctx).map(|_| "removed"),
    };
    match res {
        Ok(r) => r.to_string(),
        Err(e) if e.contains("named terminum of its own") => format!("refused: {}", one_line(&e)),
        Err(e) => format!("failed: {}", one_line(&e)),
    }
}

fn one_line(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `hooks … --host-mode`: the link made (install), then one marker line per agent.
pub fn run(ctx: &Ctx, args: &HostArgs) -> Result<(), String> {
    let linked = if args.action == "install" { link(ctx) } else { Ok(()) };
    for a in AGENTS {
        let r = match &linked {
            Err(e) => format!("failed: {}", one_line(e)),
            Ok(()) => one(ctx, args, a),
        };
        ctx.say(format!("{} {a} {r}", args.marker));
    }
    if args.action == "remove" && !AGENTS.iter().any(|a| installed(ctx, a)) {
        let _ = std::fs::remove_file(ctx.bin().join(HOST_PROGRAM));
    }
    Ok(())
}
