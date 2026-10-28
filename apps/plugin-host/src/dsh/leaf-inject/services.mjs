import AgentRegistry from "@deepseek-ai/dsh-agent";
import AttachmentStore from "@deepseek-ai/dsh-attachment";
import LlmRuntime from "@deepseek-ai/dsh-llm";
import SessionStore from "@deepseek-ai/dsh-session";
import WebService from "@deepseek-ai/dsh-web";

/**
 * 市面 Cordis 插件的 inject 键。缺任一键时 Cordis 不调用 apply，工具不会注册。
 * `tools` 由 cordis-host 的 ToolRegistry 提供。
 */
const HARNESS_SERVICES = [
  ["llm", LlmRuntime],
  ["sessions", SessionStore],
  ["agents", AgentRegistry],
  ["attachments", AttachmentStore],
  ["web", WebService],
];

export async function mountHarnessServices(ctx) {
  for (const [name, plugin] of HARNESS_SERVICES) {
    if (ctx.get(name)) continue;
    await ctx.plugin(plugin);
  }
}
