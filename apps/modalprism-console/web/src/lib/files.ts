// 配置文件的本地处理：路径匹配与 YAML 顶层开关。纯函数，不发请求。
import type { ConfigFileMeta } from "./types";

/** 把后端给出的路径（可能是模块目录里的绝对路径）匹配到配置目录的相对路径。 */
export function matchConfigFile(files: ConfigFileMeta[], want: string): string | null {
  const norm = want.replace(/\\/g, "/").replace(/^\.\//, "").toLowerCase();
  const candidates = files.map((file) => file.path.replace(/\\/g, "/").toLowerCase());
  const exact = candidates.indexOf(norm);
  if (exact >= 0) {
    return files[exact].path;
  }
  const suffix = candidates.findIndex((path) => norm.endsWith(`/${path}`));
  if (suffix >= 0) {
    return files[suffix].path;
  }
  const tail = norm.split("/").slice(-2).join("/");
  const tailHit = candidates.findIndex((path) => path.endsWith(`/${tail}`));
  return tailHit >= 0 ? files[tailHit].path : null;
}

const ENABLED_LINE = /^enabled:\s*(?:true|false)\s*(?:#.*)?$/m;

/** 在 YAML 顶层切换 enabled 开关；没有该行且要暂停时，在文件顶部插入。 */
export function setYamlEnabled(content: string, value: boolean): string {
  if (ENABLED_LINE.test(content)) {
    return content.replace(ENABLED_LINE, `enabled: ${value}`);
  }
  return value ? content : `enabled: ${value}\n\n${content}`;
}

/**
 * 替换 YAML 指定行（1-based）的标量值，保留缩进与行尾注释。
 * 行的键与 expectedKey 不符时抛错，防止 explain 之后文件被改过导致写错行。
 */
export function setYamlLineValue(
  content: string,
  line: number,
  expectedKey: string,
  value: string | number | boolean,
): string {
  const lines = content.split("\n");
  const index = line - 1;
  if (index < 0 || index >= lines.length) {
    throw new Error(`第 ${line} 行超出文件范围`);
  }
  const source = lines[index];
  const match = source.match(/^(\s*)([^#:\s][^:]*):(\s*)(.*?)(\s*(?:#.*)?)$/);
  if (!match) {
    throw new Error(`第 ${line} 行不是「key: value」格式，无法在这里修改`);
  }
  const [, indent, key, , , comment] = match;
  const actualKey = key.trim().replace(/^["']|["']$/g, "");
  if (actualKey !== expectedKey) {
    throw new Error(`第 ${line} 行是 ${actualKey}，与字段 ${expectedKey} 不一致，文件可能已变更，请重新打开`);
  }
  const rendered = typeof value === "string" ? JSON.stringify(value) : String(value);
  lines[index] = `${indent}${key}: ${rendered}${comment}`;
  return lines.join("\n");
}
