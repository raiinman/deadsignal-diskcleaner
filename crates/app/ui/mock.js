// Fake backend for working on the UI in a plain browser and for Playwright.
// Shapes match the Rust commands' JSON exactly. Loaded by api.js only when
// window.__TAURI__ is missing; nothing here touches the real disk.
//
// URL flags (for tests): ?rogue=1 makes the "backend" return a plan item
// inside Videos, to prove the UI refuses to make it selectable anyway.
// ?fast=1 removes artificial delays. ?nolow=1 reports plenty of free space.
"use strict";

(function (DC) {
  const GB = 1024 ** 3;
  const MB = 1024 ** 2;
  const HOME = "C:\\Users\\streamer";
  const LAD = HOME + "\\AppData\\Local";
  const params = new URLSearchParams(location.search);
  const FAST = params.has("fast");
  const delay = (ms) => new Promise((r) => setTimeout(r, FAST ? 0 : ms));
  const clone = (x) => JSON.parse(JSON.stringify(x));

  // ---------- config ----------
  let config = {
    protected: [HOME + "\\Videos", HOME + "\\Documents", HOME + "\\Desktop", HOME + "\\Pictures", HOME + "\\Saved Games"],
    project_roots: [HOME + "\\source", HOME + "\\projects", HOME + "\\code"],
    downloads_dir: HOME + "\\Downloads",
    downloads_min_age_days: 30,
    dev_min_age_days: 60,
    low_space_percent: 10,
  };
  const LOCKED = [HOME + "\\Videos"];
  const isProtected = (p) => config.protected.some((x) => DC.isInside(p, x));

  function view() {
    return {
      config: clone(config),
      locked: LOCKED.slice(),
      config_path: "C:\\Users\\streamer\\AppData\\Roaming\\diskcleaner\\config.json",
      home: HOME,
      platform: "windows",
      can_purge_recycle_bin: true,
    };
  }

  // Mirrors Config::sanitize in cleaner-core.
  function sanitize(c) {
    const w = [];
    const abs = (p) => /^[a-zA-Z]:\\/.test(p) || p.startsWith("\\\\");
    const uniq = (list) => {
      const seen = new Set();
      return list.filter((p) => {
        const k = DC.normPath(p);
        if (seen.has(k)) return false;
        seen.add(k);
        return true;
      });
    };
    c.protected = uniq(
      (c.protected || []).filter((p) => {
        if (!abs(p)) w.push(`ignored relative protected path ${p}`);
        return abs(p);
      })
    );
    for (const l of LOCKED) {
      if (!c.protected.some((p) => DC.normPath(p) === DC.normPath(l))) {
        w.push(`${l} is always protected and was re-added`);
        c.protected.unshift(l);
      }
    }
    c.project_roots = uniq(
      (c.project_roots || []).filter((p) => {
        if (!abs(p)) return w.push(`ignored relative project root ${p}`), false;
        if (/^[a-zA-Z]:\\?$/.test(p)) return w.push(`ignored project root ${p}: a whole drive is too broad`), false;
        if (DC.normPath(p) === DC.normPath(HOME))
          return w.push(`ignored project root ${p}: pick a folder inside your home directory`), false;
        return true;
      })
    );
    for (const k of ["downloads_min_age_days", "dev_min_age_days"]) {
      const v = Math.min(3650, Math.max(1, Math.round(Number(c[k]) || 0)));
      if (v !== c[k]) w.push(`${k} clamped to ${v}`);
      c[k] = v;
    }
    const lp = Math.min(50, Math.max(1, Math.round(Number(c.low_space_percent) || 0)));
    if (lp !== c.low_space_percent) w.push(`low_space_percent clamped to ${lp}`);
    c.low_space_percent = lp;
    return w;
  }

  // ---------- drives ----------
  const TOTAL = 2000396746752; // a "2 TB" NVMe as Windows sees it (1.82 TB)
  const freeC = () => (params.has("nolow") ? 612 * GB : 131.4 * GB) + freedPermanently;
  let freedPermanently = 0;
  const drive = () => {
    const free = Math.min(TOTAL, freeC());
    return { root: "C:\\", total: TOTAL, free, used: TOTAL - free, is_system: true };
  };
  const drives = () => [
    drive(),
    { root: "D:\\", total: 4000785105920, free: 1388 * GB, used: 4000785105920 - 1388 * GB, is_system: false },
  ];

  // ---------- disk tree ----------
  // Sizes in GB. Objects are folders; numbers are files (or folders we don't model further).
  const TREE = {
    "C:\\": {
      Users: {
        streamer: {
          Videos: { OBS: { "2026-10-25 21-14-03.mkv": 38.2, "2026-10-21 19-02-44.mkv": 41.7, "2026-10-12 20-30-10.mkv": 52.4, Replays: 44.0, Highlights: 61.5 }, "Captures": 48.3, "Edits": 39.0 },
          AppData: {
            Local: {
              Temp: 3.6, NVIDIA: { DXCache: 2.4, GLCache: 0.6 }, D3DSCache: 0.9, "npm-cache": 6.8, pip: 1.2,
              Packages: 18.4, "Microsoft": 9.6, "Google": 4.1, "Programs": 12.7, CrashDumps: 2.3, "Docker": 41.2, "obs-studio": 0.4,
            },
            Roaming: { "Code": 3.4, discord: 2.1, obs: 0.3, Spotify: 7.9 },
            LocalLow: 1.4,
          },
          Downloads: { "Win11_23H2_English_x64.iso": 6.6, "SteamSetup.exe": 0.002, "OBS-Studio-30.2-Full-Installer-x64.exe": 0.14, "unreal-assets.zip": 22.5, "VSCodeUserSetup-x64-1.93.exe": 0.1, "game-mods": 31.0, "recordings-export": 18.9 },
          projects: { "codex-agent": 24.8, "stream-overlay": 6.2, "twitch-bot": 3.1, "unity-jam-2025": 19.6 },
          source: { repos: 14.3, "old-bot": 2.2 },
          code: { portfolio: 1.6, scratch: 0.8 },
          Documents: 12.6, Pictures: 21.4, Desktop: 3.2, Music: 9.8, "Saved Games": 2.7,
          ".cargo": 6.9, ".rustup": 3.8, ".vscode": 1.2, ".codex": 4.6, "OneDrive": 38.0,
        },
        Public: 1.1,
      },
      "Program Files": { "NVIDIA Corporation": 3.1, "obs-studio": 0.6, "Docker": 2.8, "Microsoft Visual Studio": 28.4, "Epic Games": 71.0, "WindowsApps": 26.5, "Common Files": 4.2 },
      "Program Files (x86)": { Steam: { steamapps: { common: { "Cyberpunk 2077": 92.1, "Baldur's Gate 3": 122.0, "Counter-Strike 2": 33.6, "Apex Legends": 74.9, "Elden Ring": 52.7, "Satisfactory": 26.0 }, shadercache: 7.8, workshop: 21.4 } }, "Microsoft": 3.6, "Battle.net": 0.8 },
      XboxGames: { "Call of Duty": 151.0, "Forza Horizon 5": 109.0 },
      Windows: 38.4,
      ProgramData: 22.7,
      "pagefile.sys": 32.0,
      "hiberfil.sys": 25.4,
      "$Recycle.Bin": 4.4,
      "swapfile.sys": 0.25,
    },
  };

  // Deterministic filler for folders we don't spell out, so drill-down never dead-ends.
  function seeded(str) {
    let h = 2166136261;
    for (const ch of str) h = Math.imul(h ^ ch.charCodeAt(0), 16777619) >>> 0;
    return () => ((h = Math.imul(h ^ (h >>> 15), 2246822507) >>> 0), (h % 10000) / 10000);
  }
  const FILLER = ["cache", "data", "assets", "bin", "logs", "packages", "shaders", "content", "lib", "media", "builds", "backup", "textures", "audio", "src", "obj", "temp", "maps", "plugins", "models"];
  function synth(path, gb) {
    const rnd = seeded(path);
    const n = gb < 0.05 ? 0 : 4 + Math.floor(rnd() * 9);
    if (n === 0) return null;
    const weights = Array.from({ length: n }, () => rnd() ** 2.2 + 0.02);
    const sum = weights.reduce((a, b) => a + b, 0);
    const out = {};
    weights.forEach((w, i) => {
      const name = FILLER[Math.floor(rnd() * FILLER.length)] + (i > 3 ? "-" + (i + 1) : "");
      const isFile = rnd() < 0.35 || gb * (w / sum) < 0.02;
      out[isFile ? name + [".pak", ".bin", ".dat", ".log", ".db"][i % 5] : name] = isFile ? -(gb * w) / sum : (gb * w) / sum;
    });
    return out; // negative = file
  }
  const sizeOf = (node) => (typeof node === "number" ? Math.abs(node) : Object.values(node).reduce((a, n) => a + sizeOf(n), 0));

  function lookup(path) {
    const parts = DC.ancestors(path);
    let node = TREE["C:\\"];
    if (DC.normPath(parts[0]) !== DC.normPath("C:\\")) return null;
    for (let i = 1; i < parts.length; i++) {
      const name = DC.baseName(parts[i]);
      const key = typeof node === "object" ? Object.keys(node).find((k) => k.toLowerCase() === name.toLowerCase()) : null;
      if (!key) return null;
      let next = node[key];
      if (typeof next === "number") {
        if (next < 0) return null; // a file
        const s = synth(parts[i], next);
        if (!s) return null;
        node[key] = s;
        next = s;
      }
      node = next;
    }
    return node;
  }

  const scans = new Map();
  async function scanDir(path, topN, scanId, onProgress) {
    const node = lookup(path);
    if (!node || typeof node !== "object") throw new Error(`not a folder: ${path}`);
    const state = { cancelled: false };
    scans.set(scanId, state);
    const total = sizeOf(node) * GB;
    const files = Math.round(total / (3.1 * MB));
    const steps = 6;
    for (let i = 1; i <= steps; i++) {
      await delay(110);
      if (state.cancelled) throw new Error("cancelled");
      onProgress && onProgress({ files: Math.round((files * i) / steps), bytes: Math.round((total * i) / steps), current: path + "\\" + Object.keys(node)[i % Object.keys(node).length] });
    }
    scans.delete(scanId);
    const sep = path.endsWith("\\") ? "" : "\\";
    const children = Object.entries(node)
      .map(([name, n]) => {
        const p = path + sep + name;
        const bytes = Math.round(sizeOf(n) * GB);
        return { path: p, name, bytes, files: typeof n === "number" && n < 0 ? 1 : Math.max(1, Math.round(bytes / (2.7 * MB))), is_dir: !(typeof n === "number" && (n < 0 || /\.[a-z0-9]{2,4}$/i.test(name))), locked: isProtected(p) };
      })
      .sort((a, b) => b.bytes - a.bytes);
    const top = children.slice(0, topN);
    const rest = children.slice(topN);
    return {
      root: path,
      total_bytes: Math.round(total),
      other_bytes: rest.reduce((a, c) => a + c.bytes, 0),
      other_count: rest.length,
      files,
      errors: DC.normPath(path) === DC.normPath("C:\\") ? 412 : 0,
      locked: isProtected(path),
      children: top,
    };
  }

  // ---------- plan ----------
  const CATS = [
    ["temp", "Temporary files", "Contents of %TEMP%. Apps recreate what they need.", "Safe"],
    ["shader-cache", "Shader caches", "NVIDIA, AMD and DirectX shader caches. Games rebuild them; the first launch after cleaning may stutter briefly.", "Safe"],
    ["crash-dumps", "Crash dumps", "Leftover crash reports in %LOCALAPPDATA%\\CrashDumps.", "Safe"],
    ["browser-cache", "Browser caches", "Chrome and Edge cache folders (Default profile). Logins and history are not touched.", "Safe"],
    ["package-cache", "Package caches", "npm, pip and Cargo download caches. Re-downloaded on the next install.", "Review"],
    ["old-installers", "Old installers in Downloads", "exe, msi, zip, 7z, rar, iso and msix files in Downloads older than the age you set.", "Review"],
    ["dev-artifacts", "Stale dev build folders", "node_modules, target, venv, dist and build folders in project roots that have not been touched for a while and have no uncommitted git changes.", "Review"],
  ];
  const ITEMS = {
    temp: [
      ["\\AppData\\Local\\Temp\\chrome_BITS_18244_1093", 1.31 * GB],
      ["\\AppData\\Local\\Temp\\NVIDIA Corporation", 742 * MB],
      ["\\AppData\\Local\\Temp\\vscode-stable-user-x64", 512 * MB],
      ["\\AppData\\Local\\Temp\\obs-studio-crash-2026-10-11.txt", 2.4 * MB],
      ["\\AppData\\Local\\Temp\\tmp8F2A.tmp", 384 * MB],
      ["\\AppData\\Local\\Temp\\SteamSetup_temp", 221 * MB],
      ["\\AppData\\Local\\Temp\\npm-28816-3b1f", 96 * MB],
    ],
    "shader-cache": [
      ["\\AppData\\Local\\NVIDIA\\DXCache\\a91c0e3d_2.nvph", 1.62 * GB],
      ["\\AppData\\Local\\NVIDIA\\DXCache\\5f1b7a20_1.nvph", 812 * MB],
      ["\\AppData\\Local\\NVIDIA\\GLCache\\8c2d", 604 * MB],
      ["\\AppData\\Local\\D3DSCache\\b3e1f6a7c5d2", 922 * MB],
    ],
    "crash-dumps": [
      ["\\AppData\\Local\\CrashDumps\\obs64.exe.21456.dmp", 1.12 * GB],
      ["\\AppData\\Local\\CrashDumps\\FortniteClient-Win64-Shipping.exe.9921.dmp", 864 * MB],
      ["\\AppData\\Local\\CrashDumps\\node.exe.30112.dmp", 341 * MB],
    ],
    "browser-cache": [
      ["\\AppData\\Local\\Google\\Chrome\\User Data\\Default\\Cache\\Cache_Data", 1.48 * GB],
      ["\\AppData\\Local\\Google\\Chrome\\User Data\\Default\\Code Cache\\js", 402 * MB],
      ["\\AppData\\Local\\Microsoft\\Edge\\User Data\\Default\\Cache\\Cache_Data", 611 * MB],
      ["\\AppData\\Local\\Microsoft\\Edge\\User Data\\Default\\GPUCache\\data_1", 23 * MB],
    ],
    "package-cache": [
      ["\\AppData\\Local\\npm-cache\\_cacache", 6.84 * GB],
      ["\\AppData\\Local\\pip\\Cache\\http-v2", 1.21 * GB],
      ["\\.cargo\\registry\\cache\\index.crates.io-1949cf8c6b5b557f", 2.37 * GB],
    ],
    "old-installers": [
      ["\\Downloads\\Win11_23H2_English_x64.iso", 6.61 * GB],
      ["\\Downloads\\unreal-assets-2025.zip", 22.5 * GB],
      ["\\Downloads\\OBS-Studio-30.2-Full-Installer-x64.exe", 141 * MB],
      ["\\Downloads\\node-v20.12.2-x64.msi", 26 * MB],
      ["\\Downloads\\VSCodeUserSetup-x64-1.89.1.exe", 95 * MB],
      ["\\Downloads\\SteamSetup.exe", 2.3 * MB],
    ],
    "dev-artifacts": [
      ["\\projects\\codex-agent\\target", 9.82 * GB],
      ["\\projects\\unity-jam-2025\\build", 4.4 * GB],
      ["\\source\\stream-overlay\\node_modules", 1.43 * GB],
      ["\\source\\old-bot\\.venv", 688 * MB],
      ["\\code\\portfolio\\.next", 512 * MB],
    ],
  };
  let plan = null;
  function buildPlan() {
    const items = [];
    for (const [cat, , , risk] of CATS) {
      for (const [rel, bytes] of ITEMS[cat]) {
        const path = HOME + rel;
        if (isProtected(path)) continue; // the real engine drops these too
        items.push({ category: cat, path, bytes: Math.round(bytes), risk, selected_by_default: risk === "Safe" });
      }
    }
    if (params.has("rogue")) {
      // Simulates a buggy backend. The UI must still refuse to select it.
      items.push({ category: "temp", path: HOME + "\\Videos\\OBS\\2026-10-25 21-14-03.mkv", bytes: Math.round(38.2 * GB), risk: "Safe", selected_by_default: true });
    }
    items.sort((a, b) => b.bytes - a.bytes);
    return items;
  }
  function planView(items) {
    const groups = CATS.map(([name, label, description, risk]) => {
      const its = items.filter((i) => i.category === name);
      return { name, label, description, risk, selected_by_default: risk === "Safe", bytes: its.reduce((a, i) => a + i.bytes, 0), items: its };
    });
    const sum = (f) => items.filter(f).reduce((a, i) => a + i.bytes, 0);
    return { groups, total_bytes: sum(() => true), safe_bytes: sum((i) => i.risk === "Safe"), review_bytes: sum((i) => i.risk === "Review"), item_count: items.length };
  }

  let recycled = null;

  DC.mockBackend = {
    async systemDrive() {
      await delay(60);
      return drive();
    },
    async listDrives() {
      await delay(60);
      return drives();
    },
    scanDir,
    async cancelScan(id) {
      const s = scans.get(id);
      if (s) s.cancelled = true;
      else for (const v of scans.values()) v.cancelled = true;
    },
    async getPlan() {
      await delay(900);
      plan = buildPlan();
      return clone(planView(plan));
    },
    // Same contract as the Rust command: only exact plan paths, safety re-check, strict mode.
    async clean(paths, mode) {
      await delay(700);
      if (mode !== undefined && mode !== null && !["trash", "recycle", "permanent"].includes(mode)) throw new Error(`unknown delete mode "${mode}"`);
      if (!plan) throw new Error("no cleanup plan yet; run Analyze first");
      const m = mode === "permanent" ? "permanent" : "trash";
      const outcomes = [];
      const rejected = [];
      const seen = new Set();
      for (const p of paths) {
        const k = DC.normPath(p);
        if (seen.has(k)) continue;
        seen.add(k);
        const it = plan.find((i) => DC.normPath(i.path) === k);
        if (!it) {
          rejected.push({ path: p, reason: "not in the current cleanup plan" });
          continue;
        }
        let error = null;
        if (isProtected(it.path)) error = "blocked: inside protected path " + config.protected.find((x) => DC.isInside(it.path, x)).toLowerCase();
        else if (/obs64\.exe/.test(it.path)) error = "The process cannot access the file because it is being used by another process. (os error 32)";
        outcomes.push({ category: it.category, path: it.path, bytes: it.bytes, error });
      }
      const ok = outcomes.filter((o) => !o.error);
      plan = plan.filter((i) => !ok.some((o) => o.path === i.path));
      const freed = ok.reduce((a, o) => a + o.bytes, 0);
      if (m === "permanent") freedPermanently += freed;
      else recycled = ok.slice();
      return { mode: m, outcomes, rejected, freed_bytes: freed, ok_count: ok.length, failed_count: outcomes.length - ok.length };
    },
    async purgeRecycled() {
      await delay(500);
      if (!recycled) throw new Error("nothing from the last clean is waiting in the Recycle Bin");
      const bytes = recycled.reduce((a, o) => a + o.bytes, 0);
      const purged = recycled.length;
      freedPermanently += bytes;
      recycled = null;
      return { purged, bytes };
    },
    async getConfig() {
      return view();
    },
    async saveConfig(c) {
      await delay(120);
      const next = clone(c);
      const warnings = sanitize(next);
      config = next;
      plan = null;
      return { view: view(), warnings };
    },
    async pickFolder() {
      await delay(80);
      const choices = [HOME + "\\Music", HOME + "\\dev\\codex-work", "D:\\Projects", HOME + "\\OneDrive\\Streams"];
      DC.mockBackend._pick = ((DC.mockBackend._pick ?? -1) + 1) % choices.length;
      return choices[DC.mockBackend._pick];
    },
  };
})(window.DC);
