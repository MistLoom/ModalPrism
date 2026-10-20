#!/usr/bin/env node
import { chromium } from "playwright";
import { mkdir } from "node:fs/promises";
import path from "node:path";

const base = process.argv[2] ?? "http://127.0.0.1:18901";
const outDir = process.env.CONSOLE_ARTIFACTS ?? "/opt/cursor/artifacts/console";
const routes = ["overview", "sources", "records", "governance", "config"];

await mkdir(outDir, { recursive: true });
const browser = await chromium.launch({ headless: true });
const context = await browser.newContext({ viewport: { width: 1400, height: 900 } });

for (const theme of ["light", "dark"]) {
  for (const hash of routes) {
    const page = await context.newPage();
    await page.goto(`${base}/console/?theme=${theme}#${hash}`, { waitUntil: "networkidle" });
    await page.waitForSelector("#viewRoot .page-title, #viewRoot .empty, #viewRoot .error", { timeout: 60000 });
    await page.waitForFunction(() => !document.querySelector("#viewRoot .loading"));
    await page.screenshot({ path: path.join(outDir, `${hash}-${theme}.png`) });
    await page.close();
  }
  const page = await context.newPage();
  await page.goto(`${base}/console/?theme=${theme}#records`, { waitUntil: "networkidle" });
  await page.waitForSelector(".rec-row, .empty");
  const row = page.locator(".rec-row").first();
  if (await row.count()) {
    await row.click();
    await page.waitForSelector(".drawer:not([hidden])");
    await page.screenshot({ path: path.join(outDir, `record-drawer-${theme}.png`) });
  }
  await page.close();
  const cfg = await context.newPage();
  await cfg.goto(`${base}/console/?theme=${theme}#config`, { waitUntil: "networkidle" });
  await cfg.waitForSelector("#cfgEditor");
  await cfg.screenshot({ path: path.join(outDir, `config-editor-${theme}.png`) });
  await cfg.close();
}

await browser.close();
console.log("screenshots in", outDir);
