// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Test-only clocks are ignored outside a private app (AC-198).

#[test]
fn ac198_test_clock_requires_a_private_app_and_positive_seconds() {
    const EXPECTED: &str = "TM_TEST_EXPECTED_BEAT";
    const BEAT: &str = "TM_TEST_INTEGRATION_BEAT";
    if let Ok(expected) = std::env::var(EXPECTED) {
        assert_eq!(super::of_test(BEAT), expected.parse::<i64>().ok());
        return;
    }
    let root = crate::testutil::temp_root();
    for (app, value, expected) in [
        (false, Some("1"), "none"),
        (true, None, "none"),
        (true, Some("invalid"), "none"),
        (true, Some("0"), "none"),
        (true, Some("-1"), "none"),
        (true, Some("9223372036854775808"), "none"),
        (true, Some("1"), "1"),
        (true, Some("10"), "10"),
    ] {
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", "dispatcher::timing_tests::ac198_test_clock_requires_a_private_app_and_positive_seconds"])
            .env(EXPECTED, expected).env_remove("TM_APP_DIR").env_remove(BEAT);
        if app {
            command.env("TM_APP_DIR", root);
        }
        if let Some(value) = value {
            command.env(BEAT, value);
        }
        let output = command.output().unwrap();
        assert!(output.status.success(), "app={app}, value={value:?}: {}{}",
            String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        assert!(String::from_utf8_lossy(&output.stdout).contains("test result: ok. 1 passed;"),
            "the child must run its clock assertion: {}", String::from_utf8_lossy(&output.stdout));
    }
}
