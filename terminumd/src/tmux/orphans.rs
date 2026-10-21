// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Control clients a dropped link left on a host (AC-81). When ssh's end goes (the network, a
//! sleeping Mac), tmux keeps the client's pipes and its unsent output, and the client never
//! exits: `detach-client` cannot end it (measured on tmux 3.4), a signal does. Once a
//! connection, terminumd ends the host's control clients that are terminum's and orphaned:
//! their parent is init, or a shell whose parent is init. A live client (this Mac's, another
//! Mac's, whatever its ssh server) has a live parent and is never touched; a client under a
//! subreaper is left alone.
//!
//! terminum's are found two ways: in tmux's own list, control clients with `ignore-size` (also
//! where tmux rewrites its command line: FreeBSD, OpenBSD); and in `ps`, those named
//! `tmux: client` (Linux names them so) with terminum's command line, also those stuck before
//! tmux listed them. Never the server: its pid is skipped, and a server started by terminum's
//! command keeps that command line (it is named `tmux: server`). The script tells what it
//! ended through a user option of the server, read and removed by terminumd.

use std::sync::Arc;
use std::time::Duration;

use super::args::quote;
use super::Server;

/// Once a connection of a host's server: its orphaned terminum control clients are ended.
pub fn sweep_once(server: &Arc<Server>) {
    if server.is_local() || !server.connected() || !super::first_on_link("orphans", server) {
        return;
    }
    let server = server.clone();
    tokio::spawn(async move { sweep(&server).await });
}

/// The control clients tmux lists with `ignore-size`, as terminum's are, from
/// `list-clients -F '#{client_pid} #{client_control_mode} #{client_flags}'`.
fn listed(lines: &[String]) -> Vec<u32> {
    lines.iter().filter_map(|l| {
        let mut f = l.split(' ');
        let (pid, control, flags) = (f.next()?, f.next()?, f.next()?);
        (control == "1" && flags.split(',').any(|x| x == "ignore-size")).then(|| pid.parse().ok()).flatten()
    }).collect()
}

/// The server's user option the script answers in: one per terminumd and connection, so two
/// Macs never read each other's and a late answer is never taken for a later sweep's.
fn answer_option(server: &Server) -> String {
    format!("@tm-orphans-{}-{}", std::process::id(), server.generation.load(std::sync::atomic::Ordering::Relaxed))
}

async fn sweep(server: &Server) {
    // the server's own pid, never a candidate: without it, nothing is swept
    let Some(pid) = server.run("display-message -p '#{pid}'").await.ok().and_then(|l| l.first()?.trim().parse::<u32>().ok()) else { return };
    let lines = server.run("list-clients -F '#{client_pid} #{client_control_mode} #{client_flags}'").await.unwrap_or_default();
    let opt = answer_option(server);
    let _ = server.run(format!("set-option -gu {opt}")).await;
    // in the background (a slow `ps` never holds up the host's commands), quiet: a control
    // client gets run-shell's output outside its answer
    if let Err(e) = server.run(format!("run-shell -b {}", quote(&script(pid, &listed(&lines), &opt)))).await {
        tracing::warn!(event = "clients.orphans", server = %server.id(), error = e);
        return;
    }
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let Ok(got) = server.run(format!("show-options -gqv {opt}")).await else { return };
        let Some(ended) = got.first().and_then(|l| l.strip_prefix("done")) else { continue };
        let _ = server.run(format!("set-option -gu {opt}")).await;
        let ended: Vec<u32> = ended.split_whitespace().filter_map(|p| p.parse().ok()).collect();
        if !ended.is_empty() {
            tracing::info!(event = "clients.orphans", server = %server.id(), ended = ?ended);
        }
        return;
    }
    let _ = server.run(format!("set-option -gu {opt}")).await;
    tracing::warn!(event = "clients.orphans", server = %server.id(), error = "no answer in 10 s: clients may have been ended unreported");
}

/// A shell, as `ps -o comm=` names it (a login shell starts with `-`; macOS gives a path).
const SHELLS: &str = "sh|bash|dash|ash|zsh|ksh|mksh|fish|-*|*/sh|*/bash|*/dash|*/zsh|*/ksh|*/fish";

/// terminum's host commands (tmux/command.rs) as `ps` shows them, after any options before them.
const SHAPES: &str = "*\" -u -C new-session -A -f ignore-size -s terminum \"|*\" -u -C attach-session -E -f ignore-size -t \"*";

/// The host's `/bin/sh` script: of the user's processes but the tmux server `server`, those that
/// are terminum's control clients (`listed` by tmux, or named `tmux: client` with its command
/// line) and orphaned get SIGTERM; their pids are set in the server's option `opt` after
/// "done". Every `ps` that fails skips its process. No `#` (run-shell expands formats) and no
/// `'` (it is quoted for the host's shell inside).
fn script(server: u32, listed: &[u32], opt: &str) -> String {
    let listed = match listed {
        [] => "\" - \"".to_string(),                                   // a pattern no pid matches
        l => l.iter().map(|p| format!("\" {p} \"")).collect::<Vec<_>>().join("|"),
    };
    let body = [
        "exec >/dev/null 2>&1",
        "set -f",
        "ps -ww -U $(id -u) -o pid= -o ppid= -o args= | { e=",
        "while read -r p q a",
        "do case \"$p\" in \"\"|*[!0-9]*) continue ;; esac",
        // listed by tmux: a tmux process still (a pid not reused); else Linux's client name and terminum's command
        &format!("case \" $p \" in \" {server} \") continue ;; {listed}) case \"$(ps -o comm= -p $p)\" in *tmux*) ;; *) continue ;; esac ;; *) case \" $a \" in {SHAPES}) ;; *) continue ;; esac"),
        "[ \"$(ps -o comm= -p $p)\" = \"tmux: client\" ] || continue ;; esac",
        "case \"$q\" in \"\"|*[!0-9]*) continue ;; esac",
        // a parent other than init: orphaned only when it is a shell whose parent is init
        "if [ $q -ne 1 ]",
        "then case \"$(ps -o comm= -p $q)\" in {SHELLS}) ;; *) continue ;; esac",
        "[ $(ps -o ppid= -p $q) -eq 1 ] || continue",
        "fi",
        "kill -TERM $p && e=\"$e $p\"",
        "done",
        &format!("tmux set-option -g {opt} \"done$e\"",),
        "}",
        "true",
    ];
    format!("/bin/sh -c '{}'", body.join("; ").replace("{ e=;", "{ e=\"\";").replace("{SHELLS}", SHELLS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listed_are_control_clients_with_terminums_flags() {
        let lines: Vec<String> = [
            "123 1 attached,focused,control-mode,ignore-size,pause-after=2,UTF-8",
            "456 0 attached,focused,UTF-8",
            "789 1 attached,control-mode,UTF-8",
            "x 1 attached,control-mode,ignore-size,pause-after=2",
            "321 1 control-mode,ignore-size",
        ].iter().map(|s| s.to_string()).collect();
        assert_eq!(listed(&lines), vec![123, 321]);
    }

    /// AC-81: the script is plain for run-shell (no format, no quote of its own) and names only pids.
    #[test]
    fn the_script_names_only_pids_and_passes_run_shell() {
        for s in [script(99, &[12, 345], "@tm-orphans-7"), script(99, &[], "@tm-orphans-7")] {
            assert!(s.starts_with("/bin/sh -c '") && s.ends_with('\''));
            assert_eq!(s.matches('\'').count(), 2);
            assert!(!s.contains('#'));
            // the server is never a candidate; by `ps` only a process tmux named its client
            assert!(s.contains("case \" $p \" in \" 99 \") continue ;; "));
            assert!(s.contains("[ \"$(ps -o comm= -p $p)\" = \"tmux: client\" ] || continue ;; esac"));
            assert!(s.contains("kill -TERM $p && e=\"$e $p\""));
            assert!(s.contains("tmux set-option -g @tm-orphans-7 \"done$e\""));
            // a ppid that is not one number (a ps that failed or ignored -p) skips its process
            assert!(s.contains("case \"$q\" in \"\"|*[!0-9]*) continue ;; esac; if [ $q -ne 1 ]"));
        }
        assert!(script(99, &[12, 345], "@o").contains("continue ;; \" 12 \"|\" 345 \") case "));
        assert!(script(99, &[], "@o").contains("continue ;; \" - \") case "));
    }

    /// AC-81: the script itself, under /bin/sh, against a stand-in `ps` (nothing is signalled:
    /// `kill` and `tmux` print). Ended: an orphan of init's, one under a shell of init's, one
    /// tmux lists whose command line tmux rewrote. Left: the server, a client under sshd, under
    /// ssh, under a shell that lives, a listed pid now another program, a line a backslash ends,
    /// a pid that is not a number.
    #[test]
    fn the_script_ends_only_orphaned_clients() {
        let dir = std::env::temp_dir().join(format!("tm-orphans-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let shape = "tmux -u -C new-session -A -f ignore-size -s terminum";
        let list = [
            format!("   10     1 {shape}"),
            format!("   11     1 {shape}"),
            "   12    50 tmux -u -C attach-session -E -f ignore-size -t $0".into(),
            format!("   13    60 {shape}"),
            "   14     1 tmux -C attach \\".into(),
            format!("   15     1 {shape}"),
            "   16     1 tmux: client (/tmp/tmux-1000/default)".into(),
            "   17     1 python3 serve.py".into(),
            format!("  abc     1 {shape}"),
            format!("   18    70 {shape}"),
            format!("   20    80 {shape}"),
        ];
        std::fs::write(dir.join("list"), list.join("\n") + "\n").unwrap();
        std::fs::write(dir.join("comm"), "10 tmux: server\n11 tmux: client\n12 tmux: client\n13 tmux: client\n14 tmux: client\n15 vim\n16 tmux\n17 python3\n18 tmux: client\n20 tmux: client\n50 sh\n60 sshd\n70 ssh\n80 bash\n").unwrap();
        std::fs::write(dir.join("ppid"), "50 1\n60 1\n70 1\n80 2\n").unwrap();
        let ps = dir.join("ps");
        std::fs::write(&ps, "#!/bin/sh\nd=$(dirname \"$0\"); for n; do :; done\ncase \"$*\" in *-U*) cat \"$d/list\" ;; *comm=*) grep \"^$n \" \"$d/comm\" | cut -d\" \" -f2- ;; *ppid=*) grep \"^$n \" \"$d/ppid\" | cut -d\" \" -f2- ;; esac\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&ps, std::fs::Permissions::from_mode(0o755)).unwrap();
        let s = script(10, &[16, 17], "@o");
        let inner = s.strip_prefix("/bin/sh -c '").and_then(|x| x.strip_suffix('\'')).unwrap()
            .replace("exec >/dev/null 2>&1; ", "").replace("kill -TERM $p", "echo KILL $p").replace("tmux set-option -g @o", "echo ANSWER");
        let out = std::process::Command::new("/bin/sh").arg("-c").arg(&inner)
            .env("PATH", format!("{}:/usr/bin:/bin", dir.display())).output().unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(out.stderr.is_empty(), "{}", String::from_utf8_lossy(&out.stderr));
        let out = String::from_utf8_lossy(&out.stdout);
        let killed: Vec<&str> = out.lines().filter_map(|l| l.strip_prefix("KILL ")).collect();
        assert_eq!(killed, ["11", "12", "16"], "{out}");
        assert!(out.contains("ANSWER done 11 12 16"), "{out}");
    }
}
