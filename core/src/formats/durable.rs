//! Durable file writes: a crash or power loss at any point leaves either the old file or the
//! complete new one, never a truncated mix.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const TEMP_SUFFIX: &str = ".pixmith-tmp";
const STALE_AFTER: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_LINK_HOPS: usize = 40;

/// Follows symbolic links (also dangling ones) to the file that should actually be written, so saving
/// over a link updates its target and keeps the link.
fn resolve_links(path: &Path) -> Result<PathBuf, String> {
    let mut current = path.to_path_buf();
    for _ in 0..MAX_LINK_HOPS {
        match fs::symlink_metadata(&current) {
            Ok(m) if m.file_type().is_symlink() => {
                let link = fs::read_link(&current).map_err(|e| format!("{}: {e}", current.display()))?;
                current = match current.parent() {
                    Some(dir) => dir.join(link),
                    None => link,
                };
            }
            _ => return Ok(current),
        }
    }
    Err(format!("{}: too many levels of symbolic links", path.display()))
}

/// Removes `*.pixmith-tmp` files next to the target that a crashed save left behind (older than a day).
fn remove_stale_temp_files(dir: &Path, keep: &Path) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let p = entry.path();
        let is_temp = p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(TEMP_SUFFIX));
        if !is_temp || p == keep {
            continue;
        }
        let old = entry
            .metadata()
            .ok()
            .filter(|m| m.is_file())
            .and_then(|m| m.modified().ok())
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > STALE_AFTER);
        if old {
            let _ = fs::remove_file(&p);
        }
    }
}

/// Writes `data` to `path`: temporary file in the same directory, flushed to disk, renamed over the
/// target, then the directory entry is flushed too. Symbolic links are written through, and the
/// permissions of an existing file are kept.
pub fn write_file(path: &str, data: &[u8]) -> Result<(), String> {
    let target = resolve_links(Path::new(path)).map_err(|e| e.to_string())?;
    let dir = match target.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let name = target.file_name().ok_or_else(|| format!("{path}: not a file path"))?;
    let mut tmp_name = name.to_os_string();
    tmp_name.push(TEMP_SUFFIX);
    let tmp = dir.join(tmp_name);
    let existing = fs::metadata(&target).ok().filter(|m| m.is_file());

    let write = || -> std::io::Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            // Never wider than the original file, even before its permissions are copied; a new file
            // gets the usual document mode, narrowed by the umask.
            options.mode(existing.as_ref().map_or(0o666, |m| m.permissions().mode() & 0o777));
        }
        let mut file = options.open(&tmp)?;
        file.write_all(data)?;
        if let Some(m) = &existing {
            fs::set_permissions(&tmp, m.permissions())?;
        }
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, &target)?;
        // Make the rename itself durable. Not every file system allows syncing a directory.
        if let Ok(d) = File::open(&dir) {
            let _ = d.sync_all();
        }
        Ok(())
    };
    match write() {
        Ok(()) => {
            remove_stale_temp_files(&dir, &tmp);
            Ok(())
        }
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(format!("{path}: {e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("pixmith-durable-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn s(p: &Path) -> &str {
        p.to_str().unwrap()
    }

    fn temp_files(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(TEMP_SUFFIX))
            .collect();
        v.sort();
        v
    }

    #[test]
    fn creates_and_replaces_without_leaving_temp_files() {
        let dir = temp_dir("replace");
        let f = dir.join("a.aseprite");
        write_file(s(&f), b"one").unwrap();
        assert_eq!(fs::read(&f).unwrap(), b"one");
        write_file(s(&f), b"second version").unwrap();
        assert_eq!(fs::read(&f).unwrap(), b"second version");
        assert!(temp_files(&dir).is_empty());
    }

    #[test]
    fn failure_keeps_the_original_and_cleans_up() {
        let dir = temp_dir("failure");
        let f = dir.join("a.bin");
        write_file(s(&f), b"original").unwrap();
        // Renaming a file over a directory fails.
        let blocked = dir.join("target");
        fs::create_dir(&blocked).unwrap();
        assert!(write_file(s(&blocked), b"x").is_err());
        assert!(temp_files(&dir).is_empty());
        assert_eq!(fs::read(&f).unwrap(), b"original");
        assert!(write_file(s(&dir.join("missing/dir/file")), b"x").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn saving_over_a_symlink_writes_through_and_keeps_the_link() {
        use std::os::unix::fs::symlink;
        let dir = temp_dir("symlink");
        let real = dir.join("real");
        fs::create_dir(&real).unwrap();
        let target = real.join("doc.aseprite");
        fs::write(&target, b"old").unwrap();

        // Absolute link, relative link, and a chain of two.
        let abs = dir.join("abs-link");
        symlink(&target, &abs).unwrap();
        let rel = dir.join("rel-link");
        symlink("real/doc.aseprite", &rel).unwrap();
        let chain = dir.join("chain-link");
        symlink("rel-link", &chain).unwrap();
        for (i, link) in [&abs, &rel, &chain].into_iter().enumerate() {
            let data = format!("new {i}");
            write_file(s(link), data.as_bytes()).unwrap();
            assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink(), "{}", link.display());
            assert_eq!(fs::read(&target).unwrap(), data.as_bytes());
        }
        assert!(temp_files(&dir).is_empty() && temp_files(&real).is_empty());

        // A dangling link creates its target.
        let dangling = dir.join("dangling");
        symlink("made-by-save", &dangling).unwrap();
        write_file(s(&dangling), b"fresh").unwrap();
        assert!(fs::symlink_metadata(&dangling).unwrap().file_type().is_symlink());
        assert_eq!(fs::read(dir.join("made-by-save")).unwrap(), b"fresh");

        // Loops are an error, not a hang.
        let a = dir.join("loop-a");
        let b = dir.join("loop-b");
        symlink("loop-b", &a).unwrap();
        symlink("loop-a", &b).unwrap();
        assert!(write_file(s(&a), b"x").unwrap_err().contains("symbolic links"));
    }

    #[cfg(unix)]
    #[test]
    fn permissions_of_an_existing_file_are_kept() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("perms");
        for mode in [0o600, 0o640, 0o664, 0o755] {
            let f = dir.join(format!("m{mode:o}"));
            fs::write(&f, b"old").unwrap();
            fs::set_permissions(&f, fs::Permissions::from_mode(mode)).unwrap();
            write_file(s(&f), b"new").unwrap();
            assert_eq!(fs::metadata(&f).unwrap().permissions().mode() & 0o7777, mode, "{mode:o}");
            assert_eq!(fs::read(&f).unwrap(), b"new");
        }
        // A new file gets the same mode as any file this process creates (the umask applies).
        let fresh = dir.join("fresh");
        write_file(s(&fresh), b"x").unwrap();
        let ordinary = dir.join("ordinary");
        fs::write(&ordinary, b"x").unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&fresh), mode(&ordinary));
    }

    #[test]
    fn stale_temp_files_next_to_the_target_are_removed() {
        let dir = temp_dir("stale");
        let old = dir.join("other.aseprite.pixmith-tmp");
        let recent = dir.join("recent.aseprite.pixmith-tmp");
        let unrelated = dir.join("notes.txt");
        for f in [&old, &recent, &unrelated] {
            fs::write(f, b"junk").unwrap();
        }
        let two_days_ago = SystemTime::now() - Duration::from_secs(2 * 24 * 60 * 60);
        File::options().write(true).open(&old).unwrap().set_modified(two_days_ago).unwrap();
        let old_unrelated = dir.join("keep.txt");
        fs::write(&old_unrelated, b"keep").unwrap();
        File::options().write(true).open(&old_unrelated).unwrap().set_modified(two_days_ago).unwrap();

        write_file(s(&dir.join("doc.aseprite")), b"data").unwrap();
        assert!(!old.exists(), "a day-old temp file is removed");
        assert!(recent.exists(), "a fresh temp file may belong to a save in progress");
        assert!(unrelated.exists() && old_unrelated.exists());
        assert_eq!(temp_files(&dir), vec!["recent.aseprite.pixmith-tmp".to_string()]);
    }

    #[test]
    fn a_stale_temp_file_of_the_same_name_does_not_break_saving() {
        let dir = temp_dir("same-temp");
        let f = dir.join("doc.aseprite");
        fs::write(dir.join("doc.aseprite.pixmith-tmp"), vec![7u8; 1000]).unwrap();
        write_file(s(&f), b"short").unwrap();
        assert_eq!(fs::read(&f).unwrap(), b"short");
        assert!(temp_files(&dir).is_empty());
    }
}
