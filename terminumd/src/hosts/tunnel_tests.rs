// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
use super::*;

#[test]
fn platforms_with_a_helper() {
    assert_eq!(platform("Darwin", "arm64"), Some("macos-aarch64"));
    assert_eq!(platform("Linux", "aarch64"), Some("linux-aarch64"));
    assert_eq!(platform("FreeBSD", "amd64"), None);
}

#[test]
fn only_this_builds_helper_is_checked_through_the_forward() {
    // AC-84: an older helper is replaced, never held to the forward's check (it failed
    // there and was retried with a backoff instead of updated)
    assert!(current(crate::agent::AGENT_ID));
    assert!(!current("0123456789ab"), "another build");
    assert!(!current(""), "no helper on the host");
}

#[test]
fn the_helper_answer_is_the_last_non_empty_line() {
    assert_eq!(answer("Welcome to devbox\nlast login: today\n0123456789ab\n"), "0123456789ab", "rc noise before");
    assert_eq!(answer("noise\n0123456789ab\n\n  \n"), "0123456789ab", "a trailing blank line");
    assert_eq!(answer("noise\r\n0123456789ab\r\n\r\n"), "0123456789ab", "CRLF");
    assert_eq!(answer(""), "");
}

#[tokio::test]
async fn a_forward_counts_only_when_the_helper_answers() {
    let dir = crate::testutil::temp_root().join(format!("tm-fw-{}", crate::auth::crypto::random_hex::<4>()));
    std::fs::create_dir_all(&dir).unwrap();
    let (up, refused) = (dir.join("up.sock"), dir.join("no.sock"));
    let (l1, l2) = (tokio::net::UnixListener::bind(&up).unwrap(), tokio::net::UnixListener::bind(&refused).unwrap());
    tokio::spawn(async move {
        let (mut s, _) = l1.accept().await.unwrap();
        let mut b = [0u8; 64];
        let _ = s.read(&mut b).await;
        let _ = s.write_all(b"HTTP/1.1 403 Forbidden\r\n\r\n").await;
    });
    // an sshd refusing the channel: ssh accepted on the Mac, then closes it
    tokio::spawn(async move { drop(l2.accept().await.unwrap()) });
    assert_eq!(answers(tokio::net::UnixStream::connect(&up).await.unwrap()).await, Ok(()));
    assert!(answers(tokio::net::UnixStream::connect(&refused).await.unwrap()).await.is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Issue #8 (AC-83): what reaches the host's login shell, run by zsh as sshd does (`$SHELL -c`):
/// the helper's removal with no old helper to match, which zsh's nomatch aborted.
#[test]
fn a_zsh_login_shell_runs_the_helpers_scripts_as_sh() {
    if std::process::Command::new("zsh").arg("-c").arg("true").output().is_err() {
        return eprintln!("skipped: zsh is not on PATH");
    }
    let home = crate::testutil::fresh_dir("zsh-host");
    std::fs::create_dir_all(home.join(".terminum/bin")).unwrap();
    std::fs::write(home.join(".terminum/bin/terminumd-agent-0123abcd"), "x").unwrap();
    let mut c = Command::new("ssh");
    with_command(&mut c, &["alex@devbox.example".to_string()], &remove_script());
    let remote = c.as_std().get_args().last().unwrap().to_str().unwrap().to_string();
    assert!(remote.starts_with("exec /bin/sh -c '"), "{remote}");
    let out = std::process::Command::new("zsh").arg("-c").arg(&remote).env("HOME", &home).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!home.join(".terminum").exists(), "the helper and its folders went");
    // as it was: the script itself in zsh stops at the first glob that matches nothing
    std::fs::create_dir_all(home.join(".terminum/bin")).unwrap();
    let old = std::process::Command::new("zsh").arg("-c").arg(remove_script()).env("HOME", &home).output().unwrap();
    assert!(String::from_utf8_lossy(&old.stderr).contains("no matches found"), "{old:?}");
    let _ = std::fs::remove_dir_all(&home);
}

/// An app with host `devbox` in `status`, its record `rec`; no link is started.
fn with_host(tag: &str, rec: serde_json::Value, status: crate::tmux::server::Status) -> (std::path::PathBuf, Shared) {
    let (root, app) = crate::testutil::app(tag);
    app.looks.settings.write().hosts.insert("devbox".into(), rec);
    let link = crate::tmux::server::Link::Ssh { ssh: "unused".into(), args: vec!["alex@devbox.example".into()] };
    let host = crate::tmux::server::Server::new("devbox", "default", link, app.changed.clone());
    *host.status.lock() = status;
    app.tmux.add_host(host);
    (root, app)
}

#[tokio::test]
async fn ac83_a_hosts_files_are_set_up_with_no_question_and_a_failure_or_a_no_is_said() {
    use crate::tmux::server::Status;
    let state = |app: &Shared| crate::hosts::offer(app, "devbox")["state"].as_str().unwrap().to_string();
    let (root, app) = with_host("offer-new", serde_json::json!({"ssh": ["alex@devbox.example"]}), Status::Connecting);
    assert_eq!(state(&app), "enabling", "no decision yet, the host connecting: being set up, never asked");
    set(&app, "devbox", "failed", "the host has no helper for its platform");
    app.tunnels.0.lock().get_mut("devbox").unwrap().failed_at = Some(Instant::now());
    let failed = crate::hosts::offer(&app, "devbox");
    assert_eq!((failed["state"].as_str(), failed["why"].as_str()), (Some("ask"), Some("the host has no helper for its platform")), "a failed setup says why and offers to try again");
    app.looks.settings.write().hosts["devbox"]["files"] = "declined".into();
    app.looks.settings.write().hosts["devbox"]["agents"] = "off".into();   // nothing here reaches ssh
    assert_eq!(state(&app), "dismissed", "the user's Not now or Remove: nothing is set up");
    // the next attach tries a failed setup again; a host the user declined starts nothing
    attached(&app, "devbox");
    assert!(!failed_none(&app), "a link that drops and comes back does not try a fresh failure again");
    retry(&app, "devbox", true);
    assert!(failed_none(&app), "the user adding the host again does");
    let (root2, old) = with_host("offer-old", serde_json::json!({"ssh": ["alex@devbox.example"], "helper": "0123456789ab"}), Status::Attached);
    assert_eq!(state(&old), "dismissed", "a helper's build and no state: the user removed it before removals were recorded");
    drop(old);
    std::fs::remove_dir_all(root2).unwrap();
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
    let (root, app) = with_host("offer-off", serde_json::json!({"ssh": ["alex@devbox.example"]}), Status::Off);
    assert_eq!(state(&app), "ask", "a host that is off is not being set up");
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

fn failed_none(app: &Shared) -> bool {
    failed(app, "devbox").is_none()
}

#[tokio::test]
async fn ac99_a_hosts_agents_are_turned_on_by_themselves_once_unless_the_user_turned_them_off() {
    for (rec, tried, why) in [
        (serde_json::json!({"ssh": ["alex@devbox.example"], "agents": "off"}), false, "the user turned them off"),
        (serde_json::json!({"ssh": ["alex@devbox.example"], "agents": "enabled"}), false, "on already"),
        (serde_json::json!({"ssh": ["alex@devbox.example"]}), true, "no word of the user's yet"),
    ] {
        // no server for the host, so nothing reaches ssh: the setup ends at "not attached"
        let (root, app) = crate::testutil::app("auto-agents");
        app.looks.settings.write().hosts.insert("devbox".into(), rec);
        crate::hosts::agents::auto(&app, "devbox");
        crate::hosts::agents::auto(&app, "devbox");
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert_eq!(crate::hosts::link::agents_state(&app, "devbox"), if tried { "failed: devbox is not attached" } else if app.looks.settings.read().hosts["devbox"]["agents"] == "enabled" { "connecting" } else { "off" }, "{why}");
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }
}
