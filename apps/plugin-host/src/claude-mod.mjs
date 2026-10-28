import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { execFileSync, spawn } from "node:child_process";
import { claudeModAdmissionError } from "./claude-mod/compat.mjs";

/** Claude Code 事件名 → Leaf Mod 钩子名（与 wire 转发一致） */
const OFFICIAL_EVENT_TO_LEAF = {
  SessionStart: "session.start",
  SessionEnd: "session.end",
  Stop: "turn.complete",
  PreToolUse: "tool.check",
  PostToolUse: "tool.result",
  UserPromptSubmit: "prompt.submit",
};

function expandPluginRoot(command, pluginRoot) {
  return command.replaceAll("${CLAUDE_PLUGIN_ROOT}", pluginRoot);
}

/** Windows `canonicalize` 会带 `\\?\`，cmd 不能把它当工作目录。 */
function hostPath(p) {
  if (typeof p !== "string") return p;
  if (p.startsWith("\\\\?\\UNC\\")) return `\\\\${p.slice(8)}`;
  if (p.startsWith("\\\\?\\")) return p.slice(4);
  return p;
}

let cachedGitBash;
function gitBashFromPath() {
  if (cachedGitBash !== undefined) return cachedGitBash;
  cachedGitBash = null;
  if (process.platform !== "win32") return null;
  try {
    const out = execFileSync("where.exe", ["git"], { encoding: "utf8", windowsHide: true });
    const git = out.split(/\r?\n/).map((s) => s.trim()).find(Boolean);
    if (!git) return null;
    const bash = path.resolve(path.dirname(git), "..", "bin", "bash.exe");
    if (existsSync(bash)) cachedGitBash = bash;
  } catch {
    cachedGitBash = null;
  }
  return cachedGitBash;
}

/** 避免误调仓库内 `bash` ability 可执行文件 */
function resolveShellCommand(command) {
  const trimmed = command.trimStart();
  if (!/^bash\s/i.test(trimmed)) {
    return command;
  }
  const candidates = [
    process.env.LEAF_BASH_PATH,
    gitBashFromPath(),
    "C:\\Program Files\\Git\\bin\\bash.exe",
    "C:\\Program Files (x86)\\Git\\bin\\bash.exe",
    "/bin/bash",
    "/usr/bin/bash",
  ].filter(Boolean);
  for (const bashPath of candidates) {
    if (existsSync(bashPath)) {
      const rest = trimmed.replace(/^bash\s+/i, "");
      return `"${bashPath}" ${rest}`;
    }
  }
  return trimmed.replace(/^bash\s+/i, "sh ");
}

function collectCommandEntries(groups) {
  if (!Array.isArray(groups)) return [];
  const out = [];
  for (const group of groups) {
    const inner = group?.hooks;
    if (!Array.isArray(inner)) continue;
    for (const h of inner) {
      if (h?.type === "command" && h.command) {
        out.push({ command: h.command, timeout: h.timeout ?? 30 });
      }
    }
  }
  return out;
}

/** 仅支持「cat <<'EOF' … JSON … EOF」类无外部依赖的 .sh（便于无 bash 环境跑官方轻量 Mod） */
async function tryEmulateCatScript(scriptPath) {
  try {
    const content = await readFile(scriptPath, "utf8");
    const m = content.match(/cat\s+<<\s*['"]EOF['"]\s*\r?\n([\s\S]*?)\r?\nEOF/);
    if (!m) return null;
    return JSON.parse(m[1]);
  } catch {
    return null;
  }
}

async function runPluginCommand(command, pluginRoot, timeoutSec) {
  const emulateOnly = process.env.LEAF_PLUGIN_HOOK_EMULATE === "1";
  const shMatch = command.match(/"([^"]+\.sh)"/);
  if (shMatch) {
    let scriptPath = shMatch[1];
    if (!path.isAbsolute(scriptPath)) {
      scriptPath = path.join(pluginRoot, scriptPath);
    }
    const emulated = await tryEmulateCatScript(scriptPath);
    if (emulated) return emulated;
    if (emulateOnly) {
      throw new Error("LEAF_PLUGIN_HOOK_EMULATE=1 且脚本无法模拟");
    }
  }
  return runCommand(command, pluginRoot, timeoutSec);
}

function runCommand(command, pluginRoot, timeoutSec) {
  return new Promise((resolve, reject) => {
    const ms = Math.max(1, timeoutSec) * 1000;
    const child = spawn(command, {
      shell: true,
      env: { ...process.env, CLAUDE_PLUGIN_ROOT: pluginRoot },
      cwd: pluginRoot,
    });
    let stdout = "";
    let stderr = "";
    const timer = setTimeout(() => {
      child.kill();
      reject(new Error(`command hook 超时 (${timeoutSec}s)`));
    }, ms);
    child.stdout?.on("data", (d) => {
      stdout += d.toString();
    });
    child.stderr?.on("data", (d) => {
      stderr += d.toString();
    });
    child.on("error", (e) => {
      clearTimeout(timer);
      reject(e);
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      if (code !== 0) {
        reject(new Error(stderr || `command hook 退出码 ${code}`));
        return;
      }
      const trimmed = stdout.trim();
      if (!trimmed) {
        resolve({});
        return;
      }
      try {
        resolve(JSON.parse(trimmed));
      } catch {
        resolve({ output: trimmed });
      }
    });
  });
}

function registerCommandHooks(manifest, pluginRoot, register) {
  const table = manifest.hooks;
  if (!table || typeof table !== "object") return;
  for (const [officialEvent, groups] of Object.entries(table)) {
    const entries = collectCommandEntries(groups);
    if (entries.length === 0) continue;
    const leafName = OFFICIAL_EVENT_TO_LEAF[officialEvent] ?? officialEvent;
    register(leafName, async (_$, event, next) => {
      let acc = await next(event);
      for (const entry of entries) {
        let cmd = expandPluginRoot(entry.command, pluginRoot);
        cmd = resolveShellCommand(cmd);
        try {
          const out = await runPluginCommand(cmd, pluginRoot, entry.timeout);
          acc = mergeHookOutput(acc, out);
        } catch (e) {
          acc = mergeHookOutput(acc, { error: String(e?.message ?? e) });
        }
      }
      return acc;
    });
  }
}

function mergeHookOutput(base, extra) {
  if (!extra || typeof extra !== "object") return base;
  if (!base || typeof base !== "object" || Array.isArray(base)) return extra;
  const merged = { ...base, ...extra };
  const left = base.hookSpecificOutput;
  const right = extra.hookSpecificOutput;
  if (left && typeof left === "object" && right && typeof right === "object") {
    const contexts = [left.additionalContext, right.additionalContext].filter(
      (s) => typeof s === "string" && s,
    );
    merged.hookSpecificOutput = { ...left, ...right };
    if (contexts.length > 1) {
      merged.hookSpecificOutput.additionalContext = contexts.join("\n");
    }
  }
  return merged;
}

/** 从 `.claude-plugin/plugin.json` 的 `userConfig` 取默认值，再叠装配里的 `config`。 */
async function pluginOptionsFromRoot(pluginRoot, entryConfig) {
  const defaults = {};
  const pluginJson = path.join(pluginRoot, ".claude-plugin", "plugin.json");
  if (existsSync(pluginJson)) {
    try {
      const doc = JSON.parse(await readFile(pluginJson, "utf8"));
      const userConfig = doc.userConfig;
      if (userConfig && typeof userConfig === "object") {
        for (const [key, spec] of Object.entries(userConfig)) {
          if (spec && typeof spec === "object" && "default" in spec) {
            defaults[key] = spec.default;
          }
        }
      }
    } catch {
      // 元数据损坏时仍用空默认，由 Mod 自身 optionsOf 兜底
    }
  }
  const override =
    entryConfig && typeof entryConfig === "object" && !Array.isArray(entryConfig) ? entryConfig : {};
  return { ...defaults, ...override };
}

export async function loadClaudeMod(root, $, register, _registerTool, entryMeta = {}) {
  const pluginRoot = hostPath(root);
  const admission = claudeModAdmissionError(pluginRoot);
  if (admission !== null) {
    throw new Error(admission);
  }
  const hooksPath = path.join(pluginRoot, "hooks", "hooks.json");
  const raw = await readFile(hooksPath, "utf8");
  const manifest = JSON.parse(raw);
  registerCommandHooks(manifest, pluginRoot, register);
  const pluginOptions = await pluginOptionsFromRoot(pluginRoot, entryMeta.config);

  for (const rel of manifest.modules ?? []) {
    const file = path.join(pluginRoot, "hooks", rel.replace(/^\.\//, ""));
    const mod = await import(pathToFileURL(file).href);
    if (typeof mod.register === "function") {
      mod.register(
        (event, filterOrFn, maybeFn) => {
          let handler = filterOrFn;
          if (typeof filterOrFn !== "function") {
            handler = maybeFn;
          }
          if (typeof handler !== "function") return;
          register(event, handler);
        },
        pluginOptions,
      );
    }
  }
}
