import { randomUUID } from "node:crypto";
import { Context } from "@deepseek-ai/cordis";
import SystemPrompt from "@deepseek-ai/dsh-system-prompt";
import ToolRegistry, { defineTool } from "@deepseek-ai/dsh-tools";

function wrapPluginExport(mod) {
  const p = mod.default ?? mod;
  if (typeof p === "function") return p;
  if (p && typeof p.apply === "function") {
    const fn = (ctx, config) => p.apply(ctx, config ?? p.config);
    if (Array.isArray(p.inject)) fn.inject = p.inject;
    return fn;
  }
  throw new Error("dsh_cordis: 插件需 export default function 或 { apply, inject }");
}

function toolResultToLeafValue(result) {
  if (result?.isError) {
    const text =
      result.content
        ?.filter((b) => b.type === "text")
        .map((b) => b.text)
        .join("\n") ||
      result.error?.message ||
      "tool_error";
    throw new Error(text);
  }
  const blocks = result?.content ?? [];
  if (blocks.length === 1 && blocks[0].type === "text") {
    const t = blocks[0].text;
    try {
      return JSON.parse(t);
    } catch {
      return t;
    }
  }
  return { content: blocks };
}

/**
 * 最小 Cordis 栈：SystemPrompt + ToolRegistry(native)，工具经 NDJSON 桥接到 Rust。
 */
export async function createCordisHost({ registerHook, registerTool }) {
  const root = new Context();
  await root.plugin(SystemPrompt);
  await root.plugin(ToolRegistry, { mode: "native" });

  const synced = new Set();

  const syncTools = () => {
    const view = root.tools.view(undefined);
    for (const [name, definition] of view.visible) {
      if (synced.has(name)) continue;
      synced.add(name);
      registerTool({
        name,
        description: definition.description ?? "",
        parameters: definition.parameters ?? { type: "object", properties: {} },
        execute: async (args) => {
          // dsh-tools 要求 caller signal；缺省会读 undefined.aborted
          const signal = new AbortController().signal;
          const result = await root.tools.execute({
            name,
            arguments: args,
            signal,
            callId: randomUUID(),
          });
          return toolResultToLeafValue(result);
        },
      });
    }
  };

  root.on("tools/change", () => syncTools());

  return {
    ctx: root,
    defineTool,
    async loadPlugin(entryUrl, pluginConfig) {
      const mod = await import(entryUrl);
      const plugin = wrapPluginExport(mod);
      if (pluginConfig !== undefined && pluginConfig !== null) {
        await root.plugin(plugin, pluginConfig);
      } else {
        await root.plugin(plugin);
      }
      syncTools();
    },
    registerHook,
  };
}
