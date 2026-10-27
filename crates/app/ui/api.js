// The only place the UI talks to the backend.
//
// Inside the Tauri app, window.__TAURI__ exists (withGlobalTauri) and every
// call goes to a Rust command. Opened as a plain file in a browser (design
// work, Playwright tests), mock.js is loaded instead and serves fake data.
// The mock is never loaded inside the app.
"use strict";
window.DC = window.DC || {};

(function (DC) {
  const tauri = window.__TAURI__ && window.__TAURI__.core && window.__TAURI__.core.invoke ? window.__TAURI__ : null;
  let nextScanId = 1;

  function loadMock() {
    return new Promise((resolve, reject) => {
      const s = document.createElement("script");
      s.src = "mock.js";
      s.onload = () => resolve(DC.mockBackend);
      s.onerror = () => reject(new Error("mock.js failed to load"));
      document.head.append(s);
    });
  }

  function tauriBackend() {
    const invoke = tauri.core.invoke;
    return {
      systemDrive: () => invoke("system_drive"),
      listDrives: () => invoke("list_drives"),
      scanDir: (path, topN, scanId, onProgress) => {
        const ch = new tauri.core.Channel();
        ch.onmessage = (p) => onProgress && onProgress(p);
        return invoke("scan_dir", { path, topN, scanId, onProgress: ch });
      },
      cancelScan: (scanId) => invoke("cancel_scan", { scanId }),
      getPlan: () => invoke("get_plan"),
      clean: (paths, mode) => invoke("clean", { paths, mode }),
      purgeRecycled: () => invoke("purge_recycled"),
      getConfig: () => invoke("get_config"),
      saveConfig: (config) => invoke("save_config", { config }),
      pickFolder: (title) => invoke("pick_folder", { title }),
    };
  }

  const ready = (tauri ? Promise.resolve(tauriBackend()) : loadMock()).then((b) => {
    DC.api.backend = b;
    return DC.api;
  });

  // Errors from Rust commands arrive as plain strings; normalise to Error.
  const call = (name) => async (...args) => {
    await ready;
    const b = DC.api.backend;
    try {
      return await b[name](...args);
    } catch (e) {
      throw e instanceof Error ? e : new Error(String(e));
    }
  };

  DC.api = {
    isMock: !tauri,
    ready,
    systemDrive: call("systemDrive"),
    listDrives: call("listDrives"),
    getPlan: call("getPlan"),
    clean: call("clean"),
    purgeRecycled: call("purgeRecycled"),
    getConfig: call("getConfig"),
    saveConfig: call("saveConfig"),
    pickFolder: call("pickFolder"),
    cancelScan: call("cancelScan"),
    // Returns { id, promise }. Cancel with DC.api.cancelScan(id).
    scanDir(path, topN, onProgress) {
      const id = nextScanId++;
      return { id, promise: call("scanDir")(path, topN, id, onProgress) };
    },
  };
})(window.DC);
