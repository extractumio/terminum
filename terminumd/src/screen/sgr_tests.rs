// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Tests of the SGR parser of captures.

use super::*;

fn parse(text: &str) -> Vec<Vec<(char, Style)>> {
    parse_capture(text, &mut Style::default())
}

fn text_of(line: &[(char, Style)]) -> String {
    line.iter().map(|(c, _)| c).collect()
}

fn st(fg: Color, bg: Color, flags: u16) -> Style {
    Style { fg, bg, flags }
}

#[test]
fn plain_text_has_the_default_style() {
    let lines = parse("ab\n");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0], vec![('a', Style::default()), ('b', Style::default())]);
}

#[test]
fn lines_split_on_newline_without_the_empty_tail() {
    assert_eq!(lines("").count(), 0);
    assert_eq!(lines("\n").collect::<Vec<_>>(), vec![""]);
    assert_eq!(lines("a\n\nb").collect::<Vec<_>>(), vec!["a", "", "b"]);
    assert_eq!(lines("a\nb\n").collect::<Vec<_>>(), vec!["a", "b"]);
}

#[test]
fn the_state_carries_over_to_the_next_line() {
    let lines = parse("\x1b[1;31mred\nstill\x1b[0m plain\n");
    let red = st(Color::Palette(1), Color::Default, BOLD);
    assert!(lines[0].iter().all(|&(_, s)| s == red));
    assert_eq!(lines[1][0], ('s', red));
    assert_eq!(lines[1][4], ('l', red));
    assert_eq!(lines[1][5], (' ', Style::default()));
}

#[test]
fn a_starting_state_is_used_and_left_as_the_last_line_left_it() {
    let mut style = st(Color::Palette(2), Color::Default, 0);
    let lines = parse_capture("a\x1b[44m\n", &mut style);
    assert_eq!(lines[0][0].1.fg, Color::Palette(2));
    assert_eq!(style, st(Color::Palette(2), Color::Palette(4), 0));
}

#[test]
fn reset_forms_clear_everything() {
    for reset in ["\x1b[0m", "\x1b[m", "\x1b[;m"] {
        let lines = parse(&format!("\x1b[1;3;4;7;38;5;9;48;2;1;2;3ma{reset}b"));
        assert_ne!(lines[0][0].1, Style::default(), "{reset:?}");
        assert_eq!(lines[0][1].1, Style::default(), "{reset:?}");
    }
}

#[test]
fn every_flag_sets_and_clears() {
    let cases = [
        ("1", "22", BOLD),
        ("2", "22", FAINT),
        ("3", "23", ITALIC),
        ("4", "24", UNDERLINE),
        ("4:3", "4:0", UNDERLINE),
        ("21", "24", UNDERLINE),
        ("5", "25", BLINK),
        ("7", "27", INVERSE),
        ("8", "28", INVISIBLE),
        ("9", "29", STRIKE),
    ];
    for (on, off, flag) in cases {
        let mut s = Style::default();
        apply_sgr(&mut s, on);
        assert_eq!(s.flags, flag, "{on}");
        apply_sgr(&mut s, off);
        assert_eq!(s.flags, 0, "{off}");
    }
    let mut s = Style::default();
    apply_sgr(&mut s, "1;2");
    apply_sgr(&mut s, "22");
    assert_eq!(s.flags, 0);
}

#[test]
fn basic_and_bright_colors_map_to_the_palette() {
    let mut s = Style::default();
    apply_sgr(&mut s, "30;47");
    assert_eq!((s.fg, s.bg), (Color::Palette(0), Color::Palette(7)));
    apply_sgr(&mut s, "97;100");
    assert_eq!((s.fg, s.bg), (Color::Palette(15), Color::Palette(8)));
    apply_sgr(&mut s, "39");
    assert_eq!(s.fg, Color::Default);
    apply_sgr(&mut s, "49");
    assert_eq!(s.bg, Color::Default);
}

#[test]
fn colors_256_and_rgb_in_both_forms() {
    let cases = [
        ("38;5;196", Color::Palette(196), Color::Default),
        ("38:5:196", Color::Palette(196), Color::Default),
        ("48;5;17", Color::Default, Color::Palette(17)),
        ("48:5:17", Color::Default, Color::Palette(17)),
        ("38;2;1;2;3", Color::Rgb(1, 2, 3), Color::Default),
        ("38:2::1:2:3", Color::Rgb(1, 2, 3), Color::Default),
        ("38:2:1:2:3", Color::Rgb(1, 2, 3), Color::Default),
        ("48;2;255;128;0", Color::Default, Color::Rgb(255, 128, 0)),
        ("48:2::255:128:0", Color::Default, Color::Rgb(255, 128, 0)),
    ];
    for (params, fg, bg) in cases {
        let mut s = Style::default();
        apply_sgr(&mut s, params);
        assert_eq!((s.fg, s.bg), (fg, bg), "{params}");
    }
}

#[test]
fn an_extended_color_consumes_its_arguments_only() {
    let mut s = Style::default();
    apply_sgr(&mut s, "38;5;1;1;48;2;9;8;7;4");
    assert_eq!(s, st(Color::Palette(1), Color::Rgb(9, 8, 7), BOLD | UNDERLINE));
    // the underline color is read and ignored, its arguments are not flags
    let mut s = Style::default();
    apply_sgr(&mut s, "58;5;3;58:2::1:2:3;1");
    assert_eq!(s, st(Color::Default, Color::Default, BOLD));
}

#[test]
fn malformed_colors_change_nothing() {
    for params in ["38;5", "38;2;1;2", "38;5;300", "38:2:1:2", "38:5", "38;7;1", "38"] {
        let mut s = Style::default();
        apply_sgr(&mut s, params);
        assert_eq!(s.fg, Color::Default, "{params}");
    }
}

#[test]
fn overline_and_unknown_codes_are_ignored() {
    let mut s = Style::default();
    apply_sgr(&mut s, "53;55;73;999999999999");
    assert_eq!(s, Style::default());
}

#[test]
fn osc8_hyperlinks_are_skipped() {
    let lines = parse("a\x1b]8;;https://example.com\x1b\\link\x1b]8;;\x1b\\b\x1b]0;title\x07c\n");
    assert_eq!(text_of(&lines[0]), "alinkbc");
}

#[test]
fn other_csi_and_private_sequences_are_skipped() {
    let lines = parse("a\x1b[2Jb\x1b[?25lc\x1b[>4;1md\x1b[1 qe\n");
    assert_eq!(text_of(&lines[0]), "abcde");
    assert!(lines[0].iter().all(|&(_, s)| s == Style::default()), "private `>4;1m` is not SGR");
}

#[test]
fn lone_escapes_produce_no_cells() {
    assert_eq!(text_of(&parse("ab\x1b\n")[0]), "ab");
    assert_eq!(text_of(&parse("a\x1b\x1b[1mb\n")[0]), "ab");
    assert_eq!(parse("a\x1b\x1b[1mb\n")[0][1].1.flags, BOLD);
    assert_eq!(text_of(&parse("a\x1bé\n")[0]), "aé");
    assert_eq!(text_of(&parse("a\x1b(Bb\x1b7c\n")[0]), "abc");
}

#[test]
fn an_escape_cut_by_the_line_end_does_not_eat_the_next_line() {
    let lines = parse("a\x1b[38;5\nb\x1b]8;;http://x\nc\n");
    assert_eq!(lines.iter().map(|l| text_of(l)).collect::<Vec<_>>(), vec!["a", "b", "c"]);
}

#[test]
fn control_characters_are_passed_to_the_caller() {
    assert_eq!(text_of(&parse("a\tb\0c\n")[0]), "a\tb\0c");
}
