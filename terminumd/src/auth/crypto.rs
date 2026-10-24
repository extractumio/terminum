// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The hashes behind sign-ins (AC-09, AC-10): SHA-256, HMAC-SHA256 (RFC 2104), PBKDF2-HMAC-SHA256
//! (RFC 8018) over it for the web password, random hex, and comparisons whose time does not
//! depend on where two values differ. `sha2` only: no crate of its own for HMAC or PBKDF2.

use sha2::{Digest, Sha256};

const BLOCK: usize = 64;

pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex(&sha256(data))
}

/// Lowercase hex of `bytes`.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Bytes of a hex string (either case); None when it is not hex of whole bytes.
pub fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

/// `N` random bytes as lowercase hex (from the OS's generator through `rand`'s ThreadRng).
pub fn random_hex<const N: usize>() -> String {
    hex(&rand::random::<[u8; N]>())
}

/// Equal strings, compared in a time that does not depend on where they differ.
pub fn same(a: &str, b: &str) -> bool {
    same_bytes(a.as_bytes(), b.as_bytes())
}

pub fn same_bytes(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// HMAC-SHA256 with the key's inner and outer pads absorbed once, so PBKDF2's rounds hash only
/// their own 32 bytes.
#[derive(Clone)]
struct Hmac {
    inner: Sha256,
    outer: Sha256,
}

impl Hmac {
    fn new(key: &[u8]) -> Hmac {
        let mut k = [0u8; BLOCK];
        if key.len() > BLOCK {
            k[..32].copy_from_slice(&sha256(key));
        } else {
            k[..key.len()].copy_from_slice(key);
        }
        let mut inner = Sha256::new();
        inner.update(k.map(|b| b ^ 0x36));
        let mut outer = Sha256::new();
        outer.update(k.map(|b| b ^ 0x5c));
        Hmac { inner, outer }
    }

    fn mac(&self, parts: &[&[u8]]) -> [u8; 32] {
        let mut inner = self.inner.clone();
        for p in parts {
            inner.update(p);
        }
        let mut outer = self.outer.clone();
        outer.update(inner.finalize());
        outer.finalize().into()
    }
}

/// SHA-256's first state.
const IV: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];

/// PBKDF2's rounds after the first, each the HMAC of the last 32 bytes: one compression from the
/// key's inner pad, one from its outer, both absorbed once. On sha2's compression function, which
/// Cargo.toml compiles optimized in a debug build too; the generic hasher is compiled with this
/// crate, and 600,000 rounds of it took 3 s unoptimized (the tests sign in often).
struct Rounds {
    inner: [u32; 8],
    outer: [u32; 8],
}

impl Rounds {
    fn new(key: &[u8]) -> Rounds {
        let mut k = [0u8; BLOCK];
        if key.len() > BLOCK {
            k[..32].copy_from_slice(&sha256(key));
        } else {
            k[..key.len()].copy_from_slice(key);
        }
        let absorbed = |pad: u8| {
            let mut s = IV;
            sha2::block_api::compress256(&mut s, &[k.map(|b| b ^ pad)]);
            s
        };
        Rounds { inner: absorbed(0x36), outer: absorbed(0x5c) }
    }

    /// HMAC of 32 bytes: each hash's one more block holds them, its padding and the length
    /// (a pad's 64 bytes and these 32, in bits: 768).
    fn mac32(&self, msg: &[u8; 32]) -> [u8; 32] {
        let mut block = [0u8; BLOCK];
        (block[32], block[62]) = (0x80, 0x03);
        block[..32].copy_from_slice(msg);
        let mut s = self.inner;
        sha2::block_api::compress256(&mut s, &[block]);
        Self::put(&mut block, &s);
        let mut s = self.outer;
        sha2::block_api::compress256(&mut s, &[block]);
        let mut out = [0u8; 32];
        Self::put(&mut out, &s);
        out
    }

    /// A state's eight words, big-endian, at the start of `to`.
    fn put(to: &mut [u8], s: &[u32; 8]) {
        for (i, w) in s.iter().enumerate() {
            to[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
        }
    }
}

pub fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    Hmac::new(key).mac(&[msg])
}

/// PBKDF2-HMAC-SHA256 (RFC 8018 §5.2): `len` bytes from `rounds` iterations; what Python's
/// `hashlib.pbkdf2_hmac("sha256", …)` gives, so iterm-enhancer's web.json hashes verify.
pub fn pbkdf2_sha256(password: &[u8], salt: &[u8], rounds: u32, len: usize) -> Vec<u8> {
    let (prf, fast) = (Hmac::new(password), Rounds::new(password));
    let mut out = Vec::with_capacity(len);
    for block in 1..=len.div_ceil(32) as u32 {
        let mut u = prf.mac(&[salt, &block.to_be_bytes()]);
        let mut t = u;
        for _ in 1..rounds {
            u = fast.mac32(&u);
            t.iter_mut().zip(u).for_each(|(t, u)| *t ^= u);
        }
        out.extend_from_slice(&t[..(len - out.len()).min(32)]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_rfc_4231() {
        assert_eq!(hex(&hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
                   "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        assert_eq!(hex(&hmac_sha256(&[0x0b; 20], b"Hi There")),
                   "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7");
        assert_eq!(hex(&hmac_sha256(&[0xaa; 131], b"Test Using Larger Than Block-Size Key - Hash Key First")),
                   "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54");
    }

    #[test]
    fn pbkdf2_matches_rfc_7914_and_the_sha256_vectors_of_rfc_6070s_cases() {
        // RFC 7914 §11
        assert_eq!(hex(&pbkdf2_sha256(b"passwd", b"salt", 1, 64)),
                   "55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc\
                    49ca9cccf179b645991664b39d77ef317c71b845b1e30bd509112041d3a19783");
        assert_eq!(hex(&pbkdf2_sha256(b"Password", b"NaCl", 80000, 64)),
                   "4ddcd8f60b98be21830cee5ef22701f9641a4418d04c0414aeff08876b34ab56\
                    a1d425a1225833549adb841b51c9b3176a272bdebba1d078478f62b397f33c8d");
        // RFC 6070's inputs with SHA-256 (as Python's hashlib computes them)
        for (rounds, want) in [(1, "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"),
                               (2, "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43"),
                               (4096, "c5e478d59288c841aa530db6845c4c8d962893a001ce4e11a4963873aa98134a")] {
            assert_eq!(hex(&pbkdf2_sha256(b"password", b"salt", rounds, 32)), want, "{rounds} rounds");
        }
        // a length that is not a whole number of blocks
        assert_eq!(hex(&pbkdf2_sha256(b"passwordPASSWORDpassword", b"saltSALTsaltSALTsaltSALTsaltSALTsalt", 4096, 40)),
                   "348c89dbcbd32b2f32d814b8116e84cf2b17347ebc1800181c4e2a1fb8dd53e1c635518c7dac47e9");
    }

    #[test]
    fn sha256_and_hex_round_trip() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(unhex("00ff7Fa0"), Some(vec![0, 255, 127, 160]));
        assert_eq!(unhex("abc"), None, "half a byte");
        assert_eq!(unhex("zz"), None);
        assert_eq!(unhex("+1"), None, "from_str_radix would take a sign");
        let a = random_hex::<16>();
        assert_eq!(a.len(), 32);
        assert_ne!(a, random_hex::<16>());
    }

    #[test]
    fn same_compares_whole_values() {
        assert!(same("abc", "abc") && same("", ""));
        assert!(!same("abc", "abd") && !same("abc", "ab") && !same("", "a"));
        assert!(same_bytes(&[1, 2], &[1, 2]) && !same_bytes(&[1, 2], &[1, 3]));
    }
}
