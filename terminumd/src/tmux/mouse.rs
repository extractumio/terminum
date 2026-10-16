// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Mouse reports for a program that asked for mouse events (AC-06; spec §9 "The control
//! client", Typing: `send-keys -H` only for reports built here). SGR (`ESC [ < b ; x ; y M`,
//! `m` on release) when the pane set `#{mouse_sgr_flag}`, else X10 (`ESC [ M` and three bytes
//! of 32 + value). Cells come 0-based from the page and go 1-based, as a terminal reports them.

/// The button of an event; `None` is motion with no button held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Right,
    None,
    WheelUp,
    WheelDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Press,
    Release,
    Move,
}

/// Modifier keys held during the event.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub meta: bool,
    pub ctrl: bool,
}

/// The largest 1-based coordinate X10 can carry (32 + 223 = 255).
pub const X10_MAX: u16 = 223;

/// The page's `mouse` message (`button` 0 left, 1 middle, 2 right; `action` press, release,
/// move, wheelup, wheeldown) as a button and an action; `None` for anything else.
pub fn from_page(button: u8, action: &str) -> Option<(Button, Action)> {
    let b = match button {
        0 => Button::Left,
        1 => Button::Middle,
        2 => Button::Right,
        3 => Button::None,
        _ => return None,
    };
    Some(match action {
        "press" => (b, Action::Press),
        "release" => (b, Action::Release),
        "move" => (b, Action::Move),
        "wheelup" => (Button::WheelUp, Action::Press),
        "wheeldown" => (Button::WheelDown, Action::Press),
        _ => return None,
    })
}

/// The report of one event at cell (`x`, `y`) (0-based), or `None` when there is none to send:
/// a wheel only presses, no button is pressed or released, X10 cannot reach the cell.
pub fn report(x: u16, y: u16, button: Button, action: Action, mods: Mods, sgr: bool) -> Option<Vec<u8>> {
    let code: u16 = match (button, action) {
        (Button::WheelUp | Button::WheelDown, Action::Release | Action::Move) => return None,
        (Button::None, Action::Press | Action::Release) => return None,
        (Button::Left, _) => 0,
        (Button::Middle, _) => 1,
        (Button::Right, _) => 2,
        (Button::None, _) => 3,
        (Button::WheelUp, _) => 64,
        (Button::WheelDown, _) => 65,
    };
    let code = code
        + if action == Action::Move { 32 } else { 0 }
        + if mods.shift { 4 } else { 0 }
        + if mods.meta { 8 } else { 0 }
        + if mods.ctrl { 16 } else { 0 };
    let (cx, cy) = (x.checked_add(1)?, y.checked_add(1)?);
    if sgr {
        let end = if action == Action::Release { 'm' } else { 'M' };
        return Some(format!("\x1b[<{code};{cx};{cy}{end}").into_bytes());
    }
    if cx > X10_MAX || cy > X10_MAX {
        return None;
    }
    // X10 has no button in a release: 3, the modifiers kept
    let code = if action == Action::Release { (code & !3) | 3 } else { code };
    Some(vec![0x1b, b'[', b'M', (32 + code) as u8, (32 + cx) as u8, (32 + cy) as u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    const NO: Mods = Mods { shift: false, meta: false, ctrl: false };

    fn sgr(x: u16, y: u16, b: Button, a: Action) -> String {
        String::from_utf8(report(x, y, b, a, NO, true).unwrap()).unwrap()
    }

    #[test]
    fn click() {
        assert_eq!(sgr(10, 5, Button::Left, Action::Press), "\x1b[<0;11;6M", "AC-06: cell (10, 5)");
        assert_eq!(sgr(10, 5, Button::Left, Action::Release), "\x1b[<0;11;6m");
        assert_eq!(sgr(0, 0, Button::Middle, Action::Press), "\x1b[<1;1;1M");
    }

    #[test]
    fn right_click() {
        assert_eq!(sgr(3, 4, Button::Right, Action::Press), "\x1b[<2;4;5M");
        assert_eq!(sgr(3, 4, Button::Right, Action::Release), "\x1b[<2;4;5m");
    }

    #[test]
    fn drag() {
        let events = [(10, Action::Press), (20, Action::Move), (30, Action::Move), (30, Action::Release)];
        let got: Vec<String> = events.iter().map(|&(x, a)| sgr(x, 5, Button::Left, a)).collect();
        assert_eq!(got, ["\x1b[<0;11;6M", "\x1b[<32;21;6M", "\x1b[<32;31;6M", "\x1b[<0;31;6m"]);
        assert_eq!(sgr(7, 2, Button::None, Action::Move), "\x1b[<35;8;3M", "motion, no button");
    }

    #[test]
    fn wheel() {
        assert_eq!(sgr(10, 5, Button::WheelUp, Action::Press), "\x1b[<64;11;6M");
        assert_eq!(sgr(10, 5, Button::WheelDown, Action::Press), "\x1b[<65;11;6M");
        assert_eq!(report(1, 1, Button::WheelUp, Action::Release, NO, true), None, "a wheel has no release");
    }

    #[test]
    fn modifiers() {
        let m = Mods { shift: true, meta: true, ctrl: true };
        assert_eq!(report(0, 0, Button::Left, Action::Press, m, true).unwrap(), b"\x1b[<28;1;1M");
        let ctrl = Mods { ctrl: true, ..NO };
        assert_eq!(report(0, 0, Button::WheelDown, Action::Press, ctrl, true).unwrap(), b"\x1b[<81;1;1M");
    }

    #[test]
    fn x10() {
        assert_eq!(report(10, 5, Button::Left, Action::Press, NO, false).unwrap(), [0x1b, b'[', b'M', 32, 43, 38]);
        assert_eq!(report(10, 5, Button::Left, Action::Release, NO, false).unwrap(), [0x1b, b'[', b'M', 35, 43, 38]);
        assert_eq!(report(10, 5, Button::Right, Action::Move, NO, false).unwrap(), [0x1b, b'[', b'M', 66, 43, 38]);
        assert_eq!(report(10, 5, Button::WheelUp, Action::Press, NO, false).unwrap(), [0x1b, b'[', b'M', 96, 43, 38]);
        let shift = Mods { shift: true, ..NO };
        assert_eq!(report(0, 0, Button::Right, Action::Release, shift, false).unwrap()[3], 32 + 7);
        assert_eq!(report(222, 222, Button::Left, Action::Press, NO, false).unwrap()[4..], [255, 255]);
        assert_eq!(report(223, 0, Button::Left, Action::Press, NO, false), None, "beyond X10");
        assert!(report(223, 0, Button::Left, Action::Press, NO, true).is_some(), "SGR reaches it");
        assert_eq!(report(u16::MAX, 0, Button::Left, Action::Press, NO, true), None);
    }

    #[test]
    fn nothing_to_report() {
        assert_eq!(report(0, 0, Button::None, Action::Press, NO, true), None);
        assert_eq!(report(0, 0, Button::None, Action::Release, NO, false), None);
    }

    #[test]
    fn page_messages() {
        assert_eq!(from_page(0, "press"), Some((Button::Left, Action::Press)));
        assert_eq!(from_page(2, "release"), Some((Button::Right, Action::Release)));
        assert_eq!(from_page(0, "wheelup"), Some((Button::WheelUp, Action::Press)));
        assert_eq!(from_page(0, "wheeldown"), Some((Button::WheelDown, Action::Press)));
        assert_eq!(from_page(9, "press"), None);
        assert_eq!(from_page(0, "tap"), None);
    }
}
