// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Files dropped on a pane of this Mac from Finder or another app (terminum-terminal.md AC-36):
//! a browser gives the page their bytes, names, sizes and times, never their paths, but macOS
//! keeps the drag's files on its drag pasteboard until the next drag. `POST /api/os/dropped`
//! (this Mac's browser only: auth/checks.rs) reads it with osascript (JXA, no Apple Events, so
//! no Automation prompt) and answers the paths when every file dropped is one of them by name
//! and kind, and a file also by size and time (2 s): nothing is uploaded, at any size. Else
//! it says why and the page uploads copies as before.

use std::path::Path;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::app::Shared;
use crate::http::{err, ApiResult};

/// The drag pasteboard's files as a JSON list of [path, its name composed (NFC)]: the page
/// sends File.name composed too, and a name on the disk may be decomposed ("é" as e + ´).
const SCRIPT: &str = "ObjC.import('AppKit');\
const pb = $.NSPasteboard.pasteboardWithName('Apple CFPasteboard drag');\
const urls = pb.readObjectsForClassesOptions($([$.NSURL]), $({}));\
const out = [];\
for (let i = 0; i < (urls.isNil() ? 0 : urls.count); i++) { const u = urls.objectAtIndex(i);\
  if (u.isFileURL) out.push([ObjC.unwrap(u.path), ObjC.unwrap(u.lastPathComponent.precomposedStringWithCanonicalMapping)]); }\
JSON.stringify(out);";
/// osascript starts in about 0.1 s; the pasteboard server may be slow, never the page.
const WAIT: Duration = Duration::from_secs(3);
/// A browser's lastModified and the file's time agree within this (file systems round).
const SLACK_MS: i64 = 2000;
/// More files than a drop sensibly holds are not matched one by one.
const MAX_FILES: usize = 1000;

#[derive(Deserialize, Clone)]
pub struct Dropped {
    name: String,
    #[serde(default)]
    size: u64,
    /// The browser's File.lastModified (ms since 1970).
    #[serde(default)]
    mtime: i64,
    #[serde(default)]
    dir: bool,
}

#[derive(Deserialize)]
pub struct DroppedBody {
    sid: String,
    files: Vec<Dropped>,
}

/// What is known of a path on the pasteboard: (is a folder, size, time in ms), or None.
type Meta = Option<(bool, u64, i64)>;
/// A file on the drag pasteboard: its path and its name composed.
type OnBoard = (String, String);

fn meta(p: &str) -> Meta {
    let m = std::fs::metadata(p).ok()?;
    let ms = m.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64;
    Some((m.is_dir(), m.len(), ms))
}

/// The pasteboard's path for each file dropped, in the drop's order, each path taken once; None
/// when one is not there (a drag from a browser or Mail has no file on disk), when the paths
/// are another drag's, or one has a control character (each would be a key at the input).
pub fn matched(files: &[Dropped], board: &[OnBoard], meta: impl Fn(&str) -> Meta) -> Option<Vec<String>> {
    if files.is_empty() || board.iter().any(|(p, _)| !p.starts_with('/') || p.chars().any(char::is_control)) {
        return None;
    }
    let known: Vec<(&String, &String, Meta)> = board.iter()
        .filter(|(p, _)| Path::new(p.as_str()).file_name().is_some())
        .map(|(p, name)| (p, name, meta(p))).collect();
    let mut used = vec![false; known.len()];
    let mut out = Vec::with_capacity(files.len());
    for f in files {
        let i = known.iter().enumerate().position(|(i, (_, name, m))| {
            !used[i] && **name == f.name
                && m.is_some_and(|(dir, size, ms)| dir == f.dir && (dir || (size == f.size && (ms - f.mtime).abs() <= SLACK_MS)))
        })?;
        used[i] = true;
        out.push(known[i].0.clone());
    }
    Some(out)
}

/// The drag pasteboard's files, or why not.
async fn drag_paths() -> Result<Vec<OnBoard>, String> {
    let osascript = crate::config::find_program("osascript", "TM_OSASCRIPT").ok_or("osascript is not found")?;
    let run = tokio::process::Command::new(osascript).args(["-l", "JavaScript", "-e", SCRIPT])
        .stdin(std::process::Stdio::null()).kill_on_drop(true).output();
    let out = tokio::time::timeout(WAIT, run).await.map_err(|_| "macOS's drag pasteboard did not answer within 3 s".to_string())?
        .map_err(|e| format!("osascript: {e}"))?;
    if !out.status.success() {
        return Err(format!("osascript: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("osascript's answer: {e}"))
}

/// `POST /api/os/dropped` {sid, files: [{name, size, mtime, dir}]} -> {paths} in the drop's
/// order, or an error saying why (the page then uploads copies).
pub async fn os_dropped(State(app): State<Shared>, Json(b): Json<DroppedBody>) -> ApiResult {
    if !cfg!(target_os = "macos") {
        return Err(err(StatusCode::NOT_IMPLEMENTED, "not_macos", "A drop's own paths are found on macOS only"));
    }
    if b.files.is_empty() || b.files.len() > MAX_FILES {
        return Err(err(StatusCode::BAD_REQUEST, "bad_request", format!("1 to {MAX_FILES} files")));
    }
    match app.tmux.resolve(&b.sid) {
        None => return Err(err(StatusCode::NOT_FOUND, "closed", crate::page::CLOSED)),
        Some((server, _)) if !server.is_local() => {
            return Err(err(StatusCode::BAD_REQUEST, "on_host", format!("The session runs on {}: its files are uploaded", server.name)));
        }
        _ => {}
    }
    let paths = drag_paths().await.map_err(|e| err(StatusCode::BAD_GATEWAY, "pasteboard", e))?;
    let files = b.files.clone();
    let found = tokio::task::spawn_blocking(move || matched(&files, &paths, meta)).await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "internal", e.to_string()))?;
    match found {
        Some(paths) => Ok(Json(json!({"paths": paths})).into_response()),
        None => Err(err(StatusCode::NOT_FOUND, "not_found", "Their files were not found on this Mac (a drag from a browser or Mail has none)")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pasteboard's files, their names as the JXA gives them (already composed here).
    fn board(paths: &[&str]) -> Vec<OnBoard> {
        paths.iter().map(|p| (p.to_string(), Path::new(p).file_name().unwrap_or_default().to_string_lossy().into_owned())).collect()
    }

    fn f(name: &str, size: u64, mtime: i64) -> Dropped {
        Dropped { name: name.into(), size, mtime, dir: false }
    }

    fn on_disk(p: &str) -> Meta {
        match p {
            "/Users/alex/a/x.zip" => Some((false, 10, 1_000_000)),
            "/Users/alex/b/x.zip" => Some((false, 20, 1_000_000)),
            "/Users/alex/a/src" => Some((true, 96, 5)),
            "/Users/alex/c/src" => Some((false, 7, 5)),
            _ => None,
        }
    }

    #[test]
    fn each_file_dropped_is_its_own_path_on_the_pasteboard() {
        let paths = board(&["/Users/alex/a/x.zip", "/Users/alex/b/x.zip", "/Users/alex/a/src", "/Users/alex/c/src"]);
        let src = Dropped { name: "src".into(), size: 0, mtime: 0, dir: true };
        let got = matched(&[f("x.zip", 20, 1_001_500), f("x.zip", 10, 998_000), src.clone(), f("src", 7, 5)], &paths, on_disk);
        assert_eq!(got.unwrap(), ["/Users/alex/b/x.zip", "/Users/alex/a/x.zip", "/Users/alex/a/src", "/Users/alex/c/src"], "same names told apart by size, a folder by its kind, in the drop's order");
        assert!(matched(&[f("x.zip", 10, 1_002_001)], &paths, on_disk).is_none(), "2 s apart at most");
        assert!(matched(&[f("x.zip", 11, 1_000_000)], &paths, on_disk).is_none(), "another size: another drag's");
        assert!(matched(&[f("x.zip", 10, 1_000_000), f("y.txt", 1, 1)], &paths, on_disk).is_none(), "one not there: none at all");
        assert!(matched(&[f("x.zip", 10, 1_000_000), f("x.zip", 10, 1_000_000)], &paths, on_disk).is_none(), "a path is taken once");
        assert!(matched(&[], &paths, on_disk).is_none());
    }

    #[test]
    fn a_path_with_a_control_character_refuses_the_drop() {
        assert!(matched(&[f("x.zip", 10, 1_000_000)], &board(&["/Users/alex/a/x.zip", "/Users/alex/a/new\nline"]), on_disk).is_none());
        assert!(matched(&[f("x.zip", 10, 1_000_000)], &board(&["relative/x.zip"]), |_| Some((false, 10, 1_000_000))).is_none());
    }

    #[test]
    fn a_name_is_matched_composed_and_the_path_given_as_the_disk_has_it() {
        let decomposed = "/Users/alex/a/cafe\u{301}.txt";
        let on = vec![(decomposed.to_string(), "caf\u{e9}.txt".to_string())];
        assert_eq!(matched(&[f("caf\u{e9}.txt", 3, 0)], &on, |_| Some((false, 3, 0))).unwrap(), [decomposed]);
    }
}
