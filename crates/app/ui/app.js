// Screens and state. Talks to the backend only through DC.api (api.js).
"use strict";

(function (DC) {
  const { h, s, icon, fmtBytes, fmtCount, api } = DC;
  const $ = (id) => document.getElementById(id);

  const state = {
    screen: "dashboard",
    cfg: null, // ConfigView
    drive: null, // DriveSpace for the system drive
    drives: [],
    plan: null, // PlanView
    selected: new Map(), // normPath -> item
    open: new Set(), // expanded category names
    analyzing: false,
    map: { path: null, cache: new Map(), scan: null },
    lastReport: null,
  };

  // ---------------- helpers ----------------
  function protectedPaths() {
    if (!state.cfg) return [];
    return [...state.cfg.locked, ...state.cfg.config.protected];
  }
  // Defence in depth: the backend never plans protected paths, but if one
  // ever showed up the UI still refuses to make it selectable.
  function isBlocked(path) {
    return protectedPaths().some((p) => DC.isInside(path, p) || DC.isInside(p, path));
  }

  let toastTimer = null;
  function toast(msg, kind) {
    let t = $("toast");
    if (!t) {
      t = h("div", { id: "toast", class: "toast", role: "status" });
      document.body.append(t);
    }
    t.className = "toast " + (kind || "");
    t.textContent = msg;
    t.hidden = false;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => (t.hidden = true), 6000);
  }

  function busy(btn, on) {
    btn.disabled = on;
    const sp = btn.querySelector(".spinner");
    if (sp) sp.hidden = !on;
  }

  // ---------------- navigation ----------------
  function show(screen) {
    state.screen = screen;
    for (const el of document.querySelectorAll(".screen")) el.hidden = el.id !== "screen-" + screen;
    for (const b of document.querySelectorAll(".nav-item")) b.classList.toggle("active", b.dataset.screen === screen);
    if (screen === "diskmap") openDiskMap();
    if (screen === "settings") renderSettings();
    if (screen === "cleaner") renderCleaner();
    document.querySelector(".content").scrollTop = 0;
  }

  // ---------------- dashboard ----------------
  function gauge(d, lowPct) {
    const R = 120, C = 2 * Math.PI * R, SW = 26;
    const usedFrac = d.total ? d.used / d.total : 0;
    const freePct = d.total ? (d.free * 100) / d.total : 0;
    const low = d.total > 0 && freePct < lowPct;
    const threshAngle = (1 - lowPct / 100) * 2 * Math.PI - Math.PI / 2;
    const tx = (r) => 160 + r * Math.cos(threshAngle);
    const ty = (r) => 160 + r * Math.sin(threshAngle);
    return s(
      "svg",
      { viewBox: "0 0 320 320", role: "img", "aria-label": `${fmtBytes(d.free)} free of ${fmtBytes(d.total)}`, "data-testid": "gauge" },
      s("circle", { cx: 160, cy: 160, r: R, fill: "none", class: "g-track", "stroke-width": SW }),
      s("circle", {
        cx: 160, cy: 160, r: R, fill: "none", class: "g-used" + (low ? " low" : ""), "stroke-width": SW,
        "stroke-dasharray": `${Math.max(0.001, usedFrac * C)} ${C}`, transform: "rotate(-90 160 160)", "stroke-linecap": "butt",
      }),
      s("line", { x1: tx(R - SW / 2 - 4), y1: ty(R - SW / 2 - 4), x2: tx(R + SW / 2 + 4), y2: ty(R + SW / 2 + 4), class: "g-thresh", "stroke-width": 2 }, s("title", {}, `Warning threshold: ${lowPct}% free`)),
      s("text", { x: 160, y: 128, "text-anchor": "middle", class: "g-root" }, d.root),
      s("text", { x: 160, y: 172, "text-anchor": "middle", class: "g-big" }, fmtBytes(d.free)),
      s("text", { x: 160, y: 198, "text-anchor": "middle", class: "g-small" }, `free · ${freePct.toFixed(1)}%`)
    );
  }

  async function refreshDrives() {
    try {
      const [d, all] = await Promise.all([api.systemDrive(), api.listDrives()]);
      state.drive = d;
      state.drives = all;
    } catch (e) {
      toast("Could not read drive space: " + e.message, "error");
    }
    renderDashboard();
  }

  function renderDashboard() {
    const d = state.drive;
    const lowPct = state.cfg ? state.cfg.config.low_space_percent : 10;
    if (d) {
      $("gauge").replaceChildren(gauge(d, lowPct));
      $("drive-free").textContent = fmtBytes(d.free);
      $("drive-used").textContent = fmtBytes(d.used);
      $("drive-total").textContent = fmtBytes(d.total);
      const freePct = d.total ? (d.free * 100) / d.total : 0;
      const low = d.total > 0 && freePct < lowPct;
      $("low-space-banner").hidden = !low;
      $("low-space-text").textContent = `${d.root} has ${fmtBytes(d.free)} free (${freePct.toFixed(1)}%), under your ${lowPct}% warning level. Run Analyze to find space you can reclaim.`;
    }
    $("drive-list").replaceChildren(
      ...state.drives.map((x) => {
        const pct = x.total ? (x.used * 100) / x.total : 0;
        const low = x.total && (x.free * 100) / x.total < lowPct;
        const bar = h("span", { class: low ? "low" : "" });
        bar.style.width = pct.toFixed(1) + "%";
        return h(
          "li",
          { class: "drive-row" },
          h("span", { class: "drive-name" }, x.root),
          h("div", { class: "drive-bar", title: `${pct.toFixed(0)}% used` }, bar),
          h("span", { class: "drive-meta" }, `${fmtBytes(x.free)} free of ${fmtBytes(x.total)}`)
        );
      })
    );
    const p = state.plan;
    $("analysis-summary").hidden = !p;
    if (p) {
      $("sum-safe").textContent = fmtBytes(p.safe_bytes);
      $("sum-review").textContent = fmtBytes(p.review_bytes);
    }
  }

  async function analyze() {
    if (state.analyzing) return;
    state.analyzing = true;
    const btns = [$("analyze-btn"), $("reanalyze-btn"), $("cleaner-analyze")];
    btns.forEach((b) => busy(b, true));
    $("analyze-btn").querySelector(".btn-label").textContent = "Analyzing…";
    try {
      setPlan(await api.getPlan());
      renderDashboard();
      show("cleaner");
    } catch (e) {
      toast("Analyze failed: " + e.message, "error");
    } finally {
      state.analyzing = false;
      btns.forEach((b) => busy(b, false));
      $("analyze-btn").querySelector(".btn-label").textContent = "Analyze";
    }
  }

  // ---------------- cleaner ----------------
  function setPlan(plan) {
    state.plan = plan;
    state.selected.clear();
    if (!plan) return updateBadge();
    for (const g of plan.groups) {
      for (const it of g.items) {
        // Safe items start checked; Review items never do.
        if (g.risk === "Safe" && it.risk === "Safe" && it.selected_by_default && !isBlocked(it.path)) {
          state.selected.set(DC.normPath(it.path), it);
        }
      }
    }
    updateBadge();
  }

  function updateBadge() {
    const b = $("nav-cleaner-badge");
    const total = selectionTotals().bytes;
    b.hidden = !state.plan || total === 0;
    b.textContent = fmtBytes(total, 0);
  }

  function selectableItems(g) {
    return g.items.filter((it) => !isBlocked(it.path));
  }

  function selectionTotals() {
    let bytes = 0, count = 0;
    for (const it of state.selected.values()) {
      if (isBlocked(it.path)) continue;
      bytes += it.bytes;
      count++;
    }
    return { bytes, count };
  }

  function catCard(g) {
    const sel = selectableItems(g);
    const nSel = sel.filter((it) => state.selected.has(DC.normPath(it.path))).length;
    const selBytes = sel.filter((it) => state.selected.has(DC.normPath(it.path))).reduce((a, i) => a + i.bytes, 0);
    const isOpen = state.open.has(g.name);
    const empty = g.items.length === 0;
    const cb = h("input", {
      type: "checkbox",
      "aria-label": "Select all " + g.label,
      "data-testid": "cat-check-" + g.name,
      disabled: sel.length === 0,
    });
    cb.checked = sel.length > 0 && nSel === sel.length;
    cb.indeterminate = nSel > 0 && nSel < sel.length;
    cb.addEventListener("change", () => {
      for (const it of sel) {
        const k = DC.normPath(it.path);
        if (cb.checked) state.selected.set(k, it);
        else state.selected.delete(k);
      }
      renderCleaner();
    });
    const riskBadge = h("span", { class: "badge " + (g.risk === "Safe" ? "safe" : "review") }, g.risk === "Safe" ? "Safe" : "Review");
    const head = h(
      "div",
      { class: "cat-head" },
      cb,
      h("div", {}, h("div", { class: "cat-title" }, h("h3", {}, g.label), riskBadge), h("div", { class: "cat-desc" }, g.description)),
      h(
        "div",
        { class: "cat-size", "data-testid": "cat-size-" + g.name },
        empty ? "–" : fmtBytes(g.bytes),
        h("small", {}, empty ? "nothing found" : nSel > 0 && nSel < g.items.length ? `${fmtBytes(selBytes)} selected` : `${g.items.length} item${g.items.length === 1 ? "" : "s"}`)
      ),
      empty
        ? h("span")
        : h(
            "button",
            {
              class: "expand-btn",
              "aria-expanded": String(isOpen),
              "data-testid": "cat-expand-" + g.name,
              onclick: () => {
                isOpen ? state.open.delete(g.name) : state.open.add(g.name);
                renderCleaner();
              },
            },
            isOpen ? "Hide" : "Show",
            icon("chevron")
          )
    );
    const card = h("div", { class: "card cat-card" + (isOpen ? " open" : "") + (empty ? " empty" : ""), "data-testid": "cat-" + g.name, "data-risk": g.risk }, head);
    if (isOpen && !empty) {
      card.append(
        h(
          "ul",
          { class: "item-list" },
          g.items.map((it) => {
            const k = DC.normPath(it.path);
            const blocked = isBlocked(it.path);
            const box = h("input", {
              type: "checkbox",
              "data-testid": "item-check",
              "data-path": it.path,
              "aria-label": it.path,
              disabled: blocked,
            });
            box.checked = !blocked && state.selected.has(k);
            box.addEventListener("change", () => {
              if (box.checked && !isBlocked(it.path)) state.selected.set(k, it);
              else state.selected.delete(k);
              renderCleaner();
            });
            return h(
              "li",
              { class: "item-row" + (blocked ? " blocked" : ""), title: it.path },
              box,
              h("span", { class: "item-path" }, h("bdi", {}, it.path)),
              blocked ? h("span", { class: "badge locked" }, icon("lock"), "Protected") : h("span", { class: "item-size" }, fmtBytes(it.bytes))
            );
          })
        )
      );
    }
    return card;
  }

  function renderCleaner() {
    const p = state.plan;
    $("cleaner-empty").hidden = !!p;
    $("action-bar").hidden = !p;
    $("reanalyze-btn").hidden = !p;
    const list = $("cat-list");
    if (!p) {
      list.replaceChildren();
      updateBadge();
      return;
    }
    const safe = p.groups.filter((g) => g.risk === "Safe");
    const review = p.groups.filter((g) => g.risk !== "Safe");
    list.replaceChildren(
      h("div", { class: "cat-group-title" }, `Safe · ${fmtBytes(safe.reduce((a, g) => a + g.bytes, 0))}`),
      ...safe.map(catCard),
      h("div", { class: "cat-group-title" }, `Review first · ${fmtBytes(review.reduce((a, g) => a + g.bytes, 0))}`),
      ...review.map(catCard)
    );
    const t = selectionTotals();
    $("sel-total").textContent = fmtBytes(t.bytes);
    $("sel-count").textContent = `${fmtCount(t.count)} item${t.count === 1 ? "" : "s"}`;
    $("preview-btn").disabled = t.count === 0;
    updateBadge();
  }

  function mode() {
    return document.querySelector('input[name="mode"]:checked').value;
  }

  function openPreview() {
    const byCat = new Map();
    for (const it of state.selected.values()) {
      if (isBlocked(it.path)) continue;
      const e = byCat.get(it.category) || { n: 0, bytes: 0 };
      e.n++;
      e.bytes += it.bytes;
      byCat.set(it.category, e);
    }
    const label = (name) => (state.plan.groups.find((g) => g.name === name) || { label: name }).label;
    const rows = state.plan.groups
      .filter((g) => byCat.has(g.name))
      .map((g) => {
        const e = byCat.get(g.name);
        return h("tr", { "data-testid": "preview-row", "data-cat": g.name }, h("td", {}, label(g.name), " ", h("span", { class: "badge " + (g.risk === "Safe" ? "safe" : "review") }, g.risk)), h("td", { class: "num" }, fmtCount(e.n)), h("td", { class: "num" }, fmtBytes(e.bytes)));
      });
    const t = selectionTotals();
    $("preview-rows").replaceChildren(...rows);
    $("preview-count").textContent = fmtCount(t.count);
    $("preview-total").textContent = fmtBytes(t.bytes);
    const perm = mode() === "permanent";
    $("preview-title").textContent = perm ? "Preview: permanent delete" : "Preview: move to Recycle Bin";
    $("preview-mode-text").textContent = perm
      ? "Nothing has been deleted yet. These items will be deleted permanently."
      : "Nothing has been deleted yet. These items will be moved to the Recycle Bin, where you can restore them.";
    $("preview-permanent").hidden = !perm;
    $("permanent-ack").checked = false;
    const btn = $("confirm-clean");
    btn.className = "btn " + (perm ? "danger" : "primary");
    btn.textContent = perm ? `Permanently delete ${fmtCount(t.count)} items` : `Move ${fmtCount(t.count)} items to Recycle Bin`;
    btn.disabled = perm;
    $("preview-dialog").showModal();
  }

  async function doClean() {
    const m = mode();
    const paths = [...state.selected.values()].filter((it) => !isBlocked(it.path)).map((it) => it.path);
    if (!paths.length) return;
    const btn = $("preview-btn");
    busy(btn, true);
    btn.textContent = "Cleaning…";
    try {
      const report = await api.clean(paths, m);
      state.lastReport = report;
      // Drop what was removed; failed items stay so they can be retried.
      const done = new Set(report.outcomes.filter((o) => !o.error).map((o) => DC.normPath(o.path)));
      for (const g of state.plan.groups) {
        g.items = g.items.filter((it) => !done.has(DC.normPath(it.path)));
        g.bytes = g.items.reduce((a, i) => a + i.bytes, 0);
      }
      for (const k of done) state.selected.delete(k);
      state.plan.safe_bytes = state.plan.groups.filter((g) => g.risk === "Safe").reduce((a, g) => a + g.bytes, 0);
      state.plan.review_bytes = state.plan.groups.filter((g) => g.risk !== "Safe").reduce((a, g) => a + g.bytes, 0);
      renderCleaner();
      showResult(report);
      refreshDrives();
    } catch (e) {
      toast("Clean failed: " + e.message, "error");
    } finally {
      busy(btn, false);
      btn.textContent = "Preview…";
      renderCleaner();
    }
  }

  function showResult(r) {
    const trash = r.mode === "trash";
    $("result-title").textContent = r.failed_count || r.rejected.length ? "Finished with some problems" : "Done";
    $("result-freed").textContent = fmtBytes(r.freed_bytes);
    $("result-sub").textContent = `${trash ? "moved to the Recycle Bin" : "deleted permanently"} · ${fmtCount(r.ok_count)} item${r.ok_count === 1 ? "" : "s"}${r.failed_count ? ` · ${fmtCount(r.failed_count)} failed` : ""}`;
    $("result-bin-note").hidden = !(trash && r.ok_count > 0);
    const purge = $("purge-btn");
    const canPurge = !!(state.cfg && state.cfg.can_purge_recycle_bin) && trash && r.ok_count > 0;
    purge.hidden = !canPurge;
    $("purge-hint").hidden = !canPurge;
    purge.disabled = false;
    purge.className = "btn small";
    purge.dataset.armed = "";
    purge.textContent = "Remove just these items from the Recycle Bin";
    const fails = r.outcomes.filter((o) => o.error);
    const box = $("result-failures");
    box.replaceChildren();
    if (fails.length) {
      box.append(
        h("h3", { class: "small" }, `Could not remove ${fails.length} item${fails.length === 1 ? "" : "s"}`),
        h("ul", { class: "fail-list", "data-testid": "fail-list" }, fails.map((o) => h("li", {}, h("code", {}, o.path), h("span", { class: "err" }, o.error))))
      );
    }
    if (r.rejected.length) {
      box.append(
        h("h3", { class: "small" }, `Refused ${r.rejected.length} path${r.rejected.length === 1 ? "" : "s"} that were not in the plan`),
        h("ul", { class: "fail-list" }, r.rejected.map((o) => h("li", {}, h("code", {}, o.path), h("span", { class: "err" }, o.reason))))
      );
    }
    $("result-dialog").showModal();
  }

  async function purgeRecycled() {
    const b = $("purge-btn");
    if (!b.dataset.armed) {
      b.dataset.armed = "1";
      b.className = "btn small danger";
      b.textContent = `Confirm: permanently remove ${fmtCount(state.lastReport.ok_count)} items from the Recycle Bin`;
      return;
    }
    b.disabled = true;
    try {
      const r = await api.purgeRecycled();
      b.hidden = true;
      $("purge-hint").hidden = true;
      $("result-bin-note").querySelector("p").textContent = `Removed ${fmtCount(r.purged)} item${r.purged === 1 ? "" : "s"} from the Recycle Bin. ${fmtBytes(r.bytes)} is now free.`;
      refreshDrives();
    } catch (e) {
      b.disabled = false;
      toast("Could not empty those items: " + e.message, "error");
    }
  }

  // ---------------- disk map ----------------
  function parentOf(p) {
    const a = DC.ancestors(p);
    return a.length > 1 ? a[a.length - 2] : null;
  }

  function openDiskMap() {
    const sel = $("drive-select");
    const roots = state.drives.length ? state.drives.map((d) => d.root) : state.drive ? [state.drive.root] : [];
    if (sel.options.length !== roots.length) sel.replaceChildren(...roots.map((r) => h("option", { value: r }, r)));
    if (!state.map.path && roots.length) navigate(roots[0]);
    else if (state.map.path) renderMap();
  }

  async function navigate(path, force) {
    const m = state.map;
    if (m.scan) {
      api.cancelScan(m.scan.id);
      m.scan = null;
    }
    if (!force && m.cache.has(DC.normPath(path))) {
      m.path = path;
      return renderMap();
    }
    const prev = m.path;
    m.path = path;
    renderBreadcrumb();
    const prog = $("scan-progress");
    prog.hidden = false;
    if (!m.cache.has(DC.normPath(prev || ""))) {
      $("treemap").replaceChildren(h("div", { class: "tm-placeholder" }, h("span", { class: "spinner" }), `Measuring ${path}. On a full drive this can take a few minutes; you can keep using the other screens.`));
    }
    $("scan-progress-text").textContent = `Scanning ${path}…`;
    const job = api.scanDir(path, 40, (p) => {
      if (m.scan !== job) return;
      $("scan-progress-text").textContent = `Scanning ${path} · ${fmtCount(p.files)} files · ${fmtBytes(p.bytes)} · ${p.current}`;
    });
    m.scan = job;
    try {
      const res = await job.promise;
      if (m.scan !== job) return;
      m.cache.set(DC.normPath(path), res);
      m.scan = null;
      prog.hidden = true;
      renderMap();
    } catch (e) {
      if (m.scan !== job && m.scan !== null) return; // superseded by a newer scan
      m.scan = null;
      prog.hidden = true;
      if (e.message !== "cancelled") toast("Scan failed: " + e.message, "error");
      m.path = prev && m.cache.has(DC.normPath(prev)) ? prev : m.path;
      renderMap();
    }
  }

  function renderBreadcrumb() {
    const path = state.map.path;
    const parts = DC.ancestors(path);
    const nav = $("breadcrumb");
    nav.replaceChildren();
    parts.forEach((p, i) => {
      if (i) nav.append(icon("chevron", "crumb-sep"));
      const last = i === parts.length - 1;
      const name = i === 0 ? p : DC.baseName(p);
      nav.append(h("button", { class: "crumb" + (last ? " current" : ""), "data-path": p, "aria-current": last ? "page" : null, onclick: last ? null : () => navigate(p) }, isBlocked(p) && protectedPaths().some((x) => DC.isInside(p, x)) ? icon("lock") : null, name));
    });
    $("map-up").disabled = !parentOf(path);
    const sel = $("drive-select");
    const root = parts[0];
    for (const o of sel.options) if (DC.normPath(o.value) === DC.normPath(root)) sel.value = o.value;
  }

  const tooltip = () => $("tm-tooltip");
  function onHover(ev, tile, share) {
    const tt = tooltip();
    if (!ev) return (tt.hidden = true);
    const e = tile.entry;
    tt.replaceChildren(
      ...[h("div", { class: "tt-name" }, e.path || e.name),
      h("div", { class: "tt-meta" }, `${fmtBytes(e.bytes)} · ${(share * 100).toFixed(1)}% of this folder${e.files ? ` · ${fmtCount(e.files)} files` : ""}`),
      e.locked ? h("div", { class: "tt-lock" }, icon("lock"), "Protected: never cleaned") : null,
      tile.kind === "dir" ? h("div", { class: "tt-meta" }, "Click to open") : null,
    ].filter(Boolean));
    tt.hidden = false;
    const host = $("treemap").parentElement.getBoundingClientRect();
    const x = ev.clientX - host.left + 14;
    const y = ev.clientY - host.top + 14;
    const w = tt.offsetWidth, hgt = tt.offsetHeight;
    tt.style.left = Math.min(x, host.width - w - 8) + "px";
    tt.style.top = (y + hgt > host.height ? y - hgt - 24 : y) + "px";
  }

  function renderMap() {
    const m = state.map;
    renderBreadcrumb();
    const res = m.cache.get(DC.normPath(m.path));
    if (!res) return;
    DC.renderTreemap($("treemap"), res, { onDrill: (e) => navigate(e.path), onHover });
    $("map-total").replaceChildren(
      ...[h("div", { class: "big" }, fmtBytes(res.total_bytes)),
      h("div", { class: "muted" }, `${fmtCount(res.files)} files${res.errors ? ` · ${fmtCount(res.errors)} unreadable entries skipped` : ""}`),
      res.locked ? h("div", { class: "badge locked" }, icon("lock"), "Protected folder") : null,
    ].filter(Boolean));
    const tbody = $("size-table").querySelector("tbody");
    tbody.replaceChildren(
      ...[...res.children.map((c) =>
        h(
          "tr",
          { class: c.is_dir ? "drillable" : "", onclick: c.is_dir ? () => navigate(c.path) : null, title: c.path },
          h("td", { class: "name" }, c.locked ? icon("lock", "lock") : icon(c.is_dir ? "folder" : "map"), c.name),
          h("td", { class: "num" }, fmtBytes(c.bytes)),
          h("td", { class: "num" }, DC.pct(c.bytes, res.total_bytes).toFixed(1))
        )
      ),
      res.other_count ? h("tr", {}, h("td", { class: "name muted" }, `${fmtCount(res.other_count)} smaller items`), h("td", { class: "num" }, fmtBytes(res.other_bytes)), h("td", { class: "num" }, DC.pct(res.other_bytes, res.total_bytes).toFixed(1))) : null,
    ].filter(Boolean));
  }

  // ---------------- settings ----------------
  let draft = null;
  function setDirty(on) {
    $("save-settings").disabled = !on;
  }

  function pathItem(p, { locked, onRemove, note }) {
    return h(
      "li",
      { class: "path-item" + (locked ? " locked" : ""), "data-path": p },
      locked ? icon("lock", "lock") : icon("folder"),
      h("code", { title: p }, h("bdi", {}, p)),
      note ? h("span", { class: "badge locked" }, note) : null,
      onRemove ? h("button", { class: "btn-icon", title: "Remove", "aria-label": "Remove " + p, onclick: onRemove }, icon("x")) : null
    );
  }

  function renderSettings() {
    if (!state.cfg) return;
    if (!draft) draft = JSON.parse(JSON.stringify(state.cfg.config));
    $("config-path").textContent = "Saved to " + state.cfg.config_path;
    const locked = state.cfg.locked.map(DC.normPath);
    const isLocked = (p) => locked.includes(DC.normPath(p));
    $("protected-list").replaceChildren(
      ...draft.protected.map((p) =>
        pathItem(p, {
          locked: isLocked(p),
          note: isLocked(p) ? "Always protected" : null,
          onRemove: isLocked(p)
            ? null
            : () => {
                draft.protected = draft.protected.filter((x) => x !== p);
                setDirty(true);
                renderSettings();
              },
        })
      )
    );
    $("roots-list").replaceChildren(
      ...(draft.project_roots.length
        ? draft.project_roots.map((p) =>
            pathItem(p, {
              onRemove: () => {
                draft.project_roots = draft.project_roots.filter((x) => x !== p);
                setDirty(true);
                renderSettings();
              },
            })
          )
        : [h("li", { class: "muted" }, "No project roots. Dev build folders will not be checked.")])
    );
    $("dl-age").value = draft.downloads_min_age_days;
    $("dev-age").value = draft.dev_min_age_days;
    $("low-pct").value = draft.low_space_percent;
    $("dl-dir").textContent = draft.downloads_dir || "(none)";
    $("dl-dir").title = draft.downloads_dir || "";
  }

  async function pickInto(key, title) {
    try {
      const p = await api.pickFolder(title);
      if (!p) return;
      if (key === "downloads_dir") draft.downloads_dir = p;
      else if (!draft[key].some((x) => DC.normPath(x) === DC.normPath(p))) draft[key].push(p);
      setDirty(true);
      renderSettings();
    } catch (e) {
      toast("Folder picker failed: " + e.message, "error");
    }
  }

  async function saveSettings() {
    draft.downloads_min_age_days = Number($("dl-age").value);
    draft.dev_min_age_days = Number($("dev-age").value);
    draft.low_space_percent = Number($("low-pct").value);
    try {
      const r = await api.saveConfig(draft);
      state.cfg = r.view;
      draft = null;
      setDirty(false);
      const msg = $("settings-msg");
      msg.className = "banner " + (r.warnings.length ? "warn" : "info");
      msg.replaceChildren(icon(r.warnings.length ? "warn" : "check"), h("div", {}, h("p", {}, "Settings saved." + (state.plan ? " Run Analyze again to use them." : "")), r.warnings.length ? h("ul", {}, r.warnings.map((w) => h("li", {}, w))) : null));
      msg.hidden = false;
      // The backend dropped its plan; drop ours so nothing stale can be cleaned.
      setPlan(null);
      state.map.cache.clear();
      state.map.path = null;
      renderSettings();
      renderDashboard();
    } catch (e) {
      toast("Could not save settings: " + e.message, "error");
    }
  }

  // ---------------- wiring ----------------
  function wire() {
    for (const b of document.querySelectorAll(".nav-item")) b.addEventListener("click", () => show(b.dataset.screen));
    $("analyze-btn").addEventListener("click", analyze);
    $("reanalyze-btn").addEventListener("click", analyze);
    $("cleaner-analyze").addEventListener("click", analyze);
    $("goto-cleaner").addEventListener("click", () => show("cleaner"));
    $("preview-btn").addEventListener("click", openPreview);
    for (const r of document.querySelectorAll('input[name="mode"]'))
      r.addEventListener("change", () => ($("mode-warn").hidden = mode() !== "permanent"));
    $("permanent-ack").addEventListener("change", (e) => ($("confirm-clean").disabled = !e.target.checked));
    $("preview-dialog").addEventListener("close", () => {
      if ($("preview-dialog").returnValue === "confirm") doClean();
    });
    $("purge-btn").addEventListener("click", purgeRecycled);
    $("map-up").addEventListener("click", () => {
      const p = parentOf(state.map.path);
      if (p) navigate(p);
    });
    $("map-rescan").addEventListener("click", () => state.map.path && navigate(state.map.path, true));
    $("drive-select").addEventListener("change", (e) => navigate(e.target.value));
    $("scan-cancel").addEventListener("click", () => {
      if (state.map.scan) api.cancelScan(state.map.scan.id);
    });
    $("add-protected").addEventListener("click", () => pickInto("protected", "Choose a folder to protect"));
    $("add-root").addEventListener("click", () => pickInto("project_roots", "Choose a project root"));
    $("pick-dl").addEventListener("click", () => pickInto("downloads_dir", "Choose your Downloads folder"));
    for (const id of ["dl-age", "dev-age", "low-pct"]) $(id).addEventListener("input", () => setDirty(true));
    $("save-settings").addEventListener("click", saveSettings);
    let rt = null;
    window.addEventListener("resize", () => {
      clearTimeout(rt);
      rt = setTimeout(() => state.screen === "diskmap" && renderMap(), 120);
    });
  }

  async function init() {
    wire();
    await api.ready;
    $("mock-badge").hidden = !api.isMock;
    try {
      state.cfg = await api.getConfig();
    } catch (e) {
      toast("Could not load settings: " + e.message, "error");
    }
    show(new URLSearchParams(location.search).get("screen") || "dashboard");
    await refreshDrives();
  }

  // Exposed for tests and debugging only.
  DC.app = { state, show, analyze, navigate };
  init();
})(window.DC);
