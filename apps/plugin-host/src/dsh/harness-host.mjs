import { createCordisHost } from "./cordis-host.mjs";
import { registerLeafInjectPlugins } from "./leaf-inject/index.mjs";

/**
 * Harness 级 Cordis 宿主：官方 ToolRegistry 栈 + Leaf inject 适配。
 */
export async function createHarnessHost({ registerHook, registerTool, hostCall }) {
  const host = await createCordisHost({ registerHook, registerTool });
  await registerLeafInjectPlugins(host.ctx, hostCall);
  return host;
}
