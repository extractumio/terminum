// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A shown pane's frame (AC-02): what `display -p` says about the pane, and the rows that
//! changed since the last frame sent, as the page's `screen` message.

use serde_json::json;

/// The fields read with every capture, tab-separated; the folder last (it may hold a tab).
pub const FIELDS: &str = "#{cursor_x}\t#{cursor_y}\t#{alternate_on}\t#{history_size}\t#{pane_width}\t#{pane_height}\t#{mouse_any_flag}\t#{mouse_sgr_flag}\t#{history_limit}\t#{cursor_flag}\t#{window_id}\t#{window_zoomed_flag}\t#{window_panes}\t#{pid}\t#{pane_current_command}\t#{pane_current_path}";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fields {
    pub cx: usize,
    pub cy: usize,
    pub alt: bool,
    pub history: i64,
    pub width: usize,
    pub height: usize,
    pub mouse: bool,
    pub mouse_sgr: bool,
    pub limit: i64,
    pub cursor_on: bool,
    pub window: String,
    pub zoomed: bool,
    pub panes: usize,
    /// the tmux server's process: another one means the pane ids name other panes (AC-14)
    pub server_pid: u32,
    pub command: String,
    pub path: String,
}

impl Fields {
    /// The display line, or None when it is not one (an older tmux, a broken answer).
    pub fn parse(line: &str) -> Option<Fields> {
        let p: Vec<&str> = line.splitn(16, '\t').collect();
        if p.len() < 16 {
            return None;
        }
        let n = |i: usize| p[i].trim().parse::<i64>().ok();
        let flag = |i: usize| p[i].trim() == "1";
        Some(Fields {
            cx: n(0)? as usize,
            cy: n(1)? as usize,
            alt: flag(2),
            history: n(3)?,
            width: n(4)?.max(1) as usize,
            height: n(5)?.max(1) as usize,
            mouse: flag(6),
            mouse_sgr: flag(7),
            limit: n(8).unwrap_or(0),
            cursor_on: flag(9),
            window: p[10].to_string(),
            zoomed: flag(11),
            panes: n(12).unwrap_or(1).max(1) as usize,
            server_pid: n(13).unwrap_or(0) as u32,
            command: p[14].to_string(),
            path: p[15].to_string(),
        })
    }
}

/// The rows a page was last sent, to send only what changed.
#[derive(Default)]
pub struct Shown {
    rows: Vec<String>,
    size: Option<(usize, usize)>,
    alt: Option<bool>,
    mouse: Option<bool>,
}

impl Shown {
    /// Whether row `i` differs from what was last sent.
    pub fn changed(&self, i: usize, rows: &[String]) -> bool {
        self.rows.get(i) != rows.get(i)
    }

    /// Forgets what was sent: the next frame is full.
    pub fn clear(&mut self) {
        *self = Shown::default();
    }

    /// The `screen` message for `rows` (each an encoded line), or None when nothing changed.
    /// `extra` fields (a trace's keys) are added when there is a message.
    pub fn frame(&mut self, sid: &str, f: &Fields, rows: Vec<String>, extra: Option<serde_json::Value>) -> Option<String> {
        let size = (f.width, f.height);
        let full = self.size != Some(size) || self.rows.is_empty();
        let mut changed: Vec<usize> = Vec::new();
        for (i, r) in rows.iter().enumerate() {
            if full || self.rows.get(i) != Some(r) {
                changed.push(i);
            }
        }
        let flags_changed = self.alt != Some(f.alt) || self.mouse != Some(f.mouse);
        if !full && changed.is_empty() && !flags_changed && self.rows.len() == rows.len() {
            return None;
        }
        let mut out = String::with_capacity(64 + changed.iter().map(|&i| rows[i].len() + 8).sum::<usize>());
        out.push_str(&format!(
            r#"{{"t":"screen","sid":{},"full":{full},"n":{},"cols":{},"rows":{},"alt":{},"mouse":{},"ch":["#,
            json!(sid),
            rows.len(),
            f.width,
            f.height,
            f.alt,
            f.mouse
        ));
        for (k, &i) in changed.iter().enumerate() {
            if k > 0 {
                out.push(',');
            }
            out.push_str(&format!("[{i},{}]", rows[i]));
        }
        out.push(']');
        if let Some(serde_json::Value::Object(m)) = extra {
            for (k, v) in m {
                out.push_str(&format!(",{}:{}", json!(k), v));
            }
        }
        out.push('}');
        self.rows = rows;
        self.size = Some(size);
        self.alt = Some(f.alt);
        self.mouse = Some(f.mouse);
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(w: usize, h: usize) -> Fields {
        Fields { width: w, height: h, ..Default::default() }
    }

    #[test]
    fn display_lines_parse_with_a_tab_in_the_folder() {
        let f = Fields::parse("3\t4\t0\t120\t80\t24\t1\t1\t12000\t1\t@2\t0\t2\t4242\tzsh\t/tmp/a\tb").unwrap();
        assert_eq!((f.cx, f.cy, f.alt, f.history, f.width, f.height), (3, 4, false, 120, 80, 24));
        assert!(f.mouse && f.mouse_sgr && f.cursor_on && !f.zoomed);
        assert_eq!((f.window.as_str(), f.panes, f.server_pid, f.command.as_str(), f.path.as_str()), ("@2", 2, 4242, "zsh", "/tmp/a\tb"));
        assert_eq!(Fields::parse("1\t2\t3"), None);
    }

    #[test]
    fn only_changed_rows_are_sent_and_an_unchanged_screen_sends_nothing() {
        let mut s = Shown::default();
        let f = fields(10, 3);
        let rows = |a: &str| vec![r#"{"r":[],"e":true}"#.to_string(), a.to_string(), r#"{"r":[],"e":true}"#.to_string()];
        let first: serde_json::Value = serde_json::from_str(&s.frame("k", &f, rows(r#"{"r":[["a",null,null,0]],"e":true}"#), None).unwrap()).unwrap();
        assert_eq!(first["full"], true);
        assert_eq!(first["ch"].as_array().unwrap().len(), 3);
        assert_eq!(s.frame("k", &f, rows(r#"{"r":[["a",null,null,0]],"e":true}"#), None), None, "same spinner frame twice: nothing");
        let next: serde_json::Value = serde_json::from_str(&s.frame("k", &f, rows(r#"{"r":[["b",null,null,0]],"e":true}"#), Some(json!({"keys": [17]}))).unwrap()).unwrap();
        assert_eq!(next["full"], false);
        assert_eq!(next["ch"], json!([[1, {"r": [["b", null, null, 0]], "e": true}]]));
        assert_eq!(next["keys"], json!([17]));
        let resized: serde_json::Value = serde_json::from_str(&s.frame("k", &fields(12, 3), rows(r#"{"r":[["b",null,null,0]],"e":true}"#), None).unwrap()).unwrap();
        assert_eq!(resized["full"], true, "a new size is a full frame");
    }
}
