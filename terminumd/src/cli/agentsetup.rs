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
//! Both also get `terminum mcp`, terminum's MCP server (plans, progress, notes; AC-97, AC-98):
//! the plugin's `.mcp.json`, and for Codex one delimited block in `$CODEX_HOME/config.toml`
//! (text edits, no TOML parser: only the lines terminum wrote are added, replaced or taken out;
//! another's line Codex put between the markers stays). A linked file is written at its target.
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

pub(super) fn codex_home(ctx: &Ctx) -> PathBuf {
    ctx.var("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| ctx.home.join(".codex"))
}

/// Whether `agent` is on this Mac: its program on PATH or its folder in the home.
pub fn found(ctx: &Ctx, agent: &str) -> bool {
    on_path(ctx, agent).is_some() || if agent == "claude" { ctx.home.join(".claude").is_dir() } else { codex_home(ctx).is_dir() }
}

/// The stable `terminum` command (an upgrade keeps it).
pub fn program(ctx: &Ctx) -> String {
    ctx.bin().join("terminum").display().to_string()
}

/// The command a hook runs: the stable `terminum` command, as a shell word.
pub fn command(ctx: &Ctx, agent: &str) -> String {
    format!("{} hook {agent}", super::commands::quote(&program(ctx)))
}

/// One hook that runs `cmd` in the background, past terminumd's budget at most 3 s (Codex's most for its end events).
fn entry(cmd: &str) -> Value {
    json!({"hooks": [{"type": "command", "command": cmd, "async": true, "timeout": 3}]})
}

pub(super) fn marker(ctx: &Ctx) -> PathBuf {
    ctx.root.join("hooks/claude.installed")
}

pub(super) fn plugin_dir(ctx: &Ctx) -> PathBuf {
    ctx.root.join("hooks/claude/plugins").join(NAME)
}

/// Whether terminum's MCP server is in `agent`: the installed plugin's `.mcp.json`, Codex's block.
pub fn mcp_installed(ctx: &Ctx, agent: &str) -> bool {
    if agent == "claude" {
        return installed(ctx, "claude") && plugin_dir(ctx).join(".mcp.json").is_file();
    }
    std::fs::read_to_string(codex_home(ctx).join("config.toml")).is_ok_and(|t| blocks(&t).is_ok_and(|b| !b.is_empty()))
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

const MCP_START: &str = "# >>> terminum: MCP server (terminum hooks install; terminum hooks remove takes it out)";
const MCP_END: &str = "# <<< terminum";

/// `s` as a TOML basic string.
fn toml_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out + "\""
}

/// terminum's table of Codex's config.toml: `terminum mcp`, given the pane's tmux variables
/// (Codex clears an MCP server's environment), its tools run without asking.
fn mcp_table(program: &str) -> String {
    format!("[mcp_servers.terminum]\ncommand = {}\nargs = [\"mcp\"]\nenv_vars = [\"TMUX\", \"TMUX_PANE\"]\n\
             default_tools_approval_mode = \"approve\"\n", toml_str(program))
}

/// terminum's block of Codex's config.toml: its table between the markers.
pub fn codex_mcp_block(program: &str) -> String {
    format!("{MCP_START}\n{}{MCP_END}\n", mcp_table(program))
}

/// The lines of one block of terminum's that terminum did not write (a table or key Codex put
/// there, a comment), kept in place, and where terminum's table stands among them (Err: a key of
/// terminum's table changed by hand: it is not guessed at).
fn foreign(block: &str) -> Result<(Vec<&str>, usize), String> {
    let known = |t: &str| (t.starts_with("command = \"") && t.ends_with('"'))
        || ["args = [\"mcp\"]", "env_vars = [\"TMUX\", \"TMUX_PANE\"]", "default_tools_approval_mode = \"approve\""].contains(&t);
    let lines: Vec<&str> = block.split_inclusive('\n').collect();
    let (mut kept, mut at, mut ours) = (vec![], None, false);
    for &line in &lines[1..lines.len() - 1] {
        let t = line.trim();
        if t == "[mcp_servers.terminum]" && at.is_none() {
            (at, ours) = (Some(kept.len()), true);
            continue;
        }
        if t.starts_with('[') {
            ours = false;
        } else if ours && known(t) {
            continue;
        } else if ours && !t.is_empty() && !t.starts_with('#') {
            return Err(format!("terminum's block was changed by hand ({t}): nothing changed; put that line back as terminum wrote it, or edit the block yourself"));
        }
        kept.push(line);
    }
    Ok((kept, at.unwrap_or(0)))
}

/// terminum's blocks in `text` as byte ranges, start line through end line (Err: a marker unpaired).
fn blocks(text: &str) -> Result<Vec<(usize, usize)>, String> {
    let (mut out, mut open, mut at) = (vec![], None, 0);
    for line in text.split_inclusive('\n') {
        let t = line.trim();
        if t.starts_with("# >>> terminum:") {
            if open.replace(at).is_some() {
                return Err("terminum's block starts twice: nothing changed".into());
            }
        } else if t == MCP_END {
            out.push((open.take().ok_or("terminum's block ends where none starts: nothing changed")?, at + line.len()));
        }
        at += line.len();
    }
    if open.is_some() { Err(format!("terminum's block has no \"{MCP_END}\" line: nothing changed")) } else { Ok(out) }
}

/// The line outside terminum's blocks that defines an `mcp_servers.terminum` of its own (a table,
/// a dotted key, a key of `[mcp_servers]`), or an inline `mcp_servers` a table cannot extend.
fn own_server(text: &str, ours: &[(usize, usize)]) -> Option<String> {
    let key = |k: &str| k.chars().filter(|c| !c.is_whitespace() && *c != '"' && *c != '\'').collect::<String>();
    let (mut table, mut at) = (String::new(), 0);
    for line in text.split_inclusive('\n') {
        let inside = ours.iter().any(|&(s, e)| (s..e).contains(&at));
        at += line.len();
        let t = line.trim();
        if inside || t.starts_with('#') {
            continue;
        }
        let path = if let Some(h) = t.strip_prefix('[') {
            table = key(h.trim_start_matches('[').split(']').next().unwrap_or(""));
            if table == "mcp_servers" { continue } else { table.clone() }
        } else if let Some((k, _)) = t.split_once('=') {
            if table.is_empty() { key(k) } else { format!("{table}.{}", key(k)) }
        } else {
            continue;
        };
        if path == "mcp_servers" || path == "mcp_servers.terminum" || path.starts_with("mcp_servers.terminum.") {
            return Some(t.to_string());
        }
    }
    None
}

/// Codex's config.toml with terminum's block: appended, or the first one's lines of terminum's
/// replaced in place and any other's taken out, others' lines inside kept (Err: a server named
/// terminum of the user's, a broken block, a line of terminum's changed by hand).
pub fn codex_mcp_add(text: Option<&str>, program: &str) -> Result<String, String> {
    let text = text.unwrap_or("");
    let found = blocks(text)?;
    if let Some(line) = own_server(text, &found) {
        return Err(format!("it has an MCP server named terminum of its own ({line}): nothing changed; take it out, then run terminum hooks install again"));
    }
    if found.is_empty() {
        let sep = if text.is_empty() || text.ends_with('\n') { "" } else { "\n" };
        return Ok(format!("{text}{sep}{}", codex_mcp_block(program)));
    }
    edit_blocks(text, &found, |i, kept, at| if i > 0 { kept.concat() } else {
        format!("{MCP_START}\n{}{}{}{MCP_END}\n", kept[..at].concat(), mcp_table(program), kept[at..].concat())
    })
}

/// Codex's config.toml without terminum's lines (others' lines inside its block kept), the rest byte for byte.
pub fn codex_mcp_remove(text: &str) -> Result<String, String> {
    edit_blocks(text, &blocks(text)?, |_, kept, _| kept.concat())
}

/// `text` with block `i` of `found` given as `with(i, its foreign lines, where terminum's table stands)`.
fn edit_blocks(text: &str, found: &[(usize, usize)], with: impl Fn(usize, &[&str], usize) -> String) -> Result<String, String> {
    let (mut out, mut at) = (String::new(), 0);
    for (i, &(s, e)) in found.iter().enumerate() {
        let (kept, table) = foreign(&text[s..e])?;
        out.push_str(&text[at..s]);
        out.push_str(&with(i, &kept, table));
        at = e;
    }
    Ok(out + &text[at..])
}

/// `v` into `path` whole or not at all (the files of terminum's plugin).
fn write(path: &Path, v: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())? + "\n";
    crate::settings::write_atomic(path, text.as_bytes()).map_err(|e| format!("{}: {e}", path.display()))
}

/// `text` into the agent's `path` whole or not at all: a new file 0600, an existing one's permissions kept.
fn write_text(path: &Path, text: &str) -> Result<(), String> {
    let keep = std::fs::metadata(path).ok().map(|m| m.permissions());
    path.parent().map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|_| crate::config::write_replacing(path, text.as_bytes(), 0o600, keep)).map_err(|e| format!("{}: {e}", path.display()))
}

/// The file `path` names: a symbolic link's target (a dotfiles repository's file stays linked).
pub fn resolved(path: &Path) -> Result<PathBuf, String> {
    if !path.is_symlink() {
        return Ok(path.to_path_buf());
    }
    std::fs::canonicalize(path).map_err(|e| format!("{}: a link whose target cannot be read ({e}): nothing changed", path.display()))
}

/// `new` over the agent's file at `path` (resolved), backed up first, only if it still holds
/// `read`, what terminum read of it (Err: the agent wrote it meanwhile; nothing changed).
pub fn replace_checked(path: &Path, read: Option<&str>, new: &str) -> Result<(), String> {
    if std::fs::read_to_string(path).ok().as_deref() != read {
        return Err(format!("{}: it changed while terminum was editing it (Codex wrote it?): nothing changed; run the command again", path.display()));
    }
    backup(path)?;
    write_text(path, new)
}

/// A copy beside `path` (`hooks.json.terminum-<time>`) before terminum changes it.
fn backup(path: &Path) -> Result<(), String> {
    if path.is_file() {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let to = path.with_file_name(format!("{name}.terminum-{}", crate::auth::store::unix_now()));
        std::fs::copy(path, &to).map_err(|e| format!("{}: {e}", to.display()))?;
    }
    Ok(())
}

/// How long one `claude` call may take when asked for (`terminum hooks`); an install's refresh
/// and an uninstall use `QUICK` (agentrefresh).
pub(super) const ASKED: std::time::Duration = std::time::Duration::from_secs(60);

fn claude(ctx: &Ctx, wait: std::time::Duration, args: &[&str]) -> Result<String, String> {
    let prog = on_path(ctx, "claude").ok_or("claude is not on PATH")?;
    let mut cmd = std::process::Command::new(prog);
    cmd.args(args).env("PATH", ctx.var("PATH").unwrap_or(""));
    let out = super::launch::run_timeout(cmd, wait).map_err(|e| format!("claude {}: {e}", args.join(" ")))?;
    let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)).trim().to_string();
    if out.status.success() { Ok(said) } else { Err(format!("claude {}: {said}", args.join(" "))) }
}

pub(super) fn install_claude(ctx: &Ctx, wait: std::time::Duration) -> Result<(), String> {
    let market = ctx.root.join("hooks/claude");
    let plugin = plugin_dir(ctx);
    write(&market.join(".claude-plugin/marketplace.json"), &json!({"name": MARKET, "owner": {"name": "terminum"}, "description": "terminum's hooks for Claude Code",
        "plugins": [{"name": NAME, "source": "./plugins/terminum", "description": "terminum: the session's states, prompts and subagents"}]}))?;
    write(&plugin.join(".claude-plugin/plugin.json"), &json!({"name": NAME, "version": crate::config::build_id(), "author": {"name": "terminum"},
        "description": "terminum: tells terminumd what this session does (terminum hooks)"}))?;
    write(&plugin.join("hooks/hooks.json"), &claude_hooks(&command(ctx, "claude")))?;
    write(&plugin.join(".mcp.json"), &json!({"mcpServers": {NAME: {"command": program(ctx), "args": ["mcp"]}}}))?;
    // added anew each time, so Claude Code copies this build's plugin, not one it cached before;
    // a claude that does not answer is not waited for again
    if let Err(e) = claude(ctx, wait, &["plugin", "uninstall", PLUGIN]) && e.contains("no answer within") { return Err(e) }
    let _ = claude(ctx, wait, &["plugin", "marketplace", "remove", MARKET]);
    claude(ctx, wait, &["plugin", "marketplace", "add", &market.display().to_string()])?;
    claude(ctx, wait, &["plugin", "install", PLUGIN, "--scope", "user"])?;
    std::fs::write(marker(ctx), crate::config::build_id()).map_err(|e| e.to_string())?;
    ctx.say("Claude Code: terminum's plugin (hooks and MCP server) is installed; sessions started from now on use it.");
    Ok(())
}

pub(super) fn remove_claude(ctx: &Ctx, wait: std::time::Duration) -> Result<(), String> {
    claude(ctx, wait, &["plugin", "uninstall", PLUGIN])?;
    let _ = claude(ctx, wait, &["plugin", "marketplace", "remove", MARKET]);
    let _ = std::fs::remove_file(marker(ctx));
    ctx.say("Claude Code: terminum's plugin is removed.");
    Ok(())
}

fn install_codex(ctx: &Ctx) -> Result<(), String> {
    let path = resolved(&codex_home(ctx).join("hooks.json"))?;
    let text = std::fs::read_to_string(&path).ok();
    let v = codex_add(text.as_deref(), &command(ctx, "codex")).map_err(|e| format!("{}: {e}", path.display()))?;
    let conf = resolved(&codex_home(ctx).join("config.toml"))?;
    let toml = std::fs::read_to_string(&conf).ok();
    // both judged before either is written: a refusal changes nothing
    let new_toml = codex_mcp_add(toml.as_deref(), &program(ctx)).map_err(|e| format!("{}: {e}", conf.display()))?;
    if text.as_deref().and_then(|t| serde_json::from_str::<Value>(t).ok()).as_ref() == Some(&v) {
        ctx.say("Codex: terminum's hooks are in already.");
    } else {
        let json = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())? + "\n";
        replace_checked(&path, text.as_deref(), &json)?;
        ctx.say(format!("Codex: terminum's hooks are in {}. Codex runs a hook only once you trust it: open /hooks in Codex and trust terminum's.", ctx.shown(&path)));
    }
    if toml.as_deref() == Some(new_toml.as_str()) {
        ctx.say("Codex: terminum's MCP server is in already.");
    } else {
        replace_checked(&conf, toml.as_deref(), &new_toml)?;
        ctx.say(format!("Codex: terminum's MCP server is in {} (its block between \"{START}\" and \"{END}\").", ctx.shown(&conf), START = MCP_START, END = MCP_END));
    }
    if new_toml.lines().any(|l| l.trim_start().starts_with("[hooks")) {
        ctx.say("Codex: config.toml has hooks too; Codex runs both.");
    }
    ctx.say("Codex: sessions already running keep what they started with: restart them to use terminum's hooks and MCP server.");
    Ok(())
}

pub(super) fn remove_codex(ctx: &Ctx) -> Result<(), String> {
    let path = resolved(&codex_home(ctx).join("hooks.json"))?;
    let conf = resolved(&codex_home(ctx).join("config.toml"))?;
    let toml = std::fs::read_to_string(&conf).ok();
    let new_toml = toml.as_deref().map(codex_mcp_remove).transpose().map_err(|e| format!("{}: {e}", conf.display()))?;
    match std::fs::read_to_string(&path) {
        Err(_) => ctx.say("Codex: no hooks.json, no hooks to remove."),
        Ok(text) => {
            let v: Value = serde_json::from_str(&text).map_err(|e| format!("{}: it does not parse ({e}): nothing changed", path.display()))?;
            let out = codex_remove(&v, &command(ctx, "codex"));
            if out != v {
                let json = serde_json::to_string_pretty(&out).map_err(|e| e.to_string())? + "\n";
                replace_checked(&path, Some(&text), &json)?;
            }
            ctx.say("Codex: terminum's hooks are removed.");
        }
    }
    if let Some(new) = new_toml.filter(|n| toml.as_deref() != Some(n.as_str())) {
        replace_checked(&conf, toml.as_deref(), &new)?;
        ctx.say("Codex: terminum's MCP server is removed.");
    }
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
                    ("install", "claude") => install_claude(ctx, ASKED)?,
                    ("install", _) => install_codex(ctx)?,
                    (_, "claude") => remove_claude(ctx, ASKED)?,
                    _ => remove_codex(ctx)?,
                }
            }
        }
        _ => {
            let last = super::launch::health(ctx).map(|h| h["hooks"].clone()).unwrap_or(Value::Null);
            for a in AGENTS {
                let mcp = match (mcp_installed(ctx, a), installed(ctx, a)) {
                    (true, _) => "; MCP server in",
                    (false, true) => "; MCP server not in (terminum hooks install)",
                    _ => "; MCP server not in",
                };
                let state = if !found(ctx, a) { "not found".to_string() } else if installed(ctx, a) {
                    match last[a].as_u64() {
                        Some(t) => format!("found; terminum's hooks installed, last heard {} s ago", crate::auth::store::unix_now().saturating_sub(t)),
                        None => "found; terminum's hooks installed, not heard since terminumd started".to_string(),
                    }
                } else { "found; terminum's hooks not installed (terminum hooks install)".to_string() };
                let state = if found(ctx, a) { state + mcp } else { state };
                ctx.say(format!("{:<13}{state}", format!("{}:", label(a))));
            }
        }
    }
    Ok(())
}

/// One line for `terminum status`, `open` and `install` when an agent is found without terminum's
/// hooks, or has them without its MCP server (an install's refresh failed).
pub fn hint(ctx: &Ctx) -> Option<String> {
    let names = |f: &dyn Fn(&str) -> bool| AGENTS.iter().copied().filter(|a| f(a)).map(label).collect::<Vec<_>>().join(" and ");
    let hooks = names(&|a| found(ctx, a) && !installed(ctx, a));
    let mcp = names(&|a| installed(ctx, a) && !mcp_installed(ctx, a));
    let lines: Vec<String> = [
        (!hooks.is_empty()).then(|| format!("{hooks} found without terminum's hooks: run terminum hooks install for exact states, prompts and reminders")),
        (!mcp.is_empty()).then(|| format!("{mcp} has terminum's hooks without its MCP server (plans, notes): run terminum hooks install")),
    ].into_iter().flatten().collect();
    (!lines.is_empty()).then(|| lines.join("; "))
}
