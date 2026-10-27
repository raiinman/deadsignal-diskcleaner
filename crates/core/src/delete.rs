use crate::plan::{Plan, PlanItem};
use crate::safety::{self, Verdict};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Send to the Recycle Bin (default; recoverable).
    Trash,
    /// Delete immediately.
    Permanent,
}

impl Mode {
    /// Strict parse for values coming from the GUI. Anything unknown is an
    /// error, never a silent fallback to permanent.
    pub fn parse(s: &str) -> Result<Mode, String> {
        match s {
            "trash" | "recycle" => Ok(Mode::Trash),
            "permanent" => Ok(Mode::Permanent),
            other => Err(format!("unknown delete mode {other:?}")),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    pub category: String,
    pub path: PathBuf,
    pub bytes: u64,
    pub error: Option<String>,
    /// Normalised path taken before deletion, used to find the item in the
    /// Recycle Bin afterwards.
    #[serde(skip)]
    pub canonical: PathBuf,
}

/// Delete the given items, re-checking safety right before each deletion.
pub fn execute(items: &[PlanItem], mode: Mode, protected: &[PathBuf], home: &Path) -> Vec<Outcome> {
    items
        .iter()
        .map(|it| {
            let canonical = safety::normalize(&it.path);
            let error = match safety::check(&it.path, protected, home) {
                Verdict::Blocked(why) => Some(format!("blocked: {why}")),
                Verdict::Allowed => remove(&it.path, mode).err(),
            };
            Outcome {
                category: it.category.clone(),
                path: it.path.clone(),
                bytes: it.bytes,
                error,
                canonical,
            }
        })
        .collect()
}

fn remove(path: &Path, mode: Mode) -> Result<(), String> {
    match mode {
        Mode::Trash => trash::delete(path).map_err(|e| e.to_string()),
        Mode::Permanent => {
            let md = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
            if md.is_dir() {
                std::fs::remove_dir_all(path).map_err(|e| e.to_string())
            } else {
                std::fs::remove_file(path).map_err(|e| e.to_string())
            }
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Rejected {
    pub path: PathBuf,
    pub reason: String,
}

/// Map paths requested by a UI onto items of `plan`. Only exact plan items
/// are accepted (compared after normalisation); anything else, including
/// children or parents of plan items, is rejected. Duplicates are dropped.
/// The returned items are the plan's own copies, never the caller's paths.
pub fn resolve_selection(plan: &Plan, requested: &[PathBuf]) -> (Vec<PlanItem>, Vec<Rejected>) {
    let index: Vec<(PathBuf, &PlanItem)> = plan
        .items
        .iter()
        .map(|i| (safety::normalize(&i.path), i))
        .collect();
    let mut chosen: Vec<PlanItem> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut rejected = Vec::new();
    for p in requested {
        let n = safety::normalize(p);
        if seen.contains(&n) {
            continue;
        }
        seen.push(n.clone());
        match index.iter().find(|(k, _)| *k == n) {
            Some((_, it)) => chosen.push((*it).clone()),
            None => rejected.push(Rejected {
                path: p.clone(),
                reason: "not in the current cleanup plan".into(),
            }),
        }
    }
    (chosen, rejected)
}

#[derive(Debug, Clone, Serialize)]
pub struct CleanReport {
    pub mode: Mode,
    pub outcomes: Vec<Outcome>,
    pub rejected: Vec<Rejected>,
    /// Bytes of items that were removed. For `Trash` the space is only
    /// released once the Recycle Bin is emptied.
    pub freed_bytes: u64,
    pub ok_count: usize,
    pub failed_count: usize,
}

/// Validate `requested` against `plan`, then delete. Each accepted item still
/// goes through `safety::check` again inside [`execute`].
pub fn clean_selected(
    plan: &Plan,
    requested: &[PathBuf],
    mode: Mode,
    protected: &[PathBuf],
    home: &Path,
) -> CleanReport {
    let (items, rejected) = resolve_selection(plan, requested);
    let outcomes = execute(&items, mode, protected, home);
    let ok: Vec<&Outcome> = outcomes.iter().filter(|o| o.error.is_none()).collect();
    CleanReport {
        mode,
        freed_bytes: ok.iter().map(|o| o.bytes).sum(),
        ok_count: ok.len(),
        failed_count: outcomes.len() - ok.len(),
        outcomes,
        rejected,
    }
}

/// Indices of Recycle Bin entries `(original path, unix time deleted)` that
/// are one of `recycled` (normalised paths) and were deleted at or after
/// `since` (unix seconds, with a little slack for clock rounding).
pub fn purge_matches(entries: &[(PathBuf, i64)], recycled: &[PathBuf], since: i64) -> Vec<usize> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, (p, t))| *t >= since - 2 && recycled.contains(&safety::normalize(p)))
        .map(|(i, _)| i)
        .collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct PurgeReport {
    pub purged: usize,
    pub bytes: u64,
}

/// Permanently remove from the Recycle Bin only the entries this app put
/// there in its last clean. Everything else in the bin is left alone.
#[cfg(any(
    target_os = "windows",
    all(
        unix,
        not(target_os = "macos"),
        not(target_os = "ios"),
        not(target_os = "android")
    )
))]
pub fn purge_recycled(recycled: &[(PathBuf, u64)], since: i64) -> Result<PurgeReport, String> {
    use trash::os_limited;
    let list = os_limited::list().map_err(|e| e.to_string())?;
    let entries: Vec<(PathBuf, i64)> = list
        .iter()
        .map(|i| (i.original_path(), i.time_deleted))
        .collect();
    let keys: Vec<PathBuf> = recycled.iter().map(|(p, _)| p.clone()).collect();
    let hits = purge_matches(&entries, &keys, since);
    let bytes = hits
        .iter()
        .filter_map(|&i| {
            let n = safety::normalize(&entries[i].0);
            recycled.iter().find(|(p, _)| *p == n).map(|(_, b)| *b)
        })
        .sum();
    let items: Vec<trash::TrashItem> = hits.iter().map(|&i| list[i].clone()).collect();
    let purged = items.len();
    if purged > 0 {
        os_limited::purge_all(items).map_err(|e| e.to_string())?;
    }
    Ok(PurgeReport { purged, bytes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::Risk;
    use tempfile::tempdir;

    fn item(path: PathBuf) -> PlanItem {
        PlanItem {
            category: "test".into(),
            path,
            bytes: 1,
            risk: Risk::Safe,
            selected_by_default: true,
        }
    }

    #[test]
    fn permanent_deletes_allowed_and_skips_blocked() {
        let home = tempdir().unwrap();
        let videos = home.path().join("Videos");
        let rec = videos.join("stream.mkv");
        let cache = home.path().join("cache/dir");
        let file = home.path().join("cache/f.tmp");
        std::fs::create_dir_all(&videos).unwrap();
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(&rec, b"vod").unwrap();
        std::fs::write(cache.join("x"), b"x").unwrap();
        std::fs::write(&file, b"x").unwrap();

        let items = [
            item(rec.clone()),
            item(videos.clone()),
            item(cache.clone()),
            item(file.clone()),
        ];
        let out = execute(
            &items,
            Mode::Permanent,
            std::slice::from_ref(&videos),
            home.path(),
        );
        assert!(out[0].error.as_deref().unwrap().starts_with("blocked"));
        assert!(out[1].error.as_deref().unwrap().starts_with("blocked"));
        assert!(out[2].error.is_none() && out[3].error.is_none());
        assert!(rec.exists() && videos.exists());
        assert!(!cache.exists() && !file.exists());
    }

    #[test]
    fn home_and_missing_paths_report_errors() {
        let home = tempdir().unwrap();
        let out = execute(
            &[
                item(home.path().to_path_buf()),
                item(home.path().join("gone")),
            ],
            Mode::Permanent,
            &[],
            home.path(),
        );
        assert!(out[0].error.as_deref().unwrap().starts_with("blocked"));
        assert!(out[1].error.is_some());
        assert!(home.path().exists());
    }

    /// A link nested inside a deletable dir must be removed as a link, never
    /// followed into the protected target.
    #[cfg(unix)]
    #[test]
    fn nested_symlink_to_protected_is_not_followed() {
        let home = tempdir().unwrap();
        let videos = home.path().join("Videos");
        let rec = videos.join("stream.mkv");
        let tmp_child = home.path().join("Temp/sub");
        std::fs::create_dir_all(&videos).unwrap();
        std::fs::create_dir_all(&tmp_child).unwrap();
        std::fs::write(&rec, b"vod").unwrap();
        std::os::unix::fs::symlink(&videos, tmp_child.join("link")).unwrap();
        std::os::unix::fs::symlink(&videos, home.path().join("Temp/direct")).unwrap();

        let items = [
            item(tmp_child.clone()),
            item(home.path().join("Temp/direct")),
        ];
        let out = execute(
            &items,
            Mode::Permanent,
            std::slice::from_ref(&videos),
            home.path(),
        );
        assert!(out[0].error.is_none());
        // The direct link canonicalises to Videos and is blocked.
        assert!(out[1].error.as_deref().unwrap().starts_with("blocked"));
        assert!(!tmp_child.exists());
        assert!(rec.exists());
    }

    fn plan_of(items: Vec<PlanItem>) -> Plan {
        let total_bytes = items.iter().map(|i| i.bytes).sum();
        Plan { items, total_bytes }
    }

    #[test]
    fn selection_accepts_only_exact_plan_items() {
        let home = tempdir().unwrap();
        let videos = home.path().join("Videos");
        let a = home.path().join("Temp/a");
        let b = home.path().join("Temp/b");
        for d in [&videos, &a, &b] {
            std::fs::create_dir_all(d).unwrap();
        }
        let plan = plan_of(vec![item(a.clone()), item(b.clone())]);
        let sneaky = home.path().join("Temp/a/../../Videos");
        let (ok, rej) = resolve_selection(
            &plan,
            &[
                a.clone(),
                a.join("."),
                videos.clone(),
                sneaky,
                a.join("child"),
                home.path().join("Temp"),
            ],
        );
        assert_eq!(ok.len(), 1);
        assert_eq!(ok[0].path, a);
        assert_eq!(rej.len(), 3, "{rej:?}");
    }

    #[test]
    fn clean_selected_deletes_only_validated_items() {
        let home = tempdir().unwrap();
        let videos = home.path().join("Videos");
        let rec = videos.join("stream.mkv");
        let a = home.path().join("Temp/a.tmp");
        let b = home.path().join("Temp/b.tmp");
        std::fs::create_dir_all(&videos).unwrap();
        std::fs::create_dir_all(a.parent().unwrap()).unwrap();
        for f in [&rec, &a, &b] {
            std::fs::write(f, b"x").unwrap();
        }
        // A plan that (wrongly) contains Videos: the safety re-check must still block it.
        let mut bad = item(rec.clone());
        bad.bytes = 100;
        let mut ia = item(a.clone());
        ia.bytes = 7;
        let plan = plan_of(vec![ia, item(b.clone()), bad]);
        let r = clean_selected(
            &plan,
            &[a.clone(), rec.clone(), videos.clone()],
            Mode::Permanent,
            std::slice::from_ref(&videos),
            home.path(),
        );
        assert_eq!(r.ok_count, 1);
        assert_eq!(r.failed_count, 1);
        assert_eq!(r.rejected.len(), 1);
        assert_eq!(r.freed_bytes, 7);
        assert!(!a.exists() && b.exists() && rec.exists());
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"mode\":\"permanent\""));
        assert!(!json.contains("canonical"));
    }

    #[test]
    fn mode_parse_is_strict() {
        assert_eq!(Mode::parse("trash"), Ok(Mode::Trash));
        assert_eq!(Mode::parse("recycle"), Ok(Mode::Trash));
        assert_eq!(Mode::parse("permanent"), Ok(Mode::Permanent));
        assert!(Mode::parse("Permanent").is_err());
        assert!(Mode::parse("").is_err());
    }

    #[test]
    fn purge_matches_only_our_recent_items() {
        let ours = PathBuf::from("/x/Temp/a");
        let entries = vec![
            (ours.clone(), 1000),
            (ours.clone(), 10), // same path, older deletion
            (PathBuf::from("/x/Documents/cv.pdf"), 1000), // user's own deletion
        ];
        assert_eq!(purge_matches(&entries, &[ours], 999), vec![0]);
        assert!(purge_matches(&entries, &[], 0).is_empty());
    }
}
