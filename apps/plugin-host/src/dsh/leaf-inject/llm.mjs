import { LlmAdapter } from "@deepseek-ai/dsh-llm";

/** Leaf chat 在 Cordis 上的 provider 路由。 */
export const LEAF_PROVIDER = "leaf";

/**
 * 官方 LlmRuntime 的 Leaf 适配：`stream` 走宿主 `llm.complete` → `chat.completion`。
 * 不在进程内直连厂商 HTTP。
 */
class LeafChatAdapter extends LlmAdapter {
  constructor(hostCall) {
    super();
    this.hostCall = hostCall;
  }

  providerInfo(provider) {
    return { id: provider, name: "Leaf" };
  }

  async listModels() {
    try {
      const info = await this.hostCall({ ns: "agents", method: "get", args: [] });
      const model = info?.main?.model;
      if (typeof model === "string" && model.length > 0) {
        return [{ id: model, name: model }];
      }
    } catch {
      /* 目录为空时路由仍接受调用方给出的 model */
    }
    return [];
  }

  async *stream(options) {
    if (options.signal?.aborted) {
      throw Object.assign(new Error("aborted"), { code: "ABORTED" });
    }
    const messages = toLeafMessages(options);
    const reqOptions = {
      model: options.model,
      temperature: options.temperature,
      max_tokens: options.maxTokens,
      tools: toLeafTools(options.tools),
    };
    const resp = await this.hostCall({
      ns: "llm",
      method: "complete",
      args: [messages, reqOptions],
    });
    if (options.signal?.aborted) {
      throw Object.assign(new Error("aborted"), { code: "ABORTED" });
    }
    if (resp?.code === "host_unsupported") {
      throw new Error("llm.complete 未接通 chat.completion");
    }
    yield* chunksFromCompletion(resp);
  }
}

export async function registerLlmInject(ctx, hostCall) {
  const call = hostCall ?? unavailable;
  ctx.llm.registerAdapter([LEAF_PROVIDER], new LeafChatAdapter(call));
}

function unavailable() {
  return Promise.reject(new Error("host_call_unavailable"));
}

function toLeafTools(tools) {
  if (!Array.isArray(tools) || tools.length === 0) return undefined;
  return tools.map((tool) => ({
    type: "function",
    function: {
      name: tool.name,
      description: tool.description ?? "",
      parameters: tool.parameters ?? { type: "object", properties: {} },
    },
  }));
}

function blockText(blocks) {
  if (!Array.isArray(blocks)) return "";
  const parts = [];
  for (const block of blocks) {
    if (!block || typeof block !== "object") continue;
    if (block.type === "text" || block.type === "reasoning") {
      if (typeof block.text === "string" && block.text) parts.push(block.text);
    } else if (block.type === "tool-result") {
      const nested = blockText(block.content);
      if (nested) parts.push(nested);
    }
  }
  return parts.join("\n");
}

function toLeafMessages(options) {
  const out = [];
  if (typeof options.system === "string" && options.system) {
    out.push({ role: "system", content: options.system });
  }
  for (const msg of options.messages ?? []) {
    const toolBlock = (msg.content ?? []).find((b) => b?.type === "tool-result");
    if (msg.source?.kind === "tool" || toolBlock) {
      out.push({
        role: "tool",
        tool_call_id: toolBlock?.toolCallId ?? msg.source?.callId ?? "",
        content: blockText(toolBlock?.content ?? msg.content),
      });
      continue;
    }
    if (msg.role === "assistant") {
      const calls = (msg.content ?? []).filter((b) => b?.type === "tool-call");
      const leaf = { role: "assistant", content: blockText(msg.content) };
      if (calls.length > 0) {
        leaf.tool_calls = calls.map((call) => ({
          id: String(call.id ?? ""),
          type: "function",
          function: {
            name: call.name ?? "",
            arguments: typeof call.arguments === "string" ? call.arguments : "{}",
          },
        }));
      }
      out.push(leaf);
      continue;
    }
    const role = msg.role === "system" ? "system" : "user";
    out.push({ role, content: blockText(msg.content) });
  }
  return out;
}

function messageText(message) {
  const content = message?.content;
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {
    return content
      .filter((part) => part?.type === "text" && typeof part.text === "string")
      .map((part) => part.text)
      .join("");
  }
  return "";
}

function* chunksFromCompletion(resp) {
  const message = resp?.choices?.[0]?.message ?? {};
  const text = messageText(message);
  const calls = Array.isArray(message.tool_calls) ? message.tool_calls : [];
  let index = 0;
  if (text) {
    yield { type: "block-start", index, blockType: "text" };
    yield { type: "text-delta", index, text };
    yield { type: "block-end", index, block: { type: "text", text } };
    index += 1;
  }
  for (const call of calls) {
    const id = String(call.id ?? "");
    const name = call.function?.name ?? "";
    const args = typeof call.function?.arguments === "string" ? call.function.arguments : "{}";
    yield { type: "block-start", index, blockType: "tool-call" };
    yield { type: "tool-call-delta", index, id, name, argumentsDelta: args };
    yield {
      type: "block-end",
      index,
      block: { type: "tool-call", id, name, arguments: args },
    };
    index += 1;
  }
  const usage = resp?.usage;
  if (usage && typeof usage === "object") {
    yield {
      type: "usage",
      usage: {
        inputTokens: Number(usage.prompt_tokens ?? 0),
        outputTokens: Number(usage.completion_tokens ?? 0),
      },
    };
  }
  const finish = resp?.choices?.[0]?.finish_reason;
  const kind = calls.length > 0 || finish === "tool_calls" ? "tool-calls" : finish === "length" ? "max-tokens" : "stop";
  yield { type: "finish", reason: { kind } };
}
