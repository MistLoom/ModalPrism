#!/usr/bin/env node
/**
 * 在已运行的采集进程上截取控制台各视图（浅色/深色）。
 * 用法: node scripts/console-e2e-screenshots.mjs [baseUrl]
 */
import { chromium } from "playwright";
import { mkdir } from "node:fs/promises";
import path from "node:path";

const base = process.argv[2] ?? "http://127.0.0.1:17895";
const outDir = process.env.CONSOLE_ARTIFACTS ?? "/opt/cursor/artifacts/console";
const routes = [
  { hash: "overview", name: "overview" },
  { hash: "sources", name: "sources" },
  { hash: "records", name: "records" },
  { hash: "governance", name: "governance" },
  { hash: "config", name: "config" },
];

await mkdir(outDir, { recursive: true });

const browser = await chromium.launch({ headless: true });
const context = await browser.newContext({
  viewport: { width: 1400, height: 900 },
  locale: "zh-CN",
});

for (const theme of ["light", "dark"]) {
  const page = await context.newPage();
  for (const { hash, name } of routes) {
    const url = `${base}/console?theme=${theme}#${hash}`;
    await page.goto(url, { waitUntil: "networkidle", timeout: 60_000 });
    await page.waitForSelector("#viewRoot .page-title, #viewRoot .error", { timeout: 60_000 });
    await page.waitForFunction(
      () => !document.querySelector("#viewRoot .loading"),
      { timeout: 60_000 },
    );
    const file = path.join(outDir, `${name}-${theme}.png`);
    await page.screenshot({ path: file, fullPage: false });
    console.log("wrote", file);
  }
  await page.close();
}

await browser.close();
