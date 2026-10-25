// 界面认识的数据形状。后端的分片方式在这里被折叠成领域概念。

export type HealthState = "ok" | "warn" | "paused" | "misconfigured" | "error" | string;

export interface Module {
  id: string;
  kind: string;
  /** 功能域：collect/transport/parse/content/policy/other。 */
  domain?: string;
  name: string | null;
  description: string | null;
  enabled: boolean;
  labels: Record<string, string> | null;
  file: string;
  line: number;
  config: Record<string, unknown>;
}

export interface Catalog {
  generation: number;
  system: Record<string, unknown>;
  modules: Module[];
}

/** 调度器给出的信源状态。字段已由后端转成驼峰。 */
export interface SourceHealth {
  source: string;
  nextAt: string | null;
  intervalMs: number;
  healthStatus: HealthState;
  alert: boolean;
  skipReason: string | null;
  budgetUsed: number;
  lastError: string | null;
  lastOkAt: string | null;
  lastYieldAt: string | null;
  yields7d: number;
  /** 今日已产出条数，对上每日上限。 */
  todayYield: number;
}

/** planner.next 的返回：单个信源的最新调度状态（比 30 秒轮询更即时）。 */
export type PlannerNext = Omit<SourceHealth, "source">;

/** strata_queue.list 的返回行：任务队列中的一条任务。 */
export type TaskStatus = "queued" | "leased" | "done" | "cancelled";

export interface TaskRow {
  task_id: string;
  queue: string;
  status: TaskStatus;
  source_id: string;
  /** 使用的通道，取自任务体；永远不包含凭据。 */
  accessor: string;
  reason: string;
  attempt: number;
  priority: number;
  generation: number;
  created_at: string;
  retry_at: string | null;
  lease_until: string | null;
  last_error: string | null;
  locator: Record<string, unknown> | null;
}

/** 一页任务：游标指向本页最后一行，total 是同条件的总数。 */
export interface TaskPage {
  rows: TaskRow[];
  nextCursor: string | null;
  total: number;
}

/** 队列全景统计，页面直接展示而不拉全量。 */
export interface TaskStats {
  total: number;
  byStatus: Record<string, number>;
  byQueue: Record<string, number>;
  byReason: Record<string, number>;
  oldestQueuedAt: string | null;
  nextRetryAt: string | null;
  /** 租约已过期但尚未回收的任务数。 */
  leasedOverdue: number;
}

/** 一个队列的排队、租约、上限与暂停状态。 */
export interface QueueState {
  queue: string;
  queued: number;
  leased: number;
  done: number;
  cancelled: number;
  capacity: number;
  paused: boolean;
}

/** 一轮派发的结果。 */
export interface RoundOutcome {
  leased: number;
  executed: string[];
  failed: [string, string][];
  follows: number;
}

/** 清理终态任务的结果；dryRun 为真时只统计。 */
export interface TaskPurgeReport {
  matched: number;
  removed: number;
  dryRun: boolean;
}

/** 把「信源的说明」与「调度器的状态」合成一行给人看的东西。 */
export interface Source {
  id: string;
  name: string;
  description: string | null;
  enabled: boolean;
  labels: Record<string, string>;
  accessor: string;
  accessorLabel: string;
  locator: string;
  intervalMs: number;
  nextAt: string | null;
  health: HealthState;
  alert: boolean;
  skipReason: string | null;
  lastError: string | null;
  lastOkAt: string | null;
  lastYieldAt: string | null;
  yields7d: number;
  /** 今日已产出条数。 */
  todayYield: number;
  /** 每日上限；未配置时为 null。 */
  dailyLimit: number | null;
  budgetUsed: number;
  file: string;
  line: number;
  config: Record<string, unknown>;
}

export interface Commit {
  snapshot: number;
  commitIds: string[];
  publishedAt: string | null;
}

export interface Connection {
  online: boolean;
  latencyMs: number;
  message: string | null;
  endpoint: string;
}

export interface FieldOrigin {
  path: string;
  /** 设置项的功能域：scheduling/limits/retention/egress/media/credentials/processing/general。 */
  domain?: string;
  value: unknown;
  file: string;
  line: number;
  presetId: string | null;
  viaPreset: boolean;
  systemScope: boolean;
}

export interface ConfigChange {
  path: string;
  before: unknown;
  after: unknown;
}

/** 内容库里的一行：一条资源，外加被选出的 facet 单元格（键为 `struct:<变体>`）。 */
export interface LibraryRow {
  rid: string;
  seq: number;
  tombstone?: boolean;
  /** 产出这条内容的信源。 */
  sourceId?: string;
  /** 命名空间：系统采集默认 data，工作流产出用 struct。 */
  ns?: string;
  /** 用户分类标签；工作流产出行带 workflow=<slug>。 */
  labels?: Record<string, string>;
  [cell: string]: unknown;
}

/** 内容库里真实存在的一类内容。界面只展示这里列出的类型。 */
export interface ContentKind {
  variant: string;
  label: string;
  total: number;
}

/** 查询一行里某个结构化单元格。 */
export interface FacetCell {
  version: number;
  contentHash?: string | null;
  mediaType?: string | null;
  value?: unknown;
  text?: string | null;
}

export interface FacetBody {
  version: number;
  contentHash: string;
  mediaType: string | null;
  size: number;
  text: string | null;
  struct: unknown;
  part: unknown;
}

export interface Channel {
  id: string;
  name: string;
  implementation: string;
  implementationLabel: string;
  enabled: boolean;
  timeout: string;
  concurrency: number;
  retryAttempts: number;
  profile: string;
  requires?: string | null;
  file: string;
  line: number;
  config: Record<string, unknown>;
}

export interface EgressStatus {
  proxy: string | null;
  mode: string;
  upstream: { url: string; source: string } | null;
}

export interface EgressReport {
  mode?: string;
  robots: { origin: string; status: number; failed: boolean; fetchedAt: number; bodyBytes: number }[];
  circuits: {
    host: string;
    consecutive: number;
    openUntil: number | null;
    cooldownMs: number;
    openings: number;
  }[];
  rates: { key: string; tokens: number; inFlight: number; notBefore: number | null }[];
  audits: EgressAudit[];
}

export interface EgressAudit {
  origin: string;
  category: string;
  reason: string;
  at_ms: number;
}

/** 存储状态：控制/正文/暂存三个目录。 */
export interface CommitterStatus {
  control: string;
  blobs: string;
  staging: string;
  /** 各表当前行数。 */
  counts: StoreCounts;
}

/** 各表行数，给「保留与清理」卡片用。 */
export interface StoreCounts {
  resources: number;
  marks: number;
  facets: number;
  blobs: number;
  relations: number;
  state: number;
  snapshots: number;
  tasks: number;
}

/** 一次保留清理的结果：清掉的终态任务与逐表删除行数。 */
export interface SweepReport {
  dryRun: boolean;
  tasks: number;
  floor: Record<string, number>;
  snapshots: number;
  resources: number;
  resourceMarks: number;
  facets: number;
  relations: number;
  state: number;
  blobs: number;
  vectors: number;
  text: number;
}

/** 内容来源、类型与标签分布，服务端聚合（默认排除已删除）。 */
export interface ContentSummary {
  total: number;
  tombstoned: number;
  variants: { variant: string; total: number }[];
  sources: { sourceId: string; total: number }[];
  labels: { key: string; value: string; total: number }[];
}

/** 通道与解析器的运行态，键去掉了 `source/`、`accessor/`、`adapter/` 前缀。 */
export interface HealthItem {
  id: string;
  status: HealthState;
  alert: boolean;
  lastError: string | null;
  lastOkAt: string | null;
  blockedHits: number;
  selectorMiss: number;
}

export interface HealthReport {
  sources: HealthItem[];
  accessors: HealthItem[];
  adapters: HealthItem[];
}

/** 资源真实存在的 facet 种类摘要（来自 strata_read.kinds）。 */
export interface FacetSummary {
  kind: string;
  variant: string | null;
  latest: number;
  versions: number;
  media_type: string | null;
  size: number | null;
}

/** 某个 facet 的历史版本。 */
export interface StrataVersion {
  version: number;
  contentHash: string;
  createdAt: string | null;
  seq: number;
}

/** 血缘图：节点是 facet 的一次产出，边是产出关系。 */
export interface Lineage {
  nodes: { facet: string; module: string | null; version: number | null; paramsHash: string | null }[];
  edges: { from: string; to: string }[];
}

/** 关系表中的一行：src —rel→ dst。 */
export interface RelatedEdge {
  src: string;
  rel: string;
  dst: string;
  attrs: unknown;
  commitId: string | null;
}

/** 配置目录里的一个声明文件。 */
export interface ConfigFileMeta {
  path: string;
  size: number;
  modifiedMs: number | null;
}

/** 配置文件的内容。 */
export interface ConfigFileBody {
  path: string;
  content: string;
  size: number;
  modifiedMs: number | null;
}

export interface EgressCheck {
  blocked: boolean;
  proxied: boolean;
  addresses: string[];
}
