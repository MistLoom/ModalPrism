import { test, expect } from "@playwright/test";
import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const repo = path.resolve(import.meta.dirname, "../../..");
const dataDir = fs.mkdtempSync(path.join("/tmp", "mp-e2e-"));
const httpPort = 18901;
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
      "127.0.0.1:18902",
      "--push",
      "127.0.0.1:18903",
    ],
    { cwd: repo, stdio: "ignore" },
  );
  const base = `http://127.0.0.1:${httpPort}`;
  for (let i = 0; i < 60; i++) {
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

test("console loads and config roundtrip", async ({ page }) => {
  const base = `http://127.0.0.1:${httpPort}`;
  await page.goto(`${base}/console/#config`);
  await page.waitForSelector(".page-title");
  await expect(page.locator("h1")).toContainText("配置");
});
