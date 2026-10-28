import { existsSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import * as esbuild from "esbuild";

const hostRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const anchor = pathToFileURL(path.join(hostRoot, "package.json")).href;

const EXTENSIONS = [".ts", ".tsx", ".mts", ".cts", ".js", ".mjs", ".cjs"];

/** Claude 官方 Mod 的 TS 源码常用无扩展相对路径；Node 不会自动补 `.ts`。 */
function resolveExtensionlessRelative(specifier, parentURL) {
  if (!parentURL || (!specifier.startsWith("./") && !specifier.startsWith("../"))) {
    return null;
  }
  if (path.extname(specifier) !== "") {
    return null;
  }

  const parentPath = fileURLToPath(parentURL);
  const base = path.resolve(path.dirname(parentPath), specifier);

  for (const ext of EXTENSIONS) {
    const file = base + ext;
    if (existsSync(file) && statSync(file).isFile()) {
      return pathToFileURL(file).href;
    }
  }

  if (existsSync(base) && statSync(base).isDirectory()) {
    for (const ext of EXTENSIONS) {
      const index = path.join(base, `index${ext}`);
      if (existsSync(index) && statSync(index).isFile()) {
        return pathToFileURL(index).href;
      }
    }
  }

  return null;
}

/** 插件目录里的 `@deepseek-ai/*` 统一解析到宿主 node_modules，避免半套依赖。 */
export async function resolve(specifier, context, nextResolve) {
  if (specifier.startsWith("@deepseek-ai/")) {
    const pkg = specifier.split("/").slice(0, 2).join("/");
    const pkgJson = path.join(hostRoot, "node_modules", pkg, "package.json");
    if (existsSync(pkgJson)) {
      return nextResolve(specifier, { ...context, parentURL: anchor });
    }
  }

  const url = resolveExtensionlessRelative(specifier, context.parentURL);
  if (url !== null) {
    return { url, shortCircuit: true };
  }

  return nextResolve(specifier, context);
}

/** Node 内置 strip-types 不处理 `.tsx`；官方 code-modernization Mod 的视图层依赖它。 */
export async function load(url, context, nextLoad) {
  if (!url.startsWith("file://") || !url.endsWith(".tsx")) {
    return nextLoad(url, context);
  }

  const filePath = fileURLToPath(url);

  const source = readFileSync(filePath, "utf8");
  const { code } = await esbuild.transform(source, {
    loader: "tsx",
    format: "esm",
    target: "esnext",
    sourcefile: filePath,
  });

  return {
    format: "module",
    source: code,
    shortCircuit: true,
  };
}
