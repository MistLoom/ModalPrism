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
}

export interface SystemStatus {
  status: "online" | "degraded" | "maintenance";
  version: string;
  generation: number;
  startedAt: string;
}

export interface Queue {
  id: string;
  queued: number;
  running: number;
  done?: number;
  cancelled?: number;
  capacity: number;
  paused: boolean;
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
    failedPublications: number;
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
  accessMethodId: string;
  adapterIds: string[];
  endpoint: string;
  schedule: Schedule;
  health?: string | null;
  healthReason?: string | null;
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
  endpoint: string;
  schedule: Schedule;
  admission?: Record<string, unknown>;
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
  sourceId: string;
  name: string;
  enabled: boolean;
  status: string;
  schedule: Schedule;
  health?: string | null;
  nextRunAt?: string | null;
  lastRunAt?: string | null;
  todayYield?: number | null;
  etag: string;
}

export interface Task {
  id: string;
  kind: string;
  status: string;
  title?: string;
  sourceId?: string | null;
  queue?: string;
  reason?: string;
  attempt?: number;
  createdAt: string;
  updatedAt?: string | null;
  retryAt?: string | null;
  error?: string | null;
  locator?: unknown;
  runId?: string;
  publicationId?: string;
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

export interface Snapshot {
  id: string;
  snapshot: number;
  publishedAt: string;
  commitCount: number;
}

export interface InputSet {
  id: string;
  name: string;
  inputs: { kind: string; title?: string; text?: string | null; contentId?: string; [key: string]: unknown }[];
  createdAt: string;
}

export interface Workflow {
  id: string;
  name: string;
  description: string;
  latestVersionId: string | null;
  versionCount: number;
  etag: string;
  createdAt: string;
}

export interface WorkflowStep {
  id: string;
  kind: "model" | "template" | "externalCall" | "validation";
  modelId?: string;
  promptVersionId?: string;
  skillVersionIds?: string[];
  dependsOn?: string[];
  parameters?: Record<string, unknown>;
}

export interface WorkflowVersion {
  id: string;
  workflowId: string;
  version: number;
  steps: WorkflowStep[];
  createdAt: string;
}

export interface ProductionRun {
  id: string;
  inputSetId: string;
  workflowVersionId: string;
  workflowId: string;
  mode: string;
  status: string;
  attemptIds: string[];
  candidateIds: string[];
  taskId: string;
  createdAt: string;
  updatedAt: string;
}

export interface Attempt {
  id: string;
  runId: string;
  number: number;
  status: string;
  steps: { id: string; kind: string; status: string; ms: number; output?: string; error?: string }[];
  retryOfAttemptId?: string | null;
  createdAt: string;
  finishedAt?: string | null;
  error?: string | null;
}

export interface Candidate {
  id: string;
  runId: string;
  attemptId: string;
  status: string;
  title: string;
  body: string;
  outputs: Record<string, string>;
  validation: { issues: { code: string; message: string }[] };
  selectedOutputIds: string[];
  etag?: string;
  createdAt: string;
}

export interface Draft {
  id: string;
  candidateIds: string[];
  title: string;
  body: string;
  latestVersion: number;
  versions: { version: number; title: string; body: string; createdAt: string }[];
  etag?: string;
  createdAt: string;
  updatedAt: string;
}

export interface Channel {
  id: string;
  name: string;
  kind: string;
  enabled: boolean;
  settings: Record<string, unknown>;
  etag?: string;
  createdAt: string;
}

export interface Publication {
  id: string;
  draftId: string;
  title: string;
  body: string;
  channelIds: string[];
  channels: { id: string; name: string; kind: string }[];
  status: string;
  deliveryIds: string[];
  scheduledAt?: string | null;
  createdAt: string;
  updatedAt: string;
}

export interface Delivery {
  id: string;
  publicationId: string;
  channelId: string;
  channelName?: string;
  status: string;
  attempts: { at: string; status: string; error?: string }[];
  lastError?: string | null;
  retryOf?: string | null;
  createdAt: string;
}

export interface ModelResource {
  id: string;
  name: string;
  kind: "remoteApi" | "localService" | "localCli";
  endpoint: string;
  secretRef: string | null;
  secretPresent?: boolean;
  enabled: boolean;
  model?: string;
  timeoutMs?: number;
  maxTokens?: number;
  temperature?: number;
  createdAt: string;
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
  workflowVersionId?: string | null;
  publicationId?: string | null;
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
  legacy?: boolean;
}

export interface WizardStep {
  id: string;
  title?: string;
  status: "pending" | "done" | "skipped";
  updatedAt?: string;
}

export interface WizardDoc {
  projectId: string;
  templateId: string;
  status?: "active" | "completed" | "dismissed";
  steps: WizardStep[];
  createdAt?: string;
  updatedAt?: string;
}

export interface WizardCheck {
  wizard: WizardDoc;
  stepId: string;
  met: boolean;
  detail: { reason?: string; count?: number; where?: string };
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
  query: string;
  count: number;
  results: SearchHit[];
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
