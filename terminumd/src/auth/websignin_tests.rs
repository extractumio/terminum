// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-10: brakes, remembered sign-ins, the cookie and the password check.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use super::*;
use super::super::crypto::{hex, pbkdf2_sha256};
use crate::testutil::fresh_dir;

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

#[test]
fn the_cookie_is_out_of_scripts_and_other_sites_reach() {
    let c = cookie_header("tok", true);
    assert_eq!(c, format!("tm=tok; HttpOnly; SameSite=Strict; Path=/; Max-Age={}; Secure", 30 * 86400));
    assert_eq!(cookie_header("t", false), format!("tm=t; HttpOnly; SameSite=Strict; Path=/; Max-Age={}", 7 * 86400),
               "shorter over plain HTTP");
    assert!(cookie_header("", false).contains("Max-Age=0"));
    assert_eq!(cookie_value("a=1; tm=abc; b=2", COOKIE), Some("abc"));
    assert_eq!(cookie_value("xtm=abc", COOKIE), None);
    assert_eq!(cookie_value("", COOKIE), None);
}

#[test]
fn a_sign_in_outlives_a_restart_and_the_file_holds_no_token() {
    let dir = fresh_dir("websess-restart");
    let token = WebSessions::load(&dir, "s1").create(true).unwrap();
    assert_eq!(token.len(), 64);
    assert!(WebSessions::load(&dir, "s1").check(&token), "a new terminumd knows it");
    let raw = fs::read_to_string(dir.join(FILE)).unwrap();
    assert!(!raw.contains(&token));
    assert_eq!(fs::metadata(dir.join(FILE)).unwrap().permissions().mode() & 0o777, 0o600);
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let expiry = v["sessions"][sha256_hex(token.as_bytes())].as_u64().unwrap();
    assert!(expiry >= store::unix_now() + 30 * 86400 - 5, "30 days behind HTTPS");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_sign_out_stays_signed_out_after_a_restart() {
    let dir = fresh_dir("websess-out");
    let s = WebSessions::load(&dir, "s1");
    let (a, b) = (s.create(false).unwrap(), s.create(false).unwrap());
    s.sign_out(&a);
    s.sign_out("unknown");
    assert!(!s.check(&a) && s.check(&b));
    let again = WebSessions::load(&dir, "s1");
    assert!(!again.check(&a) && again.check(&b) && !again.check(""));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_new_password_signs_everyone_out() {
    let dir = fresh_dir("websess-salt");
    let token = WebSessions::load(&dir, "s1").create(false).unwrap();
    assert!(!WebSessions::load(&dir, "s2").check(&token));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn expired_and_surplus_sign_ins_go_the_ones_expiring_first() {
    let dir = fresh_dir("websess-max");
    let now = store::unix_now();
    let mut sessions = serde_json::Map::new();
    sessions.insert("old".into(), serde_json::json!(now as f64 - 1.0));
    for i in 0..150u64 {
        sessions.insert(format!("k{i}"), serde_json::json!(now as f64 + 1000.5 + i as f64));
    }
    fs::write(dir.join(FILE), serde_json::json!({"salt": "s1", "sessions": sessions}).to_string()).unwrap();
    let token = WebSessions::load(&dir, "s1").create(false).unwrap();
    let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(dir.join(FILE)).unwrap()).unwrap();
    let kept = v["sessions"].as_object().unwrap();
    assert_eq!(kept.len(), MAX_SESSIONS);
    assert!(!kept.contains_key("old"));
    assert!(!kept.contains_key("k0"), "the one that expires first went");
    assert!(kept.contains_key("k149") && kept.contains_key(&sha256_hex(token.as_bytes())));
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn an_unreadable_file_starts_empty() {
    let dir = fresh_dir("websess-broken");
    for raw in ["{not json", "[]", r#"{"salt": "s1", "sessions": {"a": "soon"}}"#, r#"{"salt": "s1"}"#] {
        fs::write(dir.join(FILE), raw).unwrap();
        let s = WebSessions::load(&dir, "s1");
        assert!(!s.check("a"), "{raw}");
        let token = s.create(false).unwrap();
        assert!(WebSessions::load(&dir, "s1").check(&token), "{raw}");
    }
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn switching_web_access_off_forgets_every_sign_in() {
    let dir = fresh_dir("websess-close");
    let s = WebSessions::load(&dir, "s1");
    let token = s.create(true).unwrap();
    s.close();
    assert!(!s.check(&token));
    assert!(!dir.join(FILE).exists());
    assert!(s.create(false).is_err(), "a check that ends after web access went off makes no sign-in");
    assert!(!dir.join(FILE).exists(), "and writes nothing");
    let token = WebSessions::load(&dir, "s1").create(false).unwrap();
    forget(&dir);
    forget(&dir);
    assert!(!WebSessions::load(&dir, "s1").check(&token), "the CLI switching off without a listener");
    fs::remove_dir_all(&dir).ok();
}

fn auth(dir: &Path, hold: Duration) -> Arc<WebAuth> {
    let mut a = WebAuth::new(cheap("right one"), Arc::new(WebSessions::load(dir, "73616c74")));
    a.hold = hold;
    Arc::new(a)
}

#[tokio::test]
async fn signing_in_gives_a_token_that_opens_the_rest() {
    let dir = fresh_dir("webauth-in");
    let a = auth(&dir, Duration::from_millis(10));
    assert_eq!(a.login("wrong".into(), "10.0.0.5".into(), false).await, Login::Wrong { retry: 0 });
    let Login::SignedIn(token) = a.login("right one".into(), "10.0.0.5".into(), false).await else { panic!("not signed in") };
    assert!(a.sessions().check(&token));
    a.sessions().sign_out(&token);
    assert!(!a.sessions().check(&token));
    fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_wrong_password_holds_the_next_check_for_a_second() {
    let dir = fresh_dir("webauth-hold");
    let a = auth(&dir, WRONG_HOLD);
    let t0 = Instant::now();
    let (wrong, right) = tokio::join!(a.login("guess".into(), "10.0.0.9".into(), false), async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        a.login("right one".into(), "10.0.0.5".into(), false).await
    });
    assert_eq!(wrong, Login::Wrong { retry: 0 });
    assert!(matches!(right, Login::SignedIn(_)));
    assert!(t0.elapsed() >= WRONG_HOLD, "the right one waited for the wrong one's hold");
    fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn guesses_sent_at_once_are_checked_one_by_one() {
    let dir = fresh_dir("webauth-flood");
    let a = auth(&dir, Duration::from_millis(5));
    let tries = (0..12).map(|_| {
        let a = a.clone();
        tokio::spawn(async move { a.login("guess".into(), "10.0.0.9".into(), false).await })
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
    assert!(matches!(a.login("right one".into(), "10.0.0.9".into(), false).await, Login::Locked { .. }), "even the right one waits");
    assert!(matches!(a.login("right one".into(), "10.0.0.5".into(), false).await, Login::SignedIn(_)), "another address does not");
    fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn hanging_up_mid_check_escapes_neither_the_brakes_nor_the_hold() {
    let dir = fresh_dir("webauth-hangup");
    let a = auth(&dir, Duration::from_millis(200));
    for _ in 0..FREE_ATTEMPTS {
        // the client goes before the answer: its request's future is dropped
        let _ = tokio::time::timeout(Duration::from_millis(1), a.login("guess".into(), "10.0.0.9".into(), false)).await;
    }
    let t0 = Instant::now();
    let next = a.login("right one".into(), "10.0.0.9".into(), false).await;
    assert!(matches!(next, Login::Locked { .. }), "{next:?}");
    assert!(t0.elapsed() >= Duration::from_millis(150), "it waited for the dropped checks' holds");
    fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
async fn a_closed_server_signs_no_one_in() {
    let dir = fresh_dir("webauth-closed");
    let a = auth(&dir, Duration::from_millis(1));
    a.sessions().close();
    assert!(matches!(a.login("right one".into(), "10.0.0.5".into(), false).await, Login::Failed(_)));
    fs::remove_dir_all(&dir).ok();
}
