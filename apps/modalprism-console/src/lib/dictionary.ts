// 语义字典：把程序值翻译成用户语言。
//
// 数据来自 `GET /dictionary`（后端内置条目 + `config/dictionary.yaml` 覆盖），
// 启动拉一次并缓存在模块里；`t()` 是纯函数，取不到时回落原值并在开发环境告警。

import { getDictionary } from "./api";

type Entry = { label: string; tone?: string };
type Table = Record<string, Record<string, Entry>>;

let table: Table = {};
let version = "";
let loaded = false;
const warned = new Set<string>();

/** 拉取字典；失败不阻断页面，回落原值。 */
export async function loadDictionary(): Promise<void> {
  try {
    const document = await getDictionary();
    table = (document.domains ?? {}) as Table;
    version = document.version ?? "";
    loaded = true;
  } catch {
    loaded = true;
  }
}

export function dictionaryLoaded(): boolean {
  return loaded;
}

/** 翻译一个值域里的程序值。未收录时返回 fallback（默认原值）并告警一次。 */
export function t(domain: string, code?: string | null, fallback?: string): string {
  if (!code) return fallback ?? "—";
  const entry = table[domain]?.[code];
  if (entry) return entry.label;
  if (!warned.has(`${domain}:${code}`)) {
    warned.add(`${domain}:${code}`);
    if (import.meta.env.DEV) console.warn(`字典缺少条目 ${domain}.${code}`);
  }
  return fallback ?? code;
}

/** 该值域里是否收录了这个值（用于「未收录」标注）。 */
export function hasEntry(domain: string, code?: string | null): boolean {
  return Boolean(code && table[domain]?.[code]);
}

/** 值域对应的语气，用于徽标配色。 */
export function toneOf(domain: string, code?: string | null): string | undefined {
  if (!code) return undefined;
  return table[domain]?.[code]?.tone;
}

export function dictionaryVersion(): string {
  return version;
}

/** 结构化值的键 → 值域推断：详情里看到的裸值必须能被翻译。 */
const FIELD_DOMAINS: Record<string, string> = {
  kind: "facet.kind",
  facetKind: "facet.kind",
  status: "task.status",
  queue: "task.queue",
  reason: "task.reason",
  relation: "relation",
  direction: "relation.direction",
  mode: "run.mode",
  errorClass: "error.class",
  mediaType: "media.type",
  contentType: "media.type",
  sourceType: "source.type",
  health: "source.health",
  stopReason: "source.stopReason",
  phase: "monitor.phase",
  level: "log.level",
  trigger: "plan.trigger",
  variant: "facet.variant",
};

/** 按字段名推断值域；已知字段的值若不在字典里，交由调用方保留原文并标注。 */
export function domainOfField(field: string): string | undefined {
  if (FIELD_DOMAINS[field]) return FIELD_DOMAINS[field];
  return undefined;
}
