import { readFileSync } from "node:fs";
import { join } from "node:path";

/** 解析能力包 `ability.yml`（仅声明子集，无第三方依赖）。 */
export function loadAbilityYaml(root) {
  const text = readFileSync(join(root, "ability.yml"), "utf8");
  const id = matchScalar(text, "id");
  const framework_version = matchScalar(text, "framework_version");
  const stateful = /^stateful:\s*true/m.test(text);
  const methods = matchMethodIds(text);
  const events = matchEvents(text);
  const depends = matchDepends(text);
  if (!id) {
    throw new Error("ability.yml 缺少 id");
  }
  return {
    id,
    framework_version: framework_version ?? "1",
    stateful,
    methods,
    events,
    depends,
  };
}

/** `framework_version` 的 schema 段（`node@1` → `1`）。 */
export function schemaVersion(frameworkVersion) {
  const at = frameworkVersion.indexOf("@");
  return at >= 0 ? frameworkVersion.slice(at + 1) : frameworkVersion;
}

function matchScalar(text, key) {
  const m = new RegExp(`^${key}:\\s*["']?([^"'\\n#]+)["']?`, "m").exec(text);
  return m?.[1]?.trim();
}

function matchMethodIds(text) {
  const methods = [];
  const section = matchSection(text, "methods");
  if (!section) {
    return methods;
  }
  for (const m of section.matchAll(/^\s+-\s+id:\s*(\S+)/gm)) {
    methods.push({ id: m[1], description: "", input: { type: "object" } });
  }
  return methods;
}

function matchEvents(text) {
  const events = [];
  const section = matchSection(text, "events");
  if (!section) {
    return events;
  }
  const chunks = section.split(/^\s+-\s+name:/m).slice(1);
  for (const chunk of chunks) {
    const name = chunk.match(/^\s*(\S+)/)?.[1];
    const dir = chunk.match(/dir:\s*(\S+)/)?.[1] ?? "emit";
    if (name) {
      events.push({
        name,
        dir,
        description: "",
        payload_schema: { type: "object" },
      });
    }
  }
  return events;
}

function matchSection(text, key) {
  const lines = text.split(/\r?\n/);
  let start = -1;
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].startsWith(`${key}:`)) {
      start = i + 1;
      break;
    }
  }
  if (start < 0) {
    return null;
  }
  const body = [];
  for (let i = start; i < lines.length; i++) {
    if (/^[a-zA-Z_][\w.]*:/.test(lines[i])) {
      break;
    }
    body.push(lines[i]);
  }
  return body.join("\n");
}

function matchDepends(text) {
  const depends = [];
  const section = matchSection(text, "depends");
  if (!section) {
    return depends;
  }
  if (/^\s*\[\s*\]\s*$/m.test(section.trim())) {
    return depends;
  }
  for (const chunk of section.split(/^\s+-\s+/m).slice(1)) {
    const id = chunk.match(/^\s*id:\s*(\S+)/)?.[1];
    if (id) {
      const alias = chunk.match(/alias:\s*(\S+)/)?.[1] ?? null;
      depends.push({ id, alias, optional: false });
      continue;
    }
    const simple = chunk.match(/^\s*(\S+)\s*$/);
    if (simple && !simple[1].includes(":")) {
      depends.push({ id: simple[1], alias: null, optional: false });
    }
  }
  return depends;
}

