// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! `hooks … --host-mode` on a fake host home: terminum-host in every command, the link, one
//! marker line per agent, idempotence, refresh by content, refusal and the host's own terminum.

use super::*;
use crate::cli::hostmode::{self, HostArgs};
use crate::cli::{agentsetup, parse, Cmd};

const M: &str = "TMH-0011223344556677";

struct Host {
    home: Home,
    codex: PathBuf,
    claude: PathBuf,
    log: PathBuf,
}

fn host() -> Host {
    let home = Home::new();
    let bin = home.path(".nvm/versions/node/v22.1.0/bin");
    let log = home.path("claude.log");
    write_exec(&bin.join("claude"), &format!("#!/bin/sh\necho \"$@\" >> '{}'\n[ \"$2\" = list ] && echo terminum@terminum-local\nexit 0\n", log.display()));
    let codex = home.path(".codex");
    std::fs::create_dir_all(&codex).unwrap();
    std::fs::create_dir_all(home.path(".terminum/bin")).unwrap();
    write_exec(&home.path(".terminum/bin/terminumd-agent"), "#!/bin/sh\n");
    Host { codex, claude: bin.join("claude"), log, home }
}

/// The result lines of one host-mode run.
fn go(h: &Host, action: &str, refresh: bool) -> Vec<String> {
    let args = HostArgs { action: action.into(), marker: M.into(), claude: Some(h.claude.display().to_string()), refresh };
    let mut ctx = h.home.ctx(&[]);
    hostmode::prepare(&mut ctx, &args);
    hostmode::run(&ctx, &args).unwrap();
    said(&ctx).into_iter().filter(|l| l.starts_with(M)).collect()
}

fn calls(h: &Host) -> String {
    std::fs::read_to_string(&h.log).unwrap_or_default()
}

fn json(p: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn the_host_mode_command_parses() {
    let a = |s: &str| s.split_whitespace().map(String::from).collect::<Vec<_>>();
    let Ok(Cmd::HostHooks(h)) = parse(&a(&format!("hooks install --host-mode --marker {M} --claude /h/.local/bin/claude --refresh"))) else { panic!() };
    assert_eq!(h, HostArgs { action: "install".into(), marker: M.into(), claude: Some("/h/.local/bin/claude".into()), refresh: true });
    for bad in ["hooks install --host-mode", "hooks install --host-mode --marker x", "hooks remove --host-mode --marker TMH-00112233 --refresh",
                "hooks install --host-mode --marker TMH-00112233 --claude claude", "hooks install --host-mode --marker TMH-00112233 codex"] {
        assert!(parse(&a(bad)).is_err(), "{bad}");
    }
}

#[test]
fn install_points_every_command_at_terminum_host_once() {
    let h = host();
    let prog = h.home.path(".terminum/bin/terminum-host").display().to_string();
    assert_eq!(go(&h, "install", false), vec![format!("{M} claude installed"), format!("{M} codex needs-trust")]);
    assert_eq!(std::fs::read_link(&prog).unwrap(), Path::new("terminumd-agent"), "a link to the helper");
    assert_eq!(mode(&h.home.path(".terminum/run")) & 0o777, 0o700);
    let plugin = h.home.path(".terminum/hooks/claude/plugins/terminum");
    assert_eq!(json(&plugin.join("hooks/hooks.json"))["hooks"]["Stop"][0]["hooks"][0]["command"], format!("{prog} hook claude"));
    assert_eq!(json(&plugin.join(".mcp.json"))["mcpServers"]["terminum"]["command"], prog.as_str());
    assert_eq!(json(&h.codex.join("hooks.json"))["hooks"]["Stop"][0]["hooks"][0]["command"], format!("{prog} hook codex"));
    assert!(std::fs::read_to_string(h.codex.join("config.toml")).unwrap().contains(&format!("command = \"{prog}\"")));
    assert!(calls(&h).contains("plugin install terminum@terminum-local --scope user"));

    // a second install: nothing changes, claude is not called again
    let (toml, hooks, n) = (std::fs::read_to_string(h.codex.join("config.toml")).unwrap(), std::fs::read_to_string(h.codex.join("hooks.json")).unwrap(), calls(&h).lines().count());
    assert_eq!(go(&h, "install", false), vec![format!("{M} claude installed"), format!("{M} codex installed")]);
    assert_eq!(std::fs::read_to_string(h.codex.join("config.toml")).unwrap(), toml);
    assert_eq!(std::fs::read_to_string(h.codex.join("hooks.json")).unwrap(), hooks);
    assert_eq!(calls(&h).lines().count(), n, "claude not called: {}", calls(&h));

    assert_eq!(go(&h, "remove", false), vec![format!("{M} claude removed"), format!("{M} codex removed")]);
    assert!(!agentsetup::codex_remove(&json(&h.codex.join("hooks.json")), "x").to_string().contains("terminum-host"));
    assert!(!std::fs::read_to_string(h.codex.join("config.toml")).unwrap().contains("terminum"));
    assert!(std::fs::symlink_metadata(&prog).is_err(), "the link goes with the last agent");
}

#[test]
fn refresh_rewrites_only_when_the_content_differs() {
    let h = host();
    go(&h, "install", false);
    let n = calls(&h).lines().count();
    // another build's marker and plugin version: content equal, nothing done
    let plugin = h.home.path(".terminum/hooks/claude/plugins/terminum");
    std::fs::write(h.home.path(".terminum/hooks/claude.installed"), "otherbuild").unwrap();
    assert_eq!(go(&h, "install", true), vec![format!("{M} claude installed"), format!("{M} codex installed")]);
    assert_eq!(calls(&h).lines().count(), n, "builds differing is no reason: {}", calls(&h));
    assert_eq!(std::fs::read_to_string(h.home.path(".terminum/hooks/claude.installed")).unwrap(), "otherbuild");
    // the hook command changed: the plugin updated in place (never uninstalled first)
    let mut v = json(&plugin.join("hooks/hooks.json"));
    v["hooks"]["Stop"][0]["hooks"][0]["command"] = "/old/terminum hook claude".into();
    std::fs::write(plugin.join("hooks/hooks.json"), v.to_string()).unwrap();
    assert_eq!(go(&h, "install", true)[0], format!("{M} claude installed"));
    let c = calls(&h);
    assert!(c.contains("plugin marketplace update terminum-local") && c.contains("plugin update terminum@terminum-local"), "{c}");
    assert_eq!(c.matches("plugin uninstall").count(), 1, "only the first install's: {c}");
    assert_eq!(json(&plugin.join("hooks/hooks.json"))["hooks"]["Stop"][0]["hooks"][0]["command"],
               format!("{} hook claude", h.home.path(".terminum/bin/terminum-host").display()));
    // an agent without terminum's is not added by a refresh
    let h2 = host();
    assert_eq!(go(&h2, "install", true), vec![format!("{M} claude off"), format!("{M} codex off")]);
    assert!(!h2.codex.join("hooks.json").exists() && calls(&h2).is_empty());
}

#[test]
fn a_foreign_terminum_entry_is_refused_and_untouched() {
    let h = host();
    let toml = "# >>> terminum: MCP server (terminum hooks install; terminum hooks remove takes it out)\n[mcp_servers.terminum]\ncommand = \"/opt/other/terminum\"\nargs = [\"mcp\"]\n# <<< terminum\n";
    std::fs::write(h.codex.join("config.toml"), toml).unwrap();
    let plugin = h.home.path(".terminum/hooks/claude/plugins/terminum");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(plugin.join(".mcp.json"), r#"{"mcpServers": {"terminum": {"command": "/opt/other/terminum", "args": ["mcp"]}}}"#).unwrap();
    let got = go(&h, "install", false);
    assert!(got.iter().all(|l| l.contains("refused: an entry named terminum runs /opt/other/terminum")), "{got:?}");
    assert_eq!(std::fs::read_to_string(h.codex.join("config.toml")).unwrap(), toml);
    assert!(!h.codex.join("hooks.json").exists() && calls(&h).is_empty());
    let got = go(&h, "remove", false);
    assert!(got.iter().all(|l| l.contains("refused")), "remove takes out only terminum-host's: {got:?}");
    assert_eq!(std::fs::read_to_string(h.codex.join("config.toml")).unwrap(), toml);
}

#[test]
fn a_host_with_its_own_terminum_is_skipped() {
    let h = host();
    let own = h.home.path(".terminum/bin/terminum");
    write_exec(&own, "#!/bin/sh\n");
    let cmd = format!("{} hook codex", own.display());
    let v = agentsetup::codex_add(None, &cmd).unwrap();
    std::fs::write(h.codex.join("hooks.json"), v.to_string()).unwrap();
    let plugin = h.home.path(".terminum/hooks/claude/plugins/terminum/hooks");
    std::fs::create_dir_all(&plugin).unwrap();
    std::fs::write(plugin.join("hooks.json"), agentsetup::claude_hooks(&format!("{} hook claude", own.display())).to_string()).unwrap();
    assert_eq!(go(&h, "install", false), vec![format!("{M} claude skipped-own"), format!("{M} codex skipped-own")]);
    assert_eq!(json(&h.codex.join("hooks.json")), v, "untouched");
    assert!(calls(&h).is_empty());
    assert_eq!(crate::hosts::agentsinstall::shown("devbox", "skipped-own"), "uses devbox's own terminum");
}

#[test]
fn not_found_without_claude_or_codex() {
    let h = host();
    std::fs::remove_dir_all(&h.codex).unwrap();
    let args = HostArgs { action: "install".into(), marker: M.into(), claude: None, refresh: false };
    let mut ctx = h.home.ctx(&[]);
    hostmode::prepare(&mut ctx, &args);
    hostmode::run(&ctx, &args).unwrap();
    let got: Vec<String> = said(&ctx).into_iter().filter(|l| l.starts_with(M)).collect();
    assert_eq!(got, vec![format!("{M} claude not-found"), format!("{M} codex not-found")]);
}
