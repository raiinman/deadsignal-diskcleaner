use crate::safety;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize)]
pub struct DirSize {
    pub path: PathBuf,
    pub bytes: u64,
}

/// One direct child of a scanned folder, for the disk map.
#[derive(Debug, Clone, Serialize)]
pub struct ScanEntry {
    pub path: PathBuf,
    pub name: String,
    pub bytes: u64,
    pub files: u64,
    pub is_dir: bool,
    /// Inside (or equal to) a protected path. Shown locked in the GUI.
    pub locked: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanResult {
    pub root: PathBuf,
    pub total_bytes: u64,
    /// Bytes in children that did not make the top-N cut.
    pub other_bytes: u64,
    pub other_count: usize,
    pub files: u64,
    /// Entries that could not be read (permissions, in-use files).
    pub errors: u64,
    /// The root itself is protected.
    pub locked: bool,
    /// Largest first.
    pub children: Vec<ScanEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScanProgress {
    pub files: u64,
    pub bytes: u64,
    pub current: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

/// How many walked entries between progress callbacks.
const PROGRESS_EVERY: u64 = 2048;

/// Total size of everything under `path`. Symlinks/junctions are not followed.
pub fn dir_size(path: &Path) -> u64 {
    WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum()
}

/// Sizes of the immediate children of `root`, largest first, for the disk map.
pub fn top_children(root: &Path, limit: usize) -> Vec<DirSize> {
    let never = AtomicBool::new(false);
    scan_children(root, limit, &[], &never, |_| {})
        .expect("never cancelled")
        .children
        .into_iter()
        .map(|c| DirSize {
            path: c.path,
            bytes: c.bytes,
        })
        .collect()
}

/// True when `p` is equal to or inside one of `protected`.
pub fn is_locked(p: &Path, protected: &[PathBuf]) -> bool {
    let n = safety::normalize(p);
    protected
        .iter()
        .any(|x| n.starts_with(safety::normalize(x)))
}

/// Walk `root` once and total the size of each direct child. Never follows
/// symlinks or junctions. Checks `cancel` on every entry and calls
/// `on_progress` every few thousand entries, so it is safe to run on a whole
/// drive from a background thread.
pub fn scan_children(
    root: &Path,
    limit: usize,
    protected: &[PathBuf],
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(&ScanProgress),
) -> Result<ScanResult, Cancelled> {
    #[derive(Default)]
    struct Acc {
        bytes: u64,
        files: u64,
        is_dir: bool,
    }
    let mut acc: HashMap<PathBuf, Acc> = HashMap::new();
    let (mut files, mut bytes, mut errors, mut seen) = (0u64, 0u64, 0u64, 0u64);

    for e in WalkDir::new(root).follow_links(false).min_depth(1) {
        if cancel.load(Ordering::Relaxed) {
            return Err(Cancelled);
        }
        let e = match e {
            Ok(e) => e,
            Err(_) => {
                errors += 1;
                continue;
            }
        };
        let Ok(rel) = e.path().strip_prefix(root) else {
            continue;
        };
        let Some(first) = rel.components().next() else {
            continue;
        };
        let slot = acc.entry(root.join(first.as_os_str())).or_default();
        if e.depth() == 1 {
            slot.is_dir = e.file_type().is_dir();
        }
        seen += 1;
        if e.file_type().is_file() {
            let len = match e.metadata() {
                Ok(m) => m.len(),
                Err(_) => {
                    errors += 1;
                    0
                }
            };
            slot.bytes += len;
            slot.files += 1;
            files += 1;
            bytes += len;
        }
        if seen % PROGRESS_EVERY == 0 {
            on_progress(&ScanProgress {
                files,
                bytes,
                current: e.path().to_path_buf(),
            });
        }
    }

    let mut children: Vec<ScanEntry> = acc
        .into_iter()
        .map(|(path, a)| ScanEntry {
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            bytes: a.bytes,
            files: a.files,
            is_dir: a.is_dir,
            locked: false,
            path,
        })
        .collect();
    children.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.name.cmp(&b.name)));
    let rest = children.split_off(limit.min(children.len()));
    for c in &mut children {
        c.locked = is_locked(&c.path, protected);
    }
    Ok(ScanResult {
        root: root.to_path_buf(),
        total_bytes: bytes,
        other_bytes: rest.iter().map(|c| c.bytes).sum(),
        other_count: rest.len(),
        files,
        errors,
        locked: is_locked(root, protected),
        children,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn sizes_and_ordering() {
        let d = tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("big")).unwrap();
        std::fs::create_dir_all(d.path().join("small")).unwrap();
        std::fs::write(d.path().join("big/a.bin"), vec![0u8; 5000]).unwrap();
        std::fs::write(d.path().join("small/b.bin"), vec![0u8; 10]).unwrap();
        std::fs::write(d.path().join("loose.bin"), vec![0u8; 100]).unwrap();
        let top = top_children(d.path(), 10);
        assert_eq!(top[0].path, d.path().join("big"));
        assert_eq!(top[0].bytes, 5000);
        assert_eq!(top.len(), 3);
        assert_eq!(dir_size(d.path()), 5110);
    }

    #[test]
    fn scan_children_limits_locks_and_reports_rest() {
        let d = tempdir().unwrap();
        let root = d.path();
        let videos = root.join("Videos");
        std::fs::create_dir_all(videos.join("OBS")).unwrap();
        std::fs::write(videos.join("OBS/stream.mkv"), vec![0u8; 9000]).unwrap();
        for (i, n) in [3000usize, 2000, 1000, 10].iter().enumerate() {
            let dir = root.join(format!("d{i}"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("f"), vec![0u8; *n]).unwrap();
        }
        std::fs::create_dir_all(root.join("empty")).unwrap();
        let never = AtomicBool::new(false);
        let r = scan_children(root, 3, std::slice::from_ref(&videos), &never, |_| {}).unwrap();
        assert_eq!(r.total_bytes, 15010);
        assert_eq!(r.files, 5);
        let names: Vec<_> = r.children.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Videos", "d0", "d1"]);
        assert!(r.children[0].locked && !r.children[1].locked);
        assert!(r.children.iter().all(|c| c.is_dir));
        assert_eq!(r.other_bytes, 1010);
        assert_eq!(r.other_count, 3);
        assert!(!r.locked);

        let inner = scan_children(&videos, 10, std::slice::from_ref(&videos), &never, |_| {});
        let inner = inner.unwrap();
        assert!(inner.locked && inner.children[0].locked);
    }

    #[test]
    fn scan_children_cancels_and_reports_progress() {
        let d = tempdir().unwrap();
        for i in 0..(PROGRESS_EVERY * 2 + 5) {
            std::fs::write(d.path().join(format!("f{i}")), b"x").unwrap();
        }
        let never = AtomicBool::new(false);
        let mut calls = 0;
        let r = scan_children(d.path(), 5, &[], &never, |p| {
            calls += 1;
            assert!(p.files > 0);
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(r.children.len(), 5);
        assert!(r.children.iter().all(|c| !c.is_dir && c.files == 1));

        let cancel = AtomicBool::new(true);
        assert_eq!(
            scan_children(d.path(), 5, &[], &cancel, |_| {}).unwrap_err(),
            Cancelled
        );
    }
}
