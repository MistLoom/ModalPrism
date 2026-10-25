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
  ContentSummary,
  EgressCheck,
  EgressReport,
  EgressStatus,
  FacetBody,
  FacetCell,
  FieldOrigin,
  HealthReport,
  LibraryRow,
  Lineage,
  Module,
  PlannerNext,
  QueueState,
  RelatedEdge,
  RoundOutcome,
  Source,
  SourceHealth,
  StrataVersion,
  SweepReport,
  TaskPage,
  TaskPurgeReport,
  TaskRow,
  TaskStats,
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

/** 向采集进程的一个运维实例调用方法。声明类与工作流模块在此之上建自己的薄封装。 */
export async function call<T>(instance: string, method: string, args: unknown = {}): Promise<T> {
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

export async function triggerSource(source: string): Promise<{ executed: number; leased: number; failed: number }> {
  const round = await call<{ leased?: number; executed?: string[]; failed?: unknown[] }>(
    "planner",
    "trigger",
    { source },
  );
  return {
    leased: round?.leased ?? 0,
    executed: round?.executed?.length ?? 0,
    failed: round?.failed?.length ?? 0,
  };
}

/** 查询单个信源的下一轮调度（不走 30 秒轮询，立即返回）。 */
export async function plannerNext(source: string): Promise<PlannerNext> {
  return call<PlannerNext>("planner", "next", { source });
}

/** 立即跑一轮调度：把到期信源的任务派出去并执行。 */
export async function plannerRound(source?: string | null): Promise<RoundOutcome> {
  const args: Record<string, unknown> = {};
  if (source) {
    args.source = source;
  }
  return call<RoundOutcome>("planner", "round", args);
}

/** 为信源新建一条人工任务；runNow 时立即执行，locator 指向具体地址时为点位采集。 */
export async function plannerManual(options: {
  source: string;
  queue?: string;
  runNow?: boolean;
  locator?: string;
}): Promise<TaskRow> {
  const args: Record<string, unknown> = { source: options.source };
  if (options.queue) {
    args.queue = options.queue;
  }
  if (options.runNow) {
    args.runNow = true;
  }
  if (options.locator?.trim()) {
    args.locator = options.locator.trim();
  }
  return call<TaskRow>("planner", "manual", args);
}

/** 每队列的排队、租约与暂停状态。 */
export async function queueStates(): Promise<QueueState[]> {
  return (await call<QueueState[] | null>("planner", "queues")) ?? [];
}

/** 暂停或恢复一个队列，返回更新后的队列卡。 */
export async function queuePause(queue: string, paused: boolean): Promise<QueueState[]> {
  return (await call<QueueState[] | null>("planner", "pause", { queue, paused })) ?? [];
}

/** 任务列表：游标分页，筛选全在服务端。 */
export interface TaskQuery {
  status?: string;
  sourceId?: string;
  queue?: string;
  reason?: string;
  /** 关键字：任务号、信源或错误信息。 */
  query?: string;
  cursor?: string | null;
  limit?: number;
}

export async function taskList(options: TaskQuery = {}): Promise<TaskPage> {
  const args: Record<string, unknown> = { limit: options.limit ?? 50 };
  if (options.status) {
    args.status = options.status;
  }
  if (options.sourceId) {
    args.source = options.sourceId;
  }
  if (options.queue) {
    args.queue = options.queue;
  }
  if (options.reason) {
    args.reason = options.reason;
  }
  if (options.query) {
    args.query = options.query;
  }
  if (options.cursor) {
    args.cursor = options.cursor;
  }
  const page = await call<TaskPage | null>("strata_queue", "list", args);
  return { rows: page?.rows ?? [], nextCursor: page?.nextCursor ?? null, total: page?.total ?? 0 };
}

/** 队列全景统计。 */
export async function taskStats(): Promise<TaskStats> {
  return call<TaskStats>("strata_queue", "stats");
}

/** 对选中的任务执行同一生命周期动作（取消/重排/删除），单条与批量走同一状态机约束。 */
export async function taskTransition(
  taskIds: string[],
  action: "cancel" | "requeue" | "remove",
): Promise<{ done: number; skipped: number }> {
  const result = await call<{ done?: number; skipped?: number }>("strata_queue", "transition", {
    taskIds,
    action,
  });
  return { done: result?.done ?? 0, skipped: result?.skipped ?? 0 };
}

/** 取消一条排队或租约中的任务。 */
export async function taskCancel(taskId: string): Promise<boolean> {
  return (await taskTransition([taskId], "cancel")).done > 0;
}

/** 重试或恢复任务：终态与排队中的任务都回到队列。 */
export async function taskRequeue(taskId: string): Promise<boolean> {
  return (await taskTransition([taskId], "requeue")).done > 0;
}

/** 删除一条终态任务。 */
export async function taskRemove(taskId: string): Promise<boolean> {
  return (await taskTransition([taskId], "remove")).done > 0;
}

/** 立即执行一条任务：排队中的先挂租约。 */
export async function taskRun(taskId: string): Promise<{ taskId: string; ok?: boolean; nacked?: string }> {
  return call("runner", "run", { taskId });
}

/** 按筛选清理终态任务（走 committer.sweep 的筛选入口）；默认只统计。 */
export async function taskPurge(options: TaskQuery & { dryRun?: boolean } = {}): Promise<TaskPurgeReport> {
  const args: Record<string, unknown> = { dryRun: options.dryRun ?? true };
  if (options.status) {
    args.status = options.status;
  }
  if (options.sourceId) {
    args.source = options.sourceId;
  }
  if (options.queue) {
    args.queue = options.queue;
  }
  if (options.reason) {
    args.reason = options.reason;
  }
  if (options.query) {
    args.query = options.query;
  }
  const report = await call<{ filtered?: number; dryRun?: boolean }>("committer", "sweep", args);
  const matched = report?.filtered ?? 0;
  const dryRun = report?.dryRun ?? Boolean(args.dryRun);
  return { matched, removed: dryRun ? 0 : matched, dryRun };
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

/** 内容统计：匹配条数与其中已删除的条数。 */
export async function contentCount(options: {
  sourceId?: string | null;
  before?: string | null;
  after?: string | null;
  includeTombstoned?: boolean;
} = {}): Promise<{ matched: number; tombstoned: number }> {
  const args: Record<string, unknown> = { includeTombstoned: options.includeTombstoned ?? false };
  if (options.sourceId) {
    args.source = options.sourceId;
  }
  if (options.before) {
    args.before = options.before;
  }
  if (options.after) {
    args.after = options.after;
  }
  return call("content_admin", "count", args);
}

/** 写入墓碑；默认只统计。单次上限 2000 行，truncated 为真时继续调用即可。 */
export async function contentPurge(options: {
  sourceId?: string | null;
  rids?: string[];
  reason?: string;
  dryRun?: boolean;
} = {}): Promise<{ matched: number; tombstoned: number; truncated: boolean; dryRun: boolean }> {
  const args: Record<string, unknown> = { dryRun: options.dryRun ?? true };
  if (options.sourceId) {
    args.source = options.sourceId;
  }
  if (options.rids && options.rids.length > 0) {
    args.rids = options.rids;
  }
  if (options.reason) {
    args.reason = options.reason;
  }
  return call("content_admin", "purge", args);
}

/** 撤销墓碑，返回恢复条数。 */
export async function contentRestore(rids: string[], reason?: string): Promise<number> {
  const result = await call<{ restored?: number }>("content_admin", "restore", {
    rids,
    ...(reason ? { reason } : {}),
  });
  return result?.restored ?? 0;
}

/** 为用户分类写入或撤销标签，返回打标条数。 */
export async function contentTag(
  rids: string[],
  set: Record<string, string>,
  unset: string[] = [],
): Promise<number> {
  const result = await call<{ labeled?: number }>("content_admin", "label", {
    rids,
    set,
    unset,
  });
  return result?.labeled ?? 0;
}

/** 内容来源与类型分布：服务端聚合，默认排除已删除。 */
export async function strataSummary(): Promise<ContentSummary> {
  return call<ContentSummary>("strata_read", "summary");
}

/** 通道与解析器的运行态。 */
export async function healthReport(): Promise<HealthReport> {
  const report = await call<Partial<HealthReport>>("health", "report");
  return {
    sources: report.sources ?? [],
    accessors: report.accessors ?? [],
    adapters: report.adapters ?? [],
  };
}

/** 按保留策略清理终态任务与过期存储行；默认只统计。 */
export async function sweep(dryRun = true): Promise<SweepReport> {
  return call<SweepReport>("committer", "sweep", { dryRun });
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
  return call<number>("config_admin", "generation");
}

/** 让采集进程重新加载配置，返回新代际。 */
export async function applyConfig(): Promise<number> {
  const result = await call<{ generation: number }>("config_admin", "load");
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

/** 内容库里真实存在的结构化类型；数量由服务端聚合，不再采样。 */
export async function contentKinds(): Promise<ContentKind[]> {
  const summary = await strataSummary();
  return summary.variants
    .filter((item) => item.variant)
    .map((item) => ({ variant: item.variant, label: contentLabel(item.variant), total: item.total }))
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
  /** 是否把已写入墓碑的内容也算进来。 */
  includeTombstoned?: boolean;
  /** 加工层级：按 facet 存在性下推（raw=原始层、md=可读层、struct=结构化）。 */
  level?: string | null;
  /** 用户分类筛选：只看带这个标签的内容。 */
  label?: { key: string; value: string } | null;
  /** 只看工作流产出：按 workflow 标签存在性在结果侧过滤。 */
  processedOnly?: boolean;
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
    includeTombstoned = false,
    level = null,
    label = null,
    processedOnly = false,
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
  // 用户分类：标签谓词两条引擎都支持（内存 eval / SQL EXISTS）。
  if (label) {
    predicates.push({ label: [label.key, label.value] });
  }

  const query: Record<string, unknown> = {
    from: "resources",
    select: ["rid", "seq", "tombstone", "ns", "labels", ...cells],
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
    // 加工层级：原始层看 raw，可读层看 md，结构化层看 struct（与类型选择器 AND 叠加）。
    if (level && level !== "processed") {
      selectors.push({ kind: level });
    }
    if (selectors.length > 0) {
      query.hasFacets = selectors;
    }
    query.orderBy = [{ field: "seq", descending: !newerFirst }];
  }
  if (snapshot !== null) {
    query.asOf = { snapshot };
  }
  if (includeTombstoned) {
    query.includeTombstoned = true;
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
  // 只看加工产出：按 workflow 标签存在性过滤（标签由工作流引擎写入）。
  if (processedOnly) {
    rows = rows.filter((row) => row.labels?.workflow);
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
        todayYield: status?.todayYield ?? 0,
        dailyLimit: number(module.config, ["admission", "daily", "limit"]),
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
