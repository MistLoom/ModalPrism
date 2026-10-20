import { test, expect } from "@playwright/test";
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const repo = path.resolve(import.meta.dirname, "../../..");
const dataDir = fs.mkdtempSync(path.join("/tmp", "mp-e2e-"));
const httpPort = 18911;
let proc: ReturnType<typeof spawn>;

test.beforeAll(async () => {
  const bin = path.join(repo, "target/debug/modalprism-collector");
  proc = spawn(
    bin,
    [
      "--data",
      dataDir,
      "--config",
      path.join(repo, "config"),
      "--http",
      `127.0.0.1:${httpPort}`,
      "--proxy",
      "127.0.0.1:18912",
      "--push",
      "127.0.0.1:18913",
    ],
    { cwd: repo, stdio: "ignore" },
  );
  const base = `http://127.0.0.1:${httpPort}`;
  for (let i = 0; i < 90; i++) {
    try {
      const r = await fetch(`${base}/health`);
      if (r.ok) return;
    } catch {
      /* wait */
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error("collector failed to start");
});

test.afterAll(() => {
  proc?.kill("SIGTERM");
});

test("console routes and trailing slash", async ({ page }) => {
  const base = `http://127.0.0.1:${httpPort}`;
  const r1 = await page.goto(`${base}/console/`);
  expect(r1?.status()).toBeLessThan(400);
  await page.goto(`${base}/console/#overview`);
  await expect(page.locator(".page-title")).toContainText("系统正在沉积");
});

test("source toggle, config edit rollback, trigger collection", async ({ page }) => {
  const base = `http://127.0.0.1:${httpPort}`;
  await page.goto(`${base}/console/#sources`);
  await page.waitForSelector(".page-title");
  const toggle = page.locator("[data-act='toggle']").first();
  if (await toggle.count()) {
    await toggle.click();
    await page.locator("#confirmOk").click();
    await expect(page.locator(".toast").last()).toBeVisible({ timeout: 15000 });
  }

  await page.goto(`${base}/console/#config`);
  await page.waitForSelector(".cfg-textarea");
  const editor = page.locator(".cfg-textarea");
  const before = await editor.inputValue();
  await editor.fill(`${before}\n# e2e-mark\n`);
  await page.locator("#btnSaveCfg").click();
  await page.locator("#confirmOk").click();
  await expect(page.locator(".toast").last()).toContainText("已保存", { timeout: 20000 });
  await page.locator("#btnRollback").click();
  await page.locator("#confirmOk").click();
  await expect(editor).toHaveValue(before, { timeout: 20000 });

  await page.goto(`${base}/console/#sources`);
  const trigger = page.locator("[data-act='trigger']").first();
  if (await trigger.count()) {
    await trigger.click();
    await expect(page.locator(".toast").last()).toBeVisible({ timeout: 10000 });
  }

  await page.goto(`${base}/console/#records`);
  await page.waitForSelector(".page-title");
  const row = page.locator(".rec-row").first();
  if (await row.count()) {
    await row.click();
    await page.waitForSelector("#drawer .drawer-head .drawer-title", { timeout: 15000 });
    await expect(page.locator("#drawer .drawer-body")).not.toContainText("加载中");
    const title = page.locator("#drawer .drawer-title");
    await expect(title).not.toHaveText("加载中…");
    const link = page.locator("#drawer .drawer-body a[href]");
    if (await link.count()) {
      await expect(link.first()).toHaveAttribute("href", /.+/);
    }
  }
});
