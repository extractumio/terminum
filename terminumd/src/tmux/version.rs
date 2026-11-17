// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! tmux's versions as terminum judges them: before 3.8 a server crashes when a session ends
//! while a window change is queued for a control client (tmux commit 035447b0), ending every
//! terminal in it (terminum.md DEP-02, terminum-lifecycle.md AC-08).

/// The first tmux whose server survives that.
const FIXED: (u32, u32) = (3, 8);

/// How terminum's tmux server is restarted, and what that costs.
pub const RESTART: &str = "restart it once at a quiet moment: tmux -L terminum kill-server (sessions, windows, splits, folders \
                           and projects come back; running programs do not)";

/// Major and minor of a version as `tmux -V` (without "tmux ") or `#{version}` give it: "3.6a"
/// is (3, 6); None for one that is no number ("master", "next-3.9", "openbsd-7.6").
pub fn number(version: &str) -> Option<(u32, u32)> {
    let (major, rest) = version.trim().split_once('.')?;
    let minor = rest.split(|c: char| !c.is_ascii_digit()).next()?;
    Some((major.parse().ok()?, minor.parse().ok()?))
}

/// What is said of a running server of `version`: None from 3.8 and for one that is no number.
pub fn old_line(version: &str) -> Option<String> {
    if number(version)? >= FIXED {
        return None;
    }
    let upgrade = if cfg!(target_os = "macos") { "brew upgrade tmux" } else { "upgrade tmux with your system's package manager" };
    Some(format!("tmux {version} runs terminum's terminals and can crash, ending every one of them (fixed in tmux 3.8): {upgrade}, then {RESTART}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_tmux_before_3_8_gets_the_line() {
        assert!(old_line("3.6a").is_some_and(|l| l.starts_with("tmux 3.6a ") && l.ends_with(RESTART)));
        assert!(old_line("3.7c").is_some() && old_line("3.2").is_some());
        for v in ["3.8", "3.8a", "3.10", "4.0", "next-3.9", "master", "openbsd-7.6", ""] {
            assert_eq!(old_line(v), None, "{v}");
        }
    }
}
