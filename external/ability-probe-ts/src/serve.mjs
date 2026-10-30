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

const { serveAbility, pending } = await import(
  sdkAbilityJs(path.dirname(fileURLToPath(import.meta.url)))
);

await serveAbility({
  ping: async (ctx) => {
    await ctx.emit("pong", { from: "probe" });
    return { ok: true, from: "external-ts" };
  },
  slow: async (_ctx, args) => {
    const ms = Number(args?.ms ?? 150);
    return pending(
      new Promise((resolve, reject) =>
        setTimeout(() => (args?.fail ? reject(new Error("boom")) : resolve({ slept: ms })), ms),
      ),
    );
  },
});
