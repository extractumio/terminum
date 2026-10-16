// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The private files of sign-ins and web access (AC-09, AC-10): JSON in the app folder, readable
//! by the user only (0600), replaced atomically so a crash never leaves half a file, and read so
//! that a broken one is set aside (`<name>.bak`) instead of stopping terminumd. Blocking I/O: an
//! async caller runs these off its workers.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// Writes `value` to `path`: a new 0600 file beside it, synced, then renamed into place.
pub fn write_private(path: &Path, value: &Value) -> io::Result<()> {
    crate::config::write_private(path, &serde_json::to_vec_pretty(value).map_err(io::Error::other)?)
}

/// What a private file holds: None when there is none.
pub fn read(path: &Path) -> Result<Option<Value>, String> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("state");
    match fs::read(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("{name} cannot be read: {e}")),
        Ok(bytes) => serde_json::from_slice(&bytes).map(Some).map_err(|e| format!("{name} cannot be read: {e}")),
    }
}

/// The file's content parsed by `parse`, or None when there is none. A file that cannot be read
/// or parsed is moved to `<name>.bak` (one, replaced each time) and the reason logged: the
/// caller starts empty and the user can still look at what was there.
pub fn read_or_set_aside<T>(path: &Path, parse: impl FnOnce(Value) -> Result<T, String>) -> Option<T> {
    let why = match read(path).and_then(|v| v.map(parse).transpose()) {
        Ok(found) => return found,
        Err(why) => why,
    };
    let bak = backup_path(path);
    match fs::rename(path, &bak) {
        Ok(()) => {
            let _ = fs::set_permissions(&bak, fs::Permissions::from_mode(0o600));
            tracing::warn!(event = "state.set_aside", file = %path.display(), reason = %why, "starting empty");
        }
        Err(e) => tracing::warn!(event = "state.unreadable", file = %path.display(), reason = %why, error = %e),
    }
    None
}

pub fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".bak");
    path.with_file_name(name)
}

/// Seconds since 1970: expiries outlive a restart of terminumd.
pub fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::fresh_dir;
    use serde_json::json;

    #[test]
    fn a_written_file_is_private_whole_and_leaves_nothing_beside_it() {
        let dir = fresh_dir("write");
        let path = dir.join("x.json");
        fs::write(&path, "old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        write_private(&path, &json!({"a": 1})).unwrap();
        assert_eq!(read(&path).unwrap(), Some(json!({"a": 1})));
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600, "the replaced file's mode is not kept");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_folder_that_cannot_be_written_says_so() {
        let dir = fresh_dir("nodir");
        assert!(write_private(&dir.join("missing/x.json"), &json!({})).is_err());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_broken_file_is_set_aside_and_a_missing_one_is_none() {
        let dir = fresh_dir("broken");
        let path = dir.join("s.json");
        assert_eq!(read_or_set_aside(&path, Ok), None);
        fs::write(&path, "{not json").unwrap();
        assert_eq!(read_or_set_aside(&path, Ok), None);
        assert!(!path.exists());
        assert_eq!(fs::read_to_string(backup_path(&path)).unwrap(), "{not json");
        fs::write(&path, "[]").unwrap();
        let refused: Option<()> = read_or_set_aside(&path, |_| Err("not an object".into()));
        assert_eq!(refused, None);
        assert_eq!(fs::read_to_string(backup_path(&path)).unwrap(), "[]", "the newer broken file replaces the older");
        fs::write(&path, "{\"k\": 2}").unwrap();
        assert_eq!(read_or_set_aside(&path, |v| Ok(v["k"].as_u64())), Some(Some(2)));
        fs::remove_dir_all(&dir).ok();
    }
}
