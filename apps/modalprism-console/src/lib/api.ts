import type {
  ApiDocGroup,
  CliCandidate,
  CollectionRun,
  ContentDetail,
  ContentFacetBody,
  ContentFacetSummary,
  ContentItem,
  DeliveryChannel,
  DeliveryEndpoint,
  DeliveryRecord,
  DeliveryRule,
  DeliveryTarget,
  DeliveryTargetGroup,
  EffectiveSettings,
  ExternalCatalog,
  FetchResult,
  HealthHistory,
  ListResponse,
  LogRecord,
  ModelResource,
  ModelTestResult,
  MonitorSummary,
  NamedResource,
  NotificationCenterSettings,
  NotificationEntry,
  Operation,
  Overview,
  Plan,
  PreconditionReport,
  Project,
  ProjectResource,
  PromptResource,
  PushChannel,
  PushPlatform,
  PushRecord,
  PushTestResult,
  ResetPreview,
  SearchEngine,
  SearchHit,
  SearchPlatform,
  SearchResult,
  SettingsHistoryEntry,
  SettingsSchema,
  Snapshot,
  Source,
  SourceGroup,
  SourcePayload,
  SourceTypeRecord,
  SystemStatus,
  Task,
  WizardDoc,
} from "./types";

export class ApiError extends Error {
  code: string;
  fieldErrors: { field: string; code: string; message: string }[];
  requestId: string;
  retryable: boolean;
  status: number;

  constructor(
    message: string,
    status: number,
    code = "internalError",
    requestId = "",
    retryable = false,
    fieldErrors: { field: string; code: string; message: string }[] = [],
  ) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
    this.requestId = requestId;
    this.retryable = retryable;
    this.fieldErrors = fieldErrors;
  }
}

interface CallOptions {
  etag?: string;
  idempotencyKey?: string;
  noIdempotency?: boolean;
  /** 指定项目上下文，用于跨项目只读预览（项目页各卡片统计）；缺省跟随顶栏当前项目。 */
  projectId?: string | null;
}

export interface Meta<T> {
  data: T;
  etag: string | null;
}

function key(): string {
  return crypto.randomUUID();
}

async function call<T>(method: string, path: string, body?: unknown, options: CallOptions = {}): Promise<T> {
  const { data } = await callWithMeta<T>(method, path, body, options);
  return data;
}

async function callWithMeta<T>(method: string, path: string, body?: unknown, options: CallOptions = {}): Promise<Meta<T>> {
  const headers: Record<string, string> = { accept: "application/json" };
  const project = options.projectId ?? window.localStorage.getItem("modalprism.currentProjectId"); // 与 PROJECT_STORAGE_KEY 一致（避免与 project.tsx 循环引用）
  if (project) headers["x-project-id"] = project;
  let payload: BodyInit | undefined;
  if (body instanceof FormData) {
    payload = body;
  } else if (body !== undefined) {
    headers["content-type"] = "application/json";
    payload = JSON.stringify(body);
  }
  if (!options.noIdempotency && method !== "GET") {
    headers["idempotency-key"] = options.idempotencyKey ?? key();
  }
  if (options.etag) {
    headers["if-match"] = options.etag;
  }
  let response: Response;
  try {
    // no-store：响应随当前项目头变化，避免浏览器按 URL 复用上一个项目的缓存。
    response = await fetch(`/api/v1${path}`, { method, headers, body: payload, cache: "no-store" });
  } catch {
    throw new ApiError("Node 控制台没有响应", 0, "dependencyUnavailable", "", true);
  }
  const text = await response.text();
  let parsed: unknown = null;
  try {
    parsed = text ? JSON.parse(text) : null;
  } catch {
    parsed = null;
  }
  if (!response.ok) {
    const error = (parsed as { error?: { code?: string; message?: string; requestId?: string; retryable?: boolean; fieldErrors?: { field: string; code: string; message: string }[] } } | null)?.error;
    throw new ApiError(
      error?.message ?? `请求失败（${response.status}）`,
      response.status,
      error?.code ?? "internalError",
      error?.requestId ?? response.headers.get("x-request-id") ?? "",
      error?.retryable ?? false,
      error?.fieldErrors ?? [],
    );
  }
  return { data: parsed as T, etag: response.headers.get("etag") };
}

function query(values: Record<string, string | number | null | undefined>): string {
  const params = new URLSearchParams();
  for (const [name, value] of Object.entries(values)) {
    if (value !== undefined && value !== null && `${value}` !== "") params.set(name, `${value}`);
  }
  const text = params.toString();
  return text ? `?${text}` : "";
}

export async function waitOperation(operationId: string, onUpdate?: (operation: Operation) => void): Promise<Operation> {
  const started = Date.now();
  for (;;) {
    const operation = await call<Operation>("GET", `/operations/${operationId}`);
    onUpdate?.(operation);
    if (["succeeded", "partiallySucceeded", "failed", "cancelled"].includes(operation.status)) {
      return operation;
    }
    if (Date.now() - started > 10 * 60 * 1000) {
      throw new ApiError("操作等待超时", 0, "internalError", "", true);
    }
    await new Promise((resolve) => setTimeout(resolve, 600));
  }
}

export async function runOperation(
  operation: Operation,
  onUpdate?: (operation: Operation) => void,
): Promise<Operation> {
  const finished = await waitOperation(operation.operationId, onUpdate);
  if (finished.status === "failed") {
    throw new ApiError(
      finished.errorSummary?.message ?? "操作失败",
      0,
      finished.errorSummary?.code ?? "internalError",
      finished.errorSummary?.requestId ?? "",
      finished.retryable,
    );
  }
  return finished;
}

// system
export const systemStatus = () => call<SystemStatus>("GET", "/system/status");
export const systemOverview = (projectId?: string) =>
  call<Overview>("GET", "/system/overview", undefined, { projectId });

// catalog
export const listSources = (
  params: { limit?: number; cursor?: string; query?: string; status?: string; scope?: string; projectId?: string } = {},
) => {
  const { projectId, ...rest } = params;
  return call<ListResponse<Source>>("GET", `/catalog/sources${query(rest)}`, undefined, { projectId });
};
export const getSource = (id: string) => callWithMeta<Source>("GET", `/catalog/sources/${encodeURIComponent(id)}`);
export const createSource = (source: SourcePayload) => call<Source>("POST", "/catalog/sources", source);
export const updateSource = (id: string, source: SourcePayload, etag: string) =>
  call<Source>("PATCH", `/catalog/sources/${encodeURIComponent(id)}`, source, { etag });
export const deleteSource = (id: string, etag: string) =>
  call<Operation>("DELETE", `/catalog/sources/${encodeURIComponent(id)}`, undefined, { etag });
export const enableSource = (id: string, etag: string) =>
  call<Source>("POST", `/catalog/sources/${encodeURIComponent(id)}/enable`, {}, { etag });
export const disableSource = (id: string, etag: string) =>
  call<Source>("POST", `/catalog/sources/${encodeURIComponent(id)}/disable`, {}, { etag });
export const probeSource = (id: string, locator?: string) =>
  call<Operation>("POST", `/catalog/sources/${encodeURIComponent(id)}/probe`, { locator, persistResult: false });
export const shareSource = (id: string) =>
  call<Source>("POST", `/catalog/sources/${encodeURIComponent(id)}/share`, {}, { noIdempotency: true });
export const unshareSource = (id: string) =>
  call<Source>("POST", `/catalog/sources/${encodeURIComponent(id)}/unshare`, {}, { noIdempotency: true });
export const listAccessMethods = () => call<ListResponse<NamedResource>>("GET", "/catalog/accessMethods?limit=200");
export const listSourceTypes = () => call<ListResponse<SourceTypeRecord>>("GET", "/catalog/sourceTypes?limit=200");
export const listSourceGroups = (params: { scope?: string } = {}) =>
  call<ListResponse<SourceGroup>>("GET", `/catalog/sourceGroups${query(params)}`);
export const createSourceGroup = (input: { name: string; description?: string }) =>
  call<SourceGroup>("POST", "/catalog/sourceGroups", input);
export const updateSourceGroup = (id: string, patch: { name?: string; description?: string }) =>
  call<SourceGroup>("PATCH", `/catalog/sourceGroups/${encodeURIComponent(id)}`, patch);
export const deleteSourceGroup = (id: string) =>
  call<null>("DELETE", `/catalog/sourceGroups/${encodeURIComponent(id)}`);
export const assignSourcesToGroup = (input: { groupId: string | null; sourceIds: string[] }) =>
  call<{ updated: string[]; skipped: { id: string; reason: string }[] }>("POST", "/catalog/sourceGroups/bindings", input);
export const listAdapters = () => call<ListResponse<NamedResource>>("GET", "/catalog/adapters?limit=200");
export const listTemplates = () => call<ListResponse<NamedResource>>("GET", "/catalog/templates?limit=200");
export const analyzeSource = (input: { url: string; instruction?: string; modelId?: string }) =>
  call<Operation>("POST", "/catalog/sourceAnalyses", input);
export const listAdapterDrafts = () => call<ListResponse<NamedResource>>("GET", "/catalog/adapterDrafts?limit=200");
export const createAdapterDraft = (input: { sourceAnalysisId: string; name: string; instruction?: string }) =>
  call<Operation>("POST", "/catalog/adapterDrafts", input);
export const approveAdapterDraft = (id: string, etag: string, comment: string) =>
  call<Operation>("POST", `/catalog/adapterDrafts/${encodeURIComponent(id)}/approve`, { validationId: id, comment }, { etag });
export const rejectAdapterDraft = (id: string, etag: string, comment: string) =>
  call<NamedResource>("POST", `/catalog/adapterDrafts/${encodeURIComponent(id)}/reject`, { decision: "reject", comment }, { etag });

// collection
export const listPlans = (params: { limit?: number; cursor?: string; status?: string; projectId?: string } = {}) => {
  const { projectId, ...rest } = params;
  return call<ListResponse<Plan>>("GET", `/collection/plans${query(rest)}`, undefined, { projectId });
};
export const createPlan = (input: Record<string, unknown>) =>
  call<Plan>("POST", "/collection/plans", input);
export const deletePlan = (id: string) =>
  call<{ id: string; deleted: boolean }>("DELETE", `/collection/plans/${encodeURIComponent(id)}`);
export const createCollectionJob = (input: { targets: string[]; name?: string; queue?: string; priority?: number }) =>
  call<{ planId: string; dispatched: unknown[]; failed: unknown[] }>("POST", "/collection/jobs", input);
export const updatePlan = (id: string, patch: Record<string, unknown>, etag: string) =>
  call<Plan>("PATCH", `/collection/plans/${encodeURIComponent(id)}`, patch, { etag });
export const pausePlan = (id: string, etag: string) =>
  call<Plan>("POST", `/collection/plans/${encodeURIComponent(id)}/pause`, {}, { etag });
export const resumePlan = (id: string, etag: string) =>
  call<Plan>("POST", `/collection/plans/${encodeURIComponent(id)}/resume`, {}, { etag });
export const createCollectionRun = (input: { sourceId: string; mode: string; locator?: string; reason?: string }) =>
  call<Operation>("POST", "/collection/runs", input);
export const listCollectionRuns = (
  params: { limit?: number; cursor?: string; status?: string; sourceId?: string; planId?: string; projectId?: string } = {},
) => {
  const { projectId, ...rest } = params;
  return call<ListResponse<CollectionRun>>("GET", `/collection/runs${query(rest)}`, undefined, { projectId });
};

// content
export const listContent = (
  params: { limit?: number; cursor?: string; offset?: number; query?: string; sourceId?: string; sourceType?: string; status?: string; projectId?: string } = {},
) => {
  const { projectId, ...rest } = params;
  return call<ListResponse<ContentItem>>("GET", `/content/items${query(rest)}`, undefined, { projectId });
};
export const getContent = (id: string) => callWithMeta<ContentDetail>("GET", `/content/items/${encodeURIComponent(id)}`);
export const patchContent = (id: string, patch: { tags?: string[]; title?: string }, etag: string) =>
  call<ContentItem>("PATCH", `/content/items/${encodeURIComponent(id)}`, patch, { etag });
export const deleteContent = (id: string, etag: string) =>
  call<Operation>("DELETE", `/content/items/${encodeURIComponent(id)}`, undefined, { etag });
export const batchContent = (input: { ids: string[]; action: string; tags?: string[] }) =>
  call<Operation>("POST", "/content/items/batch", input);
export const contentPurge = (input: { source?: string; before?: string; after?: string; dryRun?: boolean }) =>
  call<{ matched: number; deleted: number; truncated: boolean; dryRun: boolean }>("POST", "/content/items/purge", input);
export const listContentVersions = (id: string, params: { limit?: number; cursor?: string } = {}) =>
  call<ListResponse<Record<string, unknown>>>("GET", `/content/items/${encodeURIComponent(id)}/versions${query(params)}`);
export const contentLineage = (id: string) =>
  call<{ nodes: Record<string, unknown>[]; edges: Record<string, unknown>[] }>("GET", `/content/items/${encodeURIComponent(id)}/lineage`);
export const contentRelations = (id: string) =>
  call<ListResponse<Record<string, unknown>>>("GET", `/content/items/${encodeURIComponent(id)}/relations?limit=200`);
/** 该内容的全部数据面明细（kind/variant/version/mediaType/size）。 */
export const listContentFacets = (id: string) =>
  call<ListResponse<ContentFacetSummary>>("GET", `/content/items/${encodeURIComponent(id)}/facets`);
/** 读单个数据面：不给 variant 时返回该类型的默认（最新）一面。 */
export const getContentFacet = (id: string, kind: string, variant?: string | null) => {
  if (!variant) {
    return call<ContentFacetBody>("GET", `/content/items/${encodeURIComponent(id)}/facets/${encodeURIComponent(kind)}`);
  }
  return call<ContentFacetBody>(
    "GET",
    `/content/items/${encodeURIComponent(id)}/facets/${encodeURIComponent(kind)}/${encodeURIComponent(variant)}`,
  );
};
/**
 * 附件字节地址（图片/音视频/文件）。浏览器原生媒体标签无法携带项目头，
 * 所以先按项目上下文取回字节，再生成对象地址；调用方负责在用完后 revoke。
 */
export async function contentFacetBlobUrl(id: string, kind: string, variant?: string | null): Promise<string> {
  const project = window.localStorage.getItem("modalprism.currentProjectId");
  const headers: Record<string, string> = {};
  if (project) headers["x-project-id"] = project;
  const suffix = variant ? `/${encodeURIComponent(variant)}` : "";
  const path = `/api/v1/content/items/${encodeURIComponent(id)}/blobs/${encodeURIComponent(kind)}` + suffix;
  let response: Response;
  try {
    response = await fetch(path, { headers, cache: "no-store" });
  } catch {
    throw new ApiError("Node 控制台没有响应", 0, "dependencyUnavailable", "", true);
  }
  if (!response.ok) {
    throw new ApiError(response.status === 404 ? "这条内容没有该附件" : `读取附件失败（${response.status}）`, response.status);
  }
  return URL.createObjectURL(await response.blob());
}
export const listSnapshots = (params: { limit?: number; cursor?: string; offset?: number } = {}) =>
  call<ListResponse<Snapshot>>("GET", `/content/snapshots${query(params)}`);
export const replaySnapshot = (input: { snapshotId: string; scope: string; contentIds?: string[] }) =>
  call<Operation>("POST", "/content/replays", input);

// dictionary
export interface DictionaryDocument {
  version: string;
  domains: Record<string, Record<string, { label: string; tone?: string }>>;
}
/** 语义字典：内置条目 + `config/dictionary.yaml` 覆盖，启动拉一次。 */
export const getDictionary = () => call<DictionaryDocument>("GET", "/dictionary");

// settings
export const getSettingsSchema = () => call<SettingsSchema>("GET", "/settings/schema");
export const getEffectiveSettings = () => callWithMeta<EffectiveSettings>("GET", "/settings/effective");
export const validateSettings = (changes: Record<string, unknown>) =>
  call<Record<string, unknown>>("POST", "/settings/validate", { changes });
export const diffSettings = (changes: Record<string, unknown>) =>
  call<Record<string, unknown>>("POST", "/settings/diff", { changes });
export const applySettings = (changes: Record<string, unknown>, reason: string, etag: string) =>
  call<Operation>("POST", "/settings/apply", { changes, reason }, { etag });
export const settingsHistory = (params: { limit?: number; cursor?: string } = {}) =>
  call<ListResponse<SettingsHistoryEntry>>("GET", `/settings/history${query(params)}`);
export const rollbackSettings = (targetGeneration: number, reason: string, etag: string) =>
  call<Operation>("POST", "/settings/rollback", { targetGeneration, reason }, { etag });
export const listModels = () => call<ListResponse<ModelResource>>("GET", "/settings/models?limit=200");
export const createModel = (input: Record<string, unknown>) =>
  call<ModelResource>("POST", "/settings/models", input);
export const updateModel = (id: string, input: Record<string, unknown>) =>
  call<ModelResource>("PATCH", `/settings/models/${encodeURIComponent(id)}`, input);
export const deleteModel = (id: string) =>
  call<{ id: string; deleted: boolean }>("DELETE", `/settings/models/${encodeURIComponent(id)}`);
export const testModel = (input: Record<string, unknown>) =>
  call<ModelTestResult>("POST", "/settings/models/test", input);
export const discoverModels = (input: {
  kind: string;
  endpoint: string;
  secretRef?: string | null;
  secretValue?: string | null;
  headers?: Record<string, string>;
}) => call<{ items: string[]; count: number }>("POST", "/settings/models/discover", input);
export const discoverClis = () =>
  call<{ items: CliCandidate[] }>("GET", "/settings/models/cliCandidates");
export const listPrompts = () => call<ListResponse<PromptResource>>("GET", "/settings/prompts?limit=200");
export const createPrompt = (input: { name: string; content: string; description?: string }) =>
  call<PromptResource>("POST", "/settings/prompts", input);
export const createPromptVersion = (id: string, etag: string, input: { name?: string; content: string; description?: string }) =>
  call<Record<string, unknown>>("POST", `/settings/prompts/${encodeURIComponent(id)}/versions`, input, { etag });
export const listSkills = () => call<ListResponse<PromptResource>>("GET", "/settings/skills?limit=200");
export const createSkill = (input: { name: string; content: string; description?: string }) =>
  call<PromptResource>("POST", "/settings/skills", input);
export const createSkillVersion = (id: string, etag: string, input: { name?: string; content: string; description?: string }) =>
  call<Record<string, unknown>>("POST", `/settings/skills/${encodeURIComponent(id)}/versions`, input, { etag });

// delivery（内容投递）
export const listDeliveryTargets = () =>
  call<{ items: DeliveryTarget[]; groups: DeliveryTargetGroup[] }>("GET", "/delivery/targets");
export const listDeliveryEndpoints = () =>
  call<ListResponse<DeliveryEndpoint>>("GET", "/delivery/endpoints?limit=200");
export const createDeliveryEndpoint = (input: { name: string; url: string; tokenRef?: string; enabled?: boolean }) =>
  call<DeliveryEndpoint>("POST", "/delivery/endpoints", input);
export const updateDeliveryEndpoint = (
  id: string,
  input: { name: string; url: string; tokenRef?: string; enabled?: boolean },
  etag: string,
) => call<DeliveryEndpoint>("PATCH", `/delivery/endpoints/${encodeURIComponent(id)}`, input, { etag });
export const deleteDeliveryEndpoint = (id: string) =>
  call<{ id: string; deleted: boolean }>("DELETE", `/delivery/endpoints/${encodeURIComponent(id)}`);
export const listDeliveryRules = (params: { limit?: number; type?: string } = {}) =>
  call<ListResponse<DeliveryRule>>("GET", `/delivery/rules${query(params)}`);
export const createDeliveryRule = (input: {
  name: string;
  listener: { kind: string; id: string };
  targets: { kind: string; id: string }[];
  enabled?: boolean;
}) => call<DeliveryRule>("POST", "/delivery/rules", input);
export const updateDeliveryRule = (
  id: string,
  input: { name?: string; listener?: { kind: string; id: string }; targets?: { kind: string; id: string }[]; enabled?: boolean },
  etag: string,
) => call<DeliveryRule>("PATCH", `/delivery/rules/${encodeURIComponent(id)}`, input, { etag });
export const deleteDeliveryRule = (id: string) =>
  call<{ id: string; deleted: boolean }>("DELETE", `/delivery/rules/${encodeURIComponent(id)}`);
export const listDeliveryRecords = (params: { limit?: number; type?: string; status?: string } = {}) =>
  call<ListResponse<DeliveryRecord>>("GET", `/delivery/records${query(params)}`);
export const getDeliveryRecord = (id: string) =>
  call<DeliveryRecord>("GET", `/delivery/records/${encodeURIComponent(id)}`);
export const createDeliveryJob = (input: {
  contentIds: string[];
  targets: { kind: string; id: string }[];
  note?: string;
}) => call<Operation>("POST", "/delivery/jobs", input);
export const listDeliveryChannels = () =>
  call<ListResponse<DeliveryChannel>>("GET", "/delivery/channels?limit=200");
export const createDeliveryChannel = (input: {
  name: string;
  kind: "webhook" | "feishu";
  enabled: boolean;
  settings: Record<string, string>;
}) => call<DeliveryChannel>("POST", "/delivery/channels", input);
export const updateDeliveryChannel = (
  id: string,
  input: { name: string; kind: "webhook" | "feishu"; enabled: boolean; settings: Record<string, string> },
  etag: string,
) => call<DeliveryChannel>("PATCH", `/delivery/channels/${encodeURIComponent(id)}`, input, { etag });
export const deleteDeliveryChannel = (id: string, etag: string) =>
  call<{ id: string; deleted: boolean }>("DELETE", `/delivery/channels/${encodeURIComponent(id)}`, undefined, { etag });
export const testDeliveryChannel = (id: string) =>
  call<Operation>("POST", `/delivery/channels/${encodeURIComponent(id)}/test`, {});

// notifications（通知中心 + 通知推送）
export const listNotificationCenter = (params: { limit?: number; category?: string } = {}) =>
  call<{ items: NotificationEntry[]; unread: number; total: number; nextCursor: string | null }>(
    "GET",
    `/notifications/center${query(params)}`,
  );
export const markNotificationsRead = (input: { ids?: string[]; all?: boolean }) =>
  call<{ updated: number }>("POST", "/notifications/center", input);
export const getNotificationSettings = () =>
  call<NotificationCenterSettings>("GET", "/notifications/center/settings");
export const updateNotificationSettings = (input: {
  enabled?: boolean;
  desktop?: boolean;
  categories?: Record<string, boolean>;
}) => call<NotificationCenterSettings>("PATCH", "/notifications/center/settings", input);
export const getNotificationCatalog = () =>
  call<{ categories: { id: string; label: string }[] }>("GET", "/notifications/center/catalog");
export const listPushPlatforms = () =>
  call<{ items: PushPlatform[]; events: { id: string; label: string }[] }>("GET", "/notifications/push/platforms");
export const listPushChannels = () =>
  call<ListResponse<PushChannel>>("GET", "/notifications/push/channels?limit=200");
export const createPushChannel = (input: {
  name: string;
  platform: string;
  settings: Record<string, string>;
  events: string[];
  enabled: boolean;
}) => call<PushChannel>("POST", "/notifications/push/channels", input);
export const updatePushChannel = (
  id: string,
  input: { name: string; platform: string; settings: Record<string, string>; events: string[]; enabled: boolean },
  etag: string,
) => call<PushChannel>("PATCH", `/notifications/push/channels/${encodeURIComponent(id)}`, input, { etag });
export const deletePushChannel = (id: string) =>
  call<{ id: string; deleted: boolean }>("DELETE", `/notifications/push/channels/${encodeURIComponent(id)}`);
export const testPush = (input: { channelId?: string; platform?: string; settings?: Record<string, string> }) =>
  call<PushTestResult>("POST", "/notifications/push/test", input);
export const listPushRecords = (params: { limit?: number; status?: string; channelId?: string } = {}) =>
  call<ListResponse<PushRecord>>("GET", `/notifications/push/records${query(params)}`);

// external（外部调用）+ monitor
export const getExternalProtocols = () => call<ExternalCatalog>("GET", "/external/protocols");
export const getApiDocs = () => call<{ total: number; groups: ApiDocGroup[] }>("GET", "/external/apiDocs");
export const monitorSummary = () => call<MonitorSummary>("GET", "/monitor/summary");
export const listSearchPlatforms = () =>
  call<{ items: SearchPlatform[] }>("GET", "/search/platforms");
export const saveSearchPlatforms = (items: { engine: string; enabled: boolean; keyRef: string }[]) =>
  call<{ items: SearchPlatform[] }>("PUT", "/search/platforms", { items });
export const testSearchEngine = (input: { engine: string; keyRef?: string }) =>
  call<{ ok: boolean; engine: string; latencyMs: number; count: number; sample: SearchHit | null }>(
    "POST",
    "/search/engines/test",
    input,
  );

// operations
export const listOperations = (params: { limit?: number; cursor?: string; status?: string } = {}) =>
  call<ListResponse<Operation>>("GET", `/operations${query(params)}`);
export const getOperation = (id: string) => call<Operation>("GET", `/operations/${encodeURIComponent(id)}`);
export const operationEvents = (id: string, params: { limit?: number; cursor?: string } = {}) =>
  call<ListResponse<Record<string, unknown>>>("GET", `/operations/${encodeURIComponent(id)}/events${query(params)}`);
export const cancelOperation = (id: string) => call<Operation>("POST", `/operations/${encodeURIComponent(id)}/cancel`, {});
export const retryOperation = (id: string, reason?: string) =>
  call<Operation>("POST", `/operations/${encodeURIComponent(id)}/retry`, { reason });
export const listTasks = (params: { limit?: number; cursor?: string; status?: string; kind?: string; sourceId?: string } = {}) =>
  call<ListResponse<Task>>("GET", `/operations/tasks${query(params)}`);
export const batchTasks = (ids: string[], action: string) =>
  call<Operation>("POST", "/operations/tasks/batch", { ids, action });
export const runTask = (taskId: string) =>
  call<Operation>("POST", `/operations/tasks/${encodeURIComponent(taskId)}/run`);
export const purgeTasks = (input: { status?: string; kind?: string; sourceId?: string; before?: string; dryRun?: boolean }) =>
  call<{ matched: number; deleted: number; dryRun: boolean }>("POST", "/operations/tasks/purge", input);
export const listQueues = () => call<ListResponse<Record<string, unknown>>>("GET", "/operations/queues");
export const pauseQueue = (queueId: string) =>
  call<{ id: string; paused: boolean }>("POST", `/operations/queues/${encodeURIComponent(queueId)}/pause`);
export const resumeQueue = (queueId: string) =>
  call<{ id: string; paused: boolean }>("POST", `/operations/queues/${encodeURIComponent(queueId)}/resume`);
export const runDerivations = () => call<Operation>("POST", "/operations/derivations/run");
export const getHealth = () => call<Record<string, unknown>>("GET", "/operations/health");
export const listHealthHistory = (params: { limit?: number; cursor?: string; offset?: number; sourceId?: string } = {}) =>
  call<ListResponse<HealthHistory>>("GET", `/operations/healthHistory${query(params)}`);
export const recomputeHealth = () => call<Operation>("POST", "/operations/healthRecomputations", {});

// observability
export const listLogs = (params: Record<string, string | number | null | undefined> = {}) =>
  call<ListResponse<LogRecord>>("GET", `/observability/logs${query(params)}`);
export const listEvents = (params: Record<string, string | number | null | undefined> = {}) =>
  call<ListResponse<LogRecord>>("GET", `/observability/events${query(params)}`);
export const listAuditEvents = (params: Record<string, string | number | null | undefined> = {}) =>
  call<ListResponse<LogRecord>>("GET", `/observability/auditEvents${query(params)}`);
export const exportObservability = (stream: string, format: string, filters: Record<string, unknown>) =>
  call<Operation>("POST", "/observability/exports", { stream, format, filters });

// maintenance
export const previewReset = (domains: string[]) =>
  callWithMeta<ResetPreview>("POST", "/maintenance/resetPreviews", { domains });
export const createReset = (preview: ResetPreview, token: string) =>
  call<Operation>("POST", "/maintenance/resets", { previewId: preview.id, confirmationToken: token }, { etag: preview.etag });
export const previewFactoryReset = () =>
  callWithMeta<ResetPreview>("POST", "/maintenance/factoryResetPreviews", {});
export const createFactoryReset = (preview: ResetPreview, token: string) =>
  call<Operation>("POST", "/maintenance/factoryResets", { previewId: preview.id, confirmationToken: token }, { etag: preview.etag });
export const getIntegrityReport = (id: string) =>
  call<Record<string, unknown>>("GET", `/maintenance/integrityReports/${encodeURIComponent(id)}`);

// projects
export const listProjects = () => call<ListResponse<Project>>("GET", "/projects");
export const createProject = (input: { name: string; description?: string }) =>
  call<Project>("POST", "/projects", input);
export const listProjectResources = (id: string, kind?: string) => {
  const params = { kind };
  return call<ListResponse<ProjectResource>>(
    "GET",
    `/projects/${encodeURIComponent(id)}/resources${query(params)}`,
  );
};
export const bindProjectResource = (id: string, kind: string, resourceId: string) =>
  call<void>("POST", `/projects/${encodeURIComponent(id)}/resources`, { kind, id: resourceId }, { noIdempotency: true });
export const unbindProjectResource = (id: string, kind: string, resourceId: string) =>
  call<void>("DELETE", `/projects/${encodeURIComponent(id)}/resources/${encodeURIComponent(kind)}/${encodeURIComponent(resourceId)}`, undefined, { noIdempotency: true });

// wizard（只读起步清单，实时计算，不再阻塞任何操作）
export const getWizard = (id: string) => call<WizardDoc>("GET", `/projects/${encodeURIComponent(id)}/wizard`);

// search / surface
export const listSearchEngines = () => call<ListResponse<SearchEngine>>("GET", "/search/engines");
export const runSearch = (input: { query: string; engine?: string; projectId?: string; limit?: number }) =>
  call<SearchResult>("POST", "/search", input, { noIdempotency: true });
export const fetchUrl = (url: string, maxBytes?: number) =>
  call<FetchResult>("POST", "/fetch", { url, maxBytes }, { noIdempotency: true });
export const parseInput = (input: { text?: string; url?: string; selectors?: string[] }) =>
  call<Record<string, unknown>>("POST", "/parse", input, { noIdempotency: true });
export const getPreconditions = (scope?: string) => {
  const params = { scope };
  return call<PreconditionReport>("GET", `/preconditions${query(params)}`);
};
