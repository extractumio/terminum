// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A press from a wait goes only while its wait stands (terminum-schedules.md AC-162): a newer run of its schedule that
//! called the wait off between the walk's read and the press never lets the older one start too.
use super::*;
use crate::tasks::ops::NewTask;
use crate::tasks::store::Store;

#[test]
fn ac162_a_press_from_a_wait_called_off_meanwhile_is_refused() {
    Store::open(crate::testutil::fresh_dir("api-waited").join("kanban.sqlite")).with(|tx| {
        let t = ops::create(tx, HUMAN, NewTask { title: "S_TERM-41".into(), ..Default::default() }, 1)?;
        let mut press: Press = serde_json::from_value(json!({"agent": "a", "how": "schedule", "waited": true})).unwrap();
        assert!(!press.waited, "never read from a request or a kept wait");
        press.waited = true;
        super::super::waits::add(tx, &t, "a", "default", &json!({"agent": "a"}), 1)?;
        press.still_waits(tx, &t.key)?;
        super::super::waits::call_off(tx, &t.key, "Superseded by S_TERM-42, a newer run of SCH-1, while it waited for room.")?;
        assert_eq!(press.still_waits(tx, &t.key).unwrap_err(), WAIT_GONE);
        press.waited = false;
        press.still_waits(tx, &t.key)?;   // a schedule's own new press has no wait to stand on
        Ok(())
    }).unwrap();
}
