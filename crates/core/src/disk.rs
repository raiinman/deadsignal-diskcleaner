//! Drive capacity and free space (Windows: `GetDiskFreeSpaceExW` via fs4;
//! elsewhere: `statvfs`).

use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct DriveSpace {
    pub root: PathBuf,
    pub total: u64,
    /// Space available to the current user.
    pub free: u64,
    pub used: u64,
    pub is_system: bool,
}

impl DriveSpace {
    pub fn free_percent(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.free as f64 * 100.0 / self.total as f64
        }
    }

    /// True when free space is under `percent` of the drive.
    pub fn is_low(&self, percent: u8) -> bool {
        self.total > 0 && self.free_percent() < f64::from(percent)
    }
}

/// Build a `DriveSpace` from raw numbers (kept separate for testing).
pub fn from_stats(root: PathBuf, total: u64, available: u64, is_system: bool) -> DriveSpace {
    let free = available.min(total);
    DriveSpace {
        root,
        total,
        free,
        used: total - free,
        is_system,
    }
}

pub fn space(root: &Path) -> std::io::Result<DriveSpace> {
    let st = fs4::statvfs(root)?;
    let is_system = crate::safety::normalize(root) == crate::safety::normalize(&system_root());
    Ok(from_stats(
        root.to_path_buf(),
        st.total_space(),
        st.available_space(),
        is_system,
    ))
}

/// `%SystemDrive%\` (normally `C:\`) on Windows, `/` elsewhere.
pub fn system_root() -> PathBuf {
    if cfg!(windows) {
        let d = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        PathBuf::from(format!("{}\\", d.trim_end_matches('\\')))
    } else {
        PathBuf::from("/")
    }
}

/// Mounted drives with their space. On Windows: every lettered drive that
/// answers (unready drives such as an empty card reader are skipped).
/// Elsewhere: just `/`.
pub fn list_drives() -> Vec<DriveSpace> {
    let roots: Vec<PathBuf> = if cfg!(windows) {
        (b'A'..=b'Z')
            .map(|c| PathBuf::from(format!("{}:\\", c as char)))
            .collect()
    } else {
        vec![PathBuf::from("/")]
    };
    let mut v: Vec<DriveSpace> = roots
        .iter()
        .filter_map(|r| space(r).ok())
        .filter(|d| d.total > 0)
        .collect();
    v.sort_by_key(|d| !d.is_system);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn low_space_threshold() {
        let tb2 = 2_000_000_000_000;
        let d = from_stats("C:\\".into(), tb2, tb2 / 20, true);
        assert_eq!(d.used, tb2 - tb2 / 20);
        assert!((d.free_percent() - 5.0).abs() < 1e-9);
        assert!(d.is_low(10));
        assert!(!d.is_low(5));
        let empty = from_stats("X:\\".into(), 0, 10, false);
        assert_eq!(empty.free, 0);
        assert!(!empty.is_low(10));
    }

    #[test]
    fn real_root_has_space() {
        let d = space(&system_root()).unwrap();
        assert!(d.total > 0 && d.used + d.free == d.total);
        assert!(d.is_system);
        assert!(list_drives().iter().any(|d| d.is_system));
    }
}
