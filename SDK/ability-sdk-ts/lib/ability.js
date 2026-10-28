import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { FRAMEWORK_VERSION, AbilitySpec } from "./spec.js";
import { serveStdio } from "./guest.js";
import { loadAbilityYaml, schemaVersion } from "./yaml-lite.js";

/**
 * 从 `ability.yml` + 方法表启动 guest。业务只需导出 handlers。
 *
 * @example
 * serveAbility({
 *   ping: async (ctx) => {
 *     await ctx.emit("pong", {});
 *     return { ok: true };
 *   },
 * });
 */
export async function serveAbility(handlers, options = {}) {
  const root = resolveAbilityRoot(options.root);
  const decl = loadAbilityYaml(root);
  if (schemaVersion(decl.framework_version) !== FRAMEWORK_VERSION) {
    throw new Error("ability.yml framework_version 与 SDK 不匹配");
  }
  const provider = buildProvider(decl, handlers, options.lifecycle, options);
  await serveStdio(provider);
}

export function buildProvider(decl, handlers, lifecycle = {}, hooks = {}) {
  let spec = AbilitySpec.must(decl.id);
  if (decl.stateful) {
    spec.stateful = true;
  }
  for (const dep of decl.depends) {
    if (dep.alias) {
      spec.dependsAlias(dep.id, dep.alias);
    }
  }
  for (const ev of decl.events) {
    if (ev.dir === "emit") {
      spec.eventEmit(ev.name, ev.description, ev.payload_schema);
    }
  }
  for (const m of decl.methods) {
    const fn = handlers[m.id];
    if (!fn) {
      throw new Error(`缺少方法处理器: ${m.id}`);
    }
    spec.on(m.id, m.description || m.id, m.input, withLongTask(fn));
  }
  const base = spec.intoProvider();
  return {
    manifest() {
      return base.manifest();
    },
    async create(config) {
      if (hooks.onCreate) {
        await hooks.onCreate(config);
      }
      const ability = await base.create(config);
      return {
        manifest() {
          return ability.manifest();
        },
        async onLifecycle(phase, ctx) {
          const hook = lifecycle[phase];
          if (hook) {
            await hook(ctx);
          }
          await ability.onLifecycle(phase, ctx);
        },
        invoke(method, ctx, args) {
          return ability.invoke(method, ctx, args);
        },
      };
    },
  };
}

/** 标记异步 handler，便于 IDE 与文档。 */
export function handler(fn) {
  return fn;
}

/**
 * 把一个 Promise 标记为长任务：方法立即返回 `{ task_handle }`，
 * 调用方以同一方法 + `{ handle }` 参数轮询，直到 `done` / `failed`。
 */
export function pending(promise) {
  return { $pending: Promise.resolve(promise) };
}

/** 长任务注册表与轮询逻辑（协议级，不新增帧类型）。 */
export function withLongTask(fn) {
  const tasks = new Map();
  let seq = 0;
  return async (ctx, args) => {
    // 轮询：同一方法携带 { handle } 即查询任务状态，不进入业务 handler。
    if (args && typeof args.handle === "string") {
      const handle = args.handle;
      const task = tasks.get(handle);
      if (!task) {
        return { state: "not_found", handle };
      }
      const settled = await Promise.race([
        task,
        new Promise((resolve) => setTimeout(() => resolve({ running: true }), 0)),
      ]);
      if (settled.running) {
        return { state: "running", handle };
      }
      tasks.delete(handle);
      return settled.done
        ? { state: "done", data: settled.value ?? null }
        : { state: "failed", error: settled.error };
    }
    const result = await fn(ctx, args);
    if (result && typeof result === "object" && result.$pending instanceof Promise) {
      const handle = `task-${Date.now().toString(36)}-${++seq}`;
      // 入表即挂接终态处理：拒绝不会被当作 unhandledRejection 终止 Guest 进程。
      tasks.set(
        handle,
        result.$pending.then(
          (value) => ({ done: true, value }),
          (error) => ({ done: false, error: error?.message ?? String(error) }),
        ),
      );
      return { task_handle: handle };
    }
    return result;
  };
}

function resolveAbilityRoot(explicit) {
  if (explicit) {
    return explicit;
  }
  if (process.env.LEAF_ABILITY_ROOT) {
    return process.env.LEAF_ABILITY_ROOT;
  }
  let dir = process.argv[1] ? dirname(process.argv[1]) : process.cwd();
  for (;;) {
    if (existsSync(join(dir, "ability.yml"))) {
      return dir;
    }
    const parent = dirname(dir);
    if (parent === dir) {
      break;
    }
    dir = parent;
  }
  return process.cwd();
}
