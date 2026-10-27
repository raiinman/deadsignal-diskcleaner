// Screenshots of every screen against the mock backend.
// Usage: node crates/app/ui-tests/screenshots.js [outDir]
// Uses the pre-installed Playwright + Chromium; does not download browsers.
const path = require("path");
const { chromium } = require("playwright");

const UI = "file://" + path.resolve(__dirname, "../ui/index.html");
const OUT = path.resolve(process.argv[2] || path.join(__dirname, "../screenshots"));

(async () => {
  const browser = await chromium.launch();
  const errors = [];
  for (const [w, hgt, tag] of [[1920, 1080, "1080p"], [2560, 1440, "1440p"]]) {
    for (const scheme of ["dark", "light"]) {
      if (tag === "1440p" && scheme === "light") continue;
      const page = await browser.newPage({ viewport: { width: w, height: hgt }, colorScheme: scheme });
      page.on("pageerror", (e) => errors.push(`${tag}/${scheme}: ${e.message}`));
      page.on("console", (m) => m.type() === "error" && errors.push(`${tag}/${scheme}: ${m.text()}`));
      const shot = (name) => page.screenshot({ path: path.join(OUT, `${name}-${tag}-${scheme}.png`) });
      await page.goto(UI + "?fast=1");
      await page.waitForSelector("[data-testid=gauge]");
      await shot("1-dashboard");
      await page.click("[data-testid=analyze-btn]");
      await page.waitForSelector("[data-testid=cat-temp]");
      await page.click("[data-testid=cat-expand-temp]");
      await page.click("[data-testid=cat-expand-dev-artifacts]");
      await page.evaluate(() => { document.querySelector(".content").scrollTop = 0; document.activeElement.blur(); });
      await shot("2-cleaner");
      await page.click("[data-testid=preview-btn]");
      await shot("3-cleaner-preview");
      await page.click("[data-testid=preview-cancel]");
      await page.click("[data-testid=nav-diskmap]");
      await page.waitForSelector("[data-testid=treemap] svg g");
      await shot("4-diskmap-root");
      await page.click('[data-testid=treemap] g[data-name="Users"]');
      await page.waitForSelector('[data-testid=breadcrumb] .crumb.current:text("Users")');
      await page.click('[data-testid=treemap] g[data-name="streamer"]');
      await page.waitForSelector('[data-testid=breadcrumb] .crumb.current:text("streamer")');
      await page.hover('[data-testid=treemap] g[data-name="Videos"]');
      await shot("5-diskmap-home");
      await page.click("[data-testid=nav-settings]");
      await shot("6-settings");
      // Result dialog after a Recycle Bin clean.
      await page.click("[data-testid=nav-cleaner]");
      await page.click("[data-testid=preview-btn]");
      await page.click("[data-testid=confirm-clean]");
      await page.waitForSelector("[data-testid=result-dialog][open]");
      await shot("7-result");
      await page.close();
    }
  }
  await browser.close();
  if (errors.length) {
    console.error("Page errors:\n" + errors.join("\n"));
    process.exit(1);
  }
  console.log("screenshots written to", OUT);
})();
