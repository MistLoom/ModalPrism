// 界面认识的数据形状。后端的分片方式在这里被折叠成领域概念。

export type HealthState = "ok" | "warn" | "paused" | "misconfigured" | "error" | string;

export interface Module {
  id: string;
  kind: string;
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
}

/** planner.next 的返回：单个信源的最新调度状态（比 30 秒轮询更即时）。 */
export type PlannerNext = Omit<SourceHealth, "source">;

/** strata_queue.list 的返回行：任务队列中的一条任务。 */
export interface TaskRow {
  task_id: string;
  queue: string;
  status: "queued" | "leased" | "done";
  source_id: string;
  reason: string;
  attempt: number;
  priority: number;
  created_at: string;
  retry_at: string | null;
  lease_until: string | null;
  last_error: string | null;
  locator: Record<string, unknown> | null;
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
