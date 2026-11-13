// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Archived tasks stay readable; an agent cannot move or finish them (AC-115, TERM-72).

use super::*;

#[test]
fn ac115_archived_tasks_are_read_and_commented_but_never_moved_or_finished() {
    let (s, dir) = fresh("tt-archive");
    let w = who(Some("api"));
    for session in [Some(KEY), Some(OTHER), None] {
        let id = task(&s, "api", session);
        s.with(|tx| {
            ops::move_to(tx, HUMAN, &id, "FAILED", 2)?;
            ops::archive(tx, HUMAN, &id, true, 3)
        }).unwrap();
        let before = s.with(|tx| Ok((ops::get(tx, &id)?, ops::history(tx, &id)?))).unwrap();
        let refused = format!("{id} is archived: a human brings it back from the Archived list.");
        for to in ops::COLUMNS {
            assert_eq!(call(&s, &dir, &w, "move_task", json!({"id": id, "to": to})).unwrap_err(), refused);
        }
        assert_eq!(call(&s, &dir, &w, "finish_task", json!({"id": id, "result": "fixed"})).unwrap_err(), refused);
        let after = s.with(|tx| Ok((ops::get(tx, &id)?, ops::history(tx, &id)?))).unwrap();
        assert_eq!(after, before, "refused calls keep every field and add no result comment");
        assert!(call(&s, &dir, &w, "get_task", json!({"id": id})).is_ok());
        call(&s, &dir, &w, "add_comment", json!({"id": id, "text": "Please bring this back"})).unwrap();
        assert!(s.with(|tx| ops::history(tx, &id)).unwrap().as_array().unwrap().iter().any(|e| e["text"] == "Please bring this back"));
        assert!(s.with(|tx| ops::get(tx, &id)).unwrap().archived_at.is_some());
        s.with(|tx| ops::archive(tx, HUMAN, &id, false, 4)).unwrap();
        call(&s, &dir, &w, "move_task", json!({"id": id, "to": "IN PROGRESS"})).unwrap();
        call(&s, &dir, &w, "finish_task", json!({"id": id, "result": "fixed after restoration"})).unwrap();
        assert_eq!(status(&s, &id), "DONE");
    }
    let far = task(&s, "web", None);
    s.with(|tx| ops::archive(tx, HUMAN, &far, true, 3)).unwrap();
    assert_eq!(call(&s, &dir, &w, "finish_task", json!({"id": far, "result": "x"})).unwrap_err(), format!("{far} is outside this session's project."), "scope is checked before archive state");
}
