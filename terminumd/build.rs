// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! macOS builds carry macos/Info.plist as their __TEXT,__info_plist section: the bundle
//! identifier macOS keeps folder grants for and the reasons it shows (AC-08). Other targets
//! get nothing.

fn main() {
    println!("cargo:rerun-if-changed=macos/Info.plist");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
        println!("cargo:rustc-link-arg-bins=-Wl,-sectcreate,__TEXT,__info_plist,{dir}/macos/Info.plist");
    }
}
