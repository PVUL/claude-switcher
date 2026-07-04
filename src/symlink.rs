//! Atomic symlink and file operations.
//!
//! Switching profiles must never leave the system in a half-updated state, so
//! we always create a temporary link/file first and then `rename` it over the
//! destination. On the same filesystem `rename(2)` is atomic, which gives us an
//! all-or-nothing swap.

use std::path::{Path, PathBuf};
use std::{fs, io};

/// Read the target of a symlink, returning `None` if the path is missing or is
/// not a symlink.
pub fn read_link(link: &Path) -> Option<PathBuf> {
    fs::read_link(link).ok()
}

/// Point `link` at `target`, atomically replacing any existing link/file.
pub fn atomic_symlink(target: &Path, link: &Path) -> io::Result<()> {
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = temp_sibling(link);
    // Best-effort cleanup of a stale temp from a previously interrupted run.
    let _ = fs::remove_file(&tmp);
    create_symlink(target, &tmp)?;

    match fs::rename(&tmp, link) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Write bytes to `path` atomically via a temp file + rename.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = temp_sibling(path);
    fs::write(&tmp, bytes)?;
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// A temp path next to `target` so that `rename` stays on the same filesystem.
fn temp_sibling(target: &Path) -> PathBuf {
    let pid = std::process::id();
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "claude-switcher".into());
    let tmp_name = format!(".{name}.tmp.{pid}");
    match target.parent() {
        Some(parent) => parent.join(tmp_name),
        None => PathBuf::from(tmp_name),
    }
}

#[cfg(unix)]
fn create_symlink(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn create_symlink(target: &Path, link: &Path) -> io::Result<()> {
    // Profiles are directories, so use a directory symlink. This requires
    // Developer Mode or elevated privileges on Windows.
    std::os::windows::fs::symlink_dir(target, link)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn creates_and_reads_symlink() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("profile");
        fs::create_dir(&target).unwrap();
        let link = dir.path().join("active");

        atomic_symlink(&target, &link).unwrap();
        assert_eq!(read_link(&link).unwrap(), target);
    }

    #[test]
    fn replaces_existing_link_atomically() {
        let dir = tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        let link = dir.path().join("active");

        atomic_symlink(&a, &link).unwrap();
        atomic_symlink(&b, &link).unwrap();
        assert_eq!(read_link(&link).unwrap(), b);
        // No temp files left behind.
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn read_link_on_missing_is_none() {
        let dir = tempdir().unwrap();
        assert!(read_link(&dir.path().join("nope")).is_none());
    }

    #[test]
    fn atomic_write_round_trips() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("f.json");
        atomic_write(&path, b"hello").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "hello");
    }
}
