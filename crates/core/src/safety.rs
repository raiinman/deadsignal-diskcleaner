use std::path::{Component, Path, PathBuf};

/// Hard-blocked locations on top of the user's protected list.
const SYSTEM_BLOCKED: &[&str] = &[
    "C:\\Windows",
    "C:\\Program Files",
    "C:\\Program Files (x86)",
    "C:\\ProgramData",
    "/bin",
    "/boot",
    "/etc",
    "/lib",
    "/sbin",
    "/usr",
    "/var",
    "/sys",
    "/proc",
    "/dev",
];

#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    Allowed,
    Blocked(String),
}

/// Decide whether `target` may be deleted. A target is blocked if it is a
/// filesystem root, a user's home directory, inside or equal to a protected
/// path, or an ancestor of one (deleting it would take the protected path too).
pub fn check(target: &Path, protected: &[PathBuf], home: &Path) -> Verdict {
    let t = normalize(target);
    if t.parent().is_none() || t.components().count() <= 1 {
        return Verdict::Blocked("filesystem root".into());
    }
    if t == normalize(home) || is_ancestor(&t, &normalize(home)) {
        return Verdict::Blocked("home directory".into());
    }
    for p in protected.iter().map(|p| normalize(p)) {
        if t.starts_with(&p) {
            return Verdict::Blocked(format!("inside protected path {}", p.display()));
        }
        if p.starts_with(&t) {
            return Verdict::Blocked(format!("contains protected path {}", p.display()));
        }
    }
    for s in SYSTEM_BLOCKED.iter().map(|s| normalize(Path::new(s))) {
        if t.starts_with(&s) {
            return Verdict::Blocked(format!("system path {}", s.display()));
        }
    }
    Verdict::Allowed
}

fn is_ancestor(a: &Path, of: &Path) -> bool {
    of.starts_with(a)
}

/// Lexical normalisation: resolves `.`/`..`, canonicalises when the path
/// exists (so symlinks cannot smuggle a target past the check), and compares
/// case-insensitively on Windows-style drive paths.
pub fn normalize(p: &Path) -> PathBuf {
    let base = std::fs::canonicalize(p).unwrap_or_else(|_| lexical(p));
    let s = base.to_string_lossy().replace("\\\\?\\", "");
    if s.contains(':') && s.contains('\\') {
        PathBuf::from(s.to_lowercase())
    } else {
        PathBuf::from(s)
    }
}

fn lexical(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn blocks_protected_and_ancestors() {
        let home = tempdir().unwrap();
        let videos = home.path().join("Videos");
        std::fs::create_dir_all(videos.join("rec")).unwrap();
        let prot = vec![videos.clone()];
        assert!(matches!(
            check(&videos, &prot, home.path()),
            Verdict::Blocked(_)
        ));
        assert!(matches!(
            check(&videos.join("rec"), &prot, home.path()),
            Verdict::Blocked(_)
        ));
        assert!(matches!(
            check(home.path(), &prot, home.path()),
            Verdict::Blocked(_)
        ));
        assert!(matches!(
            check(Path::new("/"), &prot, home.path()),
            Verdict::Blocked(_)
        ));
    }

    #[test]
    fn blocks_dotdot_escape_into_protected() {
        let home = tempdir().unwrap();
        let videos = home.path().join("Videos");
        let cache = home.path().join("cache");
        std::fs::create_dir_all(&videos).unwrap();
        std::fs::create_dir_all(&cache).unwrap();
        let sneaky = cache.join("..").join("Videos");
        assert!(matches!(
            check(&sneaky, &[videos], home.path()),
            Verdict::Blocked(_)
        ));
    }

    #[test]
    fn allows_unrelated_path() {
        let home = tempdir().unwrap();
        let cache = home.path().join("cache");
        std::fs::create_dir_all(&cache).unwrap();
        assert_eq!(
            check(&cache, &[home.path().join("Videos")], home.path()),
            Verdict::Allowed
        );
    }
}
