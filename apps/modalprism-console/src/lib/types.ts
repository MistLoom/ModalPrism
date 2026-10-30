// 公共 API 的领域资源。字段与 docs/specs/public-api.openapi.yaml 对齐；
// 产生差异时以 OpenAPI 为准，并由 scripts/check-contract.mjs 在构建前校验路径。

export type OperationStatus =
  | "queued"
  | "running"
  | "cancelling"
  | "succeeded"
  | "partiallySucceeded"
  | "failed"
  | "cancelled";

export interface ResourceRef {
  kind: string;
  id: string;
}

export interface Operation {
  operationId: string;
  kind: string;
  status: OperationStatus;
  progress: number;
  cancellable: boolean;
  retryable: boolean;
  retryOf: string | null;
  resourceRefs: ResourceRef[];
  resultSummary: Record<string, unknown> | null;
  errorSummary: { code?: string; message?: string; fieldErrors?: FieldError[]; requestId?: string } | null;
  createdAt: string;
  completedAt: string | null;
}

export interface FieldError {
  field: string;
  code: string;
  message: string;
}

export interface ListResponse<T> {
  items: T[];
  nextCursor: string | null;
  /** 筛选口径下的总条数（列表 API 透出，供页码分页）。 */
  total?: number;
}

export interface SystemStatus {
  status: "online" | "degraded" | "maintenance";
  version: string;
  generation: number;
  startedAt: string;
  capabilities?: { parse?: boolean; workflow?: boolean };
}

export interface Queue {
  id: string;
  queued: number;
  running: number;
  done?: number;
  cancelled?: number;
  capacity: number;
  paused: boolean;
  stalled?: boolean;
  reason?: string;
}

export interface Overview {
  generatedAt: string;
  status: string;
  generation: number;
  counts: {
    sources: number;
    enabledSources: number;
    healthySources: number;
    attention: number;
    content: number;
    tombstoned: number;
    tasksQueued: number;
    tasksRunning: number;
    tasksFailed: number;
    failedDeliveries: number;
  };
  queues: Queue[];
  attention: { kind: string; id: string; title: string; status: string; message?: string | null; filter?: Record<string, string> }[];
  recentOperations: Operation[];
  sources: { id: string; name: string; health?: string | null; todayYield?: number | null; nextRunAt?: string | null }[];
  contentKinds: { variant: string; total: number }[];
}

export interface Schedule {
  mode: "fixed" | "adaptive" | "push";
  intervalMs?: number | null;
  interval?: string;
  minimumIntervalMs?: number | null;
}

export interface Source {
  id: string;
  name: string;
  description?: string | null;
  enabled: boolean;
  /** 信源类型（feed/search/web/api/push/custom），后端按声明或推导输出。 */
  sourceType: string;
  /** 归属项目；由后端按创建时的活跃项目写入。 */
  project?: string;
  /** 共享后其他项目只读可见。 */
  shared?: boolean;
  accessMethodId: string;
  adapterIds: string[];
  endpoint: string;
  /** 接收类信源的频道标识（locator.channel）；其余类型为空。 */
  channel?: string;
  schedule: Schedule;
  health?: string | null;
  healthReason?: string | null;
  // 命中限额/停止策略时的原因（quota-paused / quota-exhausted / budget / daily-cap）。
  stopReason?: string | null;
  nextRunAt?: string | null;
  lastRunAt?: string | null;
  todayYield?: number | null;
  alert?: boolean | null;
  references?: number;
  labels?: Record<string, string>;
  etag: string;
}

export interface SourceInput {
  name: string;
  accessMethodId: string;
  adapterIds: string[];
  enabled: boolean;
  sourceType?: string;
  endpoint: string;
  /** 接收类必填；与地址互斥，后端按它写 locator.channel。 */
  channel?: string;
  schedule: Schedule;
  admission?: Record<string, unknown>;
  /** 创建时是否立即共享给其他项目。 */
  shared?: boolean;
}

/** 写盘载荷：节律已迁到计划，信源声明不再提交 schedule。 */
export type SourcePayload = Omit<SourceInput, "schedule">;

/** 信源类型注册表条目（GET /catalog/sourceTypes，与后端校验同源）。 */
export interface SourceTypeRecord {
  id: string;
  label: string;
  locatorShape: string;
  allowedAccessors: string[] | null;
  /** 简易表单可落盘的访问器子集；null 表示不限（自定义类型）。前端只渲染不判断。 */
  formAccessors: string[] | null;
  allowedAdapters: string[] | null;
  forbidPaid: boolean;
  stagnationCycles: number | null;
  planDefaults: {
    trigger?: Record<string, unknown> | null;
    quota?: Record<string, unknown>;
    stopPolicy?: Record<string, unknown>;
  };
  healthExpectation: Record<string, unknown>;
}

export interface NamedResource {
  id: string;
  name: string;
  description?: string | null;
  enabled?: boolean;
  builtin?: boolean;
  etag?: string;
  [key: string]: unknown;
}

export interface Plan {
  id: string;
  name: string;
  description?: string | null;
  targets?: string[];
  trigger?: {
    kind?: string;
    interval?: string;
    floor?: string;
    ceiling?: string;
    adaptive?: boolean;
    expectedPeriod?: string;
    fireAt?: string;
  } | null;
  queue?: string | null;
  priority?: number | null;
  quota?: {
    perRunItems?: number;
    totalItems?: number;
    maxAge?: string;
    dedupWindow?: string;
  } | null;
  stopPolicy?: Record<string, unknown> | null;
  enabled: boolean;
  status: string;
  // 兼容旧信源页展示：节律已迁到 trigger，plan 实体不再返回 schedule。
  schedule?: Schedule;
  sourceId?: string;
  health?: string | null;
  nextRunAt?: string | null;
  lastRunAt?: string | null;
  todayYield?: number | null;
  // 计划维度的投递：由内容投递域的规则承载，计划只提交目标与开关。
  deliveryRuleId?: string | null;
  deliveryTargets?: DeliveryTarget[] | null;
  deliveryEnabled?: boolean | null;
  deliveryLastRunAt?: string | null;
  etag: string;
}

export interface Task {
  id: string;
  kind: string;
  status: string;
  title?: string;
  sourceId?: string | null;
  planId?: string | null;
  queue?: string;
  reason?: string;
  attempt?: number;
  createdAt: string;
  updatedAt?: string | null;
  retryAt?: string | null;
  error?: string | null;
  locator?: unknown;
  runId?: string;
}

export interface ContentItem {
  id: string;
  kind: string;
  title: string;
  summary?: string | null;
  sourceId: string;
  sourceName: string;
  status: "active" | "tombstoned";
  tags: string[];
  publishedAt?: string | null;
  createdAt: string;
  struct?: Record<string, unknown> | null;
  etag: string;
}

export interface ContentBody {
  kind?: string;
  variant?: string | null;
  version?: number;
  mediaType?: string | null;
  text?: string;
  value?: unknown;
}

export interface ContentDetail extends ContentItem {
  body: ContentBody | null;
  facets: { kind: string; variant?: string | null; version: number; versions: number; mediaType?: string | null; size?: number | null }[];
}

export interface CollectionRun {
  id: string;
  taskId: string;
  sourceId: string;
  /** 分派它的采集计划；手动采集为空。 */
  planId?: string | null;
  mode: "manual" | "scheduled" | string;
  status: "queued" | "running" | "succeeded" | "failed" | "cancelled" | string;
  queue: string;
  attempt: number;
  locator?: unknown;
  createdAt: string;
  updatedAt: string;
  error?: string | null;
}

export interface Snapshot {
  id: string;
  snapshot: number;
  publishedAt: string;
  commitCount: number;
}

export interface DeliveryChannel {
  id: string;
  name: string;
  kind: "webhook" | "feishu" | string;
  enabled: boolean;
  settings: Record<string, unknown>;
  status?: string;
  etag?: string;
  createdAt: string;
}

export interface ModelResource {
  id: string;
  name: string;
  kind: "remoteApi" | "localService" | "localCli";
  endpoint: string;
  secretRef: string | null;
  secretPresent?: boolean;
  secretInline?: boolean;
  enabled: boolean;
  model?: string;
  timeoutMs?: number;
  maxTokens?: number;
  /** 采样温度；null 表示按厂商官方缺省、不向请求注入 temperature。 */
  temperature?: number | null;
  /** 上下文窗口（tokens），登记时按模型预设带出。 */
  contextWindow?: number;
  /** 是否支持思考模式（推理）输出。 */
  reasoning?: boolean;
  /** 支持的输入模态：text / image / audio / video。 */
  modalities?: string[];
  createdAt: string;
}

export interface ModelTestResult {
  ok: boolean;
  mode: string;
  latencyMs: number;
  sample?: string;
  version?: string;
  note?: string;
}

export interface CliCandidate {
  id: string;
  label: string;
  command: string;
  found: boolean;
  version: string;
}

export interface PromptResource {
  id: string;
  name: string;
  description: string;
  builtin: boolean;
  latestVersionId: string;
  versions: { id: string; version: number; content: string; createdAt: string }[];
  etag?: string;
}

export interface SettingsField {
  key: string;
  label: string;
  group: string;
  type: "string" | "integer" | "number" | "boolean" | "enum";
  default: unknown;
  options: { value: string; label: string }[];
  presets: { value: number; label: string }[];
  unit: string;
  minimum?: number;
  maximum?: number;
  editable: boolean;
  effect: "hotReload" | "restartRequired";
  description: string;
  secret: boolean;
}

export interface SettingsGroup {
  id: string;
  label: string;
  note: string;
}

export interface SettingsSchema {
  version: number;
  groups: SettingsGroup[];
  fields: SettingsField[];
}

export interface EffectiveSettings {
  generation: number;
  values: Record<string, unknown>;
  appliedAt: string;
}

export interface SettingsHistoryEntry {
  generation: number;
  values: Record<string, unknown>;
  summary: string;
  appliedAt: string;
  rollbackOf: number | null;
}

export interface LogRecord {
  cursor: string;
  stream: string;
  timestamp: string;
  level: string;
  kind: string;
  message: string;
  requestId?: string | null;
  operationId?: string | null;
  taskId?: string | null;
  sourceId?: string | null;
  contentId?: string | null;
  runId?: string | null;
  attemptId?: string | null;
  deliveryId?: string | null;
  configGeneration?: number | null;
  data?: unknown;
}

export interface HealthHistory {
  id?: string;
  evaluatedAt: string;
  status: string;
  sources: { sourceId: string; status: string; reason?: string | null; todayYield?: number | null; alert?: boolean | null }[];
  queues?: Queue[];
  storage?: Record<string, number>;
}

export interface ResetPreview {
  id: string;
  kind: "reset" | "factory";
  domains: string[];
  requested: string[];
  counts: Record<string, { count: number; bytes: number }>;
  dependencies: { domain: string; reason: string }[];
  confirmationToken: string;
  expiresAt: string;
  etag: string;
  storageReachable: boolean;
  createdAt: string;
}

export interface Project {
  id: string;
  name: string;
  description: string;
  isDefault: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface ProjectResource {
  kind: string;
  id: string;
  boundAt?: string;
  /** 信源资源：是否为其他项目共享而来。 */
  shared?: boolean;
}

export interface WizardStep {
  id: string;
  title: string;
  description?: string;
  status: "pending" | "done" | "optional";
  met: boolean;
  optional: boolean;
  /** 未满足时去哪儿做。 */
  hint?: string;
}

export interface WizardDoc {
  projectId: string;
  /** 只读起步清单：无待办即 ready。 */
  status?: "active" | "ready";
  pending: number;
  steps: WizardStep[];
}

export interface SearchEngine {
  id: string;
  name: string;
  requiresKeyRef: string | null;
  builtin: boolean;
}

export interface SearchHit {
  title: string | null;
  url: string | null;
  snippet: string | null;
  source: string | null;
}

export interface SearchResult {
  engine: string;
  /** 请求的引擎与实际应答的引擎不同时（DDG 回退 Lite）给出请求值。 */
  requestedEngine?: string;
  query: string;
  count: number;
  results: SearchHit[];
  /** 免密钥引擎均无结果时为真，不再静默返回空。 */
  degraded?: boolean;
  note?: string;
  projectId?: string;
}

export interface FetchResult {
  url: string;
  status: number;
  contentType: string;
  truncated: boolean;
  text: string;
}

export interface PreconditionItem {
  scope: string;
  engine?: string;
  met: boolean;
  requirements: { type: string; setting?: string; message: string; where: string }[];
}

export interface PreconditionReport {
  met: boolean;
  scopes: PreconditionItem[];
}

/** 内容投递目标：模型、外部 API 或通知渠道。 */
export interface DeliveryTarget {
  kind: "model" | "endpoint" | "channel";
  id: string;
  name?: string;
  detail?: string;
  enabled?: boolean;
}

export interface DeliveryTargetGroup {
  kind: string;
  label: string;
  note: string;
}

export interface DeliveryRecordItem {
  contentId: string;
  title: string;
  sourceName: string;
}

export interface DeliveryRecordResult {
  targetKind: string;
  targetId: string;
  targetName?: string;
  status: string;
  latencyMs?: number;
  output?: unknown;
  error?: string;
}

export interface DeliveryRecord {
  id: string;
  type: "manual" | "auto" | string;
  status: string;
  note?: string;
  ruleId?: string;
  trigger?: { kind?: string; ruleName?: string; listener?: { kind?: string; id?: string; name?: string }; by?: string };
  items: DeliveryRecordItem[];
  targets: DeliveryTarget[];
  results: DeliveryRecordResult[];
  createdAt: string;
  updatedAt?: string;
}

export interface DeliveryRule {
  id: string;
  name: string;
  enabled: boolean;
  listener: { kind: "source" | "plan"; id: string; name?: string };
  targets: DeliveryTarget[];
  lastRunAt?: string | null;
  lastStatus?: string | null;
  createdAt: string;
  etag?: string;
}

export interface DeliveryEndpoint {
  id: string;
  name: string;
  url: string;
  tokenRef: string;
  enabled: boolean;
  createdAt?: string;
  etag?: string;
}

export interface NotificationEntry {
  id: string;
  category: string;
  level: string;
  title: string;
  message: string;
  read: boolean;
  createdAt: string;
}

export interface NotificationCenterSettings {
  enabled: boolean;
  desktop: boolean;
  categories: Record<string, boolean>;
}

export interface PushPlatformField {
  key: string;
  label: string;
  required: boolean;
  secret?: boolean;
  placeholder?: string;
}

export interface PushPlatform {
  id: string;
  name: string;
  icon: string;
  note: string;
  fields: PushPlatformField[];
}

export interface PushChannel {
  id: string;
  name: string;
  platform: string;
  settings: Record<string, string>;
  events: string[];
  enabled: boolean;
  createdAt?: string;
  etag?: string;
}

export interface PushRecord {
  id: string;
  channelId: string;
  channelName: string;
  platform: string;
  event: string;
  title: string;
  status: string;
  error?: string | null;
  createdAt: string;
}

export interface PushTestResult {
  ok: boolean;
  platform: string;
  name: string;
  latencyMs: number;
  detail: string;
}

export interface SearchPlatform {
  engine: string;
  enabled: boolean;
  keyRef: string;
  catalog: SearchEngine & { icon: string; docs: string; fields: { key: string; label: string; required: boolean; placeholder?: string }[] };
}

/** 一种协议下的一个方法或命令。 */
export interface ProtocolItem {
  name: string;
  usage: string;
  summary: string;
}

/** HTTP 接口条目：路径、用途与调用要点。 */
export interface ApiDocItem {
  method: string;
  path: string;
  summary: string;
  query: string | null;
  body: string | null;
}

export interface ApiDocGroup {
  id: string;
  label: string;
  note: string;
  items: ApiDocItem[];
}

export interface ExternalProtocol {
  id: string;
  name: string;
  status: "enabled" | "internal" | "disabled" | string;
  note: string;
  command: string | null;
  endpoint: string | null;
  examples: string[];
  methods: number;
  groups: ApiDocGroup[];
  items: ProtocolItem[];
}

export interface ExternalHeader {
  name: string;
  optional: boolean;
  note: string;
}

export interface ExternalConventions {
  headers: ExternalHeader[];
  errors: string;
  paging: string;
  async: string;
  etag: string;
}

export interface ExternalStep {
  title: string;
  body: string;
  commands: string[];
}

export interface ExternalCatalog {
  generatedAt: string;
  baseUrl: string;
  conventions: ExternalConventions;
  quickStart: ExternalStep[];
  protocols: ExternalProtocol[];
  cli: ProtocolItem[];
}

export interface MonitorEvent {
  seq?: number;
  kind?: string;
  phase?: string;
  instance?: string;
  method?: string;
  ability_id?: string;
  started_at?: string;
  ended_at?: string;
  error?: string | null;
  [key: string]: unknown;
}

export interface MonitorChain {
  chain_id?: string;
  instance?: string;
  method?: string;
  running?: boolean;
  started_at?: string;
  span_id?: string;
  ability_id?: string;
  [key: string]: unknown;
}

export interface MonitorSummary {
  available: boolean;
  note?: string;
  generatedAt?: string;
  stats?: { dropped: number; queued: number; recent: number };
  chains?: MonitorChain[];
  events?: MonitorEvent[];
}
