// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Pasted images and uploaded files (AC-36, AC-85): `POST /paste?sid=` (an image) and
//! `POST /upload?name=&sid=` (any file) are saved where the pane's shell runs, named by UTC
//! time and 8 random hex digits, 0600 in a 0700 folder; the page pastes the path it gets.
//! On this Mac in $TMPDIR/terminum-paste; on a host through its files agent, in
//! ~/.cache/terminum/paste. Pastes older than 7 days go at the next one.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::app::Shared;

pub const MAX_BYTES: usize = 20 << 20;
const KEEP_DAYS: u64 = 7;
/// Uploads at once (the body is held in memory).
const AT_ONCE: usize = 2;
static RUNNING: AtomicUsize = AtomicUsize::new(0);

/// The folder on a host, under its $HOME.
pub const REMOTE_DIR: &str = ".cache/terminum/paste";

#[derive(Deserialize)]
pub struct PasteQuery {
    #[serde(default)]
    sid: String,
    name: Option<String>,
}

fn refuse(status: StatusCode, msg: impl Into<String>) -> Response {
    (status, Json(json!({"error": msg.into()}))).into_response()
}

fn stamp() -> String {
    // UTC date and time without a time crate: days since 1970 to a civil date
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (days, rest) = ((secs / 86400) as i64, secs % 86400);
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}", rest / 3600, rest % 3600 / 60, rest % 60)
}

/// The file name for a pasted image of this type, or why not.
pub fn image_name(content_type: &str, data: &[u8]) -> Result<String, String> {
    let kind = content_type.split(';').next().unwrap_or("").trim().to_lowercase();
    let (ext, ok) = match kind.as_str() {
        "image/png" => ("png", data.starts_with(b"\x89PNG\r\n\x1a\n")),
        "image/jpeg" => ("jpg", data.starts_with(b"\xff\xd8\xff")),
        "image/gif" => ("gif", data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a")),
        "image/webp" => ("webp", data.len() >= 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP"),
        _ => return Err(format!("Only PNG, JPEG, GIF and WebP images can be pasted, not {}.", if kind.is_empty() { "this kind of file" } else { &kind })),
    };
    if data.is_empty() {
        return Err("The pasted image is empty.".into());
    }
    if data.len() > MAX_BYTES {
        return Err("The image is larger than 20 MB.".into());
    }
    if !ok {
        return Err(format!("The pasted file is not a {} image.", ext.to_uppercase()));
    }
    Ok(format!("{}-{}.{ext}", stamp(), crate::auth::crypto::random_hex::<4>()))
}

/// The stored name of an uploaded file: time, hex digits, its own name in safe letters.
pub fn upload_name(filename: &str, data: &[u8]) -> Result<String, String> {
    if data.is_empty() {
        return Err("The file is empty.".into());
    }
    if data.len() > MAX_BYTES {
        return Err("The file is larger than 20 MB.".into());
    }
    let base = filename.rsplit('/').next().unwrap_or("");
    let mut safe = String::new();
    for c in base.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
            safe.push(c);
        } else if !safe.ends_with('_') {
            safe.push('_');
        }
    }
    let safe: String = safe.trim_matches(|c| matches!(c, '.' | '_' | '-')).chars().take(80).collect();
    let safe = if safe.is_empty() { "file".to_string() } else { safe };
    Ok(format!("{}-{}-{safe}", stamp(), crate::auth::crypto::random_hex::<4>()))
}

/// Whether `name` is one terminumd made (it is joined to a folder, never anything else).
pub fn is_paste_name(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() > 25 && b[..8].iter().all(u8::is_ascii_digit) && b[8] == b'-' && b[9..15].iter().all(u8::is_ascii_digit) && b[15] == b'-'
        && b[16..24].iter().all(u8::is_ascii_hexdigit) && matches!(b[24], b'.' | b'-')
        && name[25..].chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Writes `data` as `name` in `folder` (made 0700, ours, not a link); older pastes go.
pub fn save_in(folder: &Path, name: &str, data: &[u8]) -> Result<PathBuf, String> {
    use std::io::Write;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    if !is_paste_name(name) {
        return Err(format!("not a paste name: {name}"));
    }
    std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let meta = std::fs::symlink_metadata(folder).map_err(|e| e.to_string())?;
    // SAFETY: getuid has no preconditions
    if !meta.is_dir() || meta.file_type().is_symlink() || meta.uid() != unsafe { libc::getuid() } {
        return Err(format!("{} is not a folder of this user's.", folder.display()));
    }
    let _ = std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o700));
    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(KEEP_DAYS * 86400);
    for old in std::fs::read_dir(folder).into_iter().flatten().flatten() {
        if old.metadata().is_ok_and(|m| m.is_file() && m.modified().is_ok_and(|t| t < cutoff)) {
            let _ = std::fs::remove_file(old.path());
        }
    }
    let path = folder.join(name);
    if std::fs::symlink_metadata(&path).is_ok() {
        return Err(format!("{name} exists already"));
    }
    let part = folder.join(format!("{name}.part"));
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(libc::O_NOFOLLOW).open(&part).map_err(|e| e.to_string())?;
    if let Err(e) = f.write_all(data).and_then(|()| f.sync_all()) {
        let _ = std::fs::remove_file(&part);
        return Err(e.to_string());
    }
    std::fs::rename(&part, &path).map_err(|e| {
        let _ = std::fs::remove_file(&part);
        e.to_string()
    })?;
    Ok(path)
}

/// This Mac's paste folder: TM_PASTE_DIR, else $TMPDIR/terminum-paste.
pub fn local_dir() -> PathBuf {
    std::env::var_os("TM_PASTE_DIR").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("terminum-paste"))
}

struct Slot;
impl Drop for Slot {
    fn drop(&mut self) {
        RUNNING.fetch_sub(1, Ordering::Relaxed);
    }
}

/// `POST /paste` and `POST /upload` (the route says which).
pub async fn save(app: &Shared, upload: bool, q: PasteQuery, headers: &HeaderMap, body: Bytes) -> Response {
    if RUNNING.fetch_add(1, Ordering::Relaxed) >= AT_ONCE {
        RUNNING.fetch_sub(1, Ordering::Relaxed);
        return refuse(StatusCode::TOO_MANY_REQUESTS, "Two uploads are running already");
    }
    let _slot = Slot;
    let content_type = headers.get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("");
    let name = if upload { upload_name(q.name.as_deref().unwrap_or(""), &body) } else { image_name(content_type, &body) };
    let name = match name {
        Ok(n) => n,
        Err(e) => return refuse(StatusCode::BAD_REQUEST, e),
    };
    let Some((server, _pane)) = app.tmux.resolve(&q.sid) else {
        return refuse(StatusCode::NOT_FOUND, super::CLOSED);
    };
    if server.is_local() {
        let data = body.to_vec();
        return match tokio::task::spawn_blocking(move || save_in(&local_dir(), &name, &data)).await {
            Ok(Ok(path)) => Json(json!({"path": path.display().to_string()})).into_response(),
            Ok(Err(e)) => refuse(StatusCode::INTERNAL_SERVER_ERROR, format!("Not saved: {e}")),
            Err(e) => refuse(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        };
    }
    match crate::remote::paste(app, &server.name, &name, body).await {
        Ok(path) => Json(json!({"path": path})).into_response(),
        Err(e) => refuse(StatusCode::BAD_GATEWAY, e),
    }
}

pub async fn paste(State(app): State<Shared>, Query(q): Query<PasteQuery>, headers: HeaderMap, body: Bytes) -> Response {
    save(&app, false, q, &headers, body).await
}

pub async fn upload(State(app): State<Shared>, Query(q): Query<PasteQuery>, headers: HeaderMap, body: Bytes) -> Response {
    save(&app, true, q, &headers, body).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_are_named_by_type_and_checked_by_their_bytes() {
        let png = b"\x89PNG\r\n\x1a\nrest";
        let n = image_name("image/png", png).unwrap();
        assert!(n.ends_with(".png") && is_paste_name(&n), "{n}");
        assert!(image_name("image/jpeg", png).unwrap_err().contains("not a JPG"));
        assert!(image_name("text/plain", b"x").unwrap_err().contains("Only PNG"));
        assert!(image_name("image/png", &vec![0; MAX_BYTES + 1]).is_err());
    }

    #[test]
    fn uploaded_names_keep_safe_letters_only() {
        let n = upload_name("../a b/$(rm -rf)~.txt", b"x").unwrap();
        assert!(n.ends_with("-rm_-rf_.txt") || n.ends_with("_rm_-rf_.txt"), "{n}");
        assert!(is_paste_name(&n));
        assert!(upload_name("x", b"").is_err());
        assert!(!is_paste_name("../x") && !is_paste_name("20260101-010101-zzzzzzzz.png"));
    }

    #[test]
    fn saved_privately_and_old_ones_go() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("tm-paste-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let name = upload_name("a.txt", b"hello").unwrap();
        let p = save_in(&dir, &name, b"hello").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"hello");
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        assert!(save_in(&dir, &name, b"again").is_err(), "never over an existing file");
        std::fs::remove_dir_all(&dir).ok();
    }
}
