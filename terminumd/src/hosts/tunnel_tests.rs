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
    let dir = std::env::temp_dir().join(format!("tm-fw-{}", crate::auth::crypto::random_hex::<4>()));
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
