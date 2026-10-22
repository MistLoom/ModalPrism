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
    spec.on(m.id, m.description || m.id, m.input, fn);
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
