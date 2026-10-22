import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

function sdkAbilityJs(fromDir) {
  let d = fromDir;
  for (;;) {
    const f = path.join(d, "SDK", "ability-sdk-ts", "lib", "ability.js");
    if (existsSync(f)) return pathToFileURL(f).href;
    const up = path.dirname(d);
    if (up === d) throw new Error("找不到 SDK/ability-sdk-ts");
    d = up;
  }
}

const { serveAbility } = await import(
  sdkAbilityJs(path.dirname(fileURLToPath(import.meta.url)))
);

await serveAbility({
  lanes: async (ctx) => {
    const flags = { middleware: false, pipeline: false, parallel: false, message: false };
    const filter = { class: "ability", name: "lane" };
    await ctx.handle(filter, {
      mode: "middleware",
      async around(ev, next) {
        const out = await next(ev);
        flags.middleware = true;
        return out;
      },
    });
    await ctx.handle(filter, {
      mode: "pipeline",
      async process(ev) {
        flags.pipeline = true;
        return ev;
      },
    });
    await ctx.handle(filter, {
      mode: "parallel",
      async onEvent() {
        flags.parallel = true;
      },
    });
    await ctx.handle(filter, {
      mode: "message",
      async onEvent() {
        flags.message = true;
      },
    });
    await ctx.emit("lane", {});
    for (let i = 0; i < 40 && !flags.message; i++) {
      await ctx.localGet("_drain");
    }
    return { ...flags };
  },
  filter: async (ctx) => {
    const filtered = ctx.withProtocolFilter("fun");
    await filtered.localSet("pf", 1);
    const v = await ctx.localGet("pf");
    return {
      shared_local: v === 1,
      same_instance: filtered.instance === ctx.instance,
    };
  },
  aspect: async (ctx) => {
    ctx.registerAspect({});
    return {};
  },
});
