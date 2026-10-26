// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
use super::*;
use std::path::PathBuf;

#[test]
fn ssh_args_forward_a_socket_without_mux_or_exit_on_failure() {
    assert_eq!(ssh_args(None), ["-o", "ExitOnForwardFailure=yes"], "agents off: files' ssh as before");
    let a = ssh_args(Some(("/home/alex/.terminum/run/m-abc-12345678.sock", &PathBuf::from("/Users/alex/.terminum/links/h.sock"))));
    assert_eq!(a, ["-o", "ControlPath=none", "-o", "ExitOnForwardFailure=no", "-R", "/home/alex/.terminum/run/m-abc-12345678.sock:/Users/alex/.terminum/links/h.sock"]);
    assert!(!a.iter().any(|s| s.contains("ClearAllForwardings")));
    assert_eq!(helper_args("/r/m.sock", "abc"), " --link /r/m.sock --mac abc");
}

#[test]
fn remote_and_local_paths_are_checked() {
    assert_eq!(remote_path("/home/alex/", "abc", "12345678").unwrap(), "/home/alex/.terminum/run/m-abc-12345678.sock");
    let long = format!("/{}", "h".repeat(70));
    assert!(remote_path(&long, "abc", "12345678").unwrap_err().contains("longer than 100"));
    assert!(remote_path("relative", "abc", "12345678").is_err());
    assert!(remote_path("/a:b", "abc", "12345678").is_err());
    assert!(local_ok(&PathBuf::from("/Users/alex/.terminum/links/x.sock")).is_ok());
    assert!(local_ok(&PathBuf::from(format!("/{}", "x".repeat(103)))).unwrap_err().contains("103"));
    assert!(local_ok(&PathBuf::from("x.sock")).is_err());
}

#[test]
fn the_probe_reads_after_its_marker_whatever_rc_files_print() {
    let m = "tm-probe-0123456789ab";
    let out = format!("Welcome to devbox!\nLinux is great\n{m}\nLinux x86_64\n/home/alex\n");
    assert_eq!(parse_probe(&out, m), Some(("Linux".into(), "x86_64".into(), "/home/alex".into())));
    assert_eq!(parse_probe(&format!("{m}\r\nDarwin arm64\r\n/Users/alex\r\n"), m), Some(("Darwin".into(), "arm64".into(), "/Users/alex".into())));
    assert_eq!(parse_probe("Linux x86_64\n/home/alex\n", m), None, "no marker: no answer");
    assert_eq!(parse_probe(&format!("{m}\nLinux\n"), m), None);
    assert!(probe_command(m).starts_with(&format!("echo {m};")));
}

#[test]
fn link_lines_and_retry_timing() {
    assert_eq!(said("terminumd-agent link: up"), Some(Said::Up));
    assert_eq!(said("terminumd-agent link: refused"), Some(Said::Refused));
    assert_eq!(said("terminumd-agent ready x"), None);
    let now = Instant::now();
    assert!(try_forward(None, now));
    assert!(!try_forward(Some(now), now + Duration::from_secs(599)));
    assert!(try_forward(Some(now), now + RETRY));
    assert!(refused_why("devbox.example").contains("devbox.example's sshd"));
}

#[test]
fn the_mac_id_is_made_once_and_kept() {
    let d = std::env::temp_dir().join(format!("tm-macid-{}", crate::auth::crypto::random_hex::<4>()));
    std::fs::create_dir_all(&d).unwrap();
    let a = mac_id(&d);
    assert_eq!(a.len(), 12);
    assert_eq!(mac_id(&d), a);
    std::fs::remove_dir_all(&d).unwrap();
}
