//! Tauri commands. Each one is a thin wrapper over cleaner-core; anything that
//! can be tested without a webview lives in the core crate.
//!
//! Trust model: the webview is treated as untrusted input. Paths it sends to
//! `clean` are re-matched against the plan held here and re-checked with
//! `safety::check` right before deletion; config it sends is sanitised
//! (Videos is always re-added to the protected list).

use cleaner_core::config::{self, Config};
use cleaner_core::delete::{self, CleanReport, Mode, PurgeReport};
use cleaner_core::disk::{self, DriveSpace};
use cleaner_core::plan::Env;
use cleaner_core::scan::{self, ScanProgress, ScanResult};
use cleaner_core::{CategoryGroup, Plan, Risk, build_plan};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

/// Items moved to the Recycle Bin by the last clean: normalised path and size.
struct Recycled {
    items: Vec<(PathBuf, u64)>,
    since: i64,
}

pub struct AppState {
    config_path: PathBuf,
    home: PathBuf,
    config: Mutex<Config>,
    /// The plan the UI is looking at. `clean` only accepts its items.
    plan: Mutex<Option<Plan>>,
    scans: Mutex<HashMap<u32, Arc<AtomicBool>>>,
    recycled: Mutex<Option<Recycled>>,
}

impl AppState {
    pub fn load() -> AppState {
        let config_path = config::default_path();
        let home = config::home_dir();
        // A broken config file falls back to defaults rather than refusing to start;
        // it is only overwritten when the user saves Settings.
        let mut cfg = Config::load(&config_path).unwrap_or_default();
        cfg.sanitize(&home);
        AppState {
            config_path,
            home,
            config: Mutex::new(cfg),
            plan: Mutex::new(None),
            scans: Mutex::new(HashMap::new()),
            recycled: Mutex::new(None),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| format!("background task failed: {e}"))
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[tauri::command]
pub fn system_drive() -> Result<DriveSpace, String> {
    disk::space(&disk::system_root()).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn list_drives() -> Vec<DriveSpace> {
    disk::list_drives()
}

/// Sizes of the direct children of `path`. Runs on a blocking thread; sends
/// progress at most ~8 times a second; stops when `cancel_scan(scan_id)` is called.
#[tauri::command]
pub async fn scan_dir(
    state: State<'_, AppState>,
    path: PathBuf,
    top_n: Option<usize>,
    scan_id: u32,
    on_progress: Channel<ScanProgress>,
) -> Result<ScanResult, String> {
    if !path.is_absolute() || !path.is_dir() {
        return Err(format!("not a folder: {}", path.display()));
    }
    let top_n = top_n.unwrap_or(40).clamp(1, 200);
    let protected = lock(&state.config).protected.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    {
        // One scan at a time: a new drill-down supersedes the previous scan.
        let mut scans = lock(&state.scans);
        for c in scans.values() {
            c.store(true, Ordering::Relaxed);
        }
        scans.clear();
        scans.insert(scan_id, cancel.clone());
    }
    let flag = cancel.clone();
    let res = blocking(move || {
        let mut last = Instant::now();
        scan::scan_children(&path, top_n, &protected, &flag, |p| {
            if last.elapsed() >= Duration::from_millis(120) {
                last = Instant::now();
                let _ = on_progress.send(p.clone());
            }
        })
    })
    .await?;
    lock(&state.scans).remove(&scan_id);
    res.map_err(|_| "cancelled".to_string())
}

#[tauri::command]
pub fn cancel_scan(state: State<'_, AppState>, scan_id: Option<u32>) {
    let mut scans = lock(&state.scans);
    match scan_id {
        Some(id) => {
            if let Some(c) = scans.remove(&id) {
                c.store(true, Ordering::Relaxed);
            }
        }
        None => {
            for (_, c) in scans.drain() {
                c.store(true, Ordering::Relaxed);
            }
        }
    }
}

#[derive(Serialize)]
pub struct PlanView {
    groups: Vec<CategoryGroup>,
    total_bytes: u64,
    safe_bytes: u64,
    review_bytes: u64,
    item_count: usize,
}

fn plan_view(plan: &Plan) -> PlanView {
    PlanView {
        groups: plan.grouped(),
        total_bytes: plan.total_bytes,
        safe_bytes: plan.bytes_where(|i| i.risk == Risk::Safe),
        review_bytes: plan.bytes_where(|i| i.risk == Risk::Review),
        item_count: plan.items.len(),
    }
}

/// Build a fresh dry-run plan and remember it for `clean`.
#[tauri::command]
pub async fn get_plan(state: State<'_, AppState>) -> Result<PlanView, String> {
    let cfg = lock(&state.config).clone();
    let plan = blocking(move || build_plan(&cfg, &Env::from_system())).await?;
    let view = plan_view(&plan);
    *lock(&state.plan) = Some(plan);
    Ok(view)
}

/// Delete `paths`. Only exact items of the plan returned by the last
/// `get_plan` are accepted; each is safety-checked again right before it is
/// removed. `mode` defaults to the Recycle Bin; only the exact string
/// "permanent" deletes permanently.
#[tauri::command]
pub async fn clean(
    state: State<'_, AppState>,
    paths: Vec<PathBuf>,
    mode: Option<String>,
) -> Result<CleanReport, String> {
    let mode = match mode.as_deref() {
        None => Mode::Trash,
        Some(m) => Mode::parse(m)?,
    };
    let plan = lock(&state.plan)
        .clone()
        .ok_or("no cleanup plan yet; run Analyze first")?;
    let protected = lock(&state.config).protected.clone();
    let home = state.home.clone();
    let since = now_unix();
    let report =
        blocking(move || delete::clean_selected(&plan, &paths, mode, &protected, &home)).await?;

    let done: Vec<&delete::Outcome> = report
        .outcomes
        .iter()
        .filter(|o| o.error.is_none())
        .collect();
    if let Some(p) = lock(&state.plan).as_mut() {
        p.items.retain(|i| !done.iter().any(|o| o.path == i.path));
        p.total_bytes = p.items.iter().map(|i| i.bytes).sum();
    }
    if mode == Mode::Trash && !done.is_empty() {
        *lock(&state.recycled) = Some(Recycled {
            items: done
                .iter()
                .map(|o| (o.canonical.clone(), o.bytes))
                .collect(),
            since,
        });
    }
    Ok(report)
}

/// Permanently remove from the Recycle Bin the items the last clean put
/// there, and nothing else. Windows only.
#[tauri::command]
pub async fn purge_recycled(state: State<'_, AppState>) -> Result<PurgeReport, String> {
    if !cfg!(windows) {
        return Err("emptying the Recycle Bin from the app is only supported on Windows".into());
    }
    let Some(rec) = lock(&state.recycled).take() else {
        return Err("nothing from the last clean is waiting in the Recycle Bin".into());
    };
    blocking(move || delete::purge_recycled(&rec.items, rec.since)).await?
}

#[derive(Serialize)]
pub struct ConfigView {
    config: Config,
    /// Protected paths the user cannot remove.
    locked: Vec<PathBuf>,
    config_path: PathBuf,
    home: PathBuf,
    platform: &'static str,
    can_purge_recycle_bin: bool,
}

fn config_view(state: &AppState) -> ConfigView {
    ConfigView {
        config: lock(&state.config).clone(),
        locked: Config::locked_protected(&state.home),
        config_path: state.config_path.clone(),
        home: state.home.clone(),
        platform: std::env::consts::OS,
        can_purge_recycle_bin: cfg!(windows),
    }
}

#[tauri::command]
pub fn get_config(state: State<'_, AppState>) -> ConfigView {
    config_view(&state)
}

#[derive(Serialize)]
pub struct SaveResult {
    view: ConfigView,
    warnings: Vec<String>,
}

#[tauri::command]
pub fn save_config(state: State<'_, AppState>, config: Config) -> Result<SaveResult, String> {
    let mut config = config;
    let warnings = config.sanitize(&state.home);
    config
        .save(&state.config_path)
        .map_err(|e| format!("cannot save {}: {e}", state.config_path.display()))?;
    *lock(&state.config) = config;
    // Protected folders or ages may have changed: the old plan is stale.
    *lock(&state.plan) = None;
    Ok(SaveResult {
        view: config_view(&state),
        warnings,
    })
}

/// Native folder picker. Returns `None` when the user cancels. The UI adds
/// the result to a list and saves it through `save_config`, which validates it.
#[tauri::command]
pub async fn pick_folder(app: AppHandle, title: Option<String>) -> Result<Option<PathBuf>, String> {
    let picked = blocking(move || {
        app.dialog()
            .file()
            .set_title(title.unwrap_or_else(|| "Choose a folder".into()))
            .blocking_pick_folder()
    })
    .await?;
    picked
        .map(|p| p.into_path().map_err(|e| e.to_string()))
        .transpose()
}
