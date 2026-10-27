// UI behaviour tests against the mock backend (ui/mock.js).
// Usage: node crates/app/ui-tests/ui.test.js
// Needs the pre-installed `playwright` package and Chromium (no download).
"use strict";
const assert = require("assert/strict");
const path = require("path");
const { chromium } = require("playwright");

const UI = "file://" + path.resolve(__dirname, "../ui/index.html");
const tests = [];
// CSS attribute value for a Windows path (backslashes must be escaped in CSS).
const css = (p) => `"${p.replace(/\\/g, "\\\\")}"`;
const test = (name, fn) => tests.push({ name, fn });

async function open(browser, query = "?fast=1") {
  const page = await browser.newPage({ viewport: { width: 1920, height: 1080 } });
  const errors = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
  await page.goto(UI + query);
  await page.waitForSelector("[data-testid=gauge]");
  page.errors = errors;
  return page;
}

async function analyzeAndExpandAll(page) {
  await page.click("[data-testid=analyze-btn]");
  await page.waitForSelector("[data-testid=cat-temp]");
  // The list re-renders on every click, so re-query by name each time.
  const ids = await page.$$eval("[data-testid^=cat-expand-]", (bs) => bs.map((b) => b.dataset.testid));
  for (const id of ids) await page.click(`[data-testid=${id}]`);
}

const itemStates = (page) =>
  page.$$eval("[data-testid=item-check]", (els) =>
    els.map((e) => ({ path: e.dataset.path, checked: e.checked, disabled: e.disabled, risk: e.closest("[data-risk]").dataset.risk }))
  );

// --------------------------------------------------------------------------

test("Safe items are checked and Review items unchecked by default", async (b) => {
  const page = await open(b);
  await analyzeAndExpandAll(page);
  const items = await itemStates(page);
  assert.ok(items.length > 20, "expected a realistic plan");
  const safe = items.filter((i) => i.risk === "Safe");
  const review = items.filter((i) => i.risk === "Review");
  assert.ok(safe.length > 0 && review.length > 0);
  assert.ok(safe.every((i) => i.checked), "every Safe item checked");
  assert.ok(review.every((i) => !i.checked), "no Review item checked");
  const cats = await page.$$eval("[data-testid^=cat-check-]", (els) =>
    els.map((e) => ({ checked: e.checked, risk: e.closest("[data-risk]").dataset.risk, disabled: e.disabled }))
  );
  assert.ok(cats.filter((c) => c.risk === "Safe" && !c.disabled).every((c) => c.checked));
  assert.ok(cats.filter((c) => c.risk === "Review").every((c) => !c.checked));
  return page;
});

test("Videos never appears in the plan or as a selectable item", async (b) => {
  const page = await open(b);
  await analyzeAndExpandAll(page);
  const items = await itemStates(page);
  assert.ok(!items.some((i) => /\\Videos(\\|$)/i.test(i.path)));
  return page;
});

test("Even if the backend returned a Videos item, the UI will not select it", async (b) => {
  const page = await open(b, "?fast=1&rogue=1");
  await analyzeAndExpandAll(page);
  const items = await itemStates(page);
  const rogue = items.filter((i) => /\\Videos\\/i.test(i.path));
  assert.equal(rogue.length, 1, "rogue item is shown");
  assert.ok(rogue[0].disabled && !rogue[0].checked, "rogue item disabled and unchecked");
  // Forcing a click on the disabled box must not select it.
  await page.click('[data-testid=item-check][data-path*="\\\\Videos\\\\"]', { force: true });
  // Select everything via the category boxes, then preview.
  const unchecked = await page.$$eval("[data-testid^=cat-check-]:not(:checked):not(:disabled)", (cs) => cs.map((c) => c.dataset.testid));
  for (const id of unchecked) await page.click(`[data-testid=${id}]`);
  const selected = await page.evaluate(() => [...DC.app.state.selected.values()].map((i) => i.path));
  assert.ok(!selected.some((p) => /\\Videos\\/i.test(p)), "Videos not in selection");
  await page.click("[data-testid=preview-btn]");
  const expected = await page.evaluate(() =>
    DC.fmtBytes(DC.app.state.plan.groups.flatMap((g) => g.items).filter((i) => !/\\Videos\\/i.test(i.path)).reduce((a, i) => a + i.bytes, 0))
  );
  assert.equal(await page.textContent("[data-testid=preview-total]"), expected);
  // And the backend refuses it too, whatever the UI sends.
  const report = await page.evaluate(() => DC.api.clean(["C:\\Users\\streamer\\Videos", "C:\\Users\\streamer\\Videos\\OBS"], "trash"));
  assert.equal(report.ok_count, 0);
  assert.equal(report.rejected.length, 2);
  return page;
});

test("Videos is locked in Settings and on the Disk Map", async (b) => {
  const page = await open(b);
  await page.click("[data-testid=nav-settings]");
  const videos = page.locator(`[data-testid=protected-list] li[data-path=${css("C:\\Users\\streamer\\Videos")}]`);
  await videos.waitFor();
  assert.equal(await videos.locator("button").count(), 0, "no remove button on Videos");
  assert.match(await videos.textContent(), /Always protected/);
  // Saving a config without Videos gets it re-added by the backend.
  const r = await page.evaluate(async () => {
    const v = await DC.api.getConfig();
    v.config.protected = v.config.protected.filter((p) => !/Videos$/.test(p));
    return DC.api.saveConfig(v.config);
  });
  assert.ok(r.view.config.protected.some((p) => /\\Videos$/.test(p)));
  assert.ok(r.warnings.some((w) => /always protected/.test(w)));

  await page.click("[data-testid=nav-diskmap]");
  await page.click('[data-testid=treemap] g[data-name="Users"]');
  await page.click('[data-testid=treemap] g[data-name="streamer"]');
  const tile = page.locator('[data-testid=treemap] g[data-name="Videos"]');
  await tile.waitFor();
  assert.equal(await tile.getAttribute("data-locked"), "1");
  return page;
});

test("Preview shows correct per-category and total sizes", async (b) => {
  const page = await open(b);
  await analyzeAndExpandAll(page);
  // Add one Review category and untick one Safe item.
  await page.click("[data-testid=cat-check-package-cache]");
  await page.locator("[data-testid=cat-temp] [data-testid=item-check]").first().click();
  const exp = await page.evaluate(() => {
    const sel = [...DC.app.state.selected.values()];
    const by = {};
    for (const i of sel) {
      by[i.category] = by[i.category] || { n: 0, bytes: 0 };
      by[i.category].n++;
      by[i.category].bytes += i.bytes;
    }
    // Independent of the UI's selection map: recompute from the DOM checkboxes.
    const domPaths = [...document.querySelectorAll("[data-testid=item-check]:checked")].map((e) => e.dataset.path);
    const all = DC.app.state.plan.groups.flatMap((g) => g.items);
    const domBytes = all.filter((i) => domPaths.includes(i.path)).reduce((a, i) => a + i.bytes, 0);
    return { by, n: sel.length, bytes: sel.reduce((a, i) => a + i.bytes, 0), domBytes, domN: domPaths.length };
  });
  assert.equal(exp.bytes, exp.domBytes);
  assert.equal(exp.n, exp.domN);
  assert.equal(await page.textContent("[data-testid=sel-total]"), await page.evaluate((x) => DC.fmtBytes(x), exp.bytes));
  await page.click("[data-testid=preview-btn]");
  await page.waitForSelector("[data-testid=preview-dialog][open]");
  assert.equal(await page.textContent("[data-testid=preview-count]"), String(exp.n));
  assert.equal(await page.textContent("[data-testid=preview-total]"), await page.evaluate((x) => DC.fmtBytes(x), exp.bytes));
  const rows = await page.$$eval("[data-testid=preview-row]", (rs) => rs.map((r) => ({ cat: r.dataset.cat, n: r.children[1].textContent, size: r.children[2].textContent })));
  assert.deepEqual(rows.map((r) => r.cat).sort(), Object.keys(exp.by).sort());
  for (const r of rows) {
    assert.equal(r.n, String(exp.by[r.cat].n));
    assert.equal(r.size, await page.evaluate((x) => DC.fmtBytes(x), exp.by[r.cat].bytes));
  }
  assert.match(await page.textContent("[data-testid=confirm-clean]"), /Recycle Bin/);
  return page;
});

test("Permanent mode warns and needs an explicit acknowledgement", async (b) => {
  const page = await open(b);
  await analyzeAndExpandAll(page);
  await page.click("[data-testid=mode-permanent]", { force: true });
  assert.ok(await page.isVisible("#mode-warn"));
  await page.click("[data-testid=preview-btn]");
  assert.ok(await page.isDisabled("[data-testid=confirm-clean]"));
  await page.check("[data-testid=permanent-ack]");
  assert.ok(await page.isEnabled("[data-testid=confirm-clean]"));
  await page.click("[data-testid=confirm-clean]");
  await page.waitForSelector("[data-testid=result-dialog][open]");
  assert.ok(await page.isHidden("[data-testid=purge-btn]"), "no Recycle Bin button after permanent delete");
  return page;
});

test("Recycle Bin clean reports that space is not free yet and offers a scoped purge", async (b) => {
  const page = await open(b);
  await analyzeAndExpandAll(page);
  await page.click("[data-testid=preview-btn]");
  await page.click("[data-testid=confirm-clean]");
  await page.waitForSelector("[data-testid=result-dialog][open]");
  assert.match(await page.textContent("#result-bin-note"), /not free yet/);
  await page.click("[data-testid=purge-btn]");
  assert.match(await page.textContent("[data-testid=purge-btn]"), /Confirm/);
  await page.click("[data-testid=purge-btn]");
  await page.waitForFunction(() => /is now free/.test(document.querySelector("#result-bin-note").textContent));
  return page;
});

test("Treemap drill-down and breadcrumb navigation", async (b) => {
  const page = await open(b);
  await page.click("[data-testid=nav-diskmap]");
  await page.waitForSelector("[data-testid=treemap] g.tm-tile");
  const crumbs = () => page.$$eval("[data-testid=breadcrumb] .crumb", (cs) => cs.map((c) => c.textContent));
  assert.deepEqual(await crumbs(), ["C:\\"]);
  await page.click('[data-testid=treemap] g[data-name="Users"]');
  await page.waitForSelector('[data-testid=treemap] g[data-name="streamer"]');
  assert.deepEqual(await crumbs(), ["C:\\", "Users"]);
  await page.click('[data-testid=treemap] g[data-name="streamer"]');
  await page.waitForSelector('[data-testid=treemap] g[data-name="AppData"]');
  await page.click('[data-testid=treemap] g[data-name="AppData"]');
  await page.waitForSelector('[data-testid=treemap] g[data-name="Local"]');
  assert.deepEqual(await crumbs(), ["C:\\", "Users", "streamer", "AppData"]);
  // Table lists the same children, largest first.
  const sizes = await page.$$eval("[data-testid=size-table] tbody tr", (rs) => rs.length);
  assert.ok(sizes >= 3);
  await page.click(`[data-testid=breadcrumb] .crumb[data-path=${css("C:\\Users")}]`);
  await page.waitForSelector('[data-testid=treemap] g[data-name="streamer"]');
  assert.deepEqual(await crumbs(), ["C:\\", "Users"]);
  await page.click("#map-up");
  await page.waitForSelector('[data-testid=treemap] g[data-name="Windows"]');
  assert.deepEqual(await crumbs(), ["C:\\"]);
  // Tiles stay inside the canvas and do not overlap.
  const bad = await page.evaluate(() => {
    const svg = document.querySelector("[data-testid=treemap] svg");
    const W = +svg.getAttribute("width"), H = +svg.getAttribute("height");
    const rs = [...svg.querySelectorAll("g.tm-tile")].map((g) => {
      const [x, y] = g.getAttribute("transform").match(/[-\d.]+/g).map(Number);
      const r = g.querySelector("rect");
      return { x, y, w: +r.getAttribute("width"), h: +r.getAttribute("height") };
    });
    const out = rs.filter((r) => r.x < -0.5 || r.y < -0.5 || r.x + r.w > W + 0.5 || r.y + r.h > H + 0.5).length;
    let overlaps = 0;
    for (let i = 0; i < rs.length; i++)
      for (let j = i + 1; j < rs.length; j++) {
        const a = rs[i], c = rs[j];
        if (a.x + a.w > c.x + 0.5 && c.x + c.w > a.x + 0.5 && a.y + a.h > c.y + 0.5 && c.y + c.h > a.y + 0.5) overlaps++;
      }
    return { out, overlaps, n: rs.length };
  });
  assert.equal(bad.out, 0);
  assert.equal(bad.overlaps, 0);
  return page;
});

test("squarify: areas proportional, aspect ratios sane", async (b) => {
  const page = await open(b);
  const r = await page.evaluate(() => {
    const vals = [500, 300, 120, 80, 40, 20, 10, 5, 3, 1];
    const rects = DC.squarify(vals, 0, 0, 800, 500);
    const total = vals.reduce((a, v) => a + v, 0);
    const errs = rects.map((q, i) => Math.abs((q.w * q.h) / (800 * 500) - vals[i] / total));
    const worst = Math.max(...rects.slice(0, 6).map((q) => Math.max(q.w / q.h, q.h / q.w)));
    const zero = DC.squarify([0, 5], 0, 0, 100, 100);
    return { maxErr: Math.max(...errs), worst, zeroW: zero[0].w, oneArea: zero[1].w * zero[1].h };
  });
  assert.ok(r.maxErr < 1e-9, "area error " + r.maxErr);
  assert.ok(r.worst < 4, "aspect ratio " + r.worst);
  assert.equal(r.zeroW, 0);
  assert.ok(Math.abs(r.oneArea - 10000) < 1e-6);
  return page;
});

test("Scans can be cancelled and keep the previous view", async (b) => {
  const page = await open(b, "?"); // real (slow) mock delays
  await page.click("[data-testid=nav-diskmap]");
  await page.waitForSelector('[data-testid=treemap] g[data-name="Users"]');
  await page.click('[data-testid=treemap] g[data-name="Users"]');
  await page.waitForSelector("[data-testid=scan-progress]:not([hidden])");
  await page.click("[data-testid=scan-cancel]");
  await page.waitForSelector("[data-testid=scan-progress]", { state: "hidden" });
  const crumbs = await page.$$eval("[data-testid=breadcrumb] .crumb", (cs) => cs.map((c) => c.textContent));
  assert.deepEqual(crumbs, ["C:\\"]);
  assert.ok(await page.isVisible('[data-testid=treemap] g[data-name="Users"]'));
  return page;
});

test("Low-space warning follows the threshold", async (b) => {
  const page = await open(b);
  await page.waitForSelector("[data-testid=low-space-banner]:not([hidden])");
  const ok = await open(b, "?fast=1&nolow=1");
  await ok.waitForTimeout(100);
  assert.ok(await ok.isHidden("[data-testid=low-space-banner]"));
  await ok.close();
  return page;
});

test("Settings: add a project root with the folder picker and save", async (b) => {
  const page = await open(b);
  await page.click("[data-testid=nav-settings]");
  await page.click("[data-testid=add-root]");
  await page.waitForSelector(`[data-testid=roots-list] li[data-path=${css("C:\\Users\\streamer\\Music")}]`);
  assert.ok(await page.isEnabled("[data-testid=save-settings]"));
  await page.fill("#dev-age", "0");
  await page.click("[data-testid=save-settings]");
  await page.waitForSelector("#settings-msg:not([hidden])");
  const cfg = await page.evaluate(() => DC.api.getConfig());
  assert.ok(cfg.config.project_roots.includes("C:\\Users\\streamer\\Music"));
  assert.equal(cfg.config.dev_min_age_days, 1, "clamped to at least 1 day");
  assert.match(await page.textContent("#settings-msg"), /clamped/);
  return page;
});

// --------------------------------------------------------------------------

(async () => {
  const browser = await chromium.launch();
  let failed = 0;
  for (const t of tests) {
    let page;
    try {
      page = await t.fn(browser);
      if (page && page.errors.length) throw new Error("page errors: " + page.errors.join("; "));
      console.log("ok   ", t.name);
    } catch (e) {
      failed++;
      console.log("FAIL ", t.name, "\n      ", e.message.split("\n").slice(0, 6).join("\n       "));
    } finally {
      if (page) await page.close();
    }
  }
  await browser.close();
  console.log(`\n${tests.length - failed} passed, ${failed} failed`);
  process.exit(failed ? 1 : 0);
})();
