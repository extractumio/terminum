// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Display sorts before pagination, stable ties, and the saved order for non-board callers (AC-105).
use super::*;
use crate::tasks::{ops::{create, NewTask}, store::{Store, HUMAN}};

#[test]
fn ac105_display_sort_precedes_pagination_and_keeps_stable_ties() {
    let s = Store::open(crate::testutil::fresh_dir("column-sort").join("kanban.sqlite"));
    s.with(|tx| {
        let urgent = create(tx, HUMAN, NewTask { title: "urgent".into(), severity: Some("P0".into()), ..Default::default() }, 1)?;
        let mut made = Vec::new();
        for n in 0..205 {
            made.push(create(tx, HUMAN, NewTask { title: format!("task {n}"), ..Default::default() }, 100 + n / 2)?.id);
        }
        let ids = |sort, after| list(tx, &Query { sort, after, ..Default::default() }).map(|l| l.into_iter().map(|t| t.id).collect::<Vec<_>>());
        let recent = ids(Sort::Recent, 0)?;
        assert_eq!(recent.len(), 200);
        assert_eq!(recent[0], made[204]);
        assert_eq!(recent[1], made[203], "newest number first at the same second");
        let rest = ids(Sort::Recent, 200)?;
        assert_eq!(rest.last(), Some(&urgent.id));
        let all: Vec<_> = recent.into_iter().chain(rest).collect();
        assert_eq!(all.len(), 206);
        assert_eq!(all.iter().collect::<std::collections::HashSet<_>>().len(), 206);
        let priority = ids(Sort::Priority, 0)?;
        assert_eq!(priority[0], urgent.id);
        assert_eq!(priority[1], made[204]);
        assert_eq!(ids(Sort::Ranked, 0)?[0], urgent.id);
        tx.execute("UPDATE tasks SET updated_at = 999 WHERE slug = ?1", [&made[0]]).map_err(sql)?;
        assert_eq!(ids(Sort::Recent, 0)?[0], made[0], "an update moves the task");
        assert_eq!(ids(Sort::Priority, 0)?[1], made[0]);
        tx.execute("UPDATE tasks SET archived_at = number", []).map_err(sql)?;
        let archived = list(tx, &Query { sort: Sort::Recent, archived: true, ..Default::default() })?;
        assert_eq!(archived[0].id, made[204], "archive order stays its own");
        Ok(())
    }).unwrap();
}

#[test]
fn ac169_queued_human_waits_are_separate_from_filtered_pagination_totals() {
    let s = Store::open(crate::testutil::fresh_dir("queued-counts").join("kanban.sqlite"));
    s.with(|tx| {
        let first = create(tx, HUMAN, NewTask { title: "first".into(), ..Default::default() }, 1)?;
        let second = create(tx, HUMAN, NewTask { title: "second".into(), severity: Some("P0".into()), ..Default::default() }, 1)?;
        for id in [&first.id, &second.id] { super::super::ops::move_to(tx, HUMAN, id, "WAIT FOR HUMAN", 2)?; }
        crate::dispatcher::waits::add(tx, &first, "not-in-pool", "default", &json!({"also": [second.id]}), 3)?;
        let q = Query { statuses: vec!["WAIT FOR HUMAN".into()], limit: 1, ..Default::default() };
        assert_eq!((counts(tx, &q)?["WAIT FOR HUMAN"].clone(), counts(tx, &q)?["queued_waiting"].clone()), (json!(2), json!(2)));
        assert_eq!(list(tx, &Query { after: 1, ..q.clone() })?.len(), 1, "all queued cards stay reachable");
        let filtered = counts(tx, &Query { severities: vec!["P0".into()], ..q.clone() })?;
        assert_eq!((filtered["WAIT FOR HUMAN"].clone(), filtered["queued_waiting"].clone()), (json!(1), json!(1)));
        crate::dispatcher::waits::remove(tx, &first.key)?;
        assert_eq!(counts(tx, &q)?["queued_waiting"], 0, "cancellation restores human attention");
        Ok(())
    }).unwrap();
}

#[test]
fn ac105_unknown_sort_is_refused() {
    assert!(Sort::parse(None).is_ok());
    assert!(Sort::parse(Some("recent")).is_ok());
    assert!(Sort::parse(Some("priority")).is_ok());
    assert_eq!(Sort::parse(Some("rank DESC")).err().as_deref(), Some("Sort: one of recent, priority"));
}
