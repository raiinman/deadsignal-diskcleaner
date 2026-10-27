use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Folders under the home directory that are always protected, whatever the
/// config file says. `Videos` holds OBS recordings.
pub const LOCKED_PROTECTED: &[&str] = &["Videos"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Paths that are never scanned for cleanup or deleted (and nothing inside them).
    pub protected: Vec<PathBuf>,
    /// Folders searched for stale dev artifacts (node_modules, target, ...).
    pub project_roots: Vec<PathBuf>,
    pub downloads_dir: Option<PathBuf>,
    pub downloads_min_age_days: u64,
    pub dev_min_age_days: u64,
    /// The GUI warns when free space on the system drive drops below this
    /// percentage of the drive.
    pub low_space_percent: u8,
}

impl Default for Config {
    fn default() -> Self {
        let home = home_dir();
        Config {
            protected: vec![
                home.join("Videos"),
                home.join("Documents"),
                home.join("Desktop"),
                home.join("Pictures"),
                home.join("Saved Games"),
            ],
            project_roots: vec![
                home.join("source"),
                home.join("projects"),
                home.join("code"),
            ],
            downloads_dir: Some(home.join("Downloads")),
            downloads_min_age_days: 30,
            dev_min_age_days: 60,
            low_space_percent: 10,
        }
    }
}

/// Age thresholds are clamped to this range (days).
pub const MIN_AGE_DAYS: u64 = 1;
pub const MAX_AGE_DAYS: u64 = 3650;

impl Config {
    pub fn load(path: &Path) -> std::io::Result<Config> {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e),
        }
    }

    /// Write the config, creating its parent directory if needed.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let s = serde_json::to_string_pretty(self).expect("config serializes");
        std::fs::write(path, s)
    }

    /// Protected paths that the user cannot remove (see [`LOCKED_PROTECTED`]).
    pub fn locked_protected(home: &Path) -> Vec<PathBuf> {
        LOCKED_PROTECTED.iter().map(|n| home.join(n)).collect()
    }

    /// Repair a config that came from a file or the GUI. Re-adds the locked
    /// protected folders, drops relative and duplicate paths, refuses
    /// filesystem roots and the home directory as project roots, and clamps
    /// the numeric settings. Returns a message for every change it made.
    pub fn sanitize(&mut self, home: &Path) -> Vec<String> {
        let mut warnings = Vec::new();

        let mut protected = Vec::new();
        for p in std::mem::take(&mut self.protected) {
            if !p.is_absolute() {
                warnings.push(format!("ignored relative protected path {}", p.display()));
            } else if !contains_path(&protected, &p) {
                protected.push(p);
            }
        }
        for locked in Config::locked_protected(home) {
            if !contains_path(&protected, &locked) {
                warnings.push(format!(
                    "{} is always protected and was re-added",
                    locked.display()
                ));
                protected.insert(0, locked);
            }
        }
        self.protected = protected;

        let mut roots = Vec::new();
        for r in std::mem::take(&mut self.project_roots) {
            let n = crate::safety::normalize(&r);
            if !r.is_absolute() {
                warnings.push(format!("ignored relative project root {}", r.display()));
            } else if n.parent().is_none() || n.components().count() <= 1 {
                warnings.push(format!(
                    "ignored project root {}: a whole drive is too broad",
                    r.display()
                ));
            } else if n == crate::safety::normalize(home) {
                warnings.push(format!(
                    "ignored project root {}: pick a folder inside your home directory",
                    r.display()
                ));
            } else if !contains_path(&roots, &r) {
                roots.push(r);
            }
        }
        self.project_roots = roots;

        if self
            .downloads_dir
            .as_ref()
            .is_some_and(|d| !d.is_absolute())
        {
            warnings.push("ignored relative downloads folder".into());
            self.downloads_dir = None;
        }

        for (name, v) in [
            ("downloads_min_age_days", &mut self.downloads_min_age_days),
            ("dev_min_age_days", &mut self.dev_min_age_days),
        ] {
            let c = (*v).clamp(MIN_AGE_DAYS, MAX_AGE_DAYS);
            if c != *v {
                warnings.push(format!("{name} clamped to {c}"));
                *v = c;
            }
        }
        let c = self.low_space_percent.clamp(1, 50);
        if c != self.low_space_percent {
            warnings.push(format!("low_space_percent clamped to {c}"));
            self.low_space_percent = c;
        }
        warnings
    }
}

fn contains_path(list: &[PathBuf], p: &Path) -> bool {
    let n = crate::safety::normalize(p);
    list.iter().any(|x| crate::safety::normalize(x) == n)
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// `%APPDATA%\diskcleaner\config.json` on Windows,
/// `~/.config/diskcleaner/config.json` elsewhere. Shared by the CLI and GUI.
pub fn default_path() -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.unwrap_or_default()
        .join("diskcleaner")
        .join("config.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn cfg(home: &Path) -> Config {
        Config {
            protected: vec![home.join("Documents")],
            project_roots: vec![home.join("source")],
            downloads_dir: Some(home.join("Downloads")),
            downloads_min_age_days: 30,
            dev_min_age_days: 60,
            low_space_percent: 10,
        }
    }

    #[test]
    fn sanitize_readds_videos_and_dedupes() {
        let d = tempdir().unwrap();
        let home = d.path();
        let mut c = cfg(home);
        c.protected.push(home.join("Documents"));
        c.protected.push(PathBuf::from("relative"));
        let w = c.sanitize(home);
        assert_eq!(
            c.protected,
            vec![home.join("Videos"), home.join("Documents")]
        );
        assert_eq!(w.len(), 2, "{w:?}");
        // Idempotent.
        assert!(c.sanitize(home).is_empty());
    }

    #[test]
    fn sanitize_rejects_broad_project_roots_and_clamps() {
        let d = tempdir().unwrap();
        let home = d.path();
        let mut c = cfg(home);
        c.project_roots = vec![
            PathBuf::from("/"),
            home.to_path_buf(),
            home.join("source"),
            home.join("source"),
            PathBuf::from("code"),
        ];
        c.dev_min_age_days = 0;
        c.downloads_min_age_days = 1_000_000;
        c.low_space_percent = 0;
        let w = c.sanitize(home);
        assert_eq!(c.project_roots, vec![home.join("source")]);
        assert_eq!(c.dev_min_age_days, MIN_AGE_DAYS);
        assert_eq!(c.downloads_min_age_days, MAX_AGE_DAYS);
        assert_eq!(c.low_space_percent, 1);
        assert_eq!(w.len(), 7, "{w:?}");
    }

    #[test]
    fn old_config_files_get_new_defaults_and_roundtrip() {
        let d = tempdir().unwrap();
        let path = d.path().join("sub/config.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"dev_min_age_days": 90}"#).unwrap();
        let c = Config::load(&path).unwrap();
        assert_eq!(c.dev_min_age_days, 90);
        assert_eq!(c.low_space_percent, 10);
        let path2 = d.path().join("new/dir/config.json");
        c.save(&path2).unwrap();
        assert_eq!(Config::load(&path2).unwrap(), c);
    }
}
