import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { claudeModAdmissionError, UNSUPPORTED_HOOKS_MODULES } from "../src/claude-mod/compat.mjs";
import { loadClaudeMod } from "../src/claude-mod.mjs";

const hostRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

test("hooks.json 含 modules 时拒绝加载", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "leaf-mod-"));
  fs.mkdirSync(path.join(dir, "hooks"), { recursive: true });
  fs.writeFileSync(
    path.join(dir, "hooks", "hooks.json"),
    JSON.stringify({ modules: ["./register.ts"] }),
  );
  assert.equal(claudeModAdmissionError(dir), UNSUPPORTED_HOOKS_MODULES);
  await assert.rejects(
    () => loadClaudeMod(dir, {}, () => {}, () => {}),
    (err) => err.message.includes(UNSUPPORTED_HOOKS_MODULES),
  );
  fs.rmSync(dir, { recursive: true, force: true });
});

test("hooks_command fixture 可通过门禁", () => {
  const fixture = path.resolve(
    hostRoot,
    "../web-agent/tests/fixtures/token-weather",
  );
  assert.equal(claudeModAdmissionError(fixture), null);
});
