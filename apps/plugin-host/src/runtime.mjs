import { loadClaudeMod } from "./claude-mod.mjs";
import { loadDshEntry, resolveEntryPaths } from "./dsh.mjs";
import { createHostApi } from "./host-api.mjs";

export class HostRuntime {
  constructor(send) {
    this.send = send;
    this.hooks = new Map();
    this.pending = new Map();
    this.tools = new Map();
    this.$ = null;
    this.registeredHooks = [];
    this.dshRuntime = null;
    this.loadErrors = [];
  }

  async handle(msg) {
    switch (msg.type) {
      case "init":
        await this.init(msg);
        break;
      case "hook":
        void this.runHook(msg);
        break;
      case "host_result":
        this.finishHost(msg);
        break;
      case "tool_execute":
        void this.runToolExecute(msg);
        break;
      case "probe":
        void this.runProbe(msg);
        break;
      default:
        this.send({ type: "error", message: `unknown_type:${msg.type}` });
    }
  }

  async init(msg) {
    this.$ = createHostApi((call) => this.hostCall(call));
    globalThis.__leafHostCall = (call) => this.hostCall(call);
    for (const entry of msg.plugins ?? []) {
      const origin = entry.origin ?? entry.kind;
      const instanceId = entry.instanceId ?? "default";
      const registerTool = (def) => {
        const hostKey = `${instanceId}::${def.name}`;
        this.tools.set(hostKey, def);
        this.send({
          type: "tool_register",
          name: def.name,
          origin,
          instanceId,
          description: def.description ?? "",
          parameters: def.parameters ?? {},
        });
      };
      try {
        if (entry.kind === "claude_mod") {
          await loadClaudeMod(
            entry.path,
            this.$,
            (name, fn) => this.registerHook(name, fn),
            registerTool,
            entry,
          );
        } else if (entry.kind === "dsh") {
          const { abs, url } = resolveEntryPaths(entry.path);
          const meta = await loadDshEntry(
            abs,
            url,
            entry,
            (name, fn) => this.registerHook(name, fn),
            registerTool,
          );
          this.dshRuntime = {
            loader: meta.loader,
            bridgeVersion: meta.bridgeVersion,
            stack: {
              ok: meta.stack.ok,
              warnings: meta.stack.warnings,
              errors: meta.stack.errors,
              packages: meta.stack.packages,
            },
          };
        }
      } catch (e) {
        const error = String(e?.message ?? e);
        this.loadErrors.push({ path: entry.path, kind: entry.kind, error });
        console.error(`[leaf-host] 插件加载失败 ${entry.path}: ${error}`);
      }
    }
    this.send({
      type: "ready",
      hooks: [...new Set(this.registeredHooks)],
      tools: [...this.tools.keys()],
      load_errors: this.loadErrors,
      ...(this.dshRuntime ? { dsh_runtime: this.dshRuntime } : {}),
    });
  }

  registerHook(name, fn) {
    const prev = this.hooks.get(name);
    const chained = prev
      ? async ($, event, next) => prev($, event, (ev) => fn($, ev, next))
      : fn;
    this.hooks.set(name, chained);
    this.registeredHooks.push(name);
  }

  async runToolExecute(msg) {
    const tool = this.tools.get(msg.name);
    if (!tool?.execute) {
      this.send({ type: "tool_result", id: msg.id, ok: false, error: "unknown_tool" });
      return;
    }
    try {
      const value = await tool.execute(msg.args ?? {});
      this.send({ type: "tool_result", id: msg.id, ok: true, value });
    } catch (e) {
      this.send({
        type: "tool_result",
        id: msg.id,
        ok: false,
        error: String(e?.message ?? e),
      });
    }
  }

  async runHook(msg) {
    const fn = this.hooks.get(msg.name);
    if (!fn) {
      this.send({ type: "hook_result", id: msg.id, ok: false, error: "unknown_hook" });
      return;
    }
    const next = async (event) => event;
    try {
      const value = await fn(this.$, msg.payload ?? {}, next);
      this.send({ type: "hook_result", id: msg.id, ok: true, value });
      if (String(msg.name).startsWith("ui.")) {
        this.send({
          type: "ui_emit",
          payload: {
            hook: msg.name,
            input: msg.payload ?? {},
            result: value ?? null,
          },
        });
      }
    } catch (e) {
      this.send({ type: "hook_result", id: msg.id, ok: false, error: String(e?.message ?? e) });
    }
  }

  async runProbe(msg) {
    const { probeEntry } = await import("./probe.mjs");
    const entries = [];
    for (const entry of msg.plugins ?? []) {
      entries.push(await probeEntry(entry, (call) => this.hostCall(call)));
    }
    this.send({ type: "probe_result", id: msg.id, entries });
  }

  hostCall(call) {
    const id = `hc_${Date.now()}_${Math.random().toString(36).slice(2)}`;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.send({ type: "host_call", id, ...call });
    });
  }

  finishHost(msg) {
    const p = this.pending.get(msg.id);
    if (!p) return;
    this.pending.delete(msg.id);
    if (msg.ok) p.resolve(msg.value);
    else p.reject(new Error(msg.error ?? "host_call_failed"));
  }
}
