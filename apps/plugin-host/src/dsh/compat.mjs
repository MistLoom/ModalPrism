import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HOST_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");

export function compatManifestPath() {
  return path.join(HOST_ROOT, "dsh-runtime.compat.json");
}

export function readCompatManifest() {
  const raw = fs.readFileSync(compatManifestPath(), "utf8");
  return JSON.parse(raw);
}

function readInstalledVersion(pkgName) {
  try {
    const p = path.join(HOST_ROOT, "node_modules", pkgName, "package.json");
    const j = JSON.parse(fs.readFileSync(p, "utf8"));
    return j.version ?? null;
  } catch {
    return null;
  }
}

function majorOf(version) {
  const m = /^(\d+)/.exec(version ?? "");
  return m ? Number(m[1]) : null;
}

function versionOk(installed, spec) {
  if (!installed) return spec.optional === true;
  if (installed === spec.pinned) return true;
  if (spec.supportedMajor?.includes(majorOf(installed))) return true;
  if (spec.supportedPrefixes?.some((pfx) => installed.startsWith(pfx))) return true;
  return false;
}

/** 校验内嵌 Cordis 栈；严格模式由 LEAF_DSH_STRICT=1 控制。 */
export function checkEmbeddedStack() {
  const manifest = readCompatManifest();
  const strict = process.env.LEAF_DSH_STRICT === "1";
  const packages = manifest.packages ?? {};
  const report = {
    bridgeVersion: manifest.bridgeVersion ?? 1,
    strict,
    packages: {},
    ok: true,
    warnings: [],
    errors: [],
  };
  for (const [name, spec] of Object.entries(packages)) {
    const installed = readInstalledVersion(name);
    const ok = versionOk(installed, spec);
    report.packages[name] = { installed, pinned: spec.pinned, ok };
    if (!ok) {
      const msg = `${name}: 已安装 ${installed ?? "(缺失)"}，兼容清单要求 ${spec.pinned} 或受支持范围`;
      if (strict && spec.optional !== true) {
        report.errors.push(msg);
        report.ok = false;
      } else {
        report.warnings.push(msg);
      }
    }
  }
  return report;
}

const LOADERS = new Set(["thin", "cordis", "external", "auto"]);

export function normalizeLoader(entry) {
  const fromEntry = entry?.loader?.trim?.();
  if (fromEntry && LOADERS.has(fromEntry)) return fromEntry;
  const env = process.env.LEAF_DSH_LOADER?.trim();
  if (env && LOADERS.has(env)) return env;
  return readCompatManifest().defaultLoader ?? "auto";
}

export async function detectLoader(entry, entryPathFs) {
  const norm = normalizeLoader(entry);
  if (norm !== "auto") return norm;
  if (process.env.LEAF_DSH_RUNTIME) return "external";
  let text = "";
  try {
    text = fs.readFileSync(entryPathFs, "utf8").slice(0, 8192);
  } catch {
    return "thin";
  }
  if (
    /@deepseek-ai\/(dsh-tools|cordis)/.test(text) ||
    /\bdefineTool\s*\(/.test(text) ||
    /\binject\s*:\s*\[/.test(text)
  ) {
    return "cordis";
  }
  return "thin";
}
