import assert from "node:assert/strict";
import { existsSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { pathToFileURL } from "node:url";
import { createAssistantMessage, createUserMessage } from "@deepseek-ai/dsh-llm";
import { createHarnessHost } from "../src/dsh/harness-host.mjs";
import { probeEntry } from "../src/probe.mjs";
import { resolveEntryPaths } from "../src/dsh/loader.mjs";

const MODLENS = path.resolve(
  "../web-agent/vendor/dsh-npm-profile/node_modules/@liustack/modlens",
);
const hasModlens = existsSync(path.join(MODLENS, "package.json"));

async function hostWith(hostCall) {
  const tools = [];
  const host = await createHarnessHost({
    registerHook() {},
    registerTool(def) {
      tools.push(def.name);
    },
    hostCall,
  });
  return { host, tools };
}

test("inject 齐备后 Cordis 插件把工具交给宿主", async () => {
  const dir = mkdtempSync(path.join(tmpdir(), "leaf-dsh-"));
  const file = path.join(dir, "plugin.mjs");
  writeFileSync(
    file,
    `export const inject = ["tools", "llm", "sessions", "agents", "attachments"];
export function apply(ctx) {
  ctx.tools.register({
    name: "leaf_probe_tool",
    description: "probe",
    parameters: { type: "object", properties: {} },
    output: {
      schema: { type: "object", properties: { ok: { type: "boolean" } }, required: ["ok"] },
      render: () => [{ type: "text", text: "ok" }],
    },
    async execute() { return { ok: true }; },
  });
}
`,
  );
  const { host, tools } = await hostWith(async () => ({}));
  await host.loadPlugin(pathToFileURL(file).href);
  assert.deepEqual(tools, ["leaf_probe_tool"]);
  assert.equal(host.ctx.llm.listProviders()[0].id, "leaf");
  assert.equal(typeof host.ctx.sessions.create, "function");
});

test("Leaf LlmAdapter 把 completion 编成 stream chunk", async () => {
  let seen;
  const { host } = await hostWith(async (call) => {
    seen = call;
    return {
      choices: [
        {
          message: { role: "assistant", content: "pong" },
          finish_reason: "stop",
        },
      ],
      usage: { prompt_tokens: 2, completion_tokens: 1 },
    };
  });
  const chunks = [];
  for await (const chunk of host.ctx.llm.stream({
    provider: "leaf",
    model: "m",
    messages: [
      {
        id: "m1",
        role: "user",
        content: [{ type: "text", text: "ping" }],
        source: { kind: "user" },
      },
    ],
  })) {
    chunks.push(chunk);
  }
  assert.equal(seen.ns, "llm");
  assert.equal(seen.method, "complete");
  assert.equal(seen.args[0][0].content, "ping");
  assert.equal(chunks.find((c) => c.type === "text-delta").text, "pong");
  assert.equal(chunks.at(-1).reason.kind, "stop");
});

test("官方 Session.append 把 Leaf 会话消息投影到出站", async () => {
  const calls = [];
  const { host } = await hostWith(async (call) => {
    calls.push(call);
    return {};
  });
  const leafId = "11111111-1111-4111-8111-111111111111";
  const leaf = host.ctx.sessions.create(leafId, { meta: { cwd: process.cwd() } });
  leaf.append("user/message", createUserMessage({
    content: [{ type: "text", text: "hi" }],
    source: { kind: "user" },
  }), { surfaceOp: "append" });
  leaf.append("assistant/message", createAssistantMessage({
    content: [{ type: "text", text: "pong" }],
    source: { provider: "leaf", model: "m" },
  }), { surfaceOp: "append" });
  const local = host.ctx.sessions.create(undefined, { meta: { cwd: process.cwd() } });
  local.append("user/message", createUserMessage({
    content: [{ type: "text", text: "skip" }],
    source: { kind: "user" },
  }), { surfaceOp: "append" });
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(calls.map((call) => call.method), ["append_user", "append_assistant"]);
  assert.equal(calls[0].args[0].text, "hi");
  assert.equal(calls[1].args[0].text, "pong");
});

test("probe 对 modlens 返回工具名", { skip: !hasModlens }, async () => {
  const row = await probeEntry({
    path: MODLENS,
    kind: "dsh",
    loader: "cordis",
  }, async () => ({}));
  assert.equal(row.ok, true, row.errors.join(";"));
  assert.ok(row.tools.includes("modlens_read_image"), row.tools.join(","));
});

test("modlens 注册 modlens_read_image", { skip: !hasModlens }, async () => {
  const { url } = resolveEntryPaths(MODLENS);
  const { host, tools } = await hostWith(async () => ({}));
  await host.loadPlugin(url, {});
  assert.ok(tools.includes("modlens_read_image"), tools.join(","));
  assert.equal(typeof host.ctx.attachments, "object");
  assert.equal(typeof host.ctx.agents.currentInitiator, "function");
});
