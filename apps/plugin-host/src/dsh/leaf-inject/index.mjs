import { registerLlmInject } from "./llm.mjs";
import { registerSessionInject } from "./session.mjs";
import { mountHarnessServices } from "./services.mjs";

/** 挂官方 Harness 服务，再把 llm / session 接到 Leaf 出站。 */
export async function registerLeafInjectPlugins(ctx, hostCall) {
  await mountHarnessServices(ctx);
  await registerLlmInject(ctx, hostCall);
  registerSessionInject(ctx, hostCall);
}
