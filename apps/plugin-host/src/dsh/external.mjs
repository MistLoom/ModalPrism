import { pathToFileURL } from "node:url";

/** 外置 DSH 运行时：LEAF_DSH_RUNTIME 指向导出 loadDshPlugin 的模块。 */
export async function loadDshExternal(entryPath, registerHook, registerTool) {
  const runtimeUrl = process.env.LEAF_DSH_RUNTIME;
  if (!runtimeUrl) {
    throw new Error("LEAF_DSH_RUNTIME 未设置");
  }
  const href = runtimeUrl.startsWith("file:") ? runtimeUrl : pathToFileURL(runtimeUrl).href;
  const mod = await import(href);
  const load = mod.loadDshPlugin ?? mod.default;
  if (typeof load !== "function") {
    throw new Error("外置运行时需 export loadDshPlugin 或 default");
  }
  await load(entryPath, { registerHook, registerTool });
}
