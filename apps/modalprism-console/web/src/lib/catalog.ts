// 声明式目录的界面入口。
//
// 信源、访问器与适配器都是配置目录里的 YAML。写入统一走 `catalog_admin` 的方法：
// 页面不再自己拼 YAML 行，也不会各自判断代理字段该落在哪一层。
// 这里只做「读出来、改字段、写回去」，校验与回滚由采集进程负责。

import { call, implementationLabel } from "./api";

export type DeclarationKind = "source" | "accessor" | "adapter";
export type BatchAction = "enable" | "disable" | "remove";

export interface Declaration {
  id: string;
  slug: string;
  file: string;
  name: string;
  description: string | null;
  enabled: boolean;
  references: number;
  doc: Record<string, unknown>;
  /** 信源：引用的访问器与定位方式。 */
  accessor?: string | null;
  locator?: Record<string, unknown> | null;
  endpoint?: string | null;
  schedule?: string | null;
  adapters?: string[];
  /** 访问器与解析器共有的字段。 */
  implementation?: string | null;
  timeout?: string | null;
  concurrency?: number | null;
  profile?: string | null;
  /** 解析器：角色与内置标记。 */
  role?: string | null;
  produces?: unknown;
  builtin?: boolean;
  removable?: boolean;
  proxy?: string | null;
}

export interface Declarations {
  sources: Declaration[];
  accessors: Declaration[];
  adapters: Declaration[];
}

export interface Template {
  kind: DeclarationKind;
  slug: string;
  name: string;
  description: string;
  doc: Record<string, unknown>;
}

export interface BatchResult {
  kind: string;
  action: string;
  total: number;
  failed: number;
  results: { slug: string; ok: boolean; file?: string; error?: string }[];
}

export interface Saved {
  id: string;
  file: string;
}

export interface Draft {
  kind: DeclarationKind;
  slug: string;
  doc: Record<string, unknown>;
}

export async function declarations(): Promise<Declarations> {
  const list = await call<Partial<Declarations> | null>("catalog_admin", "list");
  return {
    sources: list?.sources ?? [],
    accessors: list?.accessors ?? [],
    adapters: list?.adapters ?? [],
  };
}

export async function templates(): Promise<Template[]> {
  return (await call<Template[] | null>("catalog_admin", "templates")) ?? [];
}

export async function createDeclaration(draft: Draft): Promise<Saved> {
  return call<Saved>("catalog_admin", "create", draft);
}

export async function updateDeclaration(draft: Draft): Promise<Saved> {
  return call<Saved>("catalog_admin", "update", draft);
}

export async function removeDeclaration(kind: DeclarationKind, slug: string): Promise<Saved> {
  return call<Saved>("catalog_admin", "remove", { kind, slug });
}

export async function toggleDeclaration(
  kind: DeclarationKind,
  slug: string,
  enabled: boolean,
): Promise<Saved> {
  return call<Saved>("catalog_admin", "toggle", { kind, slug, enabled });
}

export async function batchDeclarations(
  kind: DeclarationKind,
  action: BatchAction,
  slugs: string[],
): Promise<BatchResult> {
  return call<BatchResult>("catalog_admin", "batch", { kind, action, slugs });
}

/** 系统级出站代理：空串回到「跟随系统」。 */
export async function setGlobalProxy(value: string): Promise<string | null> {
  const result = await call<{ proxy?: string | null }>("catalog_admin", "proxy", { value });
  return result?.proxy ?? null;
}

export function listOf(list: Declarations, kind: DeclarationKind): Declaration[] {
  return kind === "source" ? list.sources : kind === "accessor" ? list.accessors : list.adapters;
}

// ── 代理取值 ────────────────────────────────────────────────────────────────
//
// 三层语义只在这里解释：空 = 跟随系统，`off` = 直连，其余按代理地址处理。
// 信源、访问器与系统策略共用同一套取值，界面各处因此不会说法不一。

export type ProxyMode = "inherit" | "off" | "manual";

export function proxyOf(doc: Record<string, unknown>): string | null {
  const value = readPath(doc, ["policy", "egress", "proxy"]);
  return typeof value === "string" && value.trim() ? value : null;
}

export function proxyMode(value: string | null): ProxyMode {
  if (!value || value === "system") {
    return "inherit";
  }
  return value === "off" ? "off" : "manual";
}

/** 把代理写进声明的 `policy.egress.proxy`；空值表示删掉这一层覆盖。 */
export function withProxy(doc: Record<string, unknown>, value: string | null): Record<string, unknown> {
  const next = clone(doc);
  const text = (value ?? "").trim();
  const policy = objectOf(next, "policy");
  const egress = objectOf(policy, "egress");
  if (text) {
    egress.proxy = text;
    policy.egress = egress;
    next.policy = policy;
  } else {
    delete egress.proxy;
    if (Object.keys(egress).length > 0) {
      policy.egress = egress;
      next.policy = policy;
    } else {
      delete policy.egress;
      if (Object.keys(policy).length > 0) {
        next.policy = policy;
      } else {
        delete next.policy;
      }
    }
  }
  return next;
}

export function proxyLabel(value: string | null): string {
  const mode = proxyMode(value);
  if (mode === "inherit") {
    return "跟随系统";
  }
  return mode === "off" ? "直连" : (value ?? "");
}

// ── 表单字段 ────────────────────────────────────────────────────────────────
//
// 一份字段清单同时给新建与编辑用：新建从模板起步，编辑回填磁盘上的声明。
// 清单之外的东西仍可改 —— 抽屉底部的高级区直接编辑整份 JSON。

export interface FieldSpec {
  path: string[];
  label: string;
  kind: "text" | "textarea" | "select" | "number";
  hint?: string;
  placeholder?: string;
  options?: { value: string; label: string }[];
  wide?: boolean;
}

/** 访问器与解析器可用的内置实现。标签复用 api 里的同一份名字表。 */
const ACCESSOR_IMPLS = [
  "builtin:rss",
  "builtin:http",
  "builtin:api",
  "builtin:search",
  "builtin:browser",
  "builtin:paid-api",
  "builtin:push",
  "builtin:file",
];

const ADAPTER_IMPLS = [
  "builtin:feed-items",
  "builtin:html-list",
  "builtin:json-list",
  "builtin:page-fields",
];

export function implementations(kind: Extract<DeclarationKind, "accessor" | "adapter">): { value: string; label: string }[] {
  return (kind === "accessor" ? ACCESSOR_IMPLS : ADAPTER_IMPLS).map((value) => ({
    value,
    label: implementationLabel(value) || value,
  }));
}

export function fieldsOf(kind: DeclarationKind, accessors: Declaration[]): FieldSpec[] {
  if (kind === "source") {
    return [
      { path: ["name"], label: "名称", kind: "text", placeholder: "给人看的名字" },
      { path: ["description"], label: "说明", kind: "textarea", wide: true },
      {
        path: ["access", "accessor"],
        label: "采集通道",
        kind: "select",
        options: accessors.map((item) => ({ value: item.id, label: `${item.name} · ${item.id}` })),
        hint: "决定用什么方式取数",
      },
      {
        path: ["access", "locator", "url"],
        label: "地址",
        kind: "text",
        placeholder: "https://example.com/feed.xml",
        hint: "订阅、网页与接口信源填这里",
      },
      {
        path: ["access", "locator", "query"],
        label: "查询词",
        kind: "text",
        placeholder: "artificial intelligence",
        hint: "搜索型通道用它代替地址",
      },
      {
        path: ["schedule", "interval"],
        label: "采集节律",
        kind: "text",
        placeholder: "2h",
        hint: "形如 30m、2h、1d",
      },
      {
        path: ["admission", "initialBackfill", "limit"],
        label: "首次回填条数",
        kind: "number",
        hint: "首次采集最多入库多少条，留空表示不限制",
      },
      {
        path: ["admission", "perRun", "limit"],
        label: "每轮上限",
        kind: "number",
        hint: "每一轮采集最多入库多少条，留空表示不限制",
      },
      {
        path: ["admission", "daily", "limit"],
        label: "每日上限",
        kind: "number",
        hint: "当天累计到这个条数后，当天不再派发",
      },
    ];
  }
  if (kind === "accessor") {
    return [
      { path: ["name"], label: "名称", kind: "text" },
      { path: ["description"], label: "说明", kind: "textarea", wide: true },
      {
        path: ["implementation"],
        label: "取数方式",
        kind: "select",
        options: implementations("accessor"),
        hint: "内置实现由采集进程提供",
      },
      { path: ["limits", "timeout"], label: "单次超时", kind: "text", placeholder: "30s" },
      { path: ["limits", "concurrency"], label: "并发上限", kind: "number" },
      { path: ["limits", "retry", "attempts"], label: "重试次数", kind: "number" },
      {
        path: ["egress", "profile"],
        label: "出站方式",
        kind: "select",
        options: [
          { value: "direct", label: "直连" },
          { value: "proxy", label: "经代理" },
          { value: "auto", label: "自动选择" },
        ],
      },
    ];
  }
  return [
    { path: ["name"], label: "名称", kind: "text" },
    { path: ["description"], label: "说明", kind: "textarea", wide: true },
    {
      path: ["implementation"],
      label: "解析方式",
      kind: "select",
      options: implementations("adapter"),
    },
    {
      path: ["role"],
      label: "角色",
      kind: "select",
      options: [
        { value: "search", label: "搜索 · 产出入库候选" },
        { value: "content", label: "正文 · 产出结构化内容" },
      ],
    },
    {
      path: ["spec", "item"],
      label: "条目选择器",
      kind: "text",
      placeholder: "article",
      hint: "HTML 列表解析用；JSON 解析可留空",
    },
    { path: ["spec", "link"], label: "链接选择器", kind: "text", placeholder: "a" },
    { path: ["spec", "title"], label: "标题选择器", kind: "text", placeholder: "a" },
    { path: ["spec", "items"], label: "JSON 条目路径", kind: "text", placeholder: "/items" },
  ];
}

// ── 文档路径读写 ────────────────────────────────────────────────────────────

export function readPath(doc: Record<string, unknown>, path: string[]): unknown {
  let current: unknown = doc;
  for (const key of path) {
    if (!current || typeof current !== "object") {
      return undefined;
    }
    current = (current as Record<string, unknown>)[key];
  }
  return current;
}

/** 写一个字段：空值表示删除，空的父对象一并清掉，避免留下 `{}` 噪声。 */
export function writePath(
  doc: Record<string, unknown>,
  path: string[],
  value: unknown,
): Record<string, unknown> {
  const next = clone(doc);
  if (value === null || value === undefined || value === "") {
    remove(next, path);
  } else {
    assign(next, path, value);
  }
  return next;
}

export function clone(doc: Record<string, unknown>): Record<string, unknown> {
  return JSON.parse(JSON.stringify(doc)) as Record<string, unknown>;
}

function objectOf(parent: Record<string, unknown>, key: string): Record<string, unknown> {
  const value = parent[key];
  if (value && typeof value === "object" && !Array.isArray(value)) {
    return { ...(value as Record<string, unknown>) };
  }
  return {};
}

function assign(target: Record<string, unknown>, path: string[], value: unknown): void {
  const [head, ...rest] = path;
  if (rest.length === 0) {
    target[head] = value;
    return;
  }
  const child = objectOf(target, head);
  assign(child, rest, value);
  target[head] = child;
}

function remove(target: Record<string, unknown>, path: string[]): void {
  const [head, ...rest] = path;
  if (rest.length === 0) {
    delete target[head];
    return;
  }
  const child = target[head];
  if (!child || typeof child !== "object" || Array.isArray(child)) {
    return;
  }
  const next = { ...(child as Record<string, unknown>) };
  remove(next, rest);
  if (Object.keys(next).length === 0) {
    delete target[head];
  } else {
    target[head] = next;
  }
}

/** 信源一行给人看的定位文本；与列表接口的 locator 摘要保持同一口径。 */
export function locatorText(doc: Record<string, unknown>): string {
  const locator = readPath(doc, ["access", "locator"]);
  if (!locator || typeof locator !== "object") {
    return "";
  }
  const entries = Object.entries(locator as Record<string, unknown>).filter(
    ([, value]) => typeof value === "string" && value,
  );
  return entries.map(([, value]) => String(value)).join(" · ");
}

/** 信源的管道步骤 id；用于在列表里说清「这一条会经过谁」。 */
export function pipelineOf(doc: Record<string, unknown>): string[] {
  const steps = doc.pipeline;
  if (!Array.isArray(steps)) {
    return [];
  }
  return steps.flatMap((step) => {
    const run = (step as { run?: unknown })?.run;
    return Array.isArray(run) ? run.filter((item): item is string => typeof item === "string") : [];
  });
}
