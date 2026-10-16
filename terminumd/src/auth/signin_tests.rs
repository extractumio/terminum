// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-09: codes, the sign-ins' file and the hello proof.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use super::*;
use crate::testutil::fresh_dir;

const SECRET: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const NONCE: &str = "00112233445566778899aabbccddeeff";

#[test]
fn a_code_opens_once_and_only_for_60_seconds() {
    let codes = Codes::new();
    let t0 = Instant::now();
    let code = codes.issue_at(t0);
    assert_eq!(code.len(), 32, "128 random bits");
    assert_ne!(code, codes.issue_at(t0));
    assert_eq!(codes.redeem_at(&code, t0 + Duration::from_secs(59)), Redeem::Ok);
    assert_eq!(codes.redeem_at(&code, t0 + Duration::from_secs(59)), Redeem::Spent, "spent on first use");
    let late = codes.issue_at(t0);
    assert_eq!(codes.redeem_at(&late, t0 + CODE_TTL), Redeem::Spent, "expired");
    assert_eq!(codes.redeem("never-issued"), Redeem::Spent);
    assert_eq!(codes.redeem(""), Redeem::Spent);
    let page = crate::http::code_spent_page();
    assert_eq!(page.status(), 410, "the page that answers a spent code");
}

#[test]
fn expired_codes_do_not_pile_up() {
    let codes = Codes::new();
    let t0 = Instant::now();
    for _ in 0..50 {
        codes.issue_at(t0);
    }
    codes.issue_at(t0 + CODE_TTL + Duration::from_secs(1));
    assert_eq!(codes.live.lock().len(), 1);
}

#[test]
fn a_sign_in_outlives_a_restart_and_the_file_holds_no_secret() {
    let dir = fresh_dir("signin-restart");
    let secret = Signins::load(&dir).create("terminum open").unwrap();
    assert_eq!(secret.len(), 64, "256 random bits");
    let again = Signins::load(&dir);
    assert!(again.check(&secret), "a restarted terminumd knows it");
    assert!(!again.check(&secret[..63]) && !again.check("") && !again.check(&format!("{secret}0")));
    let path = dir.join(FILE);
    assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    let raw = fs::read_to_string(&path).unwrap();
    assert!(!raw.contains(&secret));
    let stored: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let item = &stored["items"][0];
    assert_eq!(item["sha256"], sha256_hex(secret.as_bytes()));
    assert_eq!(item["by"], "terminum open");
    for field in ["sha256", "id", "proof_key"] {
        assert!(!again.check(item[field].as_str().unwrap()), "{field} from the file is not a sign-in");
    }
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn at_most_20_sign_ins_the_oldest_dropped() {
    let dir = fresh_dir("signin-max");
    let s = Signins::load(&dir);
    let now = store::unix_now();
    let first = s.create_at("a", now - 100).unwrap();
    let second = s.create_at("b", now - 99).unwrap();
    let rest: Vec<String> = (0..MAX_SIGNINS - 1).map(|_| s.create_at("c", now).unwrap()).collect();
    assert!(!s.check(&first), "the oldest went");
    assert!(s.check(&second));
    assert!(rest.iter().all(|r| s.check(r)), "made in the same second, none of the new ones is dropped");
    assert_eq!(s.count(), MAX_SIGNINS);
    assert_eq!(Signins::load(&dir).count(), MAX_SIGNINS);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_sign_in_lasts_30_days() {
    let dir = fresh_dir("signin-expiry");
    let s = Signins::load(&dir);
    let now = store::unix_now();
    let secret = s.create_at("x", now - SIGNIN_TTL_S - 1).unwrap();
    assert!(!s.check(&secret));
    assert!(s.check_at(&secret, now - SIGNIN_TTL_S));
    assert_eq!(Signins::load(&dir).count(), 0, "expired ones are not loaded");
    let id = signin_id(&secret);
    assert_eq!(s.hello(&id, NONCE).unwrap_err().code, "unknown_signin", "nor proved");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn signout_revokes_every_sign_in_also_after_a_restart() {
    let dir = fresh_dir("signin-revoke");
    let s = Signins::load(&dir);
    let a = s.create("a").unwrap();
    let b = s.create("b").unwrap();
    s.revoke_all().unwrap();
    assert!(!s.check(&a) && !s.check(&b));
    let again = Signins::load(&dir);
    assert!(!again.check(&a) && again.count() == 0);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_broken_file_is_set_aside_and_terminumd_starts_without_sign_ins() {
    let dir = fresh_dir("signin-broken");
    let path = dir.join(FILE);
    for raw in ["{not json", "[]", r#"{"items": "x"}"#, r#"{"items": [{"sha256": 1}]}"#] {
        fs::write(&path, raw).unwrap();
        let s = Signins::load(&dir);
        assert_eq!(s.count(), 0, "{raw}");
        assert!(!path.exists(), "{raw}");
        assert_eq!(fs::read_to_string(store::backup_path(&path)).unwrap(), raw);
        let secret = s.create("again").unwrap();
        assert!(Signins::load(&dir).check(&secret), "{raw}: works again");
        fs::remove_file(&path).unwrap();
    }
    // a well-formed file with a malformed item keeps the others
    let good = Signins::load(&dir);
    let secret = good.create("ok").unwrap();
    let mut v: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    v["items"].as_array_mut().unwrap().push(json!({"sha256": "zz", "id": "x", "proof_key": "y", "expires": u64::MAX, "by": ""}));
    fs::write(&path, v.to_string()).unwrap();
    let s = Signins::load(&dir);
    assert!(s.check(&secret) && s.count() == 1);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_file_that_cannot_be_written_makes_no_sign_in() {
    let dir = fresh_dir("signin-nowrite");
    let s = Signins::load(&dir.join("gone"));
    assert!(s.create("x").is_err());
    assert_eq!(s.count(), 0);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_page_and_terminumd_compute_the_same_proof() {
    // the vector the page's unit test checks too (computed independently with Python's hmac)
    // secret "00" * 32, n "00112233445566778899aabbccddeeff"
    assert_eq!(signin_id(SECRET), "60e05bd1b195af2f");
    assert_eq!(proof_key(SECRET), "87f869e25d9e150262e60375e2c14ad979145fcf5ea2bcb4654216751eb6ff3b");
    assert_eq!(proof(&proof_key(SECRET), NONCE), "d549a28d55f5e63634ad7cf712493e602f42bc73ce3cb0cbb2e40909de520185");
    let other = "0123456789abcdef".repeat(4);
    assert_eq!(proof(&proof_key(&other), NONCE), "5a1fcc67e110f68066c1d7a6193629f707e36108f8a3e2b237cfcd83b7cdc146");
    let dir = fresh_dir("signin-hello");
    let s = Signins::load(&dir);
    let secret = s.create("x").unwrap();
    let n = random_hex::<16>();
    assert_eq!(s.hello(&signin_id(&secret), &n).unwrap(), proof(&proof_key(&secret), &n));
    assert_eq!(s.hello(&signin_id(&secret).to_uppercase(), &n).unwrap(), proof(&proof_key(&secret), &n));
    assert_ne!(s.hello(&signin_id(&secret), &random_hex::<16>()).unwrap(), proof(&proof_key(&secret), &n), "a fresh nonce, a fresh answer");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_server_without_the_sign_in_cannot_answer() {
    let dir = fresh_dir("signin-other");
    let other = Signins::load(&dir);
    other.create("someone else").unwrap();
    let mine = random_hex::<32>();
    let d = other.hello(&signin_id(&mine), NONCE).unwrap_err();
    assert_eq!((d.status, d.code), (404, "unknown_signin"));
    // what it could compute without the proof key: the id or the stored SHA-256 as a key
    let want = proof(&proof_key(&mine), NONCE);
    assert_ne!(proof(&signin_id(&mine), NONCE), want);
    assert_ne!(proof(&sha256_hex(mine.as_bytes()), NONCE), want);
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn the_hello_wants_a_hex_nonce_and_id() {
    let dir = fresh_dir("signin-nonce");
    let s = Signins::load(&dir);
    let id = signin_id(&s.create("x").unwrap());
    for bad in ["", "abc", &"a".repeat(15), &"a".repeat(129), "zzzzzzzzzzzzzzzzzz", "00112233445566778899aabbccddeef "] {
        assert_eq!(s.hello(&id, bad).unwrap_err().code, "bad_nonce", "{bad:?}");
    }
    assert!(s.hello(&id, &"a".repeat(16)).is_ok() && s.hello(&id, &"F".repeat(128)).is_ok());
    for bad in ["", "abc", "a8ae6e6ee929abeaff", "a8ae6e6ee929abeg"] {
        assert_eq!(s.hello(bad, NONCE).unwrap_err().code, "bad_id", "{bad:?}");
    }
    fs::remove_dir_all(&dir).ok();
}
