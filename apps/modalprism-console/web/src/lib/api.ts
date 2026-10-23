// 与采集进程对话的唯一入口。界面其他部分只调用这里的领域方法。

import type {
  Catalog,
  Channel,
  Commit,
  CommitterStatus,
  ConfigChange,
  ConfigFileBody,
  ConfigFileMeta,
  Connection,
  ContentKind,
  EgressCheck,
  EgressReport,
  EgressStatus,
  FacetBody,
  FacetCell,
  FieldOrigin,
  LibraryRow,
  Lineage,
  Module,
  PlannerNext,
  RelatedEdge,
  Source,
  SourceHealth,
  StrataVersion,
  TaskRow,
  FacetSummary,
} from "./types";

export class ApiError extends Error {
  hint: string | null;
  status: number;

  constructor(message: string, hint: string | null, status: number) {
    super(message);
    this.name = "ApiError";
    this.hint = hint;
    this.status = status;
  }
}

async function call<T>(instance: string, method: string, args: unknown = {}): Promise<T> {
  let response: Response;
  try {
    response = await fetch(`/api/ops/${instance}/${method}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(args),
    });
  } catch {
    throw new ApiError("控制台后端没有响应", "确认 modalprism-console 仍在运行", 0);
  }
  const text = await response.text();
  let body: unknown = null;
  try {
    body = text ? JSON.parse(text) : null;
  } catch {
    body = null;
  }
  if (!response.ok) {
    const error = (body as { error?: { message?: string; hint?: string | null } } | null)?.error;
    throw new ApiError(
      error?.message ?? `请求失败（${response.status}）`,
      error?.hint ?? null,
      response.status,
    );
  }
  return (body as { data: T }).data;
}

export async function ping(): Promise<Connection> {
  const response = await fetch("/api/health", { headers: { accept: "application/json" } });
  if (!response.ok) {
    throw new ApiError("控制台后端没有响应", null, response.status);
  }
  return (await response.json()) as Connection;
}

export async function catalog(): Promise<Catalog> {
  return call<Catalog>("config_inspect", "modules");
}

export async function sourceHealth(): Promise<SourceHealth[]> {
  return (await call<SourceHealth[]>("planner", "status")) ?? [];
}

export async function commits(after = 0, limit = 20): Promise<Commit[]> {
  const rows = await call<Commit[] | null>("strata_read", "changes", { after, limit });
  return (rows ?? []).filter((row) => typeof row.snapshot === "number");
}

export async function explain(module: string): Promise<FieldOrigin[]> {
  return call<FieldOrigin[]>("config_inspect", "explain", { module });
}

export async function validateConfig(): Promise<void> {
  await call("config_inspect", "validate");
}

export async function configDiff(): Promise<ConfigChange[]> {
  return (await call<ConfigChange[] | null>("config_inspect", "diff")) ?? [];
}

export async function triggerSource(source: string): Promise<unknown[]> {
  return (await call<unknown[] | null>("planner", "trigger", { source })) ?? [];
}

/** 查询单个信源的下一轮调度（不走 30 秒轮询，立即返回）。 */
export async function plannerNext(source: string): Promise<PlannerNext> {
  return call<PlannerNext>("planner", "next", { source });
}

/** 立即跑一轮调度：把到期信源的任务派出去。 */
export async function plannerTick(): Promise<unknown[]> {
  return (await call<unknown[] | null>("planner", "tick")) ?? [];
}

/** 任务队列列表，按创建时间倒序。 */
export async function taskList(options?: { status?: string; limit?: number }): Promise<TaskRow[]> {
  const args: Record<string, unknown> = { limit: options?.limit ?? 60 };
  if (options?.status) {
    args.status = options.status;
  }
  return (await call<TaskRow[] | null>("strata_queue", "list", args)) ?? [];
}

/** 重试一个排队中的任务，把它恢复为可立即租约的状态。 */
export async function taskRetry(taskId: string): Promise<boolean> {
  const result = await call<{ retried?: boolean }>("strata_queue", "retry", { taskId });
  return Boolean(result?.retried);
}

export async function egressStatus(): Promise<EgressStatus> {
  return call<EgressStatus>("egress", "status");
}

export async function egressReport(): Promise<EgressReport> {
  const report = await call<Partial<EgressReport>>("egress", "report");
  return {
    mode: report.mode,
    robots: report.robots ?? [],
    circuits: report.circuits ?? [],
    rates: report.rates ?? [],
    audits: report.audits ?? [],
  };
}

export async function egressCheck(url: string): Promise<EgressCheck> {
  return call<EgressCheck>("egress", "check", { url });
}

/** 手动重置一台主机的熔断。 */
export async function egressReset(host: string): Promise<{ host: string; existed: boolean }> {
  return call("egress", "reset", { host });
}

/** 存储状态：控制/正文/暂存三个目录。 */
export async function committerStatus(): Promise<CommitterStatus> {
  return call<CommitterStatus>("committer", "status");
}

/** 立即把暂存区提交进库。 */
export async function committerFlush(): Promise<unknown> {
  return call("committer", "flush");
}

/** 当前配置代际。 */
export async function configGeneration(): Promise<number> {
  return call<number>("config", "generation");
}

/** 让采集进程重新加载配置，返回新代际。 */
export async function applyConfig(): Promise<number> {
  const result = await call<{ generation: number }>("config", "load");
  return typeof result === "number" ? result : (result?.generation ?? 0);
}

async function fileRequest(path: string, init?: RequestInit): Promise<Response> {
  let response: Response;
  try {
    response = await fetch(`/api/config/file?path=${encodeURIComponent(path)}`, {
      headers: { accept: "application/json" },
      ...init,
    });
  } catch {
    throw new ApiError("控制台后端没有响应", "确认 modalprism-console 仍在运行", 0);
  }
  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as {
      error?: { message?: string; hint?: string | null };
    } | null;
    throw new ApiError(
      body?.error?.message ?? `请求失败（${response.status}）`,
      body?.error?.hint ?? null,
      response.status,
    );
  }
  return response;
}

/** 列出配置目录里的声明文件。 */
export async function configFiles(): Promise<ConfigFileMeta[]> {
  let response: Response;
  try {
    response = await fetch("/api/config/files", { headers: { accept: "application/json" } });
  } catch {
    throw new ApiError("控制台后端没有响应", "确认 modalprism-console 仍在运行", 0);
  }
  if (!response.ok) {
    const body = (await response.json().catch(() => null)) as {
      error?: { message?: string; hint?: string | null };
    } | null;
    throw new ApiError(
      body?.error?.message ?? `请求失败（${response.status}）`,
      body?.error?.hint ?? null,
      response.status,
    );
  }
  const body = (await response.json()) as { files: ConfigFileMeta[] };
  return body.files ?? [];
}

/** 读取一份配置文件。 */
export async function configFile(path: string): Promise<ConfigFileBody> {
  const response = await fileRequest(path);
  return (await response.json()) as ConfigFileBody;
}

/** 保存配置文件。 */
export async function saveConfigFile(path: string, content: string): Promise<void> {
  await fileRequest(path, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ path, content }),
  });
}

/** 删除配置文件。 */
export async function deleteConfigFile(path: string): Promise<void> {
  await fileRequest(path, { method: "DELETE" });
}

/** 资源真实存在的 facet 种类。 */
export async function strataKinds(rid: string): Promise<FacetSummary[]> {
  return (
    (await call<FacetSummary[] | null>("strata_read", "kinds", { rid, includeVariants: true })) ??
    []
  );
}

/** 某个 facet 的历史版本。 */
export async function facetHistory(facetRef: string): Promise<StrataVersion[]> {
  return (await call<StrataVersion[] | null>("strata_read", "history", { facet: facetRef })) ?? [];
}

/** 血缘图：这个 facet 是谁产出的、依赖了什么。 */
export async function strataLineage(facetRef: string, depth = 1): Promise<Lineage> {
  return call<Lineage>("strata_read", "lineage", { facet: facetRef, depth });
}

/** 资源的关系表。 */
export async function strataRelated(rid: string, direction: "out" | "in" = "out"): Promise<RelatedEdge[]> {
  return (await call<RelatedEdge[] | null>("strata_read", "related", { rid, direction })) ?? [];
}

/** 资源计数：按来源、时间维度的粗粒度统计。 */
export async function resourceCount(): Promise<number> {
  const page = await call<{ rows: { rid: string }[] | null }>("strata_read", "query", {
    query: { from: "resources", select: ["rid"], limit: 1 },
  });
  return page.rows?.length ?? 0;
}

/** 内容类型的人读名字。未知变体原样保留，不让界面出现裸术语。 */
export function contentLabel(variant: string): string {
  const names: Record<string, string> = {
    discovery: "采集条目",
    article: "文章",
    "hotlist.snapshot": "热榜快照",
    hotlist: "热榜",
  };
  return names[variant] ?? variant;
}

/** 内容库里真实存在的结构化类型；空库时返回空数组，界面据此只展示实际有的类型。 */
export async function contentKinds(): Promise<ContentKind[]> {
  const page = await call<{ rows: { kind?: string; variant?: string | null }[] | null }>(
    "strata_read",
    "query",
    { query: { from: "facets", select: ["kind", "variant"], limit: 400 } },
  );
  const counts = new Map<string, number>();
  for (const row of page.rows ?? []) {
    if (row.kind !== "struct" || typeof row.variant !== "string" || !row.variant) {
      continue;
    }
    counts.set(row.variant, (counts.get(row.variant) ?? 0) + 1);
  }
  return [...counts.entries()]
    .map(([variant, total]) => ({ variant, label: contentLabel(variant), total }))
    .sort((left, right) => right.total - left.total || left.variant.localeCompare(right.variant));
}

export interface LibraryQuery {
  /** 选中的单一内容类型；null 表示全部。 */
  variant: string | null;
  /** 库里存在的全部类型，用于把各类型的单元格一次性带出。 */
  variants: string[];
  search?: string;
  snapshot?: number | null;
  limit?: number;
  /** 只看某个信源产出的内容。 */
  sourceId?: string | null;
  /** 入库时间下界（含）。 */
  since?: string | null;
  /** 入库时间上界（含）。 */
  until?: string | null;
  /** 只看带正文的条目。 */
  withBody?: boolean;
  /** 下一页锚点：只取 seq 小于该值的行（更早的内容）。 */
  before?: number | null;
  /** 上一页锚点：只取 seq 大于该值的行（更新的内容），升序拉取后反转为降序。 */
  after?: number | null;
  /** 精确匹配某条资源：页面之间互链时用它定位单条内容。 */
  rid?: string | null;
}

export interface LibraryPage {
  rows: LibraryRow[];
  /** 按当前筛选条件，后面是否还有更多行。 */
  hasMore: boolean;
}

/** 读取内容库资源行；结构化单元格以 `struct:<变体>` 为键。 */
export async function library(options: LibraryQuery): Promise<LibraryPage> {
  const {
    variant,
    variants,
    search = "",
    snapshot = null,
    limit = 40,
    sourceId = null,
    since = null,
    until = null,
    withBody = false,
    before = null,
    after = null,
    rid = null,
  } = options;
  const wanted = variant ? [variant] : variants;
  const cells = wanted.map((name) => ({ kind: "struct", variant: name }));
  const text = search.trim();
  // 向上一页（更新的内容）用升序拉取，返回后反转为统一的降序展示。
  const newerFirst = after !== null;

  const predicates: Record<string, unknown>[] = [];
  if (sourceId) {
    predicates.push({ eq: ["sourceId", sourceId] });
  }
  if (since) {
    predicates.push({ gte: ["createdAt", second(since)] });
  }
  if (until) {
    predicates.push({ lte: ["createdAt", second(until)] });
  }
  // keyset 锚点：下一页向更早翻，上一页向更新翻；seq 是数字，直接比较。
  if (before !== null) {
    predicates.push({ lt: ["seq", before] });
  }
  if (after !== null) {
    predicates.push({ gt: ["seq", after] });
  }
  if (rid) {
    predicates.push({ eq: ["rid", rid] });
  }

  const query: Record<string, unknown> = {
    from: "resources",
    select: ["rid", "seq", "tombstone", ...cells],
    // 非搜索路径多取一行，用于判断是否还有下一页。
    limit: text ? limit : limit + 1,
  };
  if (predicates.length > 0) {
    query.where = { all: predicates };
  }
  if (text) {
    // 全文检索与 hasFacets 互斥：类型与正文筛选此时退回结果侧完成，且无分页。
    query.search = { text, k: limit };
  } else {
    // 类型单选时把“必须有该类型单元格”下推为 facet 存在性，整页都是有效行；
    // 全部类型时 hasFacets 是 AND 语义，表达不了“任一类型”，只能在结果侧过滤。
    const selectors: { kind: string; variant?: string }[] = [];
    if (variant) {
      selectors.push({ kind: "struct", variant });
    }
    if (withBody) {
      // 不带 variant 表示任意变体。
      selectors.push({ kind: "html" });
    }
    if (selectors.length > 0) {
      query.hasFacets = selectors;
    }
    query.orderBy = [{ field: "seq", descending: !newerFirst }];
  }
  if (snapshot !== null) {
    query.asOf = { snapshot };
  }

  const page = await call<{ rows: LibraryRow[] | null; nextCursor: string | null }>(
    "strata_read",
    "query",
    { query },
  );
  let rows = page.rows ?? [];
  const hasMore = rows.length > limit;
  rows = rows.slice(0, limit);
  if (newerFirst) {
    rows = rows.reverse();
  }
  return {
    rows: rows.filter((row) => wanted.some((name) => row[`struct:${name}`] !== undefined)),
    hasMore,
  };
}

/** 截到整秒：搜索路径里时间在数据层按字符串比较，毫秒位会引入边界误差。 */
function second(iso: string): string {
  return iso.replace(/\.\d+Z$/, "Z");
}

/** 取一行里某个内容类型的结构化单元格。 */
export function structCell(row: LibraryRow, variant: string): Record<string, unknown> | null {
  const cell = row[`struct:${variant}`] as FacetCell | undefined;
  const value = cell?.value;
  return value && typeof value === "object" ? (value as Record<string, unknown>) : null;
}

/** 人类可读的正文：依次尝试常见的正文 facet，都不存在时返回 null。 */
export async function contentBody(rid: string): Promise<string | null> {
  for (const facetRef of [`${rid}@html#latest`, `${rid}@text#latest`, `${rid}@md:readable#latest`]) {
    try {
      const body = await call<FacetBody | null>("strata_read", "get", { facet: facetRef });
      if (body?.text) {
        return body.text;
      }
    } catch {
      // 这种正文类型不存在，换下一种。
    }
  }
  return null;
}

export async function facet(ref: string): Promise<FacetBody | null> {
  return call<FacetBody | null>("strata_read", "get", { facet: ref });
}

export interface Snapshot {
  catalog: Catalog;
  sources: Source[];
  statuses: SourceHealth[];
}

/** 一次读取信源清单：说明来自配置，节律与健康来自调度器。 */
export async function snapshot(): Promise<Snapshot> {
  const [catalogResult, statuses] = await Promise.all([catalog(), sourceHealth()]);
  return {
    catalog: catalogResult,
    statuses,
    sources: mergeSources(catalogResult.modules, statuses),
  };
}

export function mergeSources(modules: Module[], statuses: SourceHealth[]): Source[] {
  const byId = new Map(statuses.map((status) => [status.source, status]));
  return modules
    .filter((module) => module.kind === "source")
    .map((module) => {
      const status = byId.get(module.id);
      const accessorId = text(module.config, ["access", "accessor"]);
      const accessor = modules.find((item) => item.id === accessorId);
      return {
        id: module.id,
        name: module.name ?? stripPrefix(module.id),
        description: module.description ?? null,
        enabled: module.enabled,
        labels: module.labels ?? {},
        accessor: accessorId ?? "",
        accessorLabel: accessor ? channelLabel(accessor) : accessorId ? stripPrefix(accessorId) : "未指定",
        locator: locatorText(module.config),
        intervalMs: status?.intervalMs ?? parseDuration(text(module.config, ["schedule", "interval"])) ?? 0,
        nextAt: status?.nextAt ?? null,
        health: status?.healthStatus ?? (module.enabled ? "idle" : "paused"),
        alert: status?.alert ?? false,
        skipReason: status?.skipReason ?? null,
        lastError: status?.lastError ?? null,
        lastOkAt: status?.lastOkAt ?? null,
        lastYieldAt: status?.lastYieldAt ?? null,
        yields7d: status?.yields7d ?? 0,
        budgetUsed: status?.budgetUsed ?? 0,
        file: module.file,
        line: module.line,
        config: module.config,
      };
    })
    .sort((left, right) => rank(left) - rank(right) || next(left) - next(right));
}

export function mergeChannels(modules: Module[]): Channel[] {
  return modules
    .filter((module) => module.kind === "accessor")
    .map((module) => ({
      id: module.id,
      name: module.name ?? stripPrefix(module.id),
      implementation: text(module.config, ["implementation"]) ?? "",
      implementationLabel: implementationLabel(text(module.config, ["implementation"]) ?? ""),
      enabled: module.enabled,
      timeout: text(module.config, ["limits", "timeout"]) ?? "30s",
      concurrency: number(module.config, ["limits", "concurrency"]) ?? 1,
      retryAttempts: number(module.config, ["limits", "retry", "attempts"]) ?? 0,
      profile: text(module.config, ["egress", "profile"]) ?? "direct",
      requires: text(module.config, ["requires"]),
      file: module.file,
      line: module.line,
      config: module.config,
    }))
    .sort((left, right) => left.name.localeCompare(right.name, "zh-Hans-CN"));
}

export function channelLabel(module: Module): string {
  return implementationLabel(text(module.config, ["implementation"]) ?? "") || module.name || stripPrefix(module.id);
}

export function implementationLabel(implementation: string): string {
  const names: Record<string, string> = {
    "builtin:rss": "RSS / Atom",
    "builtin:http": "网页抓取",
    "builtin:api": "JSON 接口",
    "builtin:search": "站内搜索",
    "builtin:paid-api": "付费接口",
    "builtin:push": "外部推送",
    "builtin:file": "本地文件",
    "builtin:browser": "浏览器渲染",
    "builtin:feed-items": "订阅条目解析",
    "builtin:html-list": "列表页解析",
    "builtin:json-list": "JSON 列表解析",
    "builtin:page-fields": "详情页字段",
  };
  return names[implementation] ?? "";
}

export function stripPrefix(id: string): string {
  const index = id.indexOf("/");
  return index >= 0 ? id.slice(index + 1) : id;
}

function text(config: Record<string, unknown>, path: string[]): string | null {
  const value = walk(config, path);
  return typeof value === "string" ? value : null;
}

function number(config: Record<string, unknown>, path: string[]): number | null {
  const value = walk(config, path);
  return typeof value === "number" ? value : null;
}

function walk(config: unknown, path: string[]): unknown {
  let current: unknown = config;
  for (const key of path) {
    if (!current || typeof current !== "object") {
      return undefined;
    }
    current = (current as Record<string, unknown>)[key];
  }
  return current;
}

function locatorText(config: Record<string, unknown>): string {
  const locator = (config.access as { locator?: Record<string, unknown> } | undefined)?.locator;
  if (!locator) {
    return "";
  }
  if (typeof locator.url === "string") {
    return locator.url;
  }
  if (typeof locator.channel === "string") {
    return `推送通道 · ${locator.channel}`;
  }
  return Object.values(locator)
    .filter((value) => typeof value === "string")
    .join(" · ");
}

export function parseDuration(value: string | null): number | null {
  if (!value) {
    return null;
  }
  const match = /^(\d+)\s*(ms|s|m|h|d)$/.exec(value.trim());
  if (!match) {
    return null;
  }
  const count = Number(match[1]);
  const scale: Record<string, number> = { ms: 1, s: 1000, m: 60_000, h: 3_600_000, d: 86_400_000 };
  return count * (scale[match[2]] ?? 1000);
}

const HEALTH_RANK: Record<string, number> = {
  error: 0,
  misconfigured: 1,
  warn: 2,
  alert: 2,
  paused: 3,
  idle: 4,
  ok: 5,
};

function rank(source: Source): number {
  if (!source.enabled) {
    return 6;
  }
  return HEALTH_RANK[source.health] ?? 4;
}

function next(source: Source): number {
  if (!source.nextAt) {
    return Number.MAX_SAFE_INTEGER;
  }
  const at = Date.parse(source.nextAt);
  return Number.isNaN(at) ? Number.MAX_SAFE_INTEGER : at;
}
