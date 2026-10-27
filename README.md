# Dead Signal Disk Cleaner

*A [Dead Signal Interactive](https://github.com/raiinman) tool. MIT licensed.*

**Download:** grab the latest Windows installer from the [Actions tab](../../actions/workflows/build-windows.yml) (open the newest successful run, then Artifacts) or from [Releases](../../releases).

A safety-first disk cleaner for Windows, in the spirit of CCleaner, aimed at a gaming, streaming and dev machine. It has three parts: a Rust core library (`crates/core`), a CLI (`crates/cli`) and a Tauri v2 desktop GUI (`crates/app`). The app makes no network calls, has no telemetry and does not clean the registry.

## Build

```
cargo build --release          # engine + CLI only (the workspace default members)
# binary: target/release/cleaner-cli(.exe)
cargo test                     # engine + CLI tests
```

The GUI crate is a workspace member but not a default member, so `cargo build` and `cargo test` at the root never need WebView2 or webkit2gtk. See [GUI](#gui) for how to build it.

## Usage

```
cleaner-cli init-config [--config FILE]     # write the default config
cleaner-cli scan <path> [--top N]           # disk map: biggest children of a folder
cleaner-cli plan [--config FILE] [--json]   # dry run; deletes nothing
cleaner-cli clean [--config FILE] [--category NAME ...] [--include-review] [--permanent] [--yes]
```

The default config path is `%APPDATA%\diskcleaner\config.json` on Windows, or `~/.config/diskcleaner/config.json` elsewhere. If the file is missing, the built-in defaults are used.

`clean` deletes only the items marked **safe** unless you pass `--include-review`. It sends items to the Recycle Bin unless you pass `--permanent`. It prints the plan and asks `y/N` before deleting anything, unless you pass `--yes`.

### Categories

| Category | Risk | What |
|---|---|---|
| `temp` | safe | contents of `%TEMP%` |
| `shader-cache` | safe | `%LOCALAPPDATA%\NVIDIA\DXCache`, `NVIDIA\GLCache`, `D3DSCache`, `AMD\DxCache` |
| `crash-dumps` | safe | `%LOCALAPPDATA%\CrashDumps` |
| `browser-cache` | safe | Chrome and Edge `Cache`, `Code Cache` and `GPUCache` (Default profile) |
| `package-cache` | review | npm cache, pip cache, `~\.cargo\registry\cache` |
| `old-installers` | review | exe/msi/zip/7z/rar/iso/msix files directly in Downloads, older than `downloads_min_age_days` |
| `dev-artifacts` | review | `node_modules`, `target`, `.venv`/`venv`, `dist`/`build`/`.next`/`.gradle` under `project_roots`, only when the project is stale and git is clean |

A dev artifact is listed only when all of these hold:

- Its project marker file exists next to it (`package.json`, `Cargo.toml`, `pyproject.toml`, and so on). A venv must also contain `pyvenv.cfg`.
- The newest of the marker file, `.git/HEAD` and `.git/index` is older than `dev_min_age_days`.
- `git status --porcelain` is empty.

## Safety guarantees

- Every item passes `safety::check` twice: once when the plan is built, and again right before it is deleted. Blocked paths include:
  - filesystem roots
  - the home directory and its ancestors
  - anything inside, or containing, a `protected` path
  - `C:\Windows`, `Program Files`, `ProgramData` and system dirs
- Paths are canonicalised before checking, so symlinks, junctions and `..` can't get around the check. Windows paths are compared without regard to case.
- `Videos` (OBS recordings), `Documents`, `Desktop`, `Pictures` and `Saved Games` are protected by default.
- Scans never follow symlinks or junctions. Cleaning a cache root removes its children, never the root itself.
- `plan` is always a dry run. The Recycle Bin is the default delete mode.

## GUI

`crates/app` (package `diskcleaner-app`) is a Tauri v2 window over the same engine. The frontend in `crates/app/ui` is plain HTML, CSS and vanilla JS: no bundler, no npm packages, no CDN, no web fonts.

| Screen | What it does |
|---|---|
| Dashboard | Free-space gauge for the system drive, with a warning below your threshold (default 10% free), a list of all drives, and a big **Analyze** button (a dry run). |
| Cleaner | One card per category. **Safe** cards start checked and **Review** cards start unchecked. Each card shows its size and can expand to list its items, and the bar at the bottom shows the selected total. **Preview…** opens a per-category summary; deleting happens only from the confirm button in that dialog. Recycle Bin is the default. *Delete permanently* shows a warning, and you must tick "I understand" before the confirm button works. |
| Disk Map | Squarified treemap of the biggest children of a folder. Click a tile to drill down, and use the breadcrumb or **Up** to go back. Protected folders are hatched and show a lock. Scans run on a background thread, show live progress and can be cancelled. |
| Settings | Protected folders (Videos is shown locked and cannot be removed), project roots (added with the native folder picker), the Downloads folder, the age thresholds and the low-space warning level. Saved to the same `config.json` the CLI uses. |

After a clean, the result dialog shows what was removed and why anything failed. Items sent to the Recycle Bin **free no space until the bin is emptied**. On Windows the dialog offers **Remove just these items from the Recycle Bin**. It permanently removes only the entries that this clean put in the bin (matched by original path and deletion time) and leaves everything else in the bin alone.

### GUI safety model

- The webview is untrusted. `clean(paths, mode)` accepts only paths that exactly match an item of the plan held in the Rust process, which is the one built by the last `get_plan`. Each accepted item is checked again with `safety::check` right before deletion. Unknown or extra paths are refused and reported.
- `mode` defaults to the Recycle Bin. Only the exact string `"permanent"` deletes permanently, and an unknown mode is an error.
- `save_config` runs `Config::sanitize`. It re-adds `%USERPROFILE%\Videos`, drops relative and duplicate paths, refuses a whole drive or the home directory as a project root, and clamps the ages to 1–3650 days. Saving settings throws away the cached plan. The CLI applies the same sanitising when it loads the config.
- The UI also refuses to make any item inside a protected folder selectable, even if one were ever returned.
- `tauri.conf.json` sets a strict CSP (`default-src 'none'`; scripts, styles and fonts from `'self'` only; `connect-src` limited to Tauri IPC). It also sets `freezePrototype`.
- `build.rs` declares the app's commands, so each one needs an explicit `allow-*` permission. `capabilities/main.json` grants `core:default` plus those commands and nothing else. There is no shell, fs, http, opener or dialog permission: the folder picker is opened from Rust.

### Building the GUI on Windows

Prerequisites (one time):

1. **Rust** (stable, MSVC toolchain): install from <https://rustup.rs> and accept the default `x86_64-pc-windows-msvc`.
2. **Microsoft C++ Build Tools**: run the Visual Studio Build Tools installer and select *Desktop development with C++*. That includes MSVC v14x, the Windows 10/11 SDK and `rc.exe`.
3. **WebView2 Runtime**: preinstalled on Windows 11. On Windows 10, install the Evergreen runtime from Microsoft if it is missing.
4. Tauri CLI: `cargo install tauri-cli --version "^2" --locked`

Then, from the repository root:

```
cd crates\app
cargo tauri dev        # run with the debug build
cargo tauri build      # release build + installers
```

The output goes to `target\release\diskcleaner-app.exe`, with installers in `target\release\bundle\nsis\` and `target\release\bundle\msi\`. `cargo build -p diskcleaner-app --release` also builds a runnable exe without installers.

On Linux the GUI needs `libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev librsvg2-dev libayatana-appindicator3-dev libxdo-dev`.

### Working on the UI without Tauri

`crates/app/ui/api.js` is the only file that talks to the backend. With `window.__TAURI__` present (inside the app), it calls the Rust commands. Opened as a plain file in a browser, it loads `ui/mock.js` instead. The mock serves realistic fake data (`C:\Users\streamer`, sizes in GB, Videos locked) and deletes nothing. The mock is never loaded inside the app.

```
node crates/app/ui-tests/ui.test.js        # behaviour tests (Playwright + Chromium)
node crates/app/ui-tests/screenshots.js    # writes crates/app/screenshots/*.png
```

Both scripts use an existing `playwright` install (set `NODE_PATH` if it is global). They do not download browsers. Mock URL flags: `?fast=1` (no delays), `?nolow=1` (plenty of free space), `?rogue=1` (the backend returns a Videos item, to prove the UI refuses it).

## Roadmap

- **Phase 2 (done):** Tauri GUI with a treemap disk map and per-item selection.
- **Phase 3:** game-library tools (Steam, Epic and other library sizes, moving games between drives, shader cache per game).
- **Phase 4:** scheduled cleanups (Task Scheduler) with safe-only defaults.
