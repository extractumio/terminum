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

#[test]
fn ac170_order_press_rechecks_the_selected_project_at_run_admission() {
    use crate::tasks::auto;
    Store::open(crate::testutil::fresh_dir("api-order").join("kanban.sqlite")).with(|tx| {
        let mut press: Press = serde_json::from_value(json!({"order_project": "default"})).unwrap();
        assert!(press.order_project.is_none(), "a page or kept press cannot supply an order project");
        press.order_project = Some("default".into());

        tx.execute("UPDATE projects SET dir = '/srv' WHERE id = 'default'", []).map_err(ops::sql)?;
        let before = auto::setting(tx, "default")?.with(&json!({"on": true, "mode": "order"}))?;
        auto::set(tx, "default", &before, 1)?;
        assert!(press.order_allows(tx, "default")?);
        assert!(!press.order_allows(tx, "other")?, "a pick cannot run after moving projects");

        let off = before.with(&json!({"on": false}))?;
        auto::set(tx, "default", &off, 2)?;
        assert!(!press.order_allows(tx, "default")?);
        assert!(Press::default().order_allows(tx, "default")?, "a human's start is independent of the switch");
        assert!(Press::resumed(How::Auto, crate::tasks::store::DISPATCHER).order_allows(tx, "default")?, "a recorded run can resume");
        Ok(())
    }).unwrap();
}
