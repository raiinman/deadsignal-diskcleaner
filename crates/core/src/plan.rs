use crate::config::{Config, home_dir};
use crate::safety::{self, Verdict};
use crate::scan::dir_size;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};
use walkdir::WalkDir;

pub const CAT_TEMP: &str = "temp";
pub const CAT_SHADER: &str = "shader-cache";
pub const CAT_CRASH: &str = "crash-dumps";
pub const CAT_BROWSER: &str = "browser-cache";
pub const CAT_PACKAGE: &str = "package-cache";
pub const CAT_DOWNLOADS: &str = "old-installers";
pub const CAT_DEV: &str = "dev-artifacts";

/// Installer/archive extensions considered for Downloads cleanup.
const DOWNLOAD_EXTS: &[&str] = &["exe", "msi", "zip", "7z", "rar", "iso", "msix"];
/// How deep under a project root to look for dev artifacts.
const DEV_MAX_DEPTH: usize = 6;
const DAY: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Risk {
    /// Regenerated automatically; selected by default.
    Safe,
    /// Probably junk, but the user should look first; never selected by default.
    Review,
}

impl fmt::Display for Risk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Risk::Safe => "safe",
            Risk::Review => "review",
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PlanItem {
    pub category: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub risk: Risk,
    pub selected_by_default: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Plan {
    /// Largest first.
    pub items: Vec<PlanItem>,
    pub total_bytes: u64,
}

impl Plan {
    /// Items grouped by category name (alphabetical), each group largest first.
    pub fn by_category(&self) -> BTreeMap<&str, Vec<&PlanItem>> {
        let mut m: BTreeMap<&str, Vec<&PlanItem>> = BTreeMap::new();
        for it in &self.items {
            m.entry(it.category.as_str()).or_default().push(it);
        }
        m
    }

    pub fn selected(&self) -> Vec<&PlanItem> {
        self.items
            .iter()
            .filter(|i| i.selected_by_default)
            .collect()
    }

    pub fn bytes_where(&self, f: impl Fn(&PlanItem) -> bool) -> u64 {
        self.items.iter().filter(|i| f(i)).map(|i| i.bytes).sum()
    }

    /// Every known category in display order (safe first), with its items,
    /// even when empty. Unknown categories are appended at the end.
    pub fn grouped(&self) -> Vec<CategoryGroup> {
        let mut groups: Vec<CategoryGroup> = CATEGORIES
            .iter()
            .map(|c| CategoryGroup {
                name: c.name.to_string(),
                label: c.label.to_string(),
                description: c.description.to_string(),
                risk: c.risk,
                selected_by_default: c.risk == Risk::Safe,
                bytes: 0,
                items: Vec::new(),
            })
            .collect();
        for it in &self.items {
            let g = match groups.iter().position(|g| g.name == it.category) {
                Some(i) => &mut groups[i],
                None => {
                    groups.push(CategoryGroup {
                        name: it.category.clone(),
                        label: it.category.clone(),
                        description: String::new(),
                        risk: it.risk,
                        selected_by_default: it.selected_by_default,
                        bytes: 0,
                        items: Vec::new(),
                    });
                    groups.last_mut().expect("just pushed")
                }
            };
            g.bytes += it.bytes;
            g.items.push(it.clone());
        }
        groups
    }
}

pub struct CategoryInfo {
    pub name: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub risk: Risk,
}

/// Display metadata for each category, in display order.
pub const CATEGORIES: &[CategoryInfo] = &[
    CategoryInfo {
        name: CAT_TEMP,
        label: "Temporary files",
        description: "Contents of %TEMP%. Apps recreate what they need.",
        risk: Risk::Safe,
    },
    CategoryInfo {
        name: CAT_SHADER,
        label: "Shader caches",
        description: "NVIDIA, AMD and DirectX shader caches. Games rebuild them; the first launch after cleaning may stutter briefly.",
        risk: Risk::Safe,
    },
    CategoryInfo {
        name: CAT_CRASH,
        label: "Crash dumps",
        description: "Leftover crash reports in %LOCALAPPDATA%\\CrashDumps.",
        risk: Risk::Safe,
    },
    CategoryInfo {
        name: CAT_BROWSER,
        label: "Browser caches",
        description: "Chrome and Edge cache folders (Default profile). Logins and history are not touched.",
        risk: Risk::Safe,
    },
    CategoryInfo {
        name: CAT_PACKAGE,
        label: "Package caches",
        description: "npm, pip and Cargo download caches. Re-downloaded on the next install.",
        risk: Risk::Review,
    },
    CategoryInfo {
        name: CAT_DOWNLOADS,
        label: "Old installers in Downloads",
        description: "exe, msi, zip, 7z, rar, iso and msix files in Downloads older than the age you set.",
        risk: Risk::Review,
    },
    CategoryInfo {
        name: CAT_DEV,
        label: "Stale dev build folders",
        description: "node_modules, target, venv, dist and build folders in project roots that have not been touched for a while and have no uncommitted git changes.",
        risk: Risk::Review,
    },
];

#[derive(Debug, Clone, Serialize)]
pub struct CategoryGroup {
    pub name: String,
    pub label: String,
    pub description: String,
    pub risk: Risk,
    pub selected_by_default: bool,
    pub bytes: u64,
    pub items: Vec<PlanItem>,
}

/// Resolved base directories. Injectable so tests can point at temp dirs.
#[derive(Debug, Clone)]
pub struct Env {
    /// %TEMP%
    pub temp: Option<PathBuf>,
    /// %LOCALAPPDATA%
    pub local_app_data: Option<PathBuf>,
    /// %USERPROFILE%; also used as the home dir for safety checks.
    pub user_profile: PathBuf,
}

impl Env {
    pub fn from_system() -> Env {
        let var = |k: &str| {
            std::env::var_os(k)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Env {
            temp: var("TEMP").or_else(|| var("TMP")),
            local_app_data: var("LOCALAPPDATA"),
            user_profile: home_dir(),
        }
    }
}

struct Builder<'a> {
    cfg: &'a Config,
    env: &'a Env,
    items: Vec<PlanItem>,
}

impl Builder<'_> {
    /// Add an item if it passes the safety check; blocked paths are dropped.
    fn push(&mut self, category: &str, path: PathBuf, bytes: u64, risk: Risk) {
        if safety::check(&path, &self.cfg.protected, &self.env.user_profile) != Verdict::Allowed {
            return;
        }
        self.items.push(PlanItem {
            category: category.into(),
            path,
            bytes,
            risk,
            selected_by_default: risk == Risk::Safe,
        });
    }

    /// One item per direct child of `root` (never the root itself). Missing roots are skipped.
    fn children(&mut self, category: &str, root: &Path, risk: Risk) {
        let Ok(rd) = std::fs::read_dir(root) else {
            return;
        };
        for e in rd.filter_map(Result::ok) {
            let Ok(ft) = e.file_type() else { continue };
            let bytes = if ft.is_dir() {
                dir_size(&e.path())
            } else {
                e.metadata().map(|m| m.len()).unwrap_or(0)
            };
            self.push(category, e.path(), bytes, risk);
        }
    }

    fn is_protected(&self, p: &Path) -> bool {
        let p = safety::normalize(p);
        self.cfg
            .protected
            .iter()
            .any(|x| p.starts_with(safety::normalize(x)))
    }
}

/// Build a dry-run cleanup plan. Nothing is deleted here.
pub fn build_plan(cfg: &Config, env: &Env) -> Plan {
    let mut b = Builder {
        cfg,
        env,
        items: Vec::new(),
    };

    // SAFE: regenerated caches and leftovers.
    if let Some(t) = &env.temp {
        b.children(CAT_TEMP, t, Risk::Safe);
    }
    if let Some(lad) = &env.local_app_data {
        for rel in [
            "NVIDIA/DXCache",
            "NVIDIA/GLCache",
            "D3DSCache",
            "AMD/DxCache",
        ] {
            b.children(CAT_SHADER, &lad.join(rel), Risk::Safe);
        }
        b.children(CAT_CRASH, &lad.join("CrashDumps"), Risk::Safe);
        for browser in ["Google/Chrome", "Microsoft/Edge"] {
            let profile = lad.join(browser).join("User Data").join("Default");
            for cache in ["Cache", "Code Cache", "GPUCache"] {
                b.children(CAT_BROWSER, &profile.join(cache), Risk::Safe);
            }
        }
        // REVIEW: package caches are re-downloadable but cost bandwidth/time.
        b.children(CAT_PACKAGE, &lad.join("npm-cache"), Risk::Review);
        b.children(CAT_PACKAGE, &lad.join("pip").join("Cache"), Risk::Review);
    }
    b.children(
        CAT_PACKAGE,
        &env.user_profile.join(".cargo/registry/cache"),
        Risk::Review,
    );

    if let Some(dl) = &cfg.downloads_dir {
        old_downloads(&mut b, dl);
    }
    for root in &cfg.project_roots {
        dev_artifacts(&mut b, root);
    }

    let mut items = b.items;
    items.sort_by(|a, b| b.bytes.cmp(&a.bytes).then_with(|| a.path.cmp(&b.path)));
    let total_bytes = items.iter().map(|i| i.bytes).sum();
    Plan { items, total_bytes }
}

fn older_than(mtime: SystemTime, days: u64) -> bool {
    SystemTime::now()
        .duration_since(mtime)
        .is_ok_and(|age| age > Duration::from_secs(days * DAY))
}

/// Installers/archives directly in Downloads (files only) older than the configured age.
fn old_downloads(b: &mut Builder, dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.filter_map(Result::ok) {
        if !e.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        let path = e.path();
        let ext_ok = path
            .extension()
            .and_then(|x| x.to_str())
            .is_some_and(|x| DOWNLOAD_EXTS.contains(&x.to_ascii_lowercase().as_str()));
        let Ok(md) = e.metadata() else { continue };
        if ext_ok
            && md
                .modified()
                .is_ok_and(|m| older_than(m, b.cfg.downloads_min_age_days))
        {
            b.push(CAT_DOWNLOADS, path, md.len(), Risk::Review);
        }
    }
}

/// Artifact dir name, the sibling marker files (any one suffices), and an
/// optional file that must exist inside the artifact dir.
const DEV_RULES: &[(&str, &[&str], Option<&str>)] = &[
    ("node_modules", &["package.json"], None),
    ("target", &["Cargo.toml"], None),
    (
        ".venv",
        &["pyproject.toml", "requirements.txt", "setup.py", "Pipfile"],
        Some("pyvenv.cfg"),
    ),
    (
        "venv",
        &["pyproject.toml", "requirements.txt", "setup.py", "Pipfile"],
        Some("pyvenv.cfg"),
    ),
    (
        "dist",
        &["package.json", "pyproject.toml", "setup.py"],
        None,
    ),
    (
        "build",
        &[
            "package.json",
            "pyproject.toml",
            "setup.py",
            "build.gradle",
            "build.gradle.kts",
        ],
        None,
    ),
    (".next", &["package.json"], None),
    (
        ".gradle",
        &[
            "build.gradle",
            "build.gradle.kts",
            "settings.gradle",
            "settings.gradle.kts",
        ],
        None,
    ),
];

/// If `dir` is a recognised artifact, return the marker files present beside it.
fn match_artifact(dir: &Path) -> Option<Vec<PathBuf>> {
    let name = dir.file_name()?.to_str()?;
    let project = dir.parent()?;
    let (_, markers, inner) = DEV_RULES.iter().find(|(n, _, _)| *n == name)?;
    if inner.is_some_and(|f| !dir.join(f).is_file()) {
        return None;
    }
    let found: Vec<PathBuf> = markers
        .iter()
        .map(|m| project.join(m))
        .filter(|p| p.is_file())
        .collect();
    (!found.is_empty()).then_some(found)
}

/// Newest mtime among the markers and the project's .git/HEAD / .git/index.
fn project_last_touched(project: &Path, markers: &[PathBuf]) -> Option<SystemTime> {
    markers
        .iter()
        .cloned()
        .chain([project.join(".git/HEAD"), project.join(".git/index")])
        .filter_map(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
        .max()
}

/// True if `git status --porcelain` reports changes. If git is missing or the
/// dir isn't a repo, the project is treated as clean.
fn git_dirty(project: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(project)
        .args(["status", "--porcelain"])
        .output()
        .is_ok_and(|o| o.status.success() && !o.stdout.iter().all(u8::is_ascii_whitespace))
}

/// Stale build/dependency dirs under a project root. Does not follow symlinks
/// and never descends into a matched artifact, .git or protected dir.
fn dev_artifacts(b: &mut Builder, root: &Path) {
    let mut it = WalkDir::new(root)
        .follow_links(false)
        .max_depth(DEV_MAX_DEPTH)
        .into_iter();
    while let Some(Ok(e)) = it.next() {
        if !e.file_type().is_dir() {
            continue;
        }
        let path = e.path();
        if e.file_name() == ".git" || b.is_protected(path) {
            it.skip_current_dir();
            continue;
        }
        let Some(markers) = match_artifact(path) else {
            continue;
        };
        it.skip_current_dir();
        let project = path.parent().expect("artifact has a parent");
        let stale = project_last_touched(project, &markers)
            .is_some_and(|t| older_than(t, b.cfg.dev_min_age_days));
        if stale && !git_dirty(project) {
            b.push(CAT_DEV, path.to_path_buf(), dir_size(path), Risk::Review);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, File};
    use tempfile::{TempDir, tempdir};

    struct Fixture {
        _dir: TempDir,
        home: PathBuf,
        env: Env,
        cfg: Config,
    }

    /// A fake user profile with TEMP/LOCALAPPDATA inside it and nothing configured.
    fn fixture() -> Fixture {
        let dir = tempdir().unwrap();
        let home = dir.path().join("streamer");
        let lad = home.join("AppData/Local");
        fs::create_dir_all(&lad).unwrap();
        let env = Env {
            temp: Some(lad.join("Temp")),
            local_app_data: Some(lad),
            user_profile: home.clone(),
        };
        let cfg = Config {
            protected: vec![home.join("Videos")],
            project_roots: vec![],
            downloads_dir: Some(home.join("Downloads")),
            downloads_min_age_days: 30,
            dev_min_age_days: 60,
            low_space_percent: 10,
        };
        Fixture {
            _dir: dir,
            home,
            env,
            cfg,
        }
    }

    fn write(p: &Path, len: usize) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, vec![0u8; len]).unwrap();
    }

    fn age(p: &Path, days: u64) {
        let t = SystemTime::now() - Duration::from_secs(days * DAY);
        File::options()
            .write(true)
            .open(p)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    fn paths(plan: &Plan, cat: &str) -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = plan
            .items
            .iter()
            .filter(|i| i.category == cat)
            .map(|i| i.path.clone())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn empty_env_is_empty_plan() {
        let f = fixture();
        let plan = build_plan(&f.cfg, &f.env);
        assert!(plan.items.is_empty());
        assert_eq!(plan.total_bytes, 0);
    }

    #[test]
    fn safe_categories_one_item_per_child() {
        let f = fixture();
        let lad = f.env.local_app_data.clone().unwrap();
        let tmp = f.env.temp.clone().unwrap();
        write(&tmp.join("a.tmp"), 10);
        write(&tmp.join("sub/b.tmp"), 20);
        write(&lad.join("NVIDIA/DXCache/x.bin"), 300);
        write(&lad.join("NVIDIA/GLCache/g/y.bin"), 5);
        write(&lad.join("D3DSCache/abc/z"), 7);
        write(&lad.join("AMD/DxCache/w"), 9);
        write(&lad.join("CrashDumps/game.dmp"), 1000);
        write(
            &lad.join("Google/Chrome/User Data/Default/Cache/Cache_Data/f_1"),
            50,
        );
        write(
            &lad.join("Microsoft/Edge/User Data/Default/Code Cache/js/f"),
            40,
        );
        write(
            &lad.join("Microsoft/Edge/User Data/Default/GPUCache/data_0"),
            30,
        );
        // Not a cache dir: must not be touched.
        write(&lad.join("Google/Chrome/User Data/Default/Bookmarks"), 1);

        let plan = build_plan(&f.cfg, &f.env);
        assert_eq!(
            paths(&plan, CAT_TEMP),
            vec![tmp.join("a.tmp"), tmp.join("sub")]
        );
        assert_eq!(paths(&plan, CAT_SHADER).len(), 4);
        assert_eq!(
            paths(&plan, CAT_CRASH),
            vec![lad.join("CrashDumps/game.dmp")]
        );
        assert_eq!(paths(&plan, CAT_BROWSER).len(), 3);
        assert!(
            plan.items
                .iter()
                .all(|i| i.risk == Risk::Safe && i.selected_by_default)
        );
        assert!(
            !plan
                .items
                .iter()
                .any(|i| i.path == tmp || i.path.ends_with("Bookmarks"))
        );
        // Sizes recurse into dirs; sorted largest first.
        let sub = plan
            .items
            .iter()
            .find(|i| i.path == tmp.join("sub"))
            .unwrap();
        assert_eq!(sub.bytes, 20);
        assert_eq!(plan.items[0].bytes, 1000);
        assert!(plan.items.windows(2).all(|w| w[0].bytes >= w[1].bytes));
        assert_eq!(
            plan.total_bytes,
            10 + 20 + 300 + 5 + 7 + 9 + 1000 + 50 + 40 + 30
        );
        assert_eq!(plan.selected().len(), plan.items.len());
    }

    #[test]
    fn package_caches_are_review() {
        let f = fixture();
        let lad = f.env.local_app_data.clone().unwrap();
        write(&lad.join("npm-cache/_cacache/x"), 10);
        write(&lad.join("pip/Cache/http/y"), 10);
        write(
            &f.home
                .join(".cargo/registry/cache/index.crates.io-1/serde.crate"),
            10,
        );
        write(&f.home.join(".cargo/registry/src/keep.rs"), 10);
        let plan = build_plan(&f.cfg, &f.env);
        assert_eq!(paths(&plan, CAT_PACKAGE).len(), 3);
        assert!(
            plan.items
                .iter()
                .all(|i| i.risk == Risk::Review && !i.selected_by_default)
        );
        assert!(plan.selected().is_empty());
        assert!(
            !plan
                .items
                .iter()
                .any(|i| i.path.starts_with(f.home.join(".cargo/registry/src")))
        );
    }

    #[test]
    fn protected_videos_never_planned() {
        let mut f = fixture();
        let videos = f.home.join("Videos");
        write(&videos.join("OBS/stream.mkv"), 5000);
        write(&videos.join("node_modules/x"), 10);
        write(&videos.join("package.json"), 1);
        // Point every source at the protected dir; nothing may come out.
        f.env.temp = Some(videos.clone());
        f.cfg.downloads_dir = Some(videos.clone());
        f.cfg.downloads_min_age_days = 0;
        f.cfg.dev_min_age_days = 0;
        f.cfg.project_roots = vec![f.home.clone(), videos.clone()];
        age(&videos.join("package.json"), 400);
        let plan = build_plan(&f.cfg, &f.env);
        assert!(
            !plan.items.iter().any(|i| i.path.starts_with(&videos)),
            "planned protected path: {:?}",
            plan.items
        );
    }

    #[test]
    fn downloads_filtered_by_age_and_ext() {
        let f = fixture();
        let dl = f.home.join("Downloads");
        for (name, days) in [
            ("old.exe", 90),
            ("OLD.ZIP", 90),
            ("new.msi", 5),
            ("old.pdf", 90),
            ("sub/old.iso", 90),
        ] {
            write(&dl.join(name), 100);
            age(&dl.join(name), days);
        }
        let plan = build_plan(&f.cfg, &f.env);
        assert_eq!(
            paths(&plan, CAT_DOWNLOADS),
            vec![dl.join("OLD.ZIP"), dl.join("old.exe")]
        );
        assert!(plan.selected().is_empty());
    }

    /// Creates `<root>/<name>` with a marker and an artifact dir, aged `days`.
    fn project(root: &Path, name: &str, marker: &str, artifact: &str, days: u64) -> PathBuf {
        let p = root.join(name);
        write(&p.join(marker), 1);
        write(&p.join(artifact).join("blob"), 100);
        age(&p.join(marker), days);
        p
    }

    #[test]
    fn dev_artifacts_staleness_and_markers() {
        let mut f = fixture();
        let src = f.home.join("source");
        f.cfg.project_roots = vec![src.clone()];
        let old_node = project(&src, "web", "package.json", "node_modules", 120);
        let old_rust = project(&src, "deep/a/b/rs", "Cargo.toml", "target", 120);
        let fresh = project(&src, "fresh", "Cargo.toml", "target", 1);
        // No marker: a folder that merely happens to be called "target".
        write(&src.join("notes/target/keep.txt"), 100);
        // Nested node_modules inside a matched one must not be listed separately.
        write(&old_node.join("node_modules/dep/package.json"), 1);
        write(&old_node.join("node_modules/dep/node_modules/x/y"), 1);
        // venv needs pyvenv.cfg inside.
        let py = project(&src, "py", "requirements.txt", ".venv", 120);
        write(&py.join(".venv/pyvenv.cfg"), 1);
        let fake = project(&src, "fakevenv", "requirements.txt", "venv", 120);
        // Recent git activity keeps an otherwise stale project.
        let gitted = project(&src, "active", "package.json", "dist", 120);
        write(&gitted.join(".git/HEAD"), 1);

        let plan = build_plan(&f.cfg, &f.env);
        assert_eq!(
            paths(&plan, CAT_DEV),
            vec![
                old_rust.join("target"),
                py.join(".venv"),
                old_node.join("node_modules")
            ]
        );
        assert!(
            !plan
                .items
                .iter()
                .any(|i| i.path.starts_with(&fresh) || i.path.starts_with(&fake))
        );
        assert!(plan.items.iter().all(|i| i.risk == Risk::Review));
    }

    #[test]
    fn dev_walk_depth_is_bounded() {
        let mut f = fixture();
        let src = f.home.join("source");
        f.cfg.project_roots = vec![src.clone()];
        project(&src, "1/2/3/4/5/6/7", "package.json", "node_modules", 120);
        assert!(build_plan(&f.cfg, &f.env).items.is_empty());
    }

    fn git(dir: &Path, args: &[&str]) -> bool {
        Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .is_ok_and(|o| o.status.success())
    }

    #[test]
    fn dirty_git_project_skipped() {
        let mut f = fixture();
        let src = f.home.join("source");
        f.cfg.project_roots = vec![src.clone()];
        let clean = project(&src, "clean", "Cargo.toml", "target", 120);
        let dirty = project(&src, "dirty", "Cargo.toml", "target", 120);
        for p in [&clean, &dirty] {
            if !git(p, &["init", "-q"]) {
                eprintln!("git unavailable; skipping");
                return;
            }
            fs::write(p.join(".gitignore"), "target/\n").unwrap();
            let id = ["-c", "user.name=t", "-c", "user.email=t@t"];
            assert!(git(p, &[&id[..], &["add", "-A"]].concat()));
            assert!(git(p, &[&id[..], &["commit", "-qm", "init"]].concat()));
            // Backdate git metadata so only dirtiness decides.
            age(&p.join(".git/HEAD"), 120);
            age(&p.join(".git/index"), 120);
            age(&p.join("Cargo.toml"), 120);
        }
        fs::write(dirty.join("uncommitted.rs"), "fn main() {}").unwrap();
        let plan = build_plan(&f.cfg, &f.env);
        assert_eq!(paths(&plan, CAT_DEV), vec![clean.join("target")]);
    }

    #[test]
    fn risk_display_and_grouping() {
        assert_eq!(Risk::Safe.to_string(), "safe");
        assert_eq!(Risk::Review.to_string(), "review");
        let f = fixture();
        write(&f.env.temp.clone().unwrap().join("a"), 5);
        write(
            &f.env.local_app_data.clone().unwrap().join("CrashDumps/b"),
            6,
        );
        let plan = build_plan(&f.cfg, &f.env);
        let g = plan.by_category();
        assert_eq!(
            g.keys().copied().collect::<Vec<_>>(),
            vec![CAT_CRASH, CAT_TEMP]
        );
        assert_eq!(plan.bytes_where(|i| i.category == CAT_TEMP), 5);
        let json = serde_json::to_string(&plan).unwrap();
        assert!(json.contains("\"risk\":\"Safe\""));
    }

    #[test]
    fn grouped_lists_every_category_in_order() {
        let f = fixture();
        write(&f.env.temp.clone().unwrap().join("a"), 5);
        write(&f.env.temp.clone().unwrap().join("b"), 7);
        write(
            &f.env
                .local_app_data
                .clone()
                .unwrap()
                .join("npm-cache/_cacache/x"),
            3,
        );
        let mut plan = build_plan(&f.cfg, &f.env);
        plan.items.push(PlanItem {
            category: "future".into(),
            path: f.home.join("x"),
            bytes: 1,
            risk: Risk::Review,
            selected_by_default: false,
        });
        let g = plan.grouped();
        let names: Vec<_> = g.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(
            names,
            [
                CAT_TEMP,
                CAT_SHADER,
                CAT_CRASH,
                CAT_BROWSER,
                CAT_PACKAGE,
                CAT_DOWNLOADS,
                CAT_DEV,
                "future"
            ]
        );
        assert_eq!((g[0].bytes, g[0].items.len()), (12, 2));
        assert!(g[0].selected_by_default && g[0].risk == Risk::Safe);
        assert_eq!(g[4].bytes, 3);
        assert!(!g[4].selected_by_default);
        assert!(g[1].items.is_empty());
        assert_eq!(g[7].label, "future");
    }
}
