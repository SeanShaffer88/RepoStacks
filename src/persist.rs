//! Shared persistence for the catalogue, queue, and settings files.
//!
//! Every state file is written atomically (write a temp file, rename over
//! the target) and keeps a one-generation backup beside it
//! (`catalog.json` → `catalog.json.bak`). Loading is recovery-first: a
//! file that no longer parses is quarantined to `<name>.corrupt` — never
//! silently discarded — and the backup takes its place. The user only
//! sees an error when both the file and its backup are unusable.

use std::path::{Path, PathBuf};

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

pub fn backup_path(path: &Path) -> PathBuf {
    sibling(path, ".bak")
}

pub fn corrupt_path(path: &Path) -> PathBuf {
    sibling(path, ".corrupt")
}

/// Write `text` to `path` atomically, keeping the previous contents as
/// the backup generation.
pub fn save_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let tmp = sibling(path, ".tmp");
    std::fs::write(&tmp, text)?;
    if path.exists() {
        // The outgoing file becomes the backup before the new one lands,
        // so a crash anywhere in here leaves one good copy behind.
        let _ = std::fs::copy(path, backup_path(path));
    }
    std::fs::rename(&tmp, path)
}

/// Load and parse a state file with backup recovery. A missing file is
/// `T::default()`. A file that fails to parse is quarantined and the
/// backup is tried (and restored into place on success).
pub fn load_recovering<T: Default>(
    path: &Path,
    parse: impl Fn(&str) -> Result<T, String>,
) -> Result<T, String> {
    if !path.exists() {
        return Ok(T::default());
    }
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    match parse(&text) {
        Ok(value) => Ok(value),
        Err(main_err) => {
            let backup = backup_path(path);
            if backup.exists() {
                if let Ok(backup_text) = std::fs::read_to_string(&backup) {
                    if let Ok(value) = parse(&backup_text) {
                        // Quarantine the bad file (kept for inspection)
                        // and restore the last good state in its place.
                        let _ = std::fs::rename(path, corrupt_path(path));
                        let _ = std::fs::write(path, &backup_text);
                        return Ok(value);
                    }
                }
            }
            Err(format!(
                "{} could not be read ({main_err}) and no usable backup exists",
                path.display()
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_i(text: &str) -> Result<i64, String> {
        text.trim()
            .parse::<i64>()
            .map_err(|e| format!("not a number: {e}"))
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("repostacks-persist-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_file_loads_default() {
        let dir = temp_dir("missing");
        let v: i64 = load_recovering(&dir.join("nope.json"), parse_i).unwrap();
        assert_eq!(v, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_recovers_from_backup_and_quarantines() {
        let dir = temp_dir("corrupt");
        let path = dir.join("state.json");
        save_atomic(&path, "41").unwrap();
        save_atomic(&path, "42").unwrap(); // backup now holds 41
        std::fs::write(&path, "{ not json at all").unwrap();
        let v: i64 = load_recovering(&path, parse_i).unwrap();
        assert_eq!(v, 41, "the backup generation wins");
        assert!(corrupt_path(&path).exists(), "bad bytes are quarantined");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "41");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_file_without_backup_is_an_error_not_data_loss() {
        let dir = temp_dir("nobak");
        let path = dir.join("state.json");
        std::fs::write(&path, "garbage").unwrap();
        let err = load_recovering::<i64>(&path, parse_i).unwrap_err();
        assert!(err.contains("no usable backup"), "got: {err}");
        // The bad file is left exactly where it was for the user to inspect.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "garbage");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
