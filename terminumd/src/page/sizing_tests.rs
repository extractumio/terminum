// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! sizing.rs: the grids a window gets, and who holds a window when tmux refuses (AC-13).

use super::*;

/// AC-13: a phone narrower than an iPhone in portrait, a split, or a huge display get the bounds.
#[test]
fn a_grid_stays_within_its_bounds() {
    assert_eq!(bounded(80, 24), (80, 24));
    assert_eq!(bounded(20, 5), MIN);
    assert_eq!(bounded(400, 120), MAX);
    assert_eq!(bounded(30, 90), (45, 70));
}

/// AC-13: a window of several panes grows on any claim and shrinks only for typing.
#[test]
fn a_split_window_shrinks_only_for_typing() {
    let split = Fields { panes: 2, window_size: (129, 68), ..Fields::default() };
    assert_eq!(target((46, 32), &split, Why::Open), (129, 68), "a phone that only looks takes nothing from the hidden panes");
    assert_eq!(target((46, 32), &split, Why::Active), (129, 68));
    assert_eq!(target((46, 32), &split, Why::Typed), (46, 32));
    assert_eq!(target((180, 50), &split, Why::View), (180, 68), "each direction on its own");
    let one = Fields { panes: 1, ..split };
    assert_eq!(target((46, 32), &one, Why::Open), (46, 32), "a window of one pane has nothing to squeeze");
}

/// AC-13 failure: tmux refusing a resize undoes the claim (the window is not that page's), a
/// `window-size manual` it did apply stays known, and a later claim of another page is kept.
#[test]
fn ac13_a_refused_resize_gives_the_window_back_to_who_had_it() {
    let sizes = Sizes::default();
    let key = ("local:terminum".to_string(), "@1".to_string());
    let mut g = sizes.0.lock();
    let (_, first) = Sizes::record(&mut g, &key, Win { by: Some(1), manual: true, ..Win::default() });
    let (before, second) = Sizes::record(&mut g, &key, Win { by: Some(2), manual: true, ..Win::default() });
    drop(g);
    assert_eq!(before.as_ref().map(|w| (w.by, w.claim)), Some((Some(1), first)));
    sizes.undo(&key, second, before, false);
    assert_eq!(sizes.0.lock()[&key].by, Some(1), "refused: page 1 still sizes it");

    let (before, third) = Sizes::record(&mut sizes.0.lock(), &key, Win { by: Some(3), ..Win::default() });
    let (_, fourth) = Sizes::record(&mut sizes.0.lock(), &key, Win { by: Some(4), manual: true, ..Win::default() });
    sizes.undo(&key, third, before, false);
    assert_eq!(sizes.0.lock()[&key].claim, fourth, "a later claim meanwhile is not undone");

    let fresh = ("local:terminum".to_string(), "@2".to_string());
    let (before, claim) = Sizes::record(&mut sizes.0.lock(), &fresh, Win { by: Some(5), manual: true, ..Win::default() });
    sizes.undo(&fresh, claim, before, true);
    assert_eq!(sizes.0.lock()[&fresh], Win { manual: true, claim: 0, ..Win::default() }, "nobody's, but its manual size is given back later");
    let (before, claim) = Sizes::record(&mut sizes.0.lock(), &fresh, Win { by: Some(5), manual: true, ..Win::default() });
    sizes.0.lock().remove(&fresh);
    Sizes::record(&mut sizes.0.lock(), &fresh, Win { by: Some(6), ..Win::default() });
    sizes.undo(&fresh, claim, before, false);
    assert_eq!(sizes.0.lock()[&fresh].by, Some(6));
}
