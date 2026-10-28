import { loadClaudeMod } from "./claude-mod.mjs";
import { loadDshEntry, resolveEntryPaths } from "./dsh.mjs";
import { createHostApi } from "./host-api.mjs";
import { readCompatManifest } from "./dsh/compat.mjs";

const KNOWN_OUTBOUND = new Set([
  "fs.read",
  "fs.exists",
  "tools.execute",
  "tools.register",
  "process.run",
  "session.usage",
  "session.cwd",
  "ui.invalidate",
  "ui.resolve",
  "ui.render",
  "ui.show",
  "ui.hide",
  "agents.get",
  "clock.now",
  "clock.sleep",
  "command.register",
  "llm.complete",
  "session.get",
  "session.append_user",
  "session.append_assistant",
]);

export async function probeEntry(entry, hostCall) {
  const row = {
    path: entry.path,
    kind: entry.kind,
    loader: entry.loader ?? null,
    ok: true,
    hooks: [],
    tools: [],
    warnings: [],
    errors: [],
    outbound_gaps: [],
  };
  globalThis.__leafHostCall = hostCall;
  const $ = createHostApi(hostCall);
  const hooks = [];
  const tools = [];
  const registerHook = (name, _fn) => hooks.push(name);
  const registerTool = (def) => tools.push(def.name);
  try {
    if (entry.kind === "claude_mod") {
      await loadClaudeMod(entry.path, $, registerHook, registerTool, entry);
    } else if (entry.kind === "dsh") {
      const { abs, url } = resolveEntryPaths(entry.path);
      await loadDshEntry(abs, url, entry, registerHook, registerTool);
      const compat = readCompatManifest();
      if (entry.loader === "cordis" || entry.loader === "auto" || !entry.loader) {
        row.cordis_stack = compat.bridgeVersion;
      }
    } else {
      row.warnings.push(`unknown_kind:${entry.kind}`);
    }
  } catch (e) {
    row.ok = false;
    row.errors.push(String(e?.message ?? e));
  }
  row.hooks = [...new Set(hooks)];
  row.tools = [...new Set(tools)];
  return row;
}

export function diffOutbound(calls) {
  return calls.filter((c) => !KNOWN_OUTBOUND.has(`${c.ns}.${c.method}`));
}
