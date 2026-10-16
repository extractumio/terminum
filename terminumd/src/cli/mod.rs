// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! terminum: the one command a user needs after the one-line install (AC-08 … AC-10, AC-16,
//! AC-87). It is terminumd itself, run by the name `terminum` (~/.terminum/bin/terminum links
//! the newest build's terminumd) or with a command as its first argument (`terminumd install`,
//! as install.sh runs it from the package).

mod commands;
mod ctx;
mod direct;
mod importer;
mod install;
mod launch;
mod sockhttp;
mod term;
mod upgrade;

#[cfg(test)]
mod tests;

pub use ctx::Ctx;

const ADD_USAGE: &str = "usage: terminum hosts add [<name>] '<ssh destination and options>' [--files]";

const USAGE: &str = "\
usage: terminum [command]

  status                  what runs: build, pid, port, web access, tmux, hosts, pages
  open [--print]          a one-time sign-in link for this Mac's browser (60 s)
  web [status|on|off|password]
                          web access for other devices (a password of 8+ characters)
  signout                 sign out every page on this Mac
  hosts                   the remote hosts and their state
  hosts add [<name>] '<ssh destination and options>' [--files]
                          as after ssh in a shell; no name: the destination's host;
                          --files: browse its files (copies a helper once attached)
  hosts remove|enable|files-remove|off|on|retry <name>
  upgrade [--to vX.Y.Z]   download a release, check it, install it
  rollback                back to the build before
  restart                 load the launchd agent again (a new port; a port taken back)
  uninstall               remove it (keeps settings, state and logs; tmux untouched)
  import                  workspaces, bookmarks and hosts of iterm-enhancer
  logs [-n N] [-f]        terminumd's log
  install                 install the package this terminumd came in";

const COMMANDS: [&str; 15] = [
    "status", "open", "web", "signout", "hosts", "upgrade", "rollback", "restart", "uninstall", "import", "logs", "install", "help", "-h", "--help",
];

/// Is this run the command rather than the server?
pub fn wanted(args: &[String]) -> bool {
    let name = args.first().map(|a| a.rsplit('/').next().unwrap_or(a)).unwrap_or("");
    name == "terminum" || args.get(1).is_some_and(|a| COMMANDS.contains(&a.as_str()))
}

/// A parsed command line.
#[derive(Debug, PartialEq)]
pub enum Cmd {
    Status,
    Open { print: bool },
    Web(Option<String>),
    Signout,
    Hosts(Option<String>, Option<String>, Vec<String>),
    /// `hosts add [<name>] '<recipe>' [--files]`
    HostsAdd { name: Option<String>, recipe: Vec<String>, files: bool },
    Upgrade(Option<String>),
    Rollback,
    Restart,
    Uninstall,
    Import,
    Logs { lines: u64, follow: bool },
    Install,
    Help,
    Version,
}

fn usage(msg: &str) -> String {
    format!("{msg}\n(terminum help lists the commands)")
}

/// The command line after the program's name; Err(usage message).
pub fn parse(args: &[String]) -> Result<Cmd, String> {
    let Some((cmd, rest)) = args.split_first() else { return Ok(Cmd::Status) };
    let none = |c: Cmd| if rest.is_empty() { Ok(c) } else { Err(usage(&format!("usage: terminum {cmd}"))) };
    match cmd.as_str() {
        "status" => none(Cmd::Status),
        "signout" => none(Cmd::Signout),
        "rollback" => none(Cmd::Rollback),
        "restart" => none(Cmd::Restart),
        "uninstall" => none(Cmd::Uninstall),
        "import" => none(Cmd::Import),
        "install" => none(Cmd::Install),
        "help" | "-h" | "--help" => Ok(Cmd::Help),
        "--version" => Ok(Cmd::Version),
        "open" => match rest {
            [] => Ok(Cmd::Open { print: false }),
            [p] if p == "--print" => Ok(Cmd::Open { print: true }),
            _ => Err(usage("usage: terminum open [--print]")),
        },
        "web" => match rest {
            [] => Ok(Cmd::Web(None)),
            [w] if ["status", "on", "off", "password"].contains(&w.as_str()) => Ok(Cmd::Web(Some(w.clone()))),
            _ => Err(usage("usage: terminum web [status|on|off|password]")),
        },
        "hosts" => match rest {
            [] => Ok(Cmd::Hosts(None, None, vec![])),
            [h, more @ ..] if h == "add" => {
                let files = more.iter().any(|a| a == "--files");
                let words: Vec<String> = more.iter().filter(|a| *a != "--files").cloned().collect();
                match words.as_slice() {
                    [] => Err(usage(ADD_USAGE)),
                    [recipe] => Ok(Cmd::HostsAdd { name: None, recipe: vec![recipe.clone()], files }),
                    [name, recipe @ ..] => Ok(Cmd::HostsAdd { name: Some(name.clone()), recipe: recipe.to_vec(), files }),
                }
            }
            [h, more @ ..] if commands::HOST_ACTIONS.contains(&h.as_str()) => {
                Ok(Cmd::Hosts(Some(h.clone()), more.first().cloned(), more.iter().skip(1).cloned().collect()))
            }
            _ => Err(usage("usage: terminum hosts [add|remove|enable|files-remove|off|on|retry] <name>")),
        },
        "upgrade" => match rest {
            [] => Ok(Cmd::Upgrade(None)),
            [to, tag] if to == "--to" => Ok(Cmd::Upgrade(Some(tag.clone()))),
            [t] if t.starts_with("--to=") => Ok(Cmd::Upgrade(Some(t["--to=".len()..].to_string()))),
            _ => Err(usage("usage: terminum upgrade [--to vX.Y.Z]")),
        },
        "logs" => {
            let (mut lines, mut follow, mut it) = (50, false, rest.iter());
            while let Some(a) = it.next() {
                match a.as_str() {
                    "-f" => follow = true,
                    "-n" => lines = it.next().and_then(|n| n.parse().ok()).ok_or_else(|| usage("usage: terminum logs [-n N] [-f]"))?,
                    _ => return Err(usage("usage: terminum logs [-n N] [-f]")),
                }
            }
            Ok(Cmd::Logs { lines, follow })
        }
        other => Err(usage(&format!("terminum: unknown command {other}"))),
    }
}

pub fn refuse_root(euid: u32) -> Result<(), String> {
    if euid == 0 {
        return Err("Run as your user, not root: terminum lives in your home folder".into());
    }
    Ok(())
}

/// Refused as root, private files from here on (the commands that change the install).
fn as_user() -> Result<(), String> {
    // SAFETY: geteuid and umask have no preconditions
    refuse_root(unsafe { libc::geteuid() })?;
    // SAFETY: as above
    unsafe { libc::umask(0o077) };
    Ok(())
}

/// Runs `cmd`; Ok(exit status) or Err(message for stderr, status 1).
pub fn run(ctx: &Ctx, cmd: Cmd) -> Result<i32, String> {
    match cmd {
        Cmd::Help => ctx.say(USAGE),
        Cmd::Version => ctx.say(crate::config::build_id()),
        Cmd::Status => commands::status(ctx),
        Cmd::Open { print } => commands::open_page(ctx, print)?,
        Cmd::Signout => commands::signout(ctx)?,
        Cmd::Web(w) => commands::web(ctx, w.as_deref())?,
        Cmd::Hosts(None, ..) => commands::hosts_list(ctx)?,
        Cmd::Hosts(Some(h), None, _) => return Err(format!("usage: terminum hosts {h} <name>")),
        Cmd::HostsAdd { name, recipe, files } => commands::hosts_add(ctx, name.as_deref(), &recipe, files)?,
        Cmd::Hosts(Some(h), Some(_), recipe) if !recipe.is_empty() => return Err(format!("usage: terminum hosts {h} <name>")),
        Cmd::Hosts(Some(h), Some(name), _) => commands::hosts_action(ctx, &h, &name)?,
        Cmd::Upgrade(tag) => return upgrade::upgrade(ctx, tag.as_deref()),
        Cmd::Import => commands::import_state(ctx)?,
        Cmd::Logs { lines, follow } => commands::logs(ctx, lines, follow)?,
        Cmd::Install | Cmd::Rollback | Cmd::Uninstall | Cmd::Restart => {
            as_user()?;
            let _lock = install::lock(ctx)?;
            match cmd {
                Cmd::Install => {
                    let pkg = ctx.pkg.clone().ok_or("cannot tell which package this terminumd came in")?;
                    install::install(ctx, &pkg)?;
                    install::path_tip(ctx);
                }
                Cmd::Rollback => install::rollback(ctx)?,
                Cmd::Uninstall => install::uninstall(ctx)?,
                _ => install::restart(ctx)?,
            }
        }
    }
    Ok(0)
}

/// The command line of this process; the exit status.
pub fn main(args: &[String]) -> i32 {
    let argv0 = args.first().map_or("terminum", String::as_str);
    let result = parse(args.get(1..).unwrap_or(&[])).map_err(|e| (2, e)).and_then(|cmd| {
        let ctx = Ctx::from_env(argv0).map_err(|e| (1, e))?;
        run(&ctx, cmd).map_err(|e| (1, e))
    });
    match result {
        Ok(code) => code,
        Err((code, msg)) => {
            eprintln!("{msg}");
            code
        }
    }
}
