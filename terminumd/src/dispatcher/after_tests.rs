// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! What a run that waits for other tasks hears (AC-210): fixed words with IDs only.

use super::*;

#[test]
fn ac210_the_words_going_on_are_fixed_and_name_only_ids() {
    assert_eq!(go_on("API-12", &["API-9".into()]), "[terminum Dispatcher] API-12: API-9, which it waited for, is DONE, and Dispatcher resumed this conversation. \
        Go on with API-12: bring in their work (merge the project's branch where they landed), run your checks again, then finish it as you were asked.");
    assert!(go_on("API-12", &["API-9".into(), "API-10".into()]).contains("API-9, API-10, which it waited for, are DONE"));
    let press = serde_json::to_value(Press::after(How::Drop, HUMAN, &["API-9".into()])).unwrap();
    assert_eq!((press["after"].clone(), press["queue"].clone(), press["how"].clone(), press["comment"].clone()), (serde_json::json!(["API-9"]), serde_json::json!(true), serde_json::json!("drop"), serde_json::Value::Null),
               "who started it and how stay its own; it waits for room; no one's comment rides it");
}
