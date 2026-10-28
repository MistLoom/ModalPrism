import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { checkEmbeddedStack, detectLoader, readCompatManifest } from "./compat.mjs";
import { createHarnessHost } from "./harness-host.mjs";
import { loadDshExternal } from "./external.mjs";
import { loadDshThin } from "./thin.mjs";

export { checkEmbeddedStack, readCompatManifest };

/** 支持市面 npm 包目录（读 package.json main / module / dsh.bundle）。 */
function resolveImportUrl(abs) {
  if (fs.existsSync(abs) && fs.statSync(abs).isDirectory()) {
    const pkgPath = path.join(abs, "package.json");
    if (fs.existsSync(pkgPath)) {
      const pkg = JSON.parse(fs.readFileSync(pkgPath, "utf8"));
      const dot = pkg.exports?.["."];
      let rel =
        (typeof dot === "string" ? dot : dot?.default ?? dot?.import) ??
        pkg.dsh?.bundle?.main ??
        pkg.dsh?.main ??
        pkg.module ??
        pkg.main ??
        "index.js";
      if (rel && typeof rel === "object") {
        rel = rel.import ?? rel.default ?? rel.node;
      }
      const file = path.resolve(abs, String(rel));
      return pathToFileURL(file).href;
    }
  }
  return pathToFileURL(abs).href;
}

export function resolveEntryPaths(entryPath) {
  const abs = path.resolve(entryPath);
  return { abs, url: resolveImportUrl(abs) };
}

export async function loadDshEntry(entryPathFs, entryUrl, entryMeta, registerHook, registerTool) {
  const loader = await detectLoader(entryMeta ?? {}, entryPathFs);
  const stack =
    loader === "cordis" ? checkEmbeddedStack() : { ok: true, warnings: [], errors: [], packages: {} };
  if (!stack.ok) {
    throw new Error(stack.errors.join("; "));
  }
  for (const w of stack.warnings) {
    console.warn(`[leaf-dsh] ${w}`);
  }

  switch (loader) {
    case "external":
      await loadDshExternal(entryUrl, registerHook, registerTool);
      break;
    case "cordis": {
      const hostCall = (call) => globalThis.__leafHostCall?.(call) ?? Promise.reject(new Error("host_call_unavailable"));
      const host = await createHarnessHost({
        registerHook,
        registerTool,
        hostCall,
      });
      await host.loadPlugin(entryUrl, entryMeta?.config);
      break;
    }
    case "thin":
    default:
      await loadDshThin(entryUrl, registerHook, registerTool);
      break;
  }
  return { loader, stack, bridgeVersion: readCompatManifest().bridgeVersion ?? 1 };
}
