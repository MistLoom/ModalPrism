#!/usr/bin/env node
import { chromium } from "playwright";
import { mkdir } from "node:fs/promises";
import path from "node:path";

const base = process.argv[2] ?? "http://127.0.0.1:18901";
const outDir = process.env.CONSOLE_ARTIFACTS ?? "/opt/cursor/artifacts/console";
const routes = ["overview", "sources", "records", "governance", "config"];

await mkdir(outDir, { recursive: true });
const browser = await chromium.launch({ headless: true });
const context = await browser.newContext({
  viewport: { width: 1440, height: 900 },
  deviceScaleFactor: 1,
});
const mockIndex = path.resolve(import.meta.dirname, "../mock-design/index.html");
const mockUrl = `file://${mockIndex}`;
const mockRoutes = [
  ["overview", "overview"],
  ["sources", "sources"],
  ["records", "strata"],
  ["governance", "governance"],
  ["config", "config"],
];
for (const [name, route] of mockRoutes) {
  const page = await context.newPage();
  await page.goto(mockUrl, { waitUntil: "networkidle" });
  await page.locator(`.nav .nav-item[data-route="${route}"]`).click();
  await page.waitForTimeout(400);
  await page.screenshot({ path: path.join(outDir, `mock-${name}-dark.png`) });
  await page.close();
}

for (const theme of ["light", "dark"]) {
  for (const hash of routes) {
    const page = await context.newPage();
    await page.goto(`${base}/console?theme=${theme}#${hash}`, { waitUntil: "networkidle" });
    await page.waitForSelector("#viewRoot .page-title, #viewRoot .empty, #viewRoot .error", {
      timeout: 120000,
    });
    await page.waitForFunction(() => !document.querySelector("#viewRoot .loading"));
    await page.screenshot({ path: path.join(outDir, `${hash}-${theme}.png`) });
    await page.close();
  }

  const rec = await context.newPage();
  await rec.goto(`${base}/console?theme=${theme}#records`, { waitUntil: "networkidle" });
  await rec.waitForSelector("#viewRoot .page-title", { timeout: 120000 });
  const row = rec.locator(".rec-row").first();
  if (await row.count()) {
    await row.click();
    await rec.waitForSelector(".drawer-head .drawer-title", { timeout: 60000 });
    await rec.waitForFunction(() => !document.querySelector(".drawer-body .loading"));
    await rec.screenshot({ path: path.join(outDir, `record-drawer-${theme}.png`) });
  } else {
    await rec.screenshot({ path: path.join(outDir, `records-empty-${theme}.png`) });
  }
  await rec.close();

  const cfg = await context.newPage();
  await cfg.goto(`${base}/console?theme=${theme}#config`, { waitUntil: "networkidle" });
  await cfg.waitForSelector(".cfg-textarea", { timeout: 120000 });
  const ta = cfg.locator(".cfg-textarea");
  const original = await ta.inputValue();
  await ta.fill(`${original}\n# screenshot-diff\n`);
  await cfg.locator("#cfgToggleDiff").click();
  await cfg.waitForSelector(".cfg-diff-view:not([hidden])");
  await cfg.screenshot({ path: path.join(outDir, `config-editor-diff-${theme}.png`) });
  await cfg.close();

  const confirm = await context.newPage();
  await confirm.goto(`${base}/console?theme=${theme}#sources`, { waitUntil: "networkidle" });
  await confirm.waitForSelector("[data-act='toggle']");
  await confirm.locator("[data-act='toggle']").first().click();
  await confirm.waitForSelector(".confirm:not([hidden])");
  await confirm.screenshot({ path: path.join(outDir, `confirm-dialog-${theme}.png`) });
  await confirm.close();

  const palette = await context.newPage();
  await palette.goto(`${base}/console?theme=${theme}#overview`, { waitUntil: "networkidle" });
  await palette.locator("#paletteBtn").click();
  await palette.waitForSelector(".palette:not([hidden])");
  await palette.screenshot({ path: path.join(outDir, `command-palette-${theme}.png`) });
  await palette.close();

  const hist = await context.newPage();
  await hist.goto(`${base}/console?theme=${theme}#overview`, { waitUntil: "networkidle" });
  await hist.waitForSelector(".custom-select-trigger");
  await hist.locator(".sidebar-foot .custom-select-trigger").click();
  const opt = hist.locator(".sidebar-foot .custom-select-menu li").nth(1);
  if (await opt.count()) {
    await opt.click();
    await hist.waitForSelector("#historyBanner:not([hidden])");
    await hist.screenshot({ path: path.join(outDir, `history-mode-${theme}.png`) });
  }
  await hist.close();

  const src = await context.newPage();
  await src.goto(`${base}/console?theme=${theme}#sources`, { waitUntil: "networkidle" });
  await src.waitForSelector("[data-act='detail']", { timeout: 120000 });
  await src.locator("[data-act='detail']").first().click();
  await src.waitForSelector("#drawer .drawer-title", { timeout: 60000 });
  await src.screenshot({ path: path.join(outDir, `source-drawer-${theme}.png`) });
  await src.close();
}

await browser.close();
console.log("screenshots in", outDir);
