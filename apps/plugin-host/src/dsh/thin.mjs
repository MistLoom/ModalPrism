/** Cookbook 同形单文件：ctx.on + ctx.tools.register(parameters/execute)。 */
export async function loadDshThin(entryPath, registerHook, registerTool) {
  const mod = await import(entryPath);
  const ctx = {
    on: (name, handler) => {
      registerHook(name, async (_$, event, next) => {
        const out = await handler(event, async (v) => v);
        if (out !== undefined) return out;
        return next ? next(event) : undefined;
      });
    },
    tools: {
      register: (tool) => {
        const def = tool?.tool ? tool.tool : tool;
        if (def?.name) registerTool(def);
      },
    },
  };
  if (typeof mod.default === "function") {
    await mod.default(ctx);
  } else if (typeof mod.register === "function") {
    await mod.register(ctx);
  }
}
