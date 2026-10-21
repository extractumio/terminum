// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A capture of history rows against a private tmux server (`-L tm-test-hist-<pid>`, never the
//! user's): rows that scrolled off between the frame and the capture move the range with them,
//! so the page never shows other rows under these numbers (AC-02).

use std::time::Duration;

use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

use super::capture;
use crate::testutil::Tmux;
use crate::tmux::control::Control;

fn attach(s: &Tmux) -> Control {
    let mut cmd = Command::new("tmux");
    cmd.args(["-C", "-u", "-L", &s.socket, "-f", "/dev/null", "attach", "-f", "ignore-size", "-t", "main"]);
    let (ntx, nrx) = mpsc::unbounded_channel();
    let (dtx, drx) = oneshot::channel();
    std::mem::forget((nrx, drx));                       // notices and the end are not this test's
    Control::spawn(cmd, ntx, dtx).unwrap()
}

async fn history_size(c: &Control) -> i64 {
    c.run("display -p '#{history_size}'").await.unwrap()[0].trim().parse().unwrap()
}

/// The text of a captured row in the page format.
fn text(row: &serde_json::Value) -> String {
    row["r"].as_array().map(|runs| runs.iter().filter_map(|r| r[0].as_str()).collect::<String>()).unwrap_or_default().trim_end().to_string()
}

#[tokio::test]
async fn rows_that_scrolled_off_meanwhile_never_show_as_others() {
    let script = "seq -f L%04g 1 100; read x; seq -f L%04g 101 160; exec cat";
    let Some(s) = Tmux::start("hist", (80, 10), &format!("sh -c '{script}'")) else { return };
    let c = attach(&s);
    let pane = c.run("display -p '#{pane_id}'").await.unwrap().remove(0);
    let mut h = 0;
    for _ in 0..50 {
        h = history_size(&c).await;
        if h >= 90 { break; }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // the page's numbering at that frame: nothing dropped, the screen's top is the history's size
    let (top, history) = (h, h);
    c.run(format!("send-keys -t {pane} Enter")).await.unwrap();
    for _ in 0..50 {
        if history_size(&c).await >= h + 60 { break; }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(history_size(&c).await >= h + 60, "the pane went on: 60 more rows scrolled off");
    let rows = capture(&c, &pane, top, history, 2000, 80, 84, 80).await.unwrap();
    let got: Vec<String> = rows.iter().map(text).collect();
    assert_eq!(got, ["L0081", "L0082", "L0083", "L0084"], "the rows asked for, not those 60 rows on");

    // a width tmux re-wrapped the history for: an error, not rows of another place
    s.read(&["resize-window", "-t", "main", "-x", "60", "-y", "10"]);
    let err = capture(&c, &pane, top, history, 2000, 80, 84, 80).await.unwrap_err();
    assert!(err.contains("width"), "{err}");
    s.read(&["resize-window", "-t", "main", "-x", "80", "-y", "10"]);
    // a history near tmux's limit may have been trimmed: how far rows moved is not known
    let near = history_size(&c).await + 1;          // the history above the tenth tmux trims to
    let err = capture(&c, &pane, top, history, near, 80, 84, 80).await.unwrap_err();
    assert!(err.contains("trimmed"), "{err}");
}
