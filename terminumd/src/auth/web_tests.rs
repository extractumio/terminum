// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-09: web.json and the password's hash.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use super::*;
use crate::testutil::fresh_dir;

#[test]
fn the_password_is_kept_as_a_salted_hash_only() {
    let dir = fresh_dir("web-hash");
    let stored = PasswordHash::new("correct horse").unwrap();
    assert_eq!(stored.iterations, ITERATIONS);
    let cfg = WebConfig { password: Some(stored.clone()), ..WebConfig::default() };
    cfg.save(&dir).unwrap();
    let text = fs::read_to_string(dir.join(FILE)).unwrap();
    assert!(!text.contains("correct horse"));
    assert_eq!(fs::metadata(dir.join(FILE)).unwrap().permissions().mode() & 0o777, 0o600);
    let loaded = WebConfig::load(&dir).unwrap();
    assert_eq!(loaded, cfg);
    let hash = loaded.password.unwrap();
    assert!(hash.verify("correct horse"));
    assert!(!hash.verify("correct hors") && !hash.verify("") && !hash.verify("correct horse "));
    assert_ne!(stored.salt, PasswordHash::new("correct horse").unwrap().salt, "salted");
    assert_eq!(stored.salt.len(), 32, "128-bit salt");
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_hash_iterm_enhancer_wrote_verifies() {
    // hashlib.pbkdf2_hmac("sha256", b"correct horse", bytes(range(16)), 200000).hex(), as config.py stores it
    let python: PasswordHash = serde_json::from_str(
        r#"{"salt": "000102030405060708090a0b0c0d0e0f", "iterations": 200000,
            "hash": "5201b39cf5575440e5063f6e00d88ad09423ed67a885b6161f1693c88b4fd429"}"#).unwrap();
    assert!(python.verify("correct horse"));
    assert!(!python.verify("correct horsf"));
    assert_eq!(PasswordHash::with_salt("correct horse", &(0..16).collect::<Vec<u8>>(), ITERATIONS), python);
}

#[test]
fn short_passwords_are_refused() {
    assert!(PasswordHash::new("short").unwrap_err().contains("at least 8 characters"));
    assert!(PasswordHash::new("1234567").is_err());
    assert!(PasswordHash::new("äöüäöüä").is_err(), "7 characters, 14 bytes");
    assert!(PasswordHash::with_salt("äöüäöüäö", b"s", 1).verify("äöüäöüäö"));
    assert!(PasswordHash::new("12345678").is_ok());
}

#[test]
fn a_broken_hash_matches_nothing_and_says_why() {
    let good = PasswordHash::with_salt("password1", b"salt", 2);
    assert!(good.verify("password1") && good.check().is_ok());
    for broken in [
        PasswordHash { salt: "zz".into(), ..good.clone() },
        PasswordHash { salt: String::new(), ..good.clone() },
        PasswordHash { hash: "abc".into(), ..good.clone() },
        PasswordHash { hash: String::new(), ..good.clone() },
        PasswordHash { iterations: 0, ..good.clone() },
        PasswordHash { iterations: u32::MAX, ..good.clone() },
    ] {
        assert!(broken.check().is_err(), "{broken:?}");
        assert!(!broken.verify("password1"), "{broken:?}");
    }
}

#[test]
fn defaults_fill_what_is_not_set_old_keys_go_and_other_keys_are_kept() {
    let dir = fresh_dir("web-defaults");
    let cfg = WebConfig::load(&dir).unwrap();
    assert!(cfg.allow_hosts.is_empty() && cfg.password.is_none());
    // 0.5.0's second listener: enabled, host, port and history go
    fs::write(dir.join(FILE), r#"{"enabled": true, "host": "0.0.0.0", "port": 9000, "history": 5, "password": null, "note": {"x": 1}}"#).unwrap();
    let cfg = WebConfig::load(&dir).unwrap();
    assert_eq!(cfg.other.keys().collect::<Vec<_>>(), ["note"]);
    cfg.save(&dir).unwrap();
    let raw: serde_json::Value = serde_json::from_str(&fs::read_to_string(dir.join(FILE)).unwrap()).unwrap();
    assert_eq!(raw["note"], serde_json::json!({"x": 1}));
    assert_eq!(raw["allow_hosts"], serde_json::json!([]));
    assert!(raw.get("port").is_none() && raw.get("enabled").is_none());
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn a_file_that_cannot_be_read_signs_no_one_in_and_says_why() {
    let dir = fresh_dir("web-broken");
    for raw in ["{not json", "[]", r#"{"allow_hosts": "x"}"#, r#"{"password": "plain"}"#] {
        fs::write(dir.join(FILE), raw).unwrap();
        let why = WebConfig::load(&dir).unwrap_err();
        assert!(why.contains("web.json"), "{raw}: {why}");
        assert_eq!(fs::read_to_string(dir.join(FILE)).unwrap(), raw, "left for the user to fix");
    }
    fs::remove_dir_all(&dir).ok();
}

#[test]
fn browsers_sign_in_only_with_a_usable_password() {
    assert_eq!(WebConfig::default().usable_password(), Ok(None));
    let broken = WebConfig { password: Some(PasswordHash { salt: "x".into(), iterations: 1, hash: "00".into() }), ..WebConfig::default() };
    assert!(broken.usable_password().unwrap_err().contains("broken password; set it again: terminum password"));
    let set = PasswordHash::with_salt("password1", b"s", 1);
    let on = WebConfig { password: Some(set.clone()), ..WebConfig::default() };
    assert_eq!(on.usable_password(), Ok(Some(set)));
}
