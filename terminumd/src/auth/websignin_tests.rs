// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-09: brakes and the password check.

use super::*;
use super::super::crypto::{hex, pbkdf2_sha256};

/// A hash of `password` cheap enough for tests.
fn cheap(password: &str) -> PasswordHash {
    PasswordHash { salt: "73616c74".into(), iterations: 2, hash: hex(&pbkdf2_sha256(password.as_bytes(), b"salt", 2, 32)) }
}

#[test]
fn wrong_passwords_slow_down_one_address_only() {
    let mut b = Brakes::default();
    let t0 = Instant::now();
    for _ in 0..FREE_ATTEMPTS - 1 {
        b.failed("10.0.0.9", t0);
    }
    assert_eq!(b.locked_for("10.0.0.9", t0), 0, "four are free");
    b.failed("10.0.0.9", t0);
    assert_eq!(b.locked_for("10.0.0.9", t0), 15, "the fifth waits 15 s");
    assert_eq!(b.locked_for("10.0.0.5", t0), 0, "one address cannot lock out another");
    assert_eq!(b.locked_for("10.0.0.9", t0 + Duration::from_millis(14_001)), 1, "rounded up");
    assert_eq!(b.locked_for("10.0.0.9", t0 + Duration::from_secs(15)), 0);
    b.failed("10.0.0.9", t0 + Duration::from_secs(15));
    assert_eq!(b.locked_for("10.0.0.9", t0 + Duration::from_secs(15)), 30, "doubling");
    b.passed("10.0.0.9");
    assert_eq!(b.locked_for("10.0.0.9", t0 + Duration::from_secs(15)), 0, "the right password clears it");
}

#[test]
fn the_wait_doubles_up_to_five_minutes() {
    let waits: Vec<u64> = (3..=12).map(|c| lock_for(c, FREE_ATTEMPTS)).collect();
    assert_eq!(waits, [0, 0, 15, 30, 60, 120, 240, 300, 300, 300]);
    assert_eq!(lock_for(u32::MAX, FREE_ATTEMPTS), MAX_LOCK_S, "no overflow");
    assert_eq!(lock_for(FREE_ATTEMPTS_ALL, FREE_ATTEMPTS_ALL), 15);
}

#[test]
fn changing_address_does_not_escape_the_brake() {
    let mut b = Brakes::default();
    let t0 = Instant::now();
    for i in 0..FREE_ATTEMPTS_ALL {
        b.failed(&format!("2001:db8::{i}"), t0);
    }
    assert!(b.locked_for("2001:db8::ffff", t0) > 0, "a fresh address waits too");
    assert_eq!(b.locked_for("127.0.0.1", t0), 0, "guesses through the proxy never keep the Mac's own browser waiting");
    b.passed("2001:db8::ffff");
    assert_eq!(b.locked_for("2001:db8::fffe", t0), 0);
}

#[test]
fn this_macs_own_guesses_are_counted_apart() {
    let mut b = Brakes::default();
    let t0 = Instant::now();
    for _ in 0..FREE_ATTEMPTS {
        b.failed("127.0.0.1", t0);
    }
    assert!(b.locked_for("127.0.0.1", t0) > 0);
    assert_eq!(b.locked_for("100.64.0.7", t0), 0, "a tailnet device is not held by them");
}

#[test]
fn many_addresses_do_not_grow_the_brakes_forever() {
    let mut b = Brakes::default();
    let t0 = Instant::now();
    for i in 0..1002 {
        b.failed(&format!("10.1.{}.{}", i / 256, i % 256), t0);
    }
    b.failed("10.9.9.9", t0 + Duration::from_secs(1));
    assert!(b.failures.len() <= 2, "the ones no longer waiting went: {}", b.failures.len());
}

#[test]
fn a_proxy_on_this_mac_names_the_browser_others_cannot() {
    let ip = |s: &str| s.parse::<IpAddr>().unwrap();
    // the proxy appends the address it saw; what the client sent before it is not believed
    assert_eq!(client_address(ip("127.0.0.1"), Some("6.6.6.6, 100.64.0.7")), "100.64.0.7");
    assert_eq!(client_address(ip("::1"), Some("100.64.0.7")), "100.64.0.7");
    assert_eq!(client_address(ip("::ffff:127.0.0.1"), Some("100.64.0.7")), "100.64.0.7");
    assert_eq!(client_address(ip("10.0.0.9"), Some("1.2.3.4")), "10.0.0.9");
    assert_eq!(client_address(ip("::ffff:10.0.0.9"), None), "10.0.0.9");
    assert_eq!(client_address(ip("127.0.0.1"), Some("1.2.3.4, ")), "127.0.0.1", "an empty last entry");
    assert_eq!(client_address(ip("127.0.0.1"), None), "127.0.0.1");
}

fn auth(hold: Duration) -> Arc<WebAuth> {
    let mut a = WebAuth::new(cheap("right one"));
    a.hold = hold;
    Arc::new(a)
}

#[tokio::test]
async fn only_the_right_password_passes() {
    let a = auth(Duration::from_millis(10));
    assert_eq!(a.login("wrong".into(), "10.0.0.5".into()).await, Login::Wrong { retry: 0 });
    assert_eq!(a.login("right one".into(), "10.0.0.5".into()).await, Login::Right);
    assert_eq!(a.login("right one ".into(), "10.0.0.5".into()).await, Login::Wrong { retry: 0 });
}

#[tokio::test]
async fn a_wrong_password_holds_the_next_check_for_a_second() {
    let a = auth(WRONG_HOLD);
    let t0 = Instant::now();
    let (wrong, right) = tokio::join!(a.login("guess".into(), "10.0.0.9".into()), async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        a.login("right one".into(), "10.0.0.5".into()).await
    });
    assert_eq!(wrong, Login::Wrong { retry: 0 });
    assert!(matches!(right, Login::Right));
    assert!(t0.elapsed() >= WRONG_HOLD, "the right one waited for the wrong one's hold");
}

#[tokio::test]
async fn guesses_sent_at_once_are_checked_one_by_one() {
    let a = auth(Duration::from_millis(5));
    let tries = (0..12).map(|_| {
        let a = a.clone();
        tokio::spawn(async move { a.login("guess".into(), "10.0.0.9".into()).await })
    });
    let mut wrong = 0;
    let mut locked = 0;
    for t in tries {
        match t.await.unwrap() {
            Login::Wrong { .. } => wrong += 1,
            Login::Locked { retry } => {
                assert!(retry > 0);
                locked += 1;
            }
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(wrong, FREE_ATTEMPTS as usize, "after the free tries the rest wait instead of hashing");
    assert_eq!(locked, 12 - FREE_ATTEMPTS as usize);
    assert!(matches!(a.login("right one".into(), "10.0.0.9".into()).await, Login::Locked { .. }), "even the right one waits");
    assert!(matches!(a.login("right one".into(), "10.0.0.5".into()).await, Login::Right), "another address does not");
}

#[tokio::test]
async fn hanging_up_mid_check_escapes_neither_the_brakes_nor_the_hold() {
    let a = auth(Duration::from_millis(200));
    for _ in 0..FREE_ATTEMPTS {
        // the client goes before the answer: its request's future is dropped
        let _ = tokio::time::timeout(Duration::from_millis(1), a.login("guess".into(), "10.0.0.9".into())).await;
    }
    let t0 = Instant::now();
    let next = a.login("right one".into(), "10.0.0.9".into()).await;
    assert!(matches!(next, Login::Locked { .. }), "{next:?}");
    assert!(t0.elapsed() >= Duration::from_millis(150), "it waited for the dropped checks' holds");
}
