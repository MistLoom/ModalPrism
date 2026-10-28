import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HOST_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

export const UNSUPPORTED_HOOKS_MODULES = "claude_mod_unsupported:hooks_modules";

export function claudeCompatManifestPath() {
  return path.join(HOST_ROOT, "claude-mod.compat.json");
}

export function readClaudeCompatManifest() {
  const raw = fs.readFileSync(claudeCompatManifestPath(), "utf8");
  return JSON.parse(raw);
}

/** 读取 hooks/hooks.json 的 modules 列表（相对 hooks/）。 */
export function hooksJsonModules(pluginRoot) {
  const hooksPath = path.join(pluginRoot, "hooks", "hooks.json");
  if (!fs.existsSync(hooksPath)) {
    return [];
  }
  const doc = JSON.parse(fs.readFileSync(hooksPath, "utf8"));
  const modules = doc.modules;
  if (!Array.isArray(modules)) {
    return [];
  }
  return modules.filter((m) => typeof m === "string" && m.trim() !== "");
}

/**
 * 有界契约：非空 modules 即 hooks_module tier，默认拒绝加载。
 * @returns {string | null} 错误码，可加载则 null
 */
export function claudeModAdmissionError(pluginRoot) {
  const compat = readClaudeCompatManifest();
  if (compat.denyHooksJsonModules !== true) {
    return null;
  }
  const modules = hooksJsonModules(pluginRoot);
  const hasTs = modules.some((rel) => {
    const lower = rel.trim().toLowerCase();
    return lower.endsWith(".ts") || lower.endsWith(".tsx");
  });
  if (!hasTs) {
    return null;
  }
  return compat.unsupportedError ?? UNSUPPORTED_HOOKS_MODULES;
}
