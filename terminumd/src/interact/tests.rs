// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The interaction log (terminum.md AC-175): what a page may send and what is kept of it.

use super::*;
use crate::tasks::store::Store;

fn sent(at: i64, kind: &str, target: &str) -> Sent {
    Sent { at, ended: at + 5_000, kind: kind.into(), surface: "sessions".into(), target: target.into(), n: 3, amount: 0, pane: Some("tmux:local:terminum:%1".into()) }
}

#[test]
fn ac175_a_control_s_id_never_its_words() {
    assert_eq!((target("newsession"), target("m1234"), target("row-12-of-340")), ("newsession".into(), "mN".into(), "row-N-of-N".into()));
    for id in ["", "12", "a b", "pass word", "x/y", "naïve", &"a".repeat(41)] {
        assert_eq!(target(id), "", "{id:?} is no plain name");
    }
}

#[test]
fn ac175_only_known_counts_at_this_mac_s_time() {
    let now = 1_800_000_000_000;
    // the page's clock is 90 s ahead: its rows are shifted back
    let r = row(sent(now + 80_000, "click", "run7"), -90_000, now).unwrap();
    assert_eq!((r.at, r.ended, r.kind, r.target.as_str(), r.n), (now - 10_000, now - 5_000, "click", "runN", 3));
    assert_eq!(row(sent(now - 10_000, "scroll", "sessions"), 0, now).unwrap().target, "", "only a press and a drag name their control");
    assert!(row(sent(now - 10_000, "typed", ""), 0, now).is_none(), "an unknown kind");
    assert!(row(Sent { surface: "elsewhere".into(), ..sent(now - 10_000, "key", "") }, 0, now).is_none(), "an unknown place");
    assert!(row(sent(now - 2 * OLDEST, "key", ""), 0, now).is_none(), "older than a day");
    assert!(row(sent(now + 2 * MINUTE, "key", ""), 0, now).is_none(), "later than now");
    assert!(row(Sent { ended: now - 10_000 + 2 * MINUTE, ..sent(now - 10_000, "key", "") }, 0, now).is_none(), "longer than a minute");
    assert!(row(Sent { n: 0, ..sent(now - 10_000, "key", "") }, 0, now).is_none(), "nothing counted");
    assert_eq!(row(Sent { pane: Some("a\nb".into()), ..sent(now - 10_000, "key", "") }, 0, now).unwrap().pane, None, "a pane's key holds no control character");
}

#[test]
fn ac175_a_minute_takes_so_many_rows() {
    let taken = Mutex::new((0, 0));
    let now = 1_800_000_000_000;
    assert_eq!((room(&taken, now, 500), room(&taken, now + 1_000, 500), room(&taken, now + 2_000, 5)), (500, 100, 0));
    assert_eq!(room(&taken, now + MINUTE, 5), 5, "the next minute takes again");
}

#[test]
fn ac175_rows_are_kept_and_old_ones_go() {
    let s = Store::open(crate::testutil::fresh_dir("interact").join("kanban.sqlite"));
    let now = crate::inputs::now_ms();
    let rows: Vec<Row> = [now - 200 * 86_400_000, now - 3 * MINUTE, now - 2 * MINUTE, now - MINUTE].into_iter().map(|at| Row { project: Some("p1".into()), ..row(sent(at, "click", "x"), 0, at + MINUTE).unwrap() }).collect();
    let left = |tx: &Transaction| tx.query_row("SELECT count(*), min(at) FROM user_interaction", [], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))).map_err(sql);
    s.with(|tx| {
        insert(tx, &rows, "mac")?;
        trim(tx, now, KEEP_ROWS)?;
        assert_eq!(left(tx)?, (3, now - 3 * MINUTE), "older than KEEP_DAYS goes");
        trim(tx, now, 2)?;
        assert_eq!(left(tx)?, (2, now - 2 * MINUTE), "the oldest beyond the count goes");
        let kept: (String, String, String) = tx.query_row("SELECT device, project, pane FROM user_interaction LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(sql)?;
        assert_eq!(kept, ("mac".into(), "p1".into(), "tmux:local:terminum:%1".into()));
        Ok(())
    }).unwrap();
}
