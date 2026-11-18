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
    assert_eq!(target((46, 32), &split, split.window_size, Why::Open), (129, 68), "a phone that only looks takes nothing from the hidden panes");
    assert_eq!(target((46, 32), &split, split.window_size, Why::Active), (129, 68));
    assert_eq!(target((46, 32), &split, split.window_size, Why::Typed), (46, 32));
    assert_eq!(target((180, 50), &split, split.window_size, Why::View), (180, 68), "each direction on its own");
    let ends = |asked| Win { manual: true, asked: Some(asked), ..Win::default() }.ends_at(split.window_size);
    assert_eq!(target((87, 32), &split, ends((140, 60)), Why::Open), (140, 60),
        "a grow to 140×60 still on its way: an opening page's look never shrinks the window back to the fields' 129×68");
    assert_eq!(target((87, 32), &split, ends((46, 32)), Why::Open), (87, 32),
        "a typer's shrink to 46×32 still on its way: an opening page grows the window from there, not from the fields' 129×68");
    let one = Fields { panes: 1, ..split };
    assert_eq!(target((46, 32), &one, one.window_size, Why::Open), (46, 32), "a window of one pane has nothing to squeeze");
}

/// AC-13: a coding agent on the normal screen prints its transcript again on each new size and
/// leaves copies in the history: a page that only looks leaves its window as it is.
#[test]
fn ac13_a_looking_page_leaves_an_agent_s_window_as_it_is() {
    let normal = Fields::default();
    for why in [Why::Open, Why::Active, Why::View] {
        assert!(leaves(why, &normal, true), "a page that only looks: {why:?}");
        assert!(!leaves(why, &normal, false), "a shell re-wraps with tmux, sized as before: {why:?}");
        assert!(!leaves(why, &Fields { alt: true, ..Fields::default() }, true), "the alternate screen has no history to copy into: {why:?}");
    }
    assert!(!leaves(Why::Typed, &normal, true), "typing gives the window the page's grid");
}

/// AC-13: the page that sized a window changed its view while the window was left as it is: the
/// window is no longer at that page's grid, so its next key claims it at once.
#[test]
fn ac13_a_window_is_a_page_s_only_at_its_view() {
    let sized = Win { by: Some(1), manual: true, asked: Some((148, 58)), ..Win::default() };
    assert!(sized.is_of(1, Some((148, 58)), (148, 58)), "its view is the window's grid");
    assert!(!sized.is_of(1, Some((120, 58)), (148, 58)), "docked Files took 28 columns of its view");
    assert!(!sized.is_of(2, Some((148, 58)), (148, 58)), "another page's, whatever its view");
    assert!(!Win::default().is_of(1, Some((148, 58)), (148, 58)), "nobody's");
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

/// AC-13: a window is at a page's grid only when tmux's last-read size and the grid terminumd last
/// asked for agree (the quick shell closed while the resize to its narrower view was on its way).
#[test]
fn ac13_a_resize_on_its_way_is_not_the_window_s_size() {
    let sized = Win { manual: true, asked: Some((87, 30)), ..Win::default() };
    assert!(sized.holds((87, 30), (87, 30)), "settled at the page's grid: nothing to send");
    let on_its_way = Win { asked: Some((56, 30)), ..sized.clone() };
    assert!(!on_its_way.holds((87, 30), (87, 30)), "the fields say 87, but 56 is on its way: 87 goes again");
    assert!(!sized.holds((56, 30), (87, 30)), "tmux reads another size: sized again");
    assert!(!Win::default().holds((87, 30), (87, 30)), "never sized by terminumd");
}
